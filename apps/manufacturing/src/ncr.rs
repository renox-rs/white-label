//! The Non-conformance report (07 Manufacturing §7.1 *NCR*, `MFG.QCL.02`): a
//! defect on a lot (or on an item and a dispatch, when a customer disputes an
//! invoice for quality), its cause, the disposition QC decides and what
//! became of it. The services are in [`crate::ncrs`].
//!
//! ```text
//! open --disposition--> dispositioned --close--> closed
//! open --cancel--> cancelled
//! dispositioned --reopen / auto_reopen--> open      (the action did not happen)
//! ```
//!
//! A Renox transition has one fixed `to` and a user run and the system's
//! cannot share one, so:
//!
//! - the disposition is three transitions by what the rules ask: `disposition`
//!   (rework, return to supplier, scrap up to AED 2,000), `disposition_scrap`
//!   (scrap above the limit, deferred as an approval request under
//!   `STK.ACL.02`: a holder of `stock.approve` decides) and
//!   `disposition_use_as_is` (always deferred: a holder of `ncrs.approve`
//!   other than the requester, `MFG.ACL.03`); the two deferred ones have a
//!   system twin the kernel runs at the final approval (`..._approved`);
//! - `reopen` is the user's, `auto_reopen` the system's (the linked Return to
//!   supplier was cancelled, or the rework work order was cancelled or
//!   aborted).
//!
//! The effect of a disposition is written in the NCR's row and in
//! [`NcrDecision`], which keeps every disposition the NCR had (a reopened NCR
//! keeps the earlier one); the movements, the Return to supplier and the
//! rework work order follow the commit ([`crate::ncrs::perform`]), because a
//! transition's effect has no access to the other modules.

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, LOT_SUBJECT_TYPE};
use wl_kernel::machines::approved;
use wl_kernel::{PartyId, Subject, SubjectMachine};

use crate::permissions;
use crate::texts::english;

/// The subject type of an NCR.
pub const SUBJECT_TYPE: &str = "manufacturing.ncr";

/// `disposition`: rework, return to supplier, scrap up to the limit.
pub const DISPOSITION: &str = "disposition";
/// `disposition` of a scrap that may be above the limit (`STK.ACL.02`).
pub const DISPOSITION_SCRAP: &str = "disposition_scrap";
/// `disposition` *use as is* (`MFG.ACL.03`).
pub const DISPOSITION_USE_AS_IS: &str = "disposition_use_as_is";
/// `close`.
pub const CLOSE: &str = "close";
/// `cancel`.
pub const CANCEL: &str = "cancel";
/// `reopen` by QC.
pub const REOPEN: &str = "reopen";
/// `reopen` by the system.
pub const AUTO_REOPEN: &str = "auto_reopen";

/// The approval action of a scrap above the limit (`STK.ACL.02`).
pub const SCRAP_ACTION: &str = "manufacturing.ncr_scrap";
/// The approval action of *use as is* (`MFG.ACL.03`).
pub const USE_AS_IS_ACTION: &str = "manufacturing.ncr_use_as_is";

/// The scrap value, in fils, above which `STK.ACL.02` asks for a second
/// person: AED 2,000.
pub const SCRAP_APPROVAL_FILS: i64 = 200_000;

/// The key of the scrap value in the transition input, which the policy of
/// the scrap reads.
pub const SCRAP_KEY: &str = "scrap_value";

/// Where an NCR stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NcrStatus {
    /// Defect recorded.
    #[default]
    Open,
    /// Decision taken; actions in progress.
    Dispositioned,
    /// Actions done (terminal).
    Closed,
    /// Raised in error (terminal).
    Cancelled,
}

/// What QC decided for the goods.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Disposition {
    /// A rework work order makes them right.
    #[default]
    Rework,
    /// They are written off.
    Scrap,
    /// They are used as they are (approved, `MFG.ACL.03`).
    UseAsIs,
    /// They go back to the supplier, for credit or replacement.
    ReturnToSupplier,
}

/// Who caused the defect (§7.1: the supplier scorecard reads it).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Cause {
    /// The supplier of the goods.
    Supplier,
    /// The plant's own process.
    InternalProcess,
    /// Not known (yet).
    #[default]
    Unknown,
}

