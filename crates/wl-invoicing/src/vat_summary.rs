//! The VAT summary (wl-invoicing §3, §4): what a legal entity reports to the
//! FTA for one tax period ([`crate::vat_period`]). Its lifecycle:
//!
//! ```text
//! [*] --> open --prepare--> prepared --review--> reviewed --lock--> locked
//!                           prepared <--send_back-- reviewed
//! ```
//!
//! `prepare` needs the period to be over; `review` is by someone other than
//! the preparer (`FIN.ACL.10`); `send_back` needs a reason. The figures,
//! exports and `reopen` come with the issues that build `FIN.VAT.01`. The
//! status changes only through the machine. [`ensure_open`] gives each
//! running period its summary; the daily task [`TASK`] calls it for every
//! active, VAT-registered entity.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;

use crate::config::InvoicingConfig;
use crate::permissions;
use crate::texts::english;
use crate::vat_period;

/// The transitions' names.
pub const PREPARE: &str = "prepare";
/// See [`PREPARE`].
pub const REVIEW: &str = "review";
/// See [`PREPARE`].
pub const SEND_BACK: &str = "send_back";
/// See [`PREPARE`].
pub const LOCK: &str = "lock";

/// The daily scheduled task that runs [`run`].
pub const TASK: &str = "invoicing:vat-periods";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "00:05";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// Where a VAT summary stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VatSummaryStatus {
    /// Its period is running, or over and not yet prepared.
    #[default]
    Open,
    /// Prepared, waiting for the review.
    Prepared,
    /// Reviewed by a second person.
    Reviewed,
    /// Filed; the period is closed.
    Locked,
}

/// The VAT summary of one legal entity for one tax period.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_vat_summaries",
    state_machine,
    unique(legal_entity_id, period_start),
    index(status)
)]
pub struct VatSummary {
    /// The primary key.
    pub id: i64,
    /// The legal entity it is for.
    pub legal_entity_id: i64,
    /// The first day of the period.
    pub period_start: NaiveDate,
    /// The last day of the period.
    pub period_end: NaiveDate,
    /// The lifecycle state; changed only by a transition.
    pub status: VatSummaryStatus,
    /// Who prepared it.
    pub prepared_by: Option<i64>,
    /// When it was prepared.
    pub prepared_at: Option<DateTime>,
    /// Who reviewed it.
    pub reviewed_by: Option<i64>,
    /// When it was reviewed.
    pub reviewed_at: Option<DateTime>,
    /// Who locked it.
    pub locked_by: Option<i64>,
    /// When it was locked.
    pub locked_at: Option<DateTime>,
    /// Why the reviewer sent it back, when it was.
    pub send_back_reason: Option<String>,
    /// The filing reference given at `lock`.
    pub filing_reference: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What a `lock` run may carry: the filing reference.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockInput {
    /// The FTA filing reference, if known.
    pub filing_reference: Option<String>,
}

impl VatSummary {
    /// The summary of `entity_id` whose period starts on `period_start`.
    pub async fn for_period(
        db: &Db,
        entity_id: i64,
        period_start: NaiveDate,
    ) -> Result<Option<VatSummary>> {
        VatSummary::query()
            .where_(VatSummary::LEGAL_ENTITY_ID.eq(entity_id))
            .where_(VatSummary::PERIOD_START.eq(period_start))
            .first(db)
            .await
    }

