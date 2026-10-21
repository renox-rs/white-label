//! The retention the client keeps (`deduct`, `CON.ACL.14`, 02 Construction
//! §4.6 row "The client keeps part or all of the retention", §7.1
//! *Retention*; BL-CON-08).
//!
//! The client may keep part or all of a retention part, for example for
//! defects it fixed itself. **Before the part is invoiced** the QS
//! (`retentions.manage`) records the deduction with the client's notice or
//! contra-charge reference and the reason ([`record`]). The project manager
//! (`ipas.deductions.decide`, never the user who recorded it) accepts or
//! contests it, and accepting a deduction above AED 25,000 is deferred as an
//! approval request for the MD, exactly as the deductions on a certificate
//! are ([`crate::deduction`]):
//!
//! ```text
//! undecided --accept--> accepted      (above AED 25,000: the MD approves first)
//! undecided --contest--> contested
//! ```
//!
//! - Only an **accepted** deduction is applied: in the transaction of the
//!   acceptance the part loses the amount (`deduct` on a `held` part,
//!   `deduct_due` on a `due` one) and is `kept` when nothing remains. A
//!   contested deduction changes nothing; it is listed with the final
//!   account items ([`contested_of`]).
//! - After the commit the amount is booked to the project's back-charge cost
//!   code through the kernel's cost contract ([`book_cost`]), once, keyed by
//!   the deduction. **No tax document** is issued: nothing was invoiced.
//! - On an `invoiced` part `deduct` is not offered ([`record`] refuses it):
//!   the user opens a dispute on the retention invoice instead
//!   ([`dispute_invoiced`]), whose resolution `credited` is the credit note
//!   of the agreed amount and its VAT (wl-invoicing, BL-INV-04).
//!
//! Decisions where the spec is silent:
//!
//! - The spec's `deduct` is by `retentions.manage`. That permission is the
//!   QS's, at [`record`]; the part's own transitions are run by the system at
//!   the acceptance, because the user who accepts is the project manager.
//! - The back-charge cost code is [`BACK_CHARGE_COST_CODE`]; the cost object
//!   is the project. Whether VAT is still due at the one-year limit on a kept
//!   part is `⚠ open: accountant (A-13)`: `CON.RUL.07` warns while any amount
//!   is `held` or `due`.

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::invoicing::{
    DisputeBy, DisputeRequest, DraftLine, PostCost, VatCategory, invoicer, post_cost,
};
use wl_kernel::{Subject, SubjectMachine};

use crate::deduction::{APPROVAL_LIMIT, DecideInput, DeductionStatus, decide_input};
use crate::notices::aed;
use crate::permissions;
use crate::project::{self, Project};
use crate::retention::{DEDUCT, DEDUCT_DUE, DeductInput, KEEP, Retention, RetentionStatus};
use crate::texts::english;

/// The subject type of a retention deduction.
pub const SUBJECT_TYPE: &str = "construction.retention_deduction";

/// The `accept` transition's name.
pub const ACCEPT: &str = "accept";
/// `accept_approved`: the kernel runs it at the final approval.
pub const ACCEPT_APPROVED: &str = "accept_approved";
/// The `contest` transition's name.
pub const CONTEST: &str = "contest";

/// The approval policies' action key of `accept` (`CON.ACL.14`).
pub const ACCEPT_ACTION: &str = "construction.retention_deduction_accept";

/// The cost code the amounts the client keeps are booked to.
pub const BACK_CHARGE_COST_CODE: &str = "back_charge";

