//! The Retention (02 Construction §6, §7.1 *Retention*): the amount withheld
//! on certified IPAs, one record per release part (the first and the final).
//! Withholding is [`withhold_in`]; the release is in [`crate::release`].
//!
//! ```text
//! held --make_due--> due --invoice--> invoiced --settle--> settled
//! held --make_due_bond--> due
//! ```
//!
//! - `make_due` is run by the system: for the first part when the project is
//!   completed, for the final part when it is closed or the defects liability
//!   period ends, for both when the project is terminated, unless the project
//!   says the retention does not become due on termination
//!   (`retention_due_on_termination`, `⚠ open: lawyer (L-37)`).
//! - `make_due_bond` is the same step run by a user with `retentions.manage`
//!   when a retention bond that is `active` and linked to the part replaces it.
//!   The spec names one `make_due` for both; a Renox transition has one
//!   actor kind, so the user's run is a twin of it, as `terminate_approved`
//!   is of `terminate`.
//! - `invoice` (`invoices.issue`) issues the retention tax invoice at 5% for
//!   the remaining amount (`CON.ACL.11`); `settle` is run by the system when
//!   that invoice is paid, credited or written off.
//! - `deduct` (a `held` part stays `held`), `deduct_due` (a `due` part stays
//!   `due`) and `keep` (nothing remains: `kept`, terminal) are the spec's one
//!   `deduct`: a Renox transition has one destination. The system runs them
//!   when the project manager accepts the client's deduction
//!   ([`crate::retention_deduction`], which also holds the QS's
//!   `retentions.manage` and the approval of `CON.ACL.14`).

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::permissions;
use crate::project::{self, Project};
use crate::texts::english;

/// The subject type of a retention part.
pub const SUBJECT_TYPE: &str = "construction.retention";

/// The `make_due` transition's name.
pub const MAKE_DUE: &str = "make_due";
/// `make_due_bond`: a user makes a part due because a bond replaces it.
pub const MAKE_DUE_BOND: &str = "make_due_bond";
/// The `invoice` transition's name.
pub const INVOICE: &str = "invoice";
/// The `settle` transition's name.
pub const SETTLE: &str = "settle";
/// `deduct`: the client keeps part of a `held` part, which stays `held`.
pub const DEDUCT: &str = "deduct";
/// `deduct_due`: the client keeps part of a `due` part, which stays `due`.
pub const DEDUCT_DUE: &str = "deduct_due";
/// `keep`: the client keeps all that remains of a part (`kept`).
pub const KEEP: &str = "keep";

/// Which release part it is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RetentionPart {
    /// Released at practical completion.
    #[default]
    First,
    /// Released at the end of the defects liability period.
    Final,
}

/// Where a retention part stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RetentionStatus {
    /// Withheld on certified IPAs; no tax document yet.
    #[default]
    Held,
    /// Release condition met; to be invoiced.
    Due,
    /// Retention tax invoice issued.
    Invoiced,
    /// Retention invoice paid, credited or written off (terminal).
    Settled,
    /// Kept in full by the client before it was invoiced (terminal).
    Kept,
}

impl RetentionStatus {
    /// The states in which the part is not finished.
    pub const OPEN: [RetentionStatus; 3] = [
        RetentionStatus::Held,
        RetentionStatus::Due,
        RetentionStatus::Invoiced,
    ];
}

/// One release part of the retention of a project.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "construction_retentions",
    state_machine,
    unique(project_id, part)
)]
pub struct Retention {
    /// The primary key.
    pub id: i64,
    /// The project.
    pub project_id: i64,
    /// First or final.
    pub part: RetentionPart,
    /// The amount held, in fils.
    pub amount: i64,
    /// The lifecycle state; changed only by a transition.
    pub status: RetentionStatus,
    /// When it became due.
    pub due_since: Option<DateTime>,
    /// The retention tax invoice in wl-invoicing, once made.
    pub invoice_id: Option<i64>,
    /// The guarantee of the kind retention bond linked to the part.
    pub bond_id: Option<i64>,
    /// When the accountant was told the part is due.
    pub announced_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Retention {
    /// A held part of `project_id` of `amount` fils.
    pub fn new(project_id: i64, part: RetentionPart, amount: i64) -> Self {
        Retention {
            project_id,
            part,
            amount,
            ..Retention::default()
        }
    }

    /// The part as a [`Subject`]: `construction.retention:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The scope roles are assigned in: the project of the part.
    pub fn scope(&self) -> Scope {
        Scope::new(project::SUBJECT_TYPE, self.project_id)
    }
}

/// What a `make_due_bond` or `invoice` run carries to its guard: the facts
/// the caller read, because a guard cannot look them up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInput {
    /// Whether the facts were read; a run without them is refused.
    #[serde(default)]
    pub checked: bool,
    /// What stops the part from going on, if anything.
    #[serde(default)]
    pub problem: Option<String>,
}