    /// The summary of `entity_id` whose period contains `date`, given the
    /// entity's tax period setting.
    pub async fn containing(
        db: &Db,
        config: &InvoicingConfig,
        entity: &LegalEntity,
        date: NaiveDate,
    ) -> Result<Option<VatSummary>> {
        let (start, _) = vat_period::containing(config.tax_period_of(&entity.code), date);
        Self::for_period(db, entity.id, start).await
    }
}

/// Creates the summary of the period of `entity` that contains `today`,
/// `open`, unless that period already has one (in whatever state). Returns
/// the new summary, or `None` when there was one.
pub async fn ensure_open(
    db: &Db,
    config: &InvoicingConfig,
    entity: &LegalEntity,
    today: NaiveDate,
) -> Result<Option<VatSummary>> {
    let (start, end) = vat_period::containing(config.tax_period_of(&entity.code), today);
    if VatSummary::for_period(db, entity.id, start)
        .await?
        .is_some()
    {
        return Ok(None);
    }
    let mut summary = VatSummary {
        legal_entity_id: entity.id,
        period_start: start,
        period_end: end,
        status: VatSummaryStatus::Open,
        ..Default::default()
    };
    summary.insert(db).await?;
    Ok(Some(summary))
}

/// Runs [`ensure_open`] for every active, VAT-registered legal entity.
/// Returns how many summaries it created.
pub async fn run(state: &AppState, today: NaiveDate) -> Result<usize> {
    let config = crate::config_of(state);
    let entities = LegalEntity::query()
        .where_(LegalEntity::VAT_REGISTERED.eq(true))
        .order_by(LegalEntity::ID)
        .get(&state.db)
        .await?;
    let mut created = 0;
    for entity in entities.iter().filter(|e| e.is_active()) {
        created += usize::from(
            ensure_open(&state.db, &config, entity, today)
                .await?
                .is_some(),
        );
    }
    Ok(created)
}

/// Registers the daily [`TASK`].
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
}

/// The id of the user running a transition.
fn actor_id(ctx: &TransitionCtx) -> Option<i64> {
    ctx.actor().map(|user| user.id)
}

/// Writes columns other than the status, in the transition's transaction.
async fn record(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    VatSummary::query()
        .where_(VatSummary::ID.eq(id))
        .update(&mut *tx, columns)
        .await?;
    Ok(())
}

fn prepare() -> Transition<VatSummary> {
    Transition::new(PREPARE)
        .label("Prepare")
        .tone("primary")
        .permission(permissions::VAT_PREPARE)
        .from([VatSummaryStatus::Open])
        .to(VatSummaryStatus::Prepared)
        .guard("period_over", |summary: &VatSummary, _ctx| {
            Box::pin(async move {
                if summary.period_end < today() {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.vat.period_running").to_owned())
                }
            })
        })
        .effect(|summary: &VatSummary, ctx, tx| {
            let id = summary.id;
            let by = actor_id(ctx);
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("prepared_by", &by),
                        ("prepared_at", &renox::db::now()),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await
            })
        })
}

fn review() -> Transition<VatSummary> {
    Transition::new(REVIEW)
        .label("Review")
        .tone("primary")
        .permission(permissions::VAT_REVIEW)
        .from([VatSummaryStatus::Prepared])
        .to(VatSummaryStatus::Reviewed)
        .guard("not_the_preparer", |summary: &VatSummary, ctx| {
            // `FIN.ACL.10`: a second person reviews.
            Box::pin(async move {
                match (actor_id(ctx), summary.prepared_by) {
                    (Some(reviewer), Some(preparer)) if reviewer != preparer => Ok(()),
                    (Some(_), Some(_)) => Err(english("wl_invoicing.vat.same_person").to_owned()),
                    _ => Err(english("wl_invoicing.vat.reviewer_unknown").to_owned()),
                }
            })
        })
        .effect(|summary: &VatSummary, ctx, tx| {
            let id = summary.id;
            let by = actor_id(ctx);
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("reviewed_by", &by),
                        ("reviewed_at", &renox::db::now()),
                        ("send_back_reason", &None::<String>),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await
            })
        })
}

fn send_back() -> Transition<VatSummary> {
    Transition::new(SEND_BACK)
        .label("Send back")
        .tone("warning")
        .permission(permissions::VAT_REVIEW)
        // `reason_required` makes the run refuse (422) without a reason. A
        // guard on the reason would hide the action from `available`, which
        // checks guards before the reason is asked for.
        .reason_required()
        .from([VatSummaryStatus::Reviewed])
        .to(VatSummaryStatus::Prepared)
        .effect(|summary: &VatSummary, ctx, tx| {
            let id = summary.id;
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("send_back_reason", &reason),
                        ("reviewed_by", &None::<i64>),
                        ("reviewed_at", &None::<DateTime>),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await
            })
        })
}