/// What the client keeps of a retention part. Money is fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "construction_retention_deductions",
    state_machine,
    index(retention_id)
)]
pub struct RetentionDeduction {
    /// The primary key.
    pub id: i64,
    /// The project.
    pub project_id: i64,
    /// The retention part.
    pub retention_id: i64,
    /// The amount the client keeps.
    pub amount: i64,
    /// The client's notice or contra-charge reference.
    pub reference: String,
    /// Why the client keeps it.
    pub reason: String,
    /// The lifecycle state; changed only by a transition.
    pub status: DeductionStatus,
    /// The QS who recorded it.
    pub recorded_by: i64,
    /// The project manager who decided it.
    pub decided_by: Option<i64>,
    /// When it was decided.
    pub decided_at: Option<DateTime>,
    /// When the amount was booked to the back-charge cost code.
    pub booked_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RetentionDeduction {
    /// The deduction as a [`Subject`]: `construction.retention_deduction:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The scope roles are assigned in: the project of the deduction.
    pub fn scope(&self) -> Scope {
        Scope::new(project::SUBJECT_TYPE, self.project_id)
    }
}

/// A 422 with `message`.
fn refused(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// Why the deduction cannot be decided with `input`, if it cannot: the user
/// who recorded it never decides it (`CON.ACL.14`).
pub fn decide_problem(line: &RetentionDeduction, input: &DecideInput) -> Option<String> {
    match input.decided_by {
        None => Some(english("construction.bad_input", &[])),
        Some(by) if by == line.recorded_by => Some(english("construction.deduction.own_line", &[])),
        Some(_) => None,
    }
}

/// The effect of a decision inside its transaction: who decided and when.
async fn decided_effects(tx: &mut Transaction, id: i64, input: &DecideInput) -> Result {
    let by = input.decided_by;
    let at = Some(renox::db::now());
    RetentionDeduction::query()
        .where_(RetentionDeduction::ID.eq(id))
        .update(&mut *tx, &[("decided_by", &by), ("decided_at", &at)])
        .await?;
    Ok(())
}

/// The effect of an acceptance inside its transaction: the part loses the
/// amount, and is `kept` when nothing remains.
async fn apply_in(tx: &mut Transaction, line: &RetentionDeduction) -> Result {
    let part = Retention::find_or_404(&mut *tx, line.retention_id).await?;
    if !matches!(part.status, RetentionStatus::Held | RetentionStatus::Due) {
        return Err(refused(english(
            "construction.retention.deduct_finished",
            &[],
        )));
    }
    if line.amount > part.amount {
        return Err(refused(english(
            "construction.retention.deduction_too_big",
            &[("remaining", &aed(part.amount))],
        )));
    }
    let name = if line.amount == part.amount {
        KEEP
    } else if part.status == RetentionStatus::Due {
        DEDUCT_DUE
    } else {
        DEDUCT
    };
    let input = DeductInput {
        amount: line.amount,
    };
    // The part is not announced: nothing listens to it.
    let _outcome = part
        .transition(name)
        .by_system()
        .input(&input)
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// A user transition of the deduction: `permission` in the project's scope.
fn user(name: &str, label: &str) -> Transition<RetentionDeduction> {
    Transition::new(name).label(label).permission_in(
        permissions::IPAS_DEDUCTIONS_DECIDE,
        |line: &RetentionDeduction| line.scope(),
    )
}

/// The guard and the effect every decision shares.
fn deciding(
    transition: Transition<RetentionDeduction>,
    to: DeductionStatus,
) -> Transition<RetentionDeduction> {
    transition
        .from([DeductionStatus::Undecided])
        .to(to)
        .guard("separation", |line: &RetentionDeduction, ctx| {
            let problem = decide_problem(line, &decide_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|line: &RetentionDeduction, ctx, tx| {
            let id = line.id;
            let input = decide_input(ctx);
            Box::pin(async move { decided_effects(&mut *tx, id, &input).await })
        })
}

/// A decision that applies the deduction to the part.
fn accepting(transition: Transition<RetentionDeduction>) -> Transition<RetentionDeduction> {
    deciding(transition, DeductionStatus::Accepted).effect(|line: &RetentionDeduction, _ctx, tx| {
        let line = line.clone();
        Box::pin(async move { apply_in(&mut *tx, &line).await })
    })
}

impl StateMachine for RetentionDeduction {
    type State = DeductionStatus;
    const COLUMN: renox::db::Col<RetentionDeduction, DeductionStatus> = RetentionDeduction::STATUS;

    fn machine() -> Machine<Self> {
        use DeductionStatus::{Accepted, Contested, Undecided};
        Machine::new()
            .initial([Undecided])
            .state(Undecided)
            .tone("warning")
            .state(Accepted)
            .tone("success")
            .state(Contested)
            .tone("error")
            .transition(accepting(user(ACCEPT, "Accept").tone("primary")))
            .transition(accepting(
                Transition::new(ACCEPT_APPROVED)
                    .label("Accept (approved)")
                    .system(),
            ))
            .transition(deciding(user(CONTEST, "Contest").tone("danger"), Contested))
    }
}

/// The default approval policy of `accept` (`CON.ACL.14`): a deduction above
/// AED 25,000 needs one step by the MD (a holder of `projects.cancel`, the
/// company admin by default), never the project manager who asked.
pub fn accept_policy() -> PolicyDefault {
    PolicyDefault::new(ACCEPT_ACTION)
        .amount_above(APPROVAL_LIMIT)
        .step_or_permission(
            [wl_kernel::COMPANY_ADMIN_ROLE],
            permissions::PROJECTS_CANCEL,
        )
}

/// The deduction in the kernel's registry: its approval action for `accept`
/// and the project as the scope where approvers are looked for.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<RetentionDeduction>(SUBJECT_TYPE)
        .action(ACCEPT, ACCEPT_ACTION)
        .scope(|line: &RetentionDeduction| line.scope())
        .build()
}

/// Registers the machine with the kernel and with `rnx state:diagram`, and
/// the default policy of `accept`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<RetentionDeduction>();
    accept_policy().register();
}

/// Books an accepted deduction after its commit, wherever the acceptance came
/// from (the project manager, or the MD's approval).
pub(crate) fn listen(app: &mut Registry) {
    app.listen(
        |event: Transitioned<RetentionDeduction>, state: AppState| async move {
            let accepted = [ACCEPT, ACCEPT_APPROVED].contains(&event.transition.as_str())
                && event.to == DeductionStatus::Accepted.as_str();
            if accepted {
                let _booked = book_cost(&state, event.key).await?;
            }
            Ok(())
        },
    );
}

/// What the QS enters for one deduction.
#[derive(Debug, Clone)]
pub struct NewRetentionDeduction {
    /// The retention part.
    pub retention_id: i64,
    /// The amount the client keeps, in fils.
    pub amount: i64,
    /// The client's notice or contra-charge reference.
    pub reference: String,
    /// Why the client keeps it.
    pub reason: String,
}

/// Why the deduction cannot be recorded on `part`, if it cannot: the part is
/// `held` or `due` (not invoiced: the invoice is disputed instead), the
/// reference, the reason and the amount are given, and the amount does not
/// exceed what remains.
pub fn new_problem(part: &Retention, new: &NewRetentionDeduction) -> Option<String> {
    match part.status {
        RetentionStatus::Held | RetentionStatus::Due => {}
        RetentionStatus::Invoiced => {
            return Some(english("construction.retention.deduct_invoiced", &[]));
        }
        RetentionStatus::Settled | RetentionStatus::Kept => {
            return Some(english("construction.retention.deduct_finished", &[]));
        }
    }
    if new.reference.trim().is_empty() {
        return Some(english("construction.retention.deduction_reference", &[]));
    }
    if new.reason.trim().is_empty() {
        return Some(english("construction.retention.deduction_reason", &[]));
    }
    if new.amount <= 0 {
        return Some(english("construction.retention.deduction_amount", &[]));
    }
    if new.amount > part.amount {
        return Some(english(
            "construction.retention.deduction_too_big",
            &[("remaining", &aed(part.amount))],
        ));
    }
    None
}

/// `user` (the QS) records what the client keeps of the retention part.
///
/// # Errors
///
/// `Forbidden` without `retentions.manage` in the project, `Guard` (422) when
/// [`new_problem`] says so. Nothing is written then.
pub async fn record(
    state: &AppState,
    user: &AuthUser,
    new: &NewRetentionDeduction,
) -> std::result::Result<RetentionDeduction, TransitionError> {
    let part = Retention::find(&state.db, new.retention_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !user.has_permission_in(permissions::RETENTIONS_MANAGE, &part.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if let Some(problem) = new_problem(&part, new) {
        return Err(TransitionError::Guard(problem));
    }
    let mut line = RetentionDeduction {
        project_id: part.project_id,
        retention_id: part.id,
        amount: new.amount,
        reference: new.reference.trim().to_owned(),
        reason: new.reason.trim().to_owned(),
        recorded_by: user.id,
        ..RetentionDeduction::default()
    };
    line.save(&state.db).await?;
    Ok(line)
}

/// What a decision came to.
#[derive(Debug)]
pub enum Decided {
    /// The deduction is decided: `accepted` (applied to the part) or
    /// `contested`.
    Done(RetentionDeduction),
    /// The deduction is above the limit: an approval request went to the MD
    /// and the deduction is still `undecided`.
    AwaitingMd(RetentionDeduction),
}

/// `user` (the project manager) accepts deduction `id`: the part loses the
/// amount. Above AED 25,000 the MD is asked first.
///
/// # Errors
///
/// `Forbidden` without `ipas.deductions.decide` in the project, `NotFrom`
/// unless the deduction is `undecided`, `Guard` (422) for the user who
/// recorded it, and 422 when the part is finished or holds less than the
/// amount by now. Nothing changes then.
pub async fn accept(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Decided, TransitionError> {
    decide(state, user, id, ACCEPT).await
}

/// `user` (the project manager) contests deduction `id`: it is not applied
/// and is listed for the final account.
///
/// # Errors
///
/// As [`accept`].
pub async fn contest(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Decided, TransitionError> {
    decide(state, user, id, CONTEST).await
}

async fn decide(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    name: &str,
) -> std::result::Result<Decided, TransitionError> {
    let line = RetentionDeduction::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let input = DecideInput {
        amount: line.amount,
        decided_by: Some(user.id),
    };
    let outcome = line
        .transition(name)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    let now = RetentionDeduction::find_or_404(&state.db, id).await?;
    Ok(if outcome.is_pending() {
        Decided::AwaitingMd(now)
    } else {
        Decided::Done(now)
    })
}

/// Books the accepted deduction `id` to the project's back-charge cost code,
/// once. Returns whether it booked.
///
/// # Errors
///
/// A database error and the cost poster's errors; a deduction not booked can
/// be booked again, the key keeps it from being booked twice.
pub async fn book_cost(state: &AppState, id: i64) -> Result<bool> {
    let line = RetentionDeduction::find_or_404(&state.db, id).await?;
    if line.status != DeductionStatus::Accepted || line.booked_at.is_some() {
        return Ok(false);
    }
    let project = Project::find_or_404(&state.db, line.project_id).await?;
    let request = PostCost::new(
        line.subject(),
        format!("{SUBJECT_TYPE}:{}", line.id),
        project.subject(),
        line.amount,
    )
    .cost_code(BACK_CHARGE_COST_CODE);
    post_cost(state, request).await?;
    let at = Some(renox::db::now());
    RetentionDeduction::query()
        .where_(RetentionDeduction::ID.eq(line.id))
        .update(&state.db, &[("booked_at", &at)])
        .await?;
    Ok(true)
}

/// The deductions of the retention part `retention_id`, in the order they
/// were recorded.
///
/// # Errors
///
/// A database error.
pub async fn lines_of(db: &Db, retention_id: i64) -> Result<Vec<RetentionDeduction>> {
    RetentionDeduction::query()
        .where_(RetentionDeduction::RETENTION_ID.eq(retention_id))
        .order_by(RetentionDeduction::ID)
        .get(db)
        .await
}

/// The contested deductions of `project_id`: the claims the final account
/// carries, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn contested_of(db: &Db, project_id: i64) -> Result<Vec<RetentionDeduction>> {
    RetentionDeduction::query()
        .where_(RetentionDeduction::PROJECT_ID.eq(project_id))
        .where_(RetentionDeduction::STATUS.eq(DeductionStatus::Contested))
        .order_by(RetentionDeduction::ID)
        .get(db)
        .await
}

/// `user` opens a dispute on the retention invoice of the `invoiced` part
/// `id` for `amount` fils (excluding VAT) that the client keeps: the dispute
/// is for the amount and its 5% VAT, and its resolution `credited` is the
/// credit note. Returns the dispute's id.
///
/// # Errors
///
/// `Forbidden` without `retentions.manage` in the project, `Guard` (422)
/// unless the part is `invoiced`, the amount is above 0 and not above the
/// part's, and a reason is given.
pub async fn dispute_invoiced(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    amount: i64,
    reason: &str,
) -> std::result::Result<i64, TransitionError> {
    let part = Retention::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !user.has_permission_in(permissions::RETENTIONS_MANAGE, &part.scope()) {
        return Err(TransitionError::Forbidden);
    }
    let invoice = match (part.status, part.invoice_id) {
        (RetentionStatus::Invoiced, Some(invoice)) => invoice,
        _ => {
            return Err(TransitionError::Guard(english(
                "construction.retention.dispute_not_invoiced",
                &[],
            )));
        }
    };
    if amount <= 0 || amount > part.amount {
        return Err(TransitionError::Guard(english(
            "construction.retention.deduction_too_big",
            &[("remaining", &aed(part.amount))],
        )));
    }
    if reason.trim().is_empty() {
        return Err(TransitionError::Guard(english(
            "construction.retention.deduction_reason",
            &[],
        )));
    }
    let gross = DraftLine::new("retention", amount, VatCategory::Standard).gross();
    let request = DisputeRequest::new(
        wl_kernel::invoicing::InvoiceId(invoice),
        reason.trim(),
        DisputeBy::User { id: user.id },
    )
    .amount(gross)
    .source(part.subject());
    Ok(invoicer(state).dispute(state, request).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retention::RetentionPart;

    fn part(status: RetentionStatus, amount: i64) -> Retention {
        Retention {
            status,
            ..Retention::new(1, RetentionPart::Final, amount)
        }
    }

    fn new(amount: i64) -> NewRetentionDeduction {
        NewRetentionDeduction {
            retention_id: 1,
            amount,
            reference: "NOTICE-4".into(),
            reason: "Defects the client fixed itself".into(),
        }
    }

    #[test]
    fn a_held_or_due_part_takes_a_deduction_within_what_remains() {
        for status in [RetentionStatus::Held, RetentionStatus::Due] {
            let held = part(status, 5_000_000);
            assert_eq!(new_problem(&held, &new(800_000)), None, "{status:?}");
            assert_eq!(new_problem(&held, &new(5_000_000)), None, "{status:?}");
            assert!(new_problem(&held, &new(5_000_001)).is_some());
            assert!(new_problem(&held, &new(0)).is_some());
        }
    }

    #[test]
    fn the_client_s_reference_and_the_reason_are_needed() {
        let held = part(RetentionStatus::Held, 5_000_000);
        let mut without = new(800_000);
        without.reference = " ".into();
        assert!(new_problem(&held, &without).is_some_and(|text| text.contains("reference")));
        let mut without = new(800_000);
        without.reason = String::new();
        assert!(new_problem(&held, &without).is_some_and(|text| text.contains("reason")));
    }

    #[test]
    fn an_invoiced_part_is_disputed_not_deducted() {
        let invoiced = part(RetentionStatus::Invoiced, 5_000_000);
        assert!(new_problem(&invoiced, &new(800_000)).is_some_and(|text| text.contains("dispute")));
        for status in [RetentionStatus::Settled, RetentionStatus::Kept] {
            assert!(new_problem(&part(status, 5_000_000), &new(1)).is_some());
        }
    }

    #[test]
    fn the_user_who_recorded_a_deduction_never_decides_it() {
        let line = RetentionDeduction {
            recorded_by: 7,
            ..RetentionDeduction::default()
        };
        let by = |by| DecideInput {
            amount: 1,
            decided_by: by,
        };
        assert_eq!(decide_problem(&line, &by(Some(8))), None);
        assert!(decide_problem(&line, &by(Some(7))).is_some());
        assert!(decide_problem(&line, &by(None)).is_some());
    }

    #[test]
    fn accepting_is_deferred_above_aed_25000_and_contesting_never() {
        let machine = RetentionDeduction::machine();
        for name in [ACCEPT, CONTEST] {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), ["undecided"], "{name}");
            assert!(!transition.is_system(), "{name}");
        }
        let approved = machine.find(ACCEPT_APPROVED).expect("twin");
        assert!(approved.is_system());
        assert_eq!(approved.to_state(), Some("accepted"));
        assert_eq!(wl_kernel::machines::approved(ACCEPT), ACCEPT_APPROVED);
        let expected = PolicyDefault::new(ACCEPT_ACTION)
            .amount_above(2_500_000)
            .step_or_permission(
                [wl_kernel::COMPANY_ADMIN_ROLE],
                permissions::PROJECTS_CANCEL,
            );
        assert_eq!(accept_policy(), expected);
    }

    #[test]
    fn the_vat_of_a_credit_for_aed_8000_is_aed_400() {
        let gross = DraftLine::new("retention", 800_000, VatCategory::Standard).gross();
        assert_eq!(gross, 840_000);
    }
}