/// Where the NCR was raised from.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Origin {
    /// Incoming inspection, or QC by hand on a lot.
    #[default]
    Incoming,
    /// A customer's quality dispute on an invoice (BL-MFG-13).
    Dispute,
    /// A production finding (scrap above the limit, final inspection).
    Production,
    /// Any other.
    Other,
}

/// A non-conformance report.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manufacturing_ncrs",
    state_machine,
    unique(number),
    index(status),
    index(lot_id)
)]
pub struct Ncr {
    /// The primary key.
    pub id: i64,
    /// The number from the `NCR-` series.
    pub number: String,
    /// The plant (where QC works and the approvals are looked for).
    pub plant_id: i64,
    /// The item, as an inventory item id.
    pub item_id: i64,
    /// The lot, if the item is lot-tracked.
    pub lot_id: Option<i64>,
    /// The lot number at the time (the pack's).
    pub lot_number: Option<String>,
    /// The supplier of the lot at the time; none for own production.
    pub supplier_party_id: Option<i64>,
    /// The quantity the NCR covers, in thousandths.
    pub quantity_milli: i64,
    /// The defect, in words.
    pub defect: String,
    /// Who caused it.
    pub cause: Cause,
    /// Why QC changed the cause.
    pub cause_reason: Option<String>,
    /// The user who changed the cause.
    pub cause_changed_by: Option<i64>,
    /// When the cause was changed.
    pub cause_changed_at: Option<DateTime>,
    /// Where it was raised from.
    pub origin: Origin,
    /// The type of the record it was raised from (a dispute, a dispatch).
    pub source_type: Option<String>,
    /// The id of that record.
    pub source_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: NcrStatus,
    /// The disposition in force; none while `open`.
    pub disposition: Option<Disposition>,
    /// For a return to supplier: `credit` or `replace`.
    pub outcome: Option<String>,
    /// QC's note with the disposition.
    pub disposition_note: Option<String>,
    /// The user who decided.
    pub dispositioned_by: Option<i64>,
    /// When it was decided.
    pub dispositioned_at: Option<DateTime>,
    /// The line the rework work order is scheduled on.
    pub rework_line_id: Option<i64>,
    /// The BOM version the rework work order is made from.
    pub rework_bom_id: Option<i64>,
    /// The rework work order.
    pub rework_work_order_id: Option<i64>,
    /// The type of the Return to supplier (`inventory.supplier_return`).
    pub return_type: Option<String>,
    /// The id of the Return to supplier.
    pub return_id: Option<i64>,
    /// The value of the scrap, in fils.
    pub scrap_value: i64,
    /// When the action of the disposition was claimed (one run does it).
    pub action_claimed_at: Option<DateTime>,
    /// When the action happened: the rework work order completed, the scrap
    /// posted, the Return to supplier dispatched, the lot released.
    pub action_done_at: Option<DateTime>,
    /// The user who closed it.
    pub closed_by: Option<i64>,
    /// When it was closed.
    pub closed_at: Option<DateTime>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// The user who raised it; none for the system.
    pub raised_by: Option<i64>,
    /// The day it was raised (Asia/Dubai).
    pub raised_on: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Ncr {
    /// The NCR as a [`Subject`]: `manufacturing.ncr:5`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The lot as a subject, `inventory.lot:4`.
    pub fn lot(&self) -> Option<Subject> {
        self.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id))
    }

    /// The item as a subject, `inventory.item:9`.
    pub fn item(&self) -> Subject {
        Subject::new(ITEM_SUBJECT_TYPE, self.item_id)
    }

    /// The supplier of the lot at the time.
    pub fn supplier(&self) -> Option<PartyId> {
        self.supplier_party_id.map(PartyId)
    }

    /// The Return to supplier, `inventory.supplier_return:4`.
    pub fn supplier_return(&self) -> Option<Subject> {
        match (&self.return_type, self.return_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The record the NCR was raised from.
    pub fn source(&self) -> Option<Subject> {
        match (&self.source_type, self.source_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The scope QC works in and the approvals are looked for in: the plant.
    pub fn scope(&self) -> Scope {
        permissions::plant_scope(self.plant_id)
    }

    /// Whether `user` holds `permission` in the plant of the NCR.
    pub fn may(&self, user: &AuthUser, permission: &str) -> bool {
        user.has_permission_in(permission, &self.scope())
    }

    /// Whether the NCR still keeps its lot from being released: it is `open`,
    /// or `dispositioned` and the action of the disposition has not
    /// happened (§7.1: a partial NCR keeps the lot from release).
    pub fn keeps_lot(&self) -> bool {
        match self.status {
            NcrStatus::Open => true,
            NcrStatus::Dispositioned => self.action_done_at.is_none(),
            NcrStatus::Closed | NcrStatus::Cancelled => false,
        }
    }
}

/// One disposition an NCR had; a reopened NCR keeps the earlier ones.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manufacturing_ncr_decisions", index(ncr_id))]
pub struct NcrDecision {
    /// The primary key.
    pub id: i64,
    /// The NCR.
    pub ncr_id: i64,
    /// The disposition.
    pub disposition: Disposition,
    /// For a return to supplier: `credit` or `replace`.
    pub outcome: Option<String>,
    /// The quantity it covered, in thousandths.
    pub quantity_milli: i64,
    /// The user who decided.
    pub decided_by: Option<i64>,
    /// When it was decided.
    pub decided_at: DateTime,
    /// When it ended (the NCR was reopened); none while in force.
    pub ended_at: Option<DateTime>,
    /// Why it ended.
    pub ended_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What a run carries to the guards and effects; for a deferred disposition
/// it is also the approval request's payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NcrInput {
    /// Every check that fails, gathered by the service; the guard refuses
    /// while any is left.
    #[serde(default)]
    pub problems: Vec<String>,
    /// The user who runs it.
    #[serde(default)]
    pub by: Option<i64>,
    /// The disposition chosen.
    #[serde(default)]
    pub disposition: Option<Disposition>,
    /// `credit` or `replace`.
    #[serde(default)]
    pub outcome: Option<String>,
    /// QC's note.
    #[serde(default)]
    pub note: Option<String>,
    /// The scrap value at the request, in fils (the policy's condition).
    #[serde(default)]
    pub scrap_value: i64,
    /// The line of the rework work order.
    #[serde(default)]
    pub rework_line_id: Option<i64>,
    /// The BOM version of the rework work order.
    #[serde(default)]
    pub rework_bom_id: Option<i64>,
    /// `close` and `reopen`: whether the action of the disposition happened.
    #[serde(default)]
    pub action_done: bool,
}

/// The input of a run; the default without one.
pub(crate) fn input_of(ctx: &TransitionCtx) -> NcrInput {
    ctx.input_as::<NcrInput>().unwrap_or_default()
}

/// A column that is set to nothing again.
fn nothing() -> Option<String> {
    None
}

/// The rules every disposition shares, and its effect.
fn disposing(transition: Transition<Ncr>) -> Transition<Ncr> {
    transition
        .from([NcrStatus::Open])
        .to(NcrStatus::Dispositioned)
        .guard("ready", |_ncr: &Ncr, ctx| {
            let input = input_of(ctx);
            Box::pin(async move {
                if !input.problems.is_empty() {
                    Err(input.problems.join("\n"))
                } else if input.disposition.is_none() {
                    Err(english("manufacturing.ncr.choose", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|ncr: &Ncr, ctx, tx| {
            let (id, quantity) = (ncr.id, ncr.quantity_milli);
            let input = input_of(ctx);
            Box::pin(async move {
                let Some(kind) = input.disposition else {
                    return Ok(());
                };
                let stamp = now();
                Ncr::query()
                    .where_(Ncr::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("disposition", &Some(kind.as_str().to_owned())),
                            ("outcome", &input.outcome),
                            ("disposition_note", &input.note),
                            ("dispositioned_by", &input.by),
                            ("dispositioned_at", &Some(stamp)),
                            ("rework_line_id", &input.rework_line_id),
                            ("rework_bom_id", &input.rework_bom_id),
                            ("scrap_value", &input.scrap_value),
                            ("updated_at", &stamp),
                        ],
                    )
                    .await?;
                NcrDecision {
                    ncr_id: id,
                    disposition: kind,
                    outcome: input.outcome.clone(),
                    quantity_milli: quantity,
                    decided_by: input.by,
                    decided_at: stamp,
                    ..NcrDecision::default()
                }
                .insert(&mut *tx)
                .await?;
                Ok(())
            })
        })
}

/// `reopen` and `auto_reopen`: the disposition's action did not happen.
fn reopening(transition: Transition<Ncr>) -> Transition<Ncr> {
    transition
        .from([NcrStatus::Dispositioned])
        .to(NcrStatus::Open)
        .reason_required()
        .guard("not_done", |_ncr: &Ncr, ctx| {
            let done = input_of(ctx).action_done;
            Box::pin(async move {
                if done {
                    Err(english("manufacturing.ncr.reopen_done", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|ncr: &Ncr, ctx, tx| {
            let id = ncr.id;
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            Box::pin(async move {
                let stamp = now();
                NcrDecision::query()
                    .where_(NcrDecision::NCR_ID.eq(id))
                    .where_(NcrDecision::ENDED_AT.is_null())
                    .update(
                        &mut *tx,
                        &[
                            ("ended_at", &Some(stamp)),
                            ("ended_reason", &reason),
                            ("updated_at", &stamp),
                        ],
                    )
                    .await?;
                let none_time: Option<DateTime> = None;
                let none_id: Option<i64> = None;
                Ncr::query()
                    .where_(Ncr::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("disposition", &nothing()),
                            ("outcome", &nothing()),
                            ("disposition_note", &nothing()),
                            ("dispositioned_by", &none_id),
                            ("dispositioned_at", &none_time),
                            ("rework_line_id", &none_id),
                            ("rework_bom_id", &none_id),
                            ("rework_work_order_id", &none_id),
                            ("return_type", &nothing()),
                            ("return_id", &none_id),
                            ("scrap_value", &0_i64),
                            ("action_claimed_at", &none_time),
                            ("action_done_at", &none_time),
                            ("updated_at", &stamp),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

impl StateMachine for Ncr {
    type State = NcrStatus;
    const COLUMN: renox::db::Col<Ncr, NcrStatus> = Ncr::STATUS;

    fn machine() -> Machine<Self> {
        use NcrStatus::{Cancelled, Closed, Dispositioned, Open};
        let mut machine = Machine::new()
            .initial([Open])
            .state(Open)
            .tone("error")
            .state(Dispositioned)
            .tone("warning")
            .state(Closed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal();
        for name in [DISPOSITION, DISPOSITION_SCRAP, DISPOSITION_USE_AS_IS] {
            machine = machine.transition(disposing(
                Transition::new(name)
                    .label("Disposition")
                    .tone("primary")
                    .permission_in(permissions::NCRS_DISPOSITION, |ncr: &Ncr| ncr.scope()),
            ));
        }
        for name in [DISPOSITION_SCRAP, DISPOSITION_USE_AS_IS] {
            machine = machine.transition(
                disposing(Transition::new(approved(name)).label("Disposition (approved)")).system(),
            );
        }
        machine
            .transition(
                Transition::new(CLOSE)
                    .label("Close")
                    .tone("success")
                    .from([Dispositioned])
                    .to(Closed)
                    .permission_in(permissions::NCRS_DISPOSITION, |ncr: &Ncr| ncr.scope())
                    .guard("action_done", |_ncr: &Ncr, ctx| {
                        let done = input_of(ctx).action_done;
                        Box::pin(async move {
                            if done {
                                Ok(())
                            } else {
                                Err(english("manufacturing.ncr.close_not_done", &[]))
                            }
                        })
                    })
                    .effect(|ncr: &Ncr, ctx, tx| {
                        let id = ncr.id;
                        let by = input_of(ctx).by;
                        Box::pin(async move {
                            let stamp = now();
                            Ncr::query()
                                .where_(Ncr::ID.eq(id))
                                .update(
                                    &mut *tx,
                                    &[
                                        ("closed_by", &by),
                                        ("closed_at", &Some(stamp)),
                                        ("updated_at", &stamp),
                                    ],
                                )
                                .await?;
                            Ok(())
                        })
                    }),
            )
            .transition(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .tone("danger")
                    .from([Open])
                    .to(Cancelled)
                    .reason_required()
                    .permission_in(permissions::NCRS_DISPOSITION, |ncr: &Ncr| ncr.scope())
                    .effect(|ncr: &Ncr, ctx, tx| {
                        let id = ncr.id;
                        let reason = ctx.reason().map(|r| r.trim().to_owned());
                        Box::pin(async move {
                            let stamp = now();
                            Ncr::query()
                                .where_(Ncr::ID.eq(id))
                                .update(
                                    &mut *tx,
                                    &[("cancel_reason", &reason), ("updated_at", &stamp)],
                                )
                                .await?;
                            Ok(())
                        })
                    }),
            )
            .transition(reopening(
                Transition::new(REOPEN)
                    .label("Reopen")
                    .tone("warning")
                    .permission_in(permissions::NCRS_DISPOSITION, |ncr: &Ncr| ncr.scope()),
            ))
            .transition(reopening(Transition::new(AUTO_REOPEN).label("Reopen")).system())
    }
}

/// The default approval policies: a scrap above AED 2,000 needs a holder of
/// `stock.approve` (`STK.ACL.02`); *use as is* needs a holder of
/// `ncrs.approve` (`MFG.ACL.03`), never the user who asks.
pub fn policies() -> Vec<PolicyDefault> {
    vec![
        PolicyDefault::new(SCRAP_ACTION)
            .amount_above(SCRAP_APPROVAL_FILS)
            .input_key(SCRAP_KEY)
            .step_permission(permissions::STOCK_APPROVE),
        PolicyDefault::new(USE_AS_IS_ACTION).step_permission(permissions::NCRS_APPROVE),
    ]
}

/// The NCR in the kernel's registry, with the plant as the scope where
/// approvers are looked for.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<Ncr>(SUBJECT_TYPE)
        .action(DISPOSITION_SCRAP, SCRAP_ACTION)
        .action(DISPOSITION_USE_AS_IS, USE_AS_IS_ACTION)
        .scope(|ncr: &Ncr| ncr.scope())
        .build()
}

/// Registers the machine with the kernel and with `rnx state:diagram`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<Ncr>();
    for policy in policies() {
        policy.register();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_and_transition_of_the_spec_is_declared() {
        let machine = Ncr::machine();
        let names: Vec<&str> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["open", "dispositioned", "closed", "cancelled"]);
        let terminal: Vec<&str> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["closed", "cancelled"]);
        let table: [(&str, &str, &str, bool); 9] = [
            (DISPOSITION, "open", "dispositioned", false),
            (DISPOSITION_SCRAP, "open", "dispositioned", false),
            (DISPOSITION_USE_AS_IS, "open", "dispositioned", false),
            ("disposition_scrap_approved", "open", "dispositioned", true),
            (
                "disposition_use_as_is_approved",
                "open",
                "dispositioned",
                true,
            ),
            (CLOSE, "dispositioned", "closed", false),
            (CANCEL, "open", "cancelled", false),
            (REOPEN, "dispositioned", "open", false),
            (AUTO_REOPEN, "dispositioned", "open", true),
        ];
        for (name, from, to, system) in table {
            let t = machine.find(name).expect(name);
            assert_eq!(t.from_states(), [from], "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
            assert_eq!(t.is_system(), system, "{name}");
        }
        assert_eq!(machine.transitions().len(), table.len());
    }

    #[test]
    fn an_ncr_keeps_its_lot_until_the_action_happened() {
        let mut ncr = Ncr::default();
        assert!(ncr.keeps_lot());
        ncr.status = NcrStatus::Dispositioned;
        assert!(ncr.keeps_lot());
        ncr.action_done_at = Some(now());
        assert!(!ncr.keeps_lot());
        ncr.status = NcrStatus::Cancelled;
        assert!(!ncr.keeps_lot());
    }

    #[test]
    fn the_scrap_policy_reads_the_scrap_value() {
        let input = NcrInput {
            scrap_value: 250_000,
            ..NcrInput::default()
        };
        let json = renox::serde_json::to_value(&input).unwrap();
        assert_eq!(json[SCRAP_KEY], 250_000);
        assert_eq!(policies().len(), 2);
    }
}
