//! A call on a guarantee and its settlement (`call`, `settle_call`, 02
//! Construction §4.6 row "The client calls a guarantee", §7.1 *Guarantee*,
//! §9 "Termination and guarantees"; BL-CON-10).
//!
//! When the beneficiary demands payment under a guarantee, the accountant
//! (`guarantees.manage`) records the **call** with the demand notice and the
//! amount demanded ([`record`]); the guarantee is `called`. The MD, the
//! project manager and the accountant are told at once (an urgent notice: in-app
//! and by e-mail, outside quiet hours and digests), and the accountant gets a
//! task to respond to the call.
//!
//! ```text
//! active --call--> called
//! called --settle_call--> active           (partial call, or withdrawn before payment)
//! called --settle_call_full--> released    (the whole guarantee was paid)
//! ```
//!
//! [`settle`] takes the bank's debit advice and the amount paid; a call the
//! beneficiary withdrew before payment is settled with amount paid 0 and the
//! withdrawal letter in place of the debit advice, and the guarantee is
//! `active` again with its full amount, nothing booked. A partial call leaves
//! it `active` with the rest and the same expiry.
//!
//! - The amount paid is booked to the project's called-guarantees cost code
//!   ([`COST_CODE`]) after the commit of the settlement, once, keyed by the
//!   call, and **never before the bank's debit advice** ([`book_cost`]).
//! - The project manager (`projects.manage`) may mark a call **disputed**
//!   ([`dispute`]); the paid amount is then listed in the project's final
//!   account claims ([`claims_of`]).
//! - A `called` guarantee does not expire (`expire` runs from `active`
//!   only); one back to `active` after its expiry date expires on the next
//!   daily run of [`crate::expiry`], and no extension task is made while it is
//!   `called`.
//! - The platform records the call; it does not judge or block it (§9).

use renox::db::{DateTime, Db, Transaction};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::{PostCost, post_cost};
use wl_kernel::tickets::task_creator;

use crate::expiry::{kind_text, managers};
use crate::guarantee::{
    CALL, Guarantee, GuaranteeInput, GuaranteeStatus, SETTLE_CALL, SETTLE_CALL_FULL,
};
use crate::notices::{aed, holders, tell_urgent};
use crate::permissions;
use crate::project::Project;
use crate::tasks::{self, Task};
use crate::texts::english;

/// The subject type of a call on a guarantee.
pub const SUBJECT_TYPE: &str = "construction.guarantee_call";

/// The project's cost code the paid amount is booked to.
pub const COST_CODE: &str = "called_guarantees";

