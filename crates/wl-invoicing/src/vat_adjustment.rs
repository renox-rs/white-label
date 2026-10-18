//! The VAT adjustment (wl-invoicing §3): a correction carried into a VAT
//! period. A bad-debt write-off creates one of kind
//! [`AdjustmentKind::BadDebtRelief`] as [`AdjustmentStatus::Pending`]
//! (§5.9 step 3); `FIN.RUL.14` proposes it later. Amounts are AED fils.
//!
//! An invoice has at most one bad-debt relief: the table has a unique index
//! on (kind, source) limited to `bad_debt_relief`, so the other kinds (a
//! recovery adjustment per payment, for one) may repeat per source.
//!
//! The lifecycle (a Renox state machine; the status changes only through it):
//!
//! ```text
//! [*] --> pending --propose--> proposed --include--> included
//!                                       \--exclude--> excluded
//! ```
//!
//! `exclude_recovered` is the system's too: a `pending` or `proposed`
//! relief whose written-off invoice was recovered in full is excluded with
//! the reason "recovered before claim" (#66).
//!
//! `propose` is the system's (`FIN.RUL.14`): it puts the adjustment on a
//! VAT summary. `include` and `exclude` are the preparer's (`vat.prepare`),
//! each with a reason, and only while the adjustment's summary is `open` or
//! `prepared`. The machine's guards cannot read the database, so the
//! caller passes the summary's status in the run's input ([`include`] and
//! [`exclude`] do) and the effect checks it again inside the transaction.

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::invoice::Invoice;
use crate::permissions;
use crate::texts::english;
use crate::vat_summary::{VatSummary, VatSummaryStatus};

/// The transitions' names.
pub const PROPOSE: &str = "propose";
/// See [`PROPOSE`].
pub const INCLUDE: &str = "include";
/// See [`PROPOSE`].
pub const EXCLUDE: &str = "exclude";
/// The system's exclusion of a relief recovered before it was claimed.
pub const EXCLUDE_RECOVERED: &str = "exclude_recovered";

/// What an adjustment corrects (§3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentKind {
    /// A document that arrived after its period was locked.
    LateDocument,
    /// A credit note for a supply of an earlier period.
    EarlierPeriodCreditNote,
    /// Output VAT given back on a written-off invoice.
    #[default]
    BadDebtRelief,
    /// Relief paid back when a written-off invoice is paid after all.
    BadDebtRecovery,
    /// A debit note to a supplier.
    SupplierDebitNote,
    /// Entered by hand.
    Manual,
}

/// Where an adjustment stands (§3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentStatus {
    /// Waiting until its conditions are met (bad-debt relief).
    #[default]
    Pending,
    /// On a VAT summary, for the preparer to include or exclude.
    Proposed,
    /// Included in the summary.
    Included,
    /// Excluded, with a reason.
    Excluded,
}

/// A VAT adjustment. The amount is in AED fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_vat_adjustments",
    state_machine,
    index(source_type, source_id),
    index(status),
    index(summary_id)
)]
pub struct VatAdjustment {
    /// The primary key.
    pub id: i64,
    /// The VAT summary it is carried into; `None` while pending.
    pub summary_id: Option<i64>,
    /// The issuing legal entity of the source document.
    pub legal_entity_id: Option<i64>,
    /// What it corrects.
    pub kind: AdjustmentKind,
    /// The type of the source document, e.g. `invoicing.invoice`.
    pub source_type: String,
    /// The id of the source document.
    pub source_id: i64,
    /// The VAT amount, in AED fils.
    pub amount: i64,
    /// Why, when given.
    pub reason: Option<String>,
    /// Where it stands; changed only by a transition.
    pub status: AdjustmentStatus,
    /// When it was proposed.
    pub proposed_at: Option<DateTime>,
    /// Who included or excluded it.
    pub decided_by: Option<i64>,
    /// When it was included or excluded.
    pub decided_at: Option<DateTime>,
    /// The reason given with the decision.
    pub decision_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl VatAdjustment {
    /// The pending bad-debt relief of `amount` AED fils (the VAT share of
    /// the written-off amount) for `invoice`, not yet stored.
    pub fn bad_debt_relief(invoice: &Invoice, amount: i64) -> Self {
        let source = invoice.subject();
        VatAdjustment {
            legal_entity_id: invoice.legal_entity_id,
            kind: AdjustmentKind::BadDebtRelief,
            source_type: source.subject_type,
            source_id: source.subject_id,
            amount,
            status: AdjustmentStatus::Pending,
            ..Default::default()
        }
    }

    /// The source document.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// The bad-debt relief of `invoice`, if one was created.
    pub async fn relief_of(db: &Db, invoice: &Invoice) -> Result<Option<VatAdjustment>> {
        let source = invoice.subject();
        VatAdjustment::query()
            .where_(VatAdjustment::KIND.eq(AdjustmentKind::BadDebtRelief))
            .where_(VatAdjustment::SOURCE_TYPE.eq(source.subject_type))
            .where_(VatAdjustment::SOURCE_ID.eq(source.subject_id))
            .first(db)
            .await
    }
}

/// What `propose` carries: the summary to put the adjustment on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposeInput {
    /// The summary of the issuing entity's running period.
    pub summary_id: i64,
}