/// The input of a `make_due_bond` or `invoice` run.
pub(crate) fn release_input(ctx: &TransitionCtx) -> ReleaseInput {
    ctx.input_as::<ReleaseInput>().unwrap_or_default()
}

/// Why the run cannot go on with `input`, if it cannot.
pub fn release_problem(input: &ReleaseInput) -> Option<String> {
    if !input.checked {
        return Some(english("construction.bad_input", &[]));
    }
    input.problem.clone()
}

/// What a `deduct`, `deduct_due` or `keep` run carries: the amount the client
/// keeps, in fils.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeductInput {
    /// The amount, in fils.
    #[serde(default)]
    pub amount: i64,
}

/// The input of a `deduct`, `deduct_due` or `keep` run.
pub(crate) fn deduct_input(ctx: &TransitionCtx) -> DeductInput {
    ctx.input_as::<DeductInput>().unwrap_or_default()
}

/// Why `part` cannot lose `input.amount`, if it cannot: the amount is above 0
/// and not above what remains, and `whole` says whether it is all of it
/// (`keep`) or only a part (`deduct`).
pub fn deduct_problem(part: &Retention, input: &DeductInput, whole: bool) -> Option<String> {
    if input.amount <= 0 {
        return Some(english("construction.retention.deduction_amount", &[]));
    }
    if input.amount > part.amount {
        return Some(english(
            "construction.retention.deduction_too_big",
            &[("remaining", &crate::notices::aed(part.amount))],
        ));
    }
    if whole != (input.amount == part.amount) {
        return Some(english("construction.bad_input", &[]));
    }
    None
}