/// A call on a guarantee. Money is fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "construction_guarantee_calls", index(guarantee_id))]
pub struct GuaranteeCall {
    /// The primary key.
    pub id: i64,
    /// The guarantee.
    pub guarantee_id: i64,
    /// The project.
    pub project_id: i64,
    /// The beneficiary's demand notice, a document reference.
    pub demand_notice: String,
    /// The amount demanded.
    pub amount: i64,
    /// The user who recorded the call.
    pub called_by: Option<i64>,
    /// The bank's debit advice, or the withdrawal letter when nothing was
    /// paid.
    pub settlement_document: Option<String>,
    /// The amount the bank paid; `None` while the call is open.
    pub amount_paid: Option<i64>,
    /// Whether the project manager disputes it: the paid amount is a claim
    /// in the final account.
    pub disputed: bool,
    /// When the call was settled.
    pub settled_at: Option<DateTime>,
    /// When the paid amount was booked to the called-guarantees cost code.
    pub booked_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl GuaranteeCall {
    /// The call as a [`Subject`]: `construction.guarantee_call:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// Whether the call is settled.
    pub fn is_settled(&self) -> bool {
        self.settled_at.is_some()
    }

    /// Whether the call was withdrawn before payment: settled with nothing
    /// paid.
    pub fn is_withdrawn(&self) -> bool {
        self.is_settled() && self.amount_paid == Some(0)
    }

    /// The key of the task to respond to the call.
    pub fn task_key(&self) -> String {
        format!("construction.guarantee_call:{}", self.id)
    }
}

/// A 422 with `message`.
fn refused(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// The effect of `call` inside its transaction: the call is kept.
pub(crate) async fn open_in(
    tx: &mut Transaction,
    guarantee: &Guarantee,
    input: &GuaranteeInput,
    by: Option<i64>,
) -> Result {
    let mut call = GuaranteeCall {
        guarantee_id: guarantee.id,
        project_id: guarantee.project_id,
        demand_notice: input.document.trim().to_owned(),
        amount: input.amount.unwrap_or_default(),
        called_by: by,
        ..GuaranteeCall::default()
    };
    call.insert(&mut *tx).await?;
    Ok(())
}

/// The effect of `settle_call` inside its transaction: the call is closed with
/// the bank's document and the amount paid, and a partial call takes the
/// amount paid off the guarantee. A call withdrawn before payment (0 paid)
/// changes no amount, and a paid-in-full guarantee keeps its amount: it is
/// `released`.
pub(crate) async fn settle_in(
    tx: &mut Transaction,
    guarantee: &Guarantee,
    input: &GuaranteeInput,
) -> Result {
    let (Some(call_id), Some(paid)) = (input.call_id, input.amount) else {
        return Err(refused(english("construction.bad_input", &[])));
    };
    let document = Some(input.document.trim().to_owned());
    let paid_now = Some(paid);
    let at = Some(renox::db::now());
    GuaranteeCall::query()
        .where_(GuaranteeCall::ID.eq(call_id))
        .update(
            &mut *tx,
            &[
                ("settlement_document", &document),
                ("amount_paid", &paid_now),
                ("settled_at", &at),
            ],
        )
        .await?;
    if paid > 0 && paid < guarantee.amount {
        let rest = guarantee.amount - paid;
        Guarantee::query()
            .where_(Guarantee::ID.eq(guarantee.id))
            .update(&mut *tx, &[("amount", &rest)])
            .await?;
    }
    Ok(())
}

/// The open call of guarantee `id`, if it has one.
///
/// # Errors
///
/// A database error.
pub async fn open_call_of(db: &Db, id: i64) -> Result<Option<GuaranteeCall>> {
    GuaranteeCall::query()
        .where_(GuaranteeCall::GUARANTEE_ID.eq(id))
        .where_(GuaranteeCall::SETTLED_AT.is_null())
        .order_by_desc(GuaranteeCall::ID)
        .first(db)
        .await
}

/// The calls on guarantee `id`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn calls_of(db: &Db, id: i64) -> Result<Vec<GuaranteeCall>> {
    GuaranteeCall::query()
        .where_(GuaranteeCall::GUARANTEE_ID.eq(id))
        .order_by(GuaranteeCall::ID)
        .get(db)
        .await
}

/// The disputed calls of `project_id` that the bank paid: the claims the
/// final account carries, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn claims_of(db: &Db, project_id: i64) -> Result<Vec<GuaranteeCall>> {
    let disputed = GuaranteeCall::query()
        .where_(GuaranteeCall::PROJECT_ID.eq(project_id))
        .where_(GuaranteeCall::DISPUTED.eq(true))
        .order_by(GuaranteeCall::ID)
        .get(db)
        .await?;
    Ok(disputed
        .into_iter()
        .filter(|call| call.amount_paid.is_some_and(|paid| paid > 0))
        .collect())
}

/// `user` (the accountant) records the beneficiary's call on the `active`
/// guarantee `id` for `amount` fils, with the demand notice.
///
/// # Errors
///
/// `Forbidden` without `guarantees.manage` in the project, `NotFrom` unless it
/// is `active`, `Guard` (422) without the demand notice, without an amount
/// above 0 or with an amount above the guarantee's. Nothing changes then.
pub async fn record(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    amount: i64,
    demand_notice: &str,
) -> std::result::Result<Guarantee, TransitionError> {
    let guarantee = Guarantee::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let input = GuaranteeInput {
        document: demand_notice.to_owned(),
        amount: Some(amount),
        ..GuaranteeInput::default()
    };
    let _outcome = guarantee
        .transition(CALL)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(Guarantee::find_or_404(&state.db, id).await?)
}

/// `user` (the accountant) settles the open call on the `called` guarantee
/// `id`: the bank paid `paid` fils, shown by its debit advice `document`; or
/// the beneficiary withdrew the call before payment (`paid` 0) and
/// `document` is its withdrawal letter. A partial or withdrawn call leaves the
/// guarantee `active`, a call paid in full makes it `released`.
///
/// # Errors
///
/// `Forbidden` without `guarantees.manage` in the project, `NotFrom` unless it
/// is `called`, `Guard` (422) without the document, with an amount below 0 or
/// above the amount called. Nothing changes then.
pub async fn settle(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    paid: i64,
    document: &str,
) -> std::result::Result<Guarantee, TransitionError> {
    let guarantee = Guarantee::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let call = open_call_of(&state.db, id).await?;
    let input = GuaranteeInput {
        document: document.to_owned(),
        amount: Some(paid),
        called: call.as_ref().map(|call| call.amount),
        call_id: call.as_ref().map(|call| call.id),
        ..GuaranteeInput::default()
    };
    let name = if paid >= guarantee.amount {
        SETTLE_CALL_FULL
    } else {
        SETTLE_CALL
    };
    let _outcome = guarantee
        .transition(name)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(Guarantee::find_or_404(&state.db, id).await?)
}

