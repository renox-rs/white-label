//! The typed deductions on a certificate (`CON.ACL.14`, 02 Construction §3.3,
//! §4.4 step 5, §6, §7.1 *IPA* `certify`; BL-CON-05).
//!
//! A client deducts back-charges, contra-charges and liquidated damages from
//! what it certifies. When the certificate arrives the **QS** records each
//! deduction as a line ([`record`], `ipas.deductions.record`) with its type
//! (`back_charge`, `contra_charge`, `liquidated_damages`, `other`), the
//! client's reference and the amount excluding VAT, on the `submitted` IPA
//! (a line of a superseded IPA goes to the IPA that replaced it). A line of
//! type `other` needs a description: accepted, it is treated like a price
//! reduction (BL-CON-06). Lines are open to change until `certify`; after it
//! only the final account corrects them.
//!
//! The **project manager** (`ipas.deductions.decide`) accepts or contests
//! each line:
//!
//! ```text
//! undecided --accept--> accepted      (above AED 25,000: the MD approves first)
//! undecided --contest--> contested
//! ```
//!
//! - never the user who recorded the line (`SOD`, [`decide_problem`]);
//! - accepting a line above AED 25,000, checked per line excluding VAT, is
//!   deferred as an approval request for the MD ([`accept_policy`]); the
//!   line stays `undecided` until it is approved, and a rejection or a
//!   return leaves it `undecided` for the project manager to decide again.
//!   The kernel runs `accept_approved` at the approval, with the same guard;
//! - the IPA is certified only when no line is `undecided`
//!   ([`crate::certificate`]).
//!
//! A contested line stays a claim: it is listed with the project's final
//! account items once its IPA is certified ([`final_account_items`]).
//! Deductions are separate lines and are never netted into the certified
//! work amount.

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::{Subject, SubjectMachine};

use crate::certificate;
use crate::ipa::{Ipa, IpaStatus};
use crate::permissions;
use crate::project::{self, Project};
use crate::texts::english;

/// The subject type of a deduction line.
pub const SUBJECT_TYPE: &str = "construction.ipa_deduction";

/// The `accept` transition's name.
pub const ACCEPT: &str = "accept";
/// `accept_approved`: the kernel runs it at the final approval.
pub const ACCEPT_APPROVED: &str = "accept_approved";
/// The `contest` transition's name.
pub const CONTEST: &str = "contest";

/// The approval policies' action key of `accept` (`CON.ACL.14`).
pub const ACCEPT_ACTION: &str = "construction.deduction_accept";

/// The value above which accepting a line needs the MD, per line excluding
/// VAT: AED 25,000, in fils (`CON.ACL.14`).
pub const APPROVAL_LIMIT: i64 = 2_500_000;

/// What the client deducts for.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DeductionKind {
    /// Goods, services or labour the client supplied.
    #[default]
    BackCharge,
    /// Goods, services or labour another contractor supplied.
    ContraCharge,
    /// An agreed amount for late completion.
    LiquidatedDamages,
    /// Anything else; needs a description.
    Other,
}

/// Where a line stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DeductionStatus {
    /// Recorded, not decided.
    #[default]
    Undecided,
    /// The project manager accepted it.
    Accepted,
    /// The project manager does not accept it; it stays a claim.
    Contested,
}

/// One deduction on a certificate. Money is fils, excluding VAT.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "construction_ipa_deductions", state_machine, index(ipa_id))]
pub struct IpaDeduction {
    /// The primary key.
    pub id: i64,
    /// The project.
    pub project_id: i64,
    /// The IPA.
    pub ipa_id: i64,
    /// The type.
    pub kind: DeductionKind,
    /// The client's reference, e.g. `BC-12`.
    pub reference: String,
    /// What it is for; needed for the kind `other`.
    pub description: String,
    /// The amount excluding VAT.
    pub amount: i64,
    /// The lifecycle state; changed only by a transition.
    pub status: DeductionStatus,
    /// The QS who recorded it.
    pub recorded_by: i64,
    /// The project manager who decided it.
    pub decided_by: Option<i64>,
    /// When it was decided.
    pub decided_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl IpaDeduction {
    /// The line as a [`Subject`]: `construction.ipa_deduction:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The scope roles are assigned in: the project of the line.
    pub fn scope(&self) -> Scope {
        Scope::new(project::SUBJECT_TYPE, self.project_id)
    }
}

/// What an `accept` or `contest` run carries: the amount the approval policy
/// reads (`amount`, in fils) and the user who decides, which the guard checks
/// against the user who recorded the line. It is also the approval request's
/// payload, so the approved run decides on behalf of the same user.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecideInput {
    /// The amount of the line, in fils.
    #[serde(default)]
    pub amount: i64,
    /// The project manager who decides.
    #[serde(default)]
    pub decided_by: Option<i64>,
}