fn lock() -> Transition<VatSummary> {
    Transition::new(LOCK)
        .label("Lock")
        .tone("success")
        .permission(permissions::VAT_LOCK)
        .from([VatSummaryStatus::Reviewed])
        .to(VatSummaryStatus::Locked)
        .effect(|summary: &VatSummary, ctx, tx| {
            let id = summary.id;
            let by = actor_id(ctx);
            let reference = ctx
                .input_as::<LockInput>()
                .ok()
                .and_then(|input| input.filing_reference)
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty());
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("locked_by", &by),
                        ("locked_at", &renox::db::now()),
                        ("filing_reference", &reference),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await
            })
        })
}

impl StateMachine for VatSummary {
    type State = VatSummaryStatus;
    const COLUMN: renox::db::Col<VatSummary, VatSummaryStatus> = VatSummary::STATUS;

    fn machine() -> Machine<Self> {
        use VatSummaryStatus::{Locked, Open, Prepared, Reviewed};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("info")
            .state(Prepared)
            .tone("warning")
            .state(Reviewed)
            .tone("info")
            .state(Locked)
            .tone("success")
            .terminal()
            .transition(prepare())
            .transition(review())
            .transition(send_back())
            .transition(lock())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::TransitionError;

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let statuses: Vec<_> = VatSummaryStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(statuses, ["open", "prepared", "reviewed", "locked"]);
    }

    #[test]
    fn the_machine_has_the_four_transitions() {
        let machine = VatSummary::machine();
        assert_eq!(machine.initial_states(), ["open"]);
        let find = |name: &str| machine.find(name).expect("transition");
        assert_eq!(find(PREPARE).from_states(), ["open"]);
        assert_eq!(find(PREPARE).to_state(), Some("prepared"));
        assert_eq!(find(REVIEW).from_states(), ["prepared"]);
        assert_eq!(find(REVIEW).to_state(), Some("reviewed"));
        assert_eq!(find(SEND_BACK).from_states(), ["reviewed"]);
        assert_eq!(find(SEND_BACK).to_state(), Some("prepared"));
        assert!(find(SEND_BACK).needs_reason());
        assert_eq!(find(LOCK).from_states(), ["reviewed"]);
        assert_eq!(find(LOCK).to_state(), Some("locked"));
        assert!(machine.transitions().iter().all(|t| !t.is_system()));
    }

    fn summary(period_end: NaiveDate, prepared_by: Option<i64>) -> VatSummary {
        VatSummary {
            period_end,
            prepared_by,
            ..Default::default()
        }
    }

    #[renox::test]
    async fn prepare_waits_for_the_period_end() {
        let machine = VatSummary::machine();
        let transition = machine.find(PREPARE).expect("transition");
        let ctx = TransitionCtx::new(None);
        let yesterday = today().pred_opt().unwrap();
        assert!(
            transition
                .check_guards(&summary(yesterday, None), &ctx)
                .await
                .is_ok()
        );
        for end in [today(), today().succ_opt().unwrap()] {
            assert!(matches!(
                transition.check_guards(&summary(end, None), &ctx).await,
                Err(TransitionError::Guard(_))
            ));
        }
    }

    #[renox::test]
    async fn a_reviewer_who_is_unknown_is_refused() {
        let machine = VatSummary::machine();
        let transition = machine.find(REVIEW).expect("transition");
        let ctx = TransitionCtx::new(None);
        assert!(matches!(
            transition
                .check_guards(&summary(today(), Some(1)), &ctx)
                .await,
            Err(TransitionError::Guard(_))
        ));
    }
}