/// `user` (the project manager) marks call `id` disputed: its paid amount is
/// listed in the project's final account claims. Marking it twice changes
/// nothing.
///
/// # Errors
///
/// `Forbidden` without `projects.manage` in the project.
pub async fn dispute(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<GuaranteeCall, TransitionError> {
    let call = GuaranteeCall::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let project = Project::find_or_404(&state.db, call.project_id).await?;
    if !user.has_permission_in(permissions::PROJECTS_MANAGE, &project.scope()) {
        return Err(TransitionError::Forbidden);
    }
    GuaranteeCall::query()
        .where_(GuaranteeCall::ID.eq(id))
        .update(&state.db, &[("disputed", &true)])
        .await?;
    Ok(GuaranteeCall::find_or_404(&state.db, id).await?)
}

/// Books the paid amount of the settled call `id` to the project's
/// called-guarantees cost code, once. Returns whether it booked. A call
/// withdrawn before payment books nothing, and an open call has no debit
/// advice yet.
///
/// # Errors
///
/// A database error and the cost poster's errors; a call not booked can be
/// booked again, the key keeps it from being booked twice.
pub async fn book_cost(state: &AppState, id: i64) -> Result<bool> {
    let call = GuaranteeCall::find_or_404(&state.db, id).await?;
    let paid = call.amount_paid.unwrap_or_default();
    if !call.is_settled() || paid <= 0 || call.booked_at.is_some() {
        return Ok(false);
    }
    let project = Project::find_or_404(&state.db, call.project_id).await?;
    let request = PostCost::new(
        call.subject(),
        format!("{SUBJECT_TYPE}:{}", call.id),
        project.subject(),
        paid,
    )
    .cost_code(COST_CODE);
    post_cost(state, request).await?;
    let at = Some(renox::db::now());
    GuaranteeCall::query()
        .where_(GuaranteeCall::ID.eq(call.id))
        .update(&state.db, &[("booked_at", &at)])
        .await?;
    Ok(true)
}

/// Tells the MD, the project manager and the accountant at once that
/// guarantee `id` was called, and gives the accountant a task to respond.
async fn announce_call(state: &AppState, id: i64) -> Result {
    let guarantee = Guarantee::find_or_404(&state.db, id).await?;
    let Some(call) = open_call_of(&state.db, id).await? else {
        return Ok(());
    };
    let project = Project::find_or_404(&state.db, guarantee.project_id).await?;
    let kind = kind_text(&guarantee);
    let amount = aed(call.amount);
    let title = english(
        "construction.guarantee.called_title",
        &[
            ("kind", &kind),
            ("project", &project.name),
            ("amount", &amount),
        ],
    );
    let body = english(
        "construction.guarantee.called_body",
        &[
            ("bank", &guarantee.bank),
            ("beneficiary", &guarantee.beneficiary),
            ("amount", &amount),
            ("notice", &call.demand_notice),
        ],
    );
    let mut users = holders(state, &project, permissions::MD_ROLE).await?;
    users.extend(managers(state, &project).await?);
    users.extend(holders(state, &project, permissions::ACCOUNTANT_ROLE).await?);
    users.sort_unstable();
    users.dedup();
    tell_urgent(
        state,
        guarantee.subject(),
        &project.code,
        &users,
        &title,
        &body,
    )
    .await?;
    tasks::open(
        state,
        &project,
        Task {
            subject: guarantee.subject(),
            role: permissions::ACCOUNTANT_ROLE,
            assignee: None,
            source_key: call.task_key(),
            title: english(
                "construction.guarantee.respond_title",
                &[("kind", &kind), ("project", &project.name)],
            ),
            description: english(
                "construction.guarantee.respond_body",
                &[("amount", &amount), ("notice", &call.demand_notice)],
            ),
            due: None,
        },
    )
    .await?;
    Ok(())
}

/// After a settlement: the paid amount is booked to the cost code and the
/// task to respond is closed.
async fn after_settlement(state: &AppState, id: i64) -> Result {
    let latest = GuaranteeCall::query()
        .where_(GuaranteeCall::GUARANTEE_ID.eq(id))
        .where_(GuaranteeCall::SETTLED_AT.is_not_null())
        .order_by_desc(GuaranteeCall::ID)
        .first(&state.db)
        .await?;
    let Some(call) = latest else {
        return Ok(());
    };
    let _booked = book_cost(state, call.id).await?;
    let creator = task_creator(state);
    if let Some(ticket) = creator.find_open(state, &call.task_key()).await? {
        let reason = english("construction.guarantee.respond_done", &[]);
        let _closed = creator
            .cancel_from_source(state, ticket.id, &reason)
            .await?;
    }
    Ok(())
}

/// What follows the commit of a `call` and of a `settle_call`.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(
        |event: Transitioned<Guarantee>, state: AppState| async move {
            let name = event.transition.as_str();
            if name == CALL && event.to == GuaranteeStatus::Called.as_str() {
                announce_call(&state, event.key).await?;
            }
            if name == SETTLE_CALL || name == SETTLE_CALL_FULL {
                after_settlement(&state, event.key).await?;
            }
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guarantee(amount: i64) -> Guarantee {
        Guarantee {
            amount,
            ..Guarantee::new(1, crate::guarantee::GuaranteeKind::Performance, amount)
        }
    }

    fn call_input(document: &str, amount: Option<i64>) -> GuaranteeInput {
        GuaranteeInput {
            document: document.into(),
            amount,
            ..GuaranteeInput::default()
        }
    }

    fn settle_input(document: &str, paid: i64) -> GuaranteeInput {
        GuaranteeInput {
            document: document.into(),
            amount: Some(paid),
            called: Some(20_000_000),
            call_id: Some(1),
            ..GuaranteeInput::default()
        }
    }

    #[test]
    fn a_call_needs_the_demand_notice_and_an_amount_within_the_guarantee() {
        use crate::guarantee::call_problem;
        let bond = guarantee(50_000_000);
        assert_eq!(
            call_problem(&bond, &call_input("DEMAND-1.pdf", Some(20_000_000))),
            None
        );
        assert_eq!(
            call_problem(&bond, &call_input("DEMAND-1.pdf", Some(50_000_000))),
            None
        );
        assert!(
            call_problem(&bond, &call_input(" ", Some(1))).is_some_and(|p| p.contains("demand"))
        );
        assert!(call_problem(&bond, &call_input("D.pdf", None)).is_some());
        assert!(call_problem(&bond, &call_input("D.pdf", Some(0))).is_some());
        assert!(
            call_problem(&bond, &call_input("D.pdf", Some(50_000_001)))
                .is_some_and(|p| p.contains("AED 500,000.00"))
        );
    }

    #[test]
    fn a_settlement_needs_the_debit_advice_or_for_nothing_paid_the_withdrawal_letter() {
        use crate::guarantee::settle_partial_problem;
        let bond = guarantee(50_000_000);
        assert_eq!(
            settle_partial_problem(&bond, &settle_input("DEBIT-1.pdf", 20_000_000)),
            None
        );
        assert!(
            settle_partial_problem(&bond, &settle_input("", 20_000_000))
                .is_some_and(|p| p.contains("debit advice"))
        );
        assert_eq!(
            settle_partial_problem(&bond, &settle_input("WITHDRAWN.pdf", 0)),
            None
        );
        assert!(
            settle_partial_problem(&bond, &settle_input("", 0))
                .is_some_and(|p| p.contains("withdrawal letter"))
        );
        // More than the call, a negative amount, or no open call.
        assert!(settle_partial_problem(&bond, &settle_input("D.pdf", 20_000_001)).is_some());
        assert!(settle_partial_problem(&bond, &settle_input("D.pdf", -1)).is_some());
        let mut none = settle_input("D.pdf", 1);
        none.called = None;
        assert!(settle_partial_problem(&bond, &none).is_some());
    }

    #[test]
    fn only_the_whole_amount_paid_releases_the_guarantee() {
        use crate::guarantee::{settle_full_problem, settle_partial_problem};
        let bond = guarantee(20_000_000);
        let full = settle_input("DEBIT-1.pdf", 20_000_000);
        assert_eq!(settle_full_problem(&bond, &full), None);
        assert!(settle_partial_problem(&bond, &full).is_some());
        let part = settle_input("DEBIT-1.pdf", 10_000_000);
        assert!(settle_full_problem(&bond, &part).is_some());
        assert_eq!(settle_partial_problem(&bond, &part), None);
    }

    #[test]
    fn a_call_is_open_settled_or_withdrawn() {
        let mut call = GuaranteeCall::default();
        assert!(!call.is_settled());
        call.settled_at = Some(renox::db::now());
        call.amount_paid = Some(0);
        assert!(call.is_withdrawn());
        call.amount_paid = Some(1);
        assert!(!call.is_withdrawn());
        call.id = 3;
        assert_eq!(call.task_key(), "construction.guarantee_call:3");
        assert_eq!(call.subject().to_string(), "construction.guarantee_call:3");
    }
}