/// The input of an `accept` or `contest` run.
pub(crate) fn decide_input(ctx: &TransitionCtx) -> DecideInput {
    ctx.input_as::<DecideInput>().unwrap_or_default()
}

/// Why the line cannot be decided with `input`, if it cannot: the user who
/// recorded it never decides it (`CON.ACL.14`).
pub fn decide_problem(line: &IpaDeduction, input: &DecideInput) -> Option<String> {
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
    IpaDeduction::query()
        .where_(IpaDeduction::ID.eq(id))
        .update(&mut *tx, &[("decided_by", &by), ("decided_at", &at)])
        .await?;
    Ok(())
}

/// A user transition of the line: `permission` in the project's scope.
fn user(name: &str, label: &str) -> Transition<IpaDeduction> {
    Transition::new(name).label(label).permission_in(
        permissions::IPAS_DEDUCTIONS_DECIDE,
        |line: &IpaDeduction| line.scope(),
    )
}

/// The guard and the effect every decision shares.
fn deciding(transition: Transition<IpaDeduction>, to: DeductionStatus) -> Transition<IpaDeduction> {
    transition
        .from([DeductionStatus::Undecided])
        .to(to)
        .guard("separation", |line: &IpaDeduction, ctx| {
            let problem = decide_problem(line, &decide_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|line: &IpaDeduction, ctx, tx| {
            let id = line.id;
            let input = decide_input(ctx);
            Box::pin(async move { decided_effects(&mut *tx, id, &input).await })
        })
}

impl StateMachine for IpaDeduction {
    type State = DeductionStatus;
    const COLUMN: renox::db::Col<IpaDeduction, DeductionStatus> = IpaDeduction::STATUS;

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
            .transition(deciding(user(ACCEPT, "Accept").tone("primary"), Accepted))
            .transition(deciding(
                Transition::new(ACCEPT_APPROVED)
                    .label("Accept (approved)")
                    .system(),
                Accepted,
            ))
            .transition(deciding(user(CONTEST, "Contest").tone("danger"), Contested))
    }
}

/// The default approval policy of `accept` (`CON.ACL.14`): a line above AED
/// 25,000 needs one step by the MD (a holder of `projects.cancel`, the
/// company admin by default), never the project manager who asked.
pub fn accept_policy() -> PolicyDefault {
    PolicyDefault::new(ACCEPT_ACTION)
        .amount_above(APPROVAL_LIMIT)
        .step_or_permission(
            [wl_kernel::COMPANY_ADMIN_ROLE],
            permissions::PROJECTS_CANCEL,
        )
}

/// The line in the kernel's registry: its approval action for `accept` and
/// the project as the scope where approvers are looked for.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<IpaDeduction>(SUBJECT_TYPE)
        .action(ACCEPT, ACCEPT_ACTION)
        .scope(|line: &IpaDeduction| line.scope())
        .build()
}

/// Registers the machine with the kernel and with `rnx state:diagram`, and
/// the default policy of `accept`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<IpaDeduction>();
    accept_policy().register();
}

/// What the QS enters for one line.
#[derive(Debug, Clone)]
pub struct NewDeduction {
    /// The type.
    pub kind: DeductionKind,
    /// The client's reference.
    pub reference: String,
    /// What it is for; needed for the kind `other`.
    pub description: String,
    /// The amount excluding VAT, in fils.
    pub amount: i64,
}

/// Why the line cannot be recorded, if it cannot: a reference and an amount
/// above 0 are needed, and the kind `other` needs a description.
pub fn new_problem(new: &NewDeduction) -> Option<String> {
    if new.reference.trim().is_empty() {
        return Some(english("construction.deduction.reference_needed", &[]));
    }
    if new.amount <= 0 {
        return Some(english("construction.deduction.amount_needed", &[]));
    }
    if new.kind == DeductionKind::Other && new.description.trim().is_empty() {
        return Some(english("construction.deduction.description_needed", &[]));
    }
    None
}