/// A system transition that takes `input.amount` off the part.
fn deducting(
    transition: Transition<Retention>,
    from: &[RetentionStatus],
    to: RetentionStatus,
    whole: bool,
) -> Transition<Retention> {
    transition
        .system()
        .from(from.iter().copied())
        .to(to)
        .guard("amount", move |part: &Retention, ctx| {
            let problem = deduct_problem(part, &deduct_input(ctx), whole);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|part: &Retention, ctx, tx| {
            let id = part.id;
            let remaining = part.amount - deduct_input(ctx).amount;
            Box::pin(async move {
                Retention::query()
                    .where_(Retention::ID.eq(id))
                    .update(&mut *tx, &[("amount", &remaining)])
                    .await?;
                Ok(())
            })
        })
}

impl StateMachine for Retention {
    type State = RetentionStatus;
    const COLUMN: renox::db::Col<Retention, RetentionStatus> = Retention::STATUS;

    fn machine() -> Machine<Self> {
        use RetentionStatus::{Due, Held, Invoiced, Kept, Settled};
        Machine::new()
            .initial([Held])
            .state(Held)
            .tone("info")
            .state(Due)
            .tone("warning")
            .state(Invoiced)
            .tone("info")
            .state(Settled)
            .tone("success")
            .terminal()
            .state(Kept)
            .tone("neutral")
            .terminal()
            .transition(becoming_due(
                Transition::new(MAKE_DUE).label("Make due").system(),
            ))
            .transition(
                becoming_due(
                    Transition::new(MAKE_DUE_BOND)
                        .label("Make due (bond)")
                        .permission_in(permissions::RETENTIONS_MANAGE, |part: &Retention| {
                            part.scope()
                        }),
                )
                .guard("bond", |_part: &Retention, ctx| {
                    let problem = release_problem(&release_input(ctx));
                    Box::pin(async move { problem.map_or(Ok(()), Err) })
                }),
            )
            .transition(
                Transition::new(INVOICE)
                    .label("Invoice")
                    .tone("primary")
                    .permission_in(permissions::INVOICES_ISSUE, |part: &Retention| part.scope())
                    .from([Due])
                    .to(Invoiced)
                    .guard("released", |_part: &Retention, ctx| {
                        let problem = release_problem(&release_input(ctx));
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    }),
            )
            .transition(
                Transition::new(SETTLE)
                    .label("Settle")
                    .system()
                    .from([Invoiced])
                    .to(Settled),
            )
            .transition(deducting(
                Transition::new(DEDUCT).label("Deduct"),
                &[Held],
                Held,
                false,
            ))
            .transition(deducting(
                Transition::new(DEDUCT_DUE).label("Deduct (due)"),
                &[Due],
                Due,
                false,
            ))
            .transition(deducting(
                Transition::new(KEEP).label("Keep"),
                &[Held, Due],
                Kept,
                true,
            ))
    }
}

/// A transition from `held` to `due` that stamps when the part became due.
fn becoming_due(transition: Transition<Retention>) -> Transition<Retention> {
    transition
        .from([RetentionStatus::Held])
        .to(RetentionStatus::Due)
        .effect(|part: &Retention, _ctx, tx| {
            let id = part.id;
            Box::pin(async move {
                let since = Some(renox::db::now());
                Retention::query()
                    .where_(Retention::ID.eq(id))
                    .update(&mut *tx, &[("due_since", &since)])
                    .await?;
                Ok(())
            })
        })
}

/// The parts of `project_id`, first then final, read in `tx`.
///
/// # Errors
///
/// A database error.
pub async fn parts_in(tx: &mut Transaction, project_id: i64) -> Result<Vec<Retention>> {
    Retention::query()
        .where_(Retention::PROJECT_ID.eq(project_id))
        .order_by(Retention::ID)
        .get(&mut *tx)
        .await
}

/// The parts of `project_id` that are not finished.
///
/// # Errors
///
/// A database error.
pub async fn open_of(db: &Db, project_id: i64) -> Result<Vec<Retention>> {
    Retention::query()
        .where_(Retention::PROJECT_ID.eq(project_id))
        .where_(Retention::STATUS.is_in(RetentionStatus::OPEN))
        .order_by(Retention::ID)
        .get(db)
        .await
}

/// `amount` x `bps` / 10,000, rounded half up to the fil.
fn share(amount: i64, bps: i64) -> i64 {
    let scaled = i128::from(amount) * i128::from(bps);
    i64::try_from((2 * scaled + 10_000) / 20_000).unwrap_or(i64::MAX)
}

/// Withholds the retention of an IPA whose certified work of the period is
/// `work` fils, inside `tx`, and adds it to the parts of `project`
/// (`CON.IPA.01`, 02 Construction §7.1 *Retention*): retention % of the work,
/// up to the cap (the IPA that reaches it withholds only the rest, later IPAs
/// nothing), split per the project's release split, rounded half up to the
/// fil with the remainder to the final part. A part that does not exist yet
/// is made `held`. Returns the amount withheld; no tax document is issued.
///
/// # Errors
///
/// A database error.
pub async fn withhold_in(tx: &mut Transaction, project: &Project, work: i64) -> Result<i64> {
    if work <= 0 || project.retention_bps <= 0 {
        return Ok(0);
    }
    let parts = parts_in(tx, project.id).await?;
    let held: i64 = parts.iter().map(|part| part.amount).sum();
    let mut withheld = share(work, project.retention_bps);
    if let Some(cap) = project.retention_cap {
        withheld = withheld.min((cap - held).max(0));
    }
    if withheld <= 0 {
        return Ok(0);
    }
    let first = share(withheld, project.retention_first_bps);
    for (which, amount) in [
        (RetentionPart::First, first),
        (RetentionPart::Final, withheld - first),
    ] {
        if amount <= 0 {
            continue;
        }
        match parts.iter().find(|part| part.part == which) {
            Some(found) => {
                let total = found.amount + amount;
                Retention::query()
                    .where_(Retention::ID.eq(found.id))
                    .update(&mut *tx, &[("amount", &total)])
                    .await?;
            }
            None => {
                let mut made = Retention::new(project.id, which, amount);
                made.insert(&mut *tx).await?;
                wl_kernel::search::index_in(&mut *tx, &Subject::new(SUBJECT_TYPE, made.id)).await?;
            }
        }
    }
    Ok(withheld)
}

/// Makes every `held` part of `project_id` due, as the system, inside `tx`.
/// Returns how many it made due.
///
/// # Errors
///
/// A database error.
pub async fn make_all_due_in(tx: &mut Transaction, project_id: i64) -> Result<usize> {
    let mut made = 0;
    for part in parts_in(tx, project_id).await? {
        if make_due_in(tx, &part).await? {
            made += 1;
        }
    }
    Ok(made)
}

/// Makes the `held` part `which` of `project_id` due, as the system, inside
/// `tx`. Returns whether it did: a part that does not exist, or is not `held`,
/// is left alone.
///
/// # Errors
///
/// A database error.
pub async fn make_part_due_in(
    tx: &mut Transaction,
    project_id: i64,
    which: RetentionPart,
) -> Result<bool> {
    for part in parts_in(tx, project_id).await? {
        if part.part == which {
            return make_due_in(tx, &part).await;
        }
    }
    Ok(false)
}

/// Runs `make_due` on `part` when it is `held`.
async fn make_due_in(tx: &mut Transaction, part: &Retention) -> Result<bool> {
    if part.status != RetentionStatus::Held {
        return Ok(false);
    }
    // The accountant is told after the commit ([`crate::release::announce_due`]).
    let _outcome = part
        .transition(MAKE_DUE)
        .by_system()
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_is_rounded_half_up_to_the_fil() {
        assert_eq!(share(18_000_000, 1_000), 1_800_000);
        assert_eq!(share(1_000_001, 5_000), 500_001);
        assert_eq!(share(1, 5_000), 1);
        assert_eq!(share(0, 5_000), 0);
    }

    #[test]
    fn only_a_held_part_becomes_due() {
        let machine = Retention::machine();
        assert_eq!(machine.initial_states(), ["held"]);
        for (name, by_system) in [(MAKE_DUE, true), (MAKE_DUE_BOND, false)] {
            let make_due = machine.find(name).expect(name);
            assert_eq!(make_due.is_system(), by_system, "{name}");
            assert_eq!(make_due.from_states(), ["held"], "{name}");
            assert_eq!(make_due.to_state(), Some("due"), "{name}");
        }
        assert_eq!(
            machine.find(MAKE_DUE_BOND).expect("twin").guard_names(),
            ["bond"]
        );
    }

    #[test]
    fn a_part_is_invoiced_only_when_due_and_settled_by_the_system() {
        let machine = Retention::machine();
        let invoice = machine.find(INVOICE).expect("invoice");
        assert!(!invoice.is_system());
        assert_eq!(invoice.from_states(), ["due"]);
        assert_eq!(invoice.to_state(), Some("invoiced"));
        assert_eq!(invoice.guard_names(), ["released"]);
        let settle = machine.find(SETTLE).expect("settle");
        assert!(settle.is_system());
        assert_eq!(settle.from_states(), ["invoiced"]);
        assert_eq!(settle.to_state(), Some("settled"));
    }

    #[test]
    fn the_client_keeps_part_of_a_part_or_all_of_it() {
        let machine = Retention::machine();
        for (name, from, to) in [
            (DEDUCT, ["held"].as_slice(), "held"),
            (DEDUCT_DUE, ["due"].as_slice(), "due"),
            (KEEP, ["held", "due"].as_slice(), "kept"),
        ] {
            let transition = machine.find(name).expect(name);
            assert!(transition.is_system(), "{name}");
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.guard_names(), ["amount"], "{name}");
        }
        let kept = machine.states().iter().find(|s| s.name == "kept").unwrap();
        assert!(kept.terminal);
    }

    #[test]
    fn a_deduction_is_above_0_and_within_what_remains() {
        let part = Retention::new(1, RetentionPart::Final, 5_000_000);
        let amount = |amount| DeductInput { amount };
        assert_eq!(deduct_problem(&part, &amount(800_000), false), None);
        assert_eq!(deduct_problem(&part, &amount(5_000_000), true), None);
        assert!(deduct_problem(&part, &amount(0), false).is_some());
        assert!(
            deduct_problem(&part, &amount(5_000_001), false)
                .is_some_and(|text| text.contains("AED 50,000.00"))
        );
        // All of it is `keep`, a part of it is `deduct`.
        assert!(deduct_problem(&part, &amount(5_000_000), false).is_some());
        assert!(deduct_problem(&part, &amount(1), true).is_some());
    }

    #[test]
    fn a_run_without_the_facts_is_refused() {
        assert!(release_problem(&ReleaseInput::default()).is_some());
        let blocked = ReleaseInput {
            checked: true,
            problem: Some("no".into()),
        };
        assert_eq!(release_problem(&blocked), Some("no".into()));
        let open = ReleaseInput {
            checked: true,
            problem: None,
        };
        assert_eq!(release_problem(&open), None);
    }
}