/// What `include` and `exclude` carry: the status of the adjustment's
/// summary, as the caller read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionInput {
    /// The status of the summary the adjustment is on.
    pub summary_status: VatSummaryStatus,
}

/// Whether a decision can still be taken while the summary is `status`.
fn decidable(status: VatSummaryStatus) -> bool {
    matches!(status, VatSummaryStatus::Open | VatSummaryStatus::Prepared)
}

fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key).to_owned())
}

/// The id of the user running a transition.
fn actor_id(ctx: &TransitionCtx) -> Option<i64> {
    ctx.actor().map(|user| user.id)
}

/// The trimmed reason of a run, when it has one.
fn reason_of(ctx: &TransitionCtx) -> Option<String> {
    ctx.reason()
        .map(|r| r.trim().to_owned())
        .filter(|r| !r.is_empty())
}

/// Writes columns other than the status, in the transition's transaction.
async fn record(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    VatAdjustment::query()
        .where_(VatAdjustment::ID.eq(id))
        .update(&mut *tx, columns)
        .await?;
    Ok(())
}

fn propose() -> Transition<VatAdjustment> {
    Transition::new(PROPOSE)
        .label("Propose")
        .tone("info")
        .system()
        .from([AdjustmentStatus::Pending])
        .to(AdjustmentStatus::Proposed)
        .guard("summary_given", |_adjustment: &VatAdjustment, ctx| {
            Box::pin(async move {
                match ctx.input_as::<ProposeInput>() {
                    Ok(_) => Ok(()),
                    Err(_) => Err(english("wl_invoicing.adjustment.summary_required").to_owned()),
                }
            })
        })
        .effect(|adjustment: &VatAdjustment, ctx, tx| {
            let id = adjustment.id;
            let summary_id = ctx.input_as::<ProposeInput>().ok().map(|i| i.summary_id);
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("summary_id", &summary_id),
                        ("proposed_at", &renox::db::now()),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await
            })
        })
}