/// `user` (the QS) records a deduction line on IPA `ipa_id`. For a superseded
/// IPA the line goes to the IPA that replaced it.
///
/// # Errors
///
/// `Forbidden` without `ipas.deductions.record` in the project, `Guard`
/// (422) when [`new_problem`] says so or the IPA is no longer `submitted`
/// (lines are open to change until `certify`). Nothing is written then.
pub async fn record(
    state: &AppState,
    user: &AuthUser,
    ipa_id: i64,
    new: &NewDeduction,
) -> std::result::Result<IpaDeduction, TransitionError> {
    let target = certificate::resolve(&state.db, ipa_id).await?;
    let project = Project::find_or_404(&state.db, target.project_id).await?;
    if !user.has_permission_in(permissions::IPAS_DEDUCTIONS_RECORD, &project.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if let Some(problem) = new_problem(new) {
        return Err(TransitionError::Guard(problem));
    }
    if target.status != IpaStatus::Submitted {
        return Err(TransitionError::Guard(english(
            "construction.deduction.lines_closed",
            &[("number", &target.number)],
        )));
    }
    let mut line = IpaDeduction {
        project_id: target.project_id,
        ipa_id: target.id,
        kind: new.kind,
        reference: new.reference.trim().to_owned(),
        description: new.description.trim().to_owned(),
        amount: new.amount,
        recorded_by: user.id,
        ..IpaDeduction::default()
    };
    line.save(&state.db).await?;
    Ok(line)
}

/// What a decision came to.
#[derive(Debug)]
pub enum Decided {
    /// The line is decided: `accepted` or `contested`.
    Done(IpaDeduction),
    /// The line is above the limit: an approval request went to the MD and
    /// the line is still `undecided`.
    AwaitingMd(IpaDeduction),
}

/// `user` (the project manager) accepts line `id`.
///
/// # Errors
///
/// `Forbidden` without `ipas.deductions.decide` in the project, `NotFrom`
/// unless the line is `undecided`, `Guard` (422) for the user who recorded
/// it. Nothing changes then.
pub async fn accept(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Decided, TransitionError> {
    decide(state, user, id, ACCEPT).await
}

/// `user` (the project manager) contests line `id`; it stays a claim.
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
    let line = IpaDeduction::find(&state.db, id)
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
    let now = IpaDeduction::find_or_404(&state.db, id).await?;
    Ok(if outcome.is_pending() {
        Decided::AwaitingMd(now)
    } else {
        Decided::Done(now)
    })
}

/// The lines of IPA `ipa_id`, in the order they were recorded.
///
/// # Errors
///
/// A database error.
pub async fn lines_of(db: &Db, ipa_id: i64) -> Result<Vec<IpaDeduction>> {
    IpaDeduction::query()
        .where_(IpaDeduction::IPA_ID.eq(ipa_id))
        .order_by(IpaDeduction::ID)
        .get(db)
        .await
}

/// How many lines of IPA `ipa_id` are still `undecided`.
///
/// # Errors
///
/// A database error.
pub async fn undecided_count(db: &Db, ipa_id: i64) -> Result<i64> {
    let count = IpaDeduction::query()
        .where_(IpaDeduction::IPA_ID.eq(ipa_id))
        .where_(IpaDeduction::STATUS.eq(DeductionStatus::Undecided))
        .count(db)
        .await?;
    Ok(i64::try_from(count).unwrap_or(i64::MAX))
}

/// The contested lines of the certified IPAs of `project_id`: the claims the
/// final account carries, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn final_account_items(db: &Db, project_id: i64) -> Result<Vec<IpaDeduction>> {
    let certified = Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project_id))
        .where_(Ipa::STATUS.is_in([IpaStatus::Certified, IpaStatus::Invoiced]))
        .get(db)
        .await?;
    let ids: Vec<i64> = certified.iter().map(|ipa| ipa.id).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    IpaDeduction::query()
        .where_(IpaDeduction::IPA_ID.is_in(ids))
        .where_(IpaDeduction::STATUS.eq(DeductionStatus::Contested))
        .order_by(IpaDeduction::ID)
        .get(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(by: i64) -> IpaDeduction {
        IpaDeduction {
            recorded_by: by,
            amount: 500_000,
            ..IpaDeduction::default()
        }
    }

    #[test]
    fn the_user_who_recorded_a_line_never_decides_it() {
        let input = |by| DecideInput {
            amount: 500_000,
            decided_by: by,
        };
        assert_eq!(decide_problem(&line(7), &input(Some(8))), None);
        assert!(decide_problem(&line(7), &input(Some(7))).is_some_and(|p| p.contains("recorded")));
        assert!(decide_problem(&line(7), &input(None)).is_some());
    }

    #[test]
    fn a_line_needs_its_reference_amount_and_for_other_a_description() {
        let mut new = NewDeduction {
            kind: DeductionKind::BackCharge,
            reference: "BC-12".into(),
            description: String::new(),
            amount: 500_000,
        };
        assert_eq!(new_problem(&new), None);
        new.reference = " ".into();
        assert!(new_problem(&new).is_some_and(|p| p.contains("reference")));
        new.reference = "BC-12".into();
        new.amount = 0;
        assert!(new_problem(&new).is_some_and(|p| p.contains("amount")));
        new.amount = 1;
        new.kind = DeductionKind::Other;
        assert!(new_problem(&new).is_some_and(|p| p.to_lowercase().contains("describe")));
        new.description = "Scaffolding hire".into();
        assert_eq!(new_problem(&new), None);
    }

    #[test]
    fn accepting_is_deferred_above_aed_25000_and_contesting_never() {
        let machine = IpaDeduction::machine();
        for name in [ACCEPT, CONTEST] {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), ["undecided"], "{name}");
            assert!(!transition.is_system(), "{name}");
            assert_eq!(transition.guard_names(), ["separation"], "{name}");
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
}