/// What `include` and `exclude` share: the reason, the summary-state guard
/// and the effect that records the decision.
fn decision(
    transition: Transition<VatAdjustment>,
    to: AdjustmentStatus,
) -> Transition<VatAdjustment> {
    transition
        .permission(permissions::VAT_PREPARE)
        .reason_required()
        .from([AdjustmentStatus::Proposed])
        .to(to)
        .guard("reason_given", |_adjustment: &VatAdjustment, ctx| {
            Box::pin(async move {
                if reason_of(ctx).is_some() {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.adjustment.reason_required").to_owned())
                }
            })
        })
        .guard("summary_decidable", |adjustment: &VatAdjustment, ctx| {
            let on_summary = adjustment.summary_id.is_some();
            Box::pin(async move {
                match ctx.input_as::<DecisionInput>() {
                    Ok(input) if on_summary && decidable(input.summary_status) => Ok(()),
                    _ => Err(english("wl_invoicing.adjustment.summary_closed").to_owned()),
                }
            })
        })
        .effect(|adjustment: &VatAdjustment, ctx, tx| {
            let (id, summary_id) = (adjustment.id, adjustment.summary_id);
            let (by, reason) = (actor_id(ctx), reason_of(ctx));
            Box::pin(async move {
                // The guard trusted the caller's reading; this one is the
                // database's, inside the transaction.
                let summary = match summary_id {
                    Some(summary_id) => {
                        VatSummary::query()
                            .where_(VatSummary::ID.eq(summary_id))
                            .first(&mut *tx)
                            .await?
                    }
                    None => None,
                };
                if !summary.is_some_and(|s| decidable(s.status)) {
                    return Err(refused("wl_invoicing.adjustment.summary_closed"));
                }
                record(
                    tx,
                    id,
                    &[
                        ("decided_by", &by),
                        ("decided_at", &renox::db::now()),
                        ("decision_reason", &reason),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await
            })
        })
}

/// `pending | proposed --> excluded`, by the system, when the written-off
/// invoice is recovered in full before its relief was claimed. It records
/// the reason; the relief amount is written by the recover service.
fn exclude_recovered_transition() -> Transition<VatAdjustment> {
    Transition::new(EXCLUDE_RECOVERED)
        .label("Exclude, recovered")
        .tone("warning")
        .system()
        .from([AdjustmentStatus::Pending, AdjustmentStatus::Proposed])
        .to(AdjustmentStatus::Excluded)
        .effect(|adjustment: &VatAdjustment, _ctx, tx| {
            let id = adjustment.id;
            Box::pin(async move {
                let reason = english("wl_invoicing.adjustment.recovered_before_claim");
                record(
                    tx,
                    id,
                    &[
                        ("reason", &Some(reason.to_owned())),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await
            })
        })
}

fn include_transition() -> Transition<VatAdjustment> {
    decision(
        Transition::new(INCLUDE).label("Include").tone("success"),
        AdjustmentStatus::Included,
    )
}

fn exclude_transition() -> Transition<VatAdjustment> {
    decision(
        Transition::new(EXCLUDE).label("Exclude").tone("warning"),
        AdjustmentStatus::Excluded,
    )
}

impl StateMachine for VatAdjustment {
    type State = AdjustmentStatus;
    const COLUMN: renox::db::Col<VatAdjustment, AdjustmentStatus> = VatAdjustment::STATUS;

    fn machine() -> Machine<Self> {
        use AdjustmentStatus::{Excluded, Included, Pending, Proposed};
        Machine::new()
            .initial([Pending])
            .state(Pending)
            .tone("secondary")
            .state(Proposed)
            .tone("info")
            .state(Included)
            .tone("success")
            .terminal()
            .state(Excluded)
            .tone("warning")
            .terminal()
            .transition(propose())
            .transition(exclude_recovered_transition())
            .transition(include_transition())
            .transition(exclude_transition())
    }
}

/// Runs a decision (`include` or `exclude`) by `user` with `reason`, in its
/// own transaction. The summary's status is read here and checked again in
/// the transaction.
///
/// # Errors
///
/// `Forbidden` without `vat.prepare`; `NotFrom` unless the adjustment is
/// `proposed`; `Guard` (422) without a reason or when its summary is
/// `reviewed` or `locked`. Nothing is written then.
async fn decide(
    state: &AppState,
    user: &AuthUser,
    adjustment: &VatAdjustment,
    transition: &str,
    reason: &str,
) -> std::result::Result<(), TransitionError> {
    let summary_status = match adjustment.summary_id {
        Some(id) => VatSummary::find(&state.db, id).await?.map(|s| s.status),
        None => None,
    };
    // A missing summary is not decidable; `Locked` stands for "closed".
    let input = DecisionInput {
        summary_status: summary_status.unwrap_or(VatSummaryStatus::Locked),
    };
    adjustment
        .transition(transition)
        .by(user)
        .reason(reason)
        .input(&input)
        .run(state)
        .await?
        .emit(state)
        .await
        .map_err(TransitionError::Failed)
}

/// `user` includes the proposed `adjustment` in its summary, with `reason`.
/// See [`decide`] for the errors.
pub async fn include(
    state: &AppState,
    user: &AuthUser,
    adjustment: &VatAdjustment,
    reason: &str,
) -> std::result::Result<(), TransitionError> {
    decide(state, user, adjustment, INCLUDE, reason).await
}

/// `user` excludes the proposed `adjustment` from its summary, with
/// `reason`. See [`decide`] for the errors.
pub async fn exclude(
    state: &AppState,
    user: &AuthUser,
    adjustment: &VatAdjustment,
    reason: &str,
) -> std::result::Result<(), TransitionError> {
    decide(state, user, adjustment, EXCLUDE, reason).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_statuses_are_stored_in_snake_case() {
        let kinds: Vec<_> = AdjustmentKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "late_document",
                "earlier_period_credit_note",
                "bad_debt_relief",
                "bad_debt_recovery",
                "supplier_debit_note",
                "manual"
            ]
        );
        let statuses: Vec<_> = AdjustmentStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(statuses, ["pending", "proposed", "included", "excluded"]);
    }

    #[test]
    fn a_relief_is_pending_for_the_invoice_and_its_entity() {
        let invoice = Invoice {
            id: 42,
            legal_entity_id: Some(3),
            ..Default::default()
        };
        let relief = VatAdjustment::bad_debt_relief(&invoice, 5_000);
        assert_eq!(relief.kind, AdjustmentKind::BadDebtRelief);
        assert_eq!(relief.status, AdjustmentStatus::Pending);
        assert_eq!(relief.amount, 5_000);
        assert_eq!(relief.legal_entity_id, Some(3));
        assert_eq!(relief.summary_id, None);
        assert_eq!(relief.source().to_string(), "invoicing.invoice:42");
    }

    fn proposed(summary_id: Option<i64>) -> VatAdjustment {
        VatAdjustment {
            id: 1,
            summary_id,
            status: AdjustmentStatus::Proposed,
            ..Default::default()
        }
    }

    fn decision_ctx(reason: Option<&str>, status: VatSummaryStatus) -> TransitionCtx {
        let ctx = TransitionCtx::new(None).with_input(
            renox::serde_json::to_value(DecisionInput {
                summary_status: status,
            })
            .expect("json"),
        );
        match reason {
            Some(reason) => ctx.with_reason(reason),
            None => ctx,
        }
    }

    #[test]
    fn the_machine_has_propose_include_and_exclude() {
        let machine = VatAdjustment::machine();
        assert_eq!(machine.initial_states(), ["pending"]);
        let find = |name: &str| machine.find(name).expect("transition");
        assert_eq!(find(PROPOSE).from_states(), ["pending"]);
        assert_eq!(find(PROPOSE).to_state(), Some("proposed"));
        assert!(find(PROPOSE).is_system());
        for (name, to) in [(INCLUDE, "included"), (EXCLUDE, "excluded")] {
            assert_eq!(find(name).from_states(), ["proposed"]);
            assert_eq!(find(name).to_state(), Some(to));
            assert!(find(name).needs_reason());
            assert!(!find(name).is_system());
        }
    }

    #[test]
    fn exclude_recovered_is_the_systems_from_pending_or_proposed() {
        let machine = VatAdjustment::machine();
        let transition = machine.find(EXCLUDE_RECOVERED).expect("transition");
        assert_eq!(transition.from_states(), ["pending", "proposed"]);
        assert_eq!(transition.to_state(), Some("excluded"));
        assert!(transition.is_system());
        assert!(!transition.needs_reason());
    }

    #[renox::test]
    async fn a_decision_needs_a_reason() {
        let machine = VatAdjustment::machine();
        let adjustment = proposed(Some(7));
        for name in [INCLUDE, EXCLUDE] {
            let transition = machine.find(name).expect("transition");
            for reason in [None, Some("  ")] {
                let ctx = decision_ctx(reason, VatSummaryStatus::Open);
                assert!(matches!(
                    transition.check_guards(&adjustment, &ctx).await,
                    Err(TransitionError::Guard(_))
                ));
            }
            let ctx = decision_ctx(Some("Customer paid"), VatSummaryStatus::Open);
            assert!(transition.check_guards(&adjustment, &ctx).await.is_ok());
        }
    }

    #[renox::test]
    async fn a_decision_waits_for_an_open_or_prepared_summary() {
        let machine = VatAdjustment::machine();
        let transition = machine.find(INCLUDE).expect("transition");
        let adjustment = proposed(Some(7));
        for (status, allowed) in [
            (VatSummaryStatus::Open, true),
            (VatSummaryStatus::Prepared, true),
            (VatSummaryStatus::Reviewed, false),
            (VatSummaryStatus::Locked, false),
        ] {
            let ctx = decision_ctx(Some("ok"), status);
            assert_eq!(
                transition.check_guards(&adjustment, &ctx).await.is_ok(),
                allowed,
                "{status:?}"
            );
        }
        // No summary, or no input: refused.
        let ctx = decision_ctx(Some("ok"), VatSummaryStatus::Open);
        assert!(
            transition
                .check_guards(&proposed(None), &ctx)
                .await
                .is_err()
        );
        let bare = TransitionCtx::new(None).with_reason("ok");
        assert!(transition.check_guards(&adjustment, &bare).await.is_err());
    }

    #[renox::test]
    async fn propose_needs_its_summary() {
        let machine = VatAdjustment::machine();
        let transition = machine.find(PROPOSE).expect("transition");
        let adjustment = VatAdjustment::default();
        let bare = TransitionCtx::new(None);
        assert!(transition.check_guards(&adjustment, &bare).await.is_err());
        let ctx = TransitionCtx::new(None)
            .with_input(renox::serde_json::to_value(ProposeInput { summary_id: 3 }).expect("json"));
        assert!(transition.check_guards(&adjustment, &ctx).await.is_ok());
    }
}
