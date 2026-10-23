//! The Work order (07 Manufacturing §6 *Work order*, §7.1 *Work order*): an
//! instruction to produce a quantity of one item on one line, from a BOM
//! version, for a sales order or for stock.
//!
//! ```text
//! planned --release--> released --start--> in_progress <--resume-- on_hold
//!                                          in_progress --hold--> on_hold
//! in_progress --complete--> completed --close--> closed
//! planned, released, in_progress, on_hold --cancel--> cancelled   (nothing issued or reported)
//! planned, released, in_progress, on_hold --unlink--> the same state (source: sales order -> stock)
//! released, in_progress, on_hold --abort--> aborted               (after the approval, MFG.ACL.12)
//! ```
//!
//! Every state of the spec is declared. A Renox transition has one fixed
//! `to`, and a user and the system cannot share one, so:
//!
//! - `unlink` is four transitions, one per state it leaves and keeps
//!   ([`UNLINK`] for `planned`, [`UNLINK_RELEASED`], [`UNLINK_IN_PROGRESS`],
//!   [`UNLINK_ON_HOLD`]); [`unlink_name`] picks the one for a state, and each
//!   has a system twin (`auto_` before the name, `MFG.RUL.09`);
//! - `cancel` and `close` have a system twin (`auto_cancel`, `auto_close`);
//! - `abort` is deferred as an approval request for the production manager
//!   (`MFG.ACL.12`, [`crate::abort`]) and, at the final approval, the kernel
//!   runs `abort_approved`, which has the same guard and effect.
//!
//! The machine's user transitions are open to any signed-in user: the plant
//! and the line are two scopes, and [`crate::work_orders`] checks the
//! permission of the spec against both ([`WorkOrder::may`]).
//!
//! Comes with other issues (the states they need are declared): the void of a
//! production report (BL-MFG-04), the hold for material (BL-MFG-05, the
//! `hold` here takes the reason), `MFG.RUL.09` calling `unlink`, `cancel` and
//! the quantity change (BL-MFG-03).

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::inventory::ITEM_SUBJECT_TYPE;
use wl_kernel::machines::approved;
use wl_kernel::sales::SalesOrderId;
use wl_kernel::{Subject, SubjectMachine};

use crate::abort;
use crate::db::set_order;
use crate::permissions;
use crate::texts::english;

/// The subject type of a work order.
pub const SUBJECT_TYPE: &str = "manufacturing.work_order";

/// `release`.
pub const RELEASE: &str = "release";
/// `start`.
pub const START: &str = "start";
/// `hold`.
pub const HOLD: &str = "hold";
/// `resume`.
pub const RESUME: &str = "resume";
/// `complete`.
pub const COMPLETE: &str = "complete";
/// `close`.
pub const CLOSE: &str = "close";
/// The system's `close`.
pub const AUTO_CLOSE: &str = "auto_close";
/// `cancel`.
pub const CANCEL: &str = "cancel";
/// The system's `cancel` (`MFG.RUL.09`).
pub const AUTO_CANCEL: &str = "auto_cancel";
/// `unlink` of a `planned` work order.
pub const UNLINK: &str = "unlink";
/// `unlink` of a `released` work order.
pub const UNLINK_RELEASED: &str = "unlink_released";
/// `unlink` of an `in_progress` work order.
pub const UNLINK_IN_PROGRESS: &str = "unlink_in_progress";
/// `unlink` of an `on_hold` work order.
pub const UNLINK_ON_HOLD: &str = "unlink_on_hold";
/// `abort`.
pub const ABORT: &str = "abort";

/// Where a work order stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WorkOrderStatus {
    /// Created; not yet on the floor.
    #[default]
    Planned,
    /// On a line in the schedule; materials can be issued.
    Released,
    /// Production started.
    InProgress,
    /// Stopped for a reason: material, machine or quality.
    OnHold,
    /// Output reported; the output lot is in quarantine; locked.
    Completed,
    /// Actual cost calculated (terminal).
    Closed,
    /// Cancelled before any issue or output (terminal).
    Cancelled,
    /// Stopped after materials were issued (terminal).
    Aborted,
}

impl WorkOrderStatus {
    /// Whether the work order may still be stopped, held or unlinked.
    pub fn is_open(self) -> bool {
        matches!(
            self,
            WorkOrderStatus::Planned
                | WorkOrderStatus::Released
                | WorkOrderStatus::InProgress
                | WorkOrderStatus::OnHold
        )
    }
}

/// Where a work order's demand comes from.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WorkOrderSource {
    /// Minimum stock or the plan: the output goes to free stock.
    #[default]
    Stock,
    /// A sales order line.
    SalesOrder,
}

/// The reasons for a `hold` (§7.1).
pub const HOLD_REASONS: &[&str] = &["material", "machine", "quality"];

/// A work order.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manufacturing_work_orders",
    state_machine,
    index(line_id),
    index(status),
    unique(number)
)]
pub struct WorkOrder {
    /// The primary key.
    pub id: i64,
    /// The number, `WO-2026-0001`.
    pub number: String,
    /// The plant (the scope of the planner and the production manager).
    pub plant_id: i64,
    /// The line it is scheduled on; set before `release`.
    pub line_id: Option<i64>,
    /// The finished item, a wl-inventory item.
    pub item_id: i64,
    /// The BOM version, a wl-inventory BOM.
    pub bom_id: Option<i64>,
    /// The quantity to make, in thousandths.
    pub quantity_milli: i64,
    /// The part of the output that is for stock (an order was decreased).
    pub quantity_for_stock_milli: i64,
    /// The first planned day.
    pub planned_start: Option<NaiveDate>,
    /// The last planned day.
    pub planned_end: Option<NaiveDate>,
    /// A sales order line or stock.
    pub source: WorkOrderSource,
    /// The sales order, while the source is a sales order (kept after an
    /// `unlink` for the history).
    pub sales_order_id: Option<i64>,
    /// The sales order line.
    pub sales_order_line_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: WorkOrderStatus,
    /// Why it is `on_hold`: one of [`HOLD_REASONS`].
    pub hold_reason: Option<String>,
    /// The standard material cost of one unit at release, in fils.
    pub std_material_cost: i64,
    /// The standard cost of one unit at release (materials plus standard
    /// hours at the line rate), in fils.
    pub std_unit_cost: i64,
    /// When `start` ran.
    pub started_at: Option<DateTime>,
    /// When it stopped (`complete`, or the final approval of `abort`).
    pub stopped_at: Option<DateTime>,
    /// The output lot, a wl-inventory lot; opened by the first report.
    pub output_lot_id: Option<i64>,
    /// The good output reported, in thousandths.
    pub good_milli: i64,
    /// The scrap reported, in thousandths.
    pub scrap_milli: i64,
    /// Whether a material was issued to it.
    pub has_issue: bool,
    /// The actual cost once calculated, in fils.
    pub actual_cost: Option<i64>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// Why it was unlinked from its sales order.
    pub unlink_reason: Option<String>,
    /// When it was unlinked.
    pub unlinked_at: Option<DateTime>,
    /// Why it was aborted.
    pub abort_reason: Option<String>,
    /// The user who asked for the abort.
    pub abort_requested_by: Option<i64>,
    /// The user who approved it last.
    pub abort_approved_by: Option<i64>,
    /// The scrap value at the abort, in fils.
    pub abort_scrap_value: Option<i64>,
    /// The loss of the abort, in fils; negative is a gain.
    pub abort_loss: Option<i64>,
    /// When the abort was final.
    pub aborted_at: Option<DateTime>,
    /// When the abort was announced (sales told, event emitted).
    pub abort_announced_at: Option<DateTime>,
    /// The user who created it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl WorkOrder {
    /// The work order as a [`Subject`]: `manufacturing.work_order:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The finished item as a subject.
    pub fn item(&self) -> Subject {
        Subject::new(ITEM_SUBJECT_TYPE, self.item_id)
    }

    /// The scope the approval looks for approvers in: the plant.
    pub fn scope(&self) -> Scope {
        permissions::plant_scope(self.plant_id)
    }

    /// Whether `user` holds `permission` in the plant of the work order or in
    /// its line (`MFG.ACL.01`).
    pub fn may(&self, user: &AuthUser, permission: &str) -> bool {
        user.has_permission_in(permission, &self.scope())
            || self.line_id.is_some_and(|line| {
                user.has_permission_in(permission, &permissions::line_scope(line))
            })
    }

    /// Whether `user` may see the work order: any permission of the vertical
    /// that acts on it, in its plant or its line.
    pub fn may_see(&self, user: &AuthUser) -> bool {
        [
            permissions::WORK_ORDERS_VIEW,
            permissions::WORK_ORDERS_CREATE,
            permissions::WORK_ORDERS_RELEASE,
            permissions::WORK_ORDERS_REPORT,
            permissions::WORK_ORDERS_ISSUE,
            permissions::WORK_ORDERS_CLOSE,
            permissions::WORK_ORDERS_ABORT_APPROVE,
        ]
        .iter()
        .any(|permission| self.may(user, permission))
    }

    /// The sales order the work order serves, while it is linked.
    pub fn order(&self) -> Option<SalesOrderId> {
        match (self.source, self.sales_order_id) {
            (WorkOrderSource::SalesOrder, Some(id)) => Some(SalesOrderId(id)),
            _ => None,
        }
    }

    /// Whether a material was issued or output reported: such a work order
    /// is stopped only by `abort` (`MFG.ACL.12`).
    pub fn has_work_in_progress(&self) -> bool {
        self.has_issue || self.good_milli > 0 || self.scrap_milli > 0
    }

    /// Whether the work order *started* (§7.1): `in_progress` or `on_hold`,
    /// or material issued or output reported.
    pub fn started(&self) -> bool {
        matches!(
            self.status,
            WorkOrderStatus::InProgress | WorkOrderStatus::OnHold
        ) || self.has_work_in_progress()
    }
}

/// The name of the `unlink` transition that leaves and keeps `status`.
pub fn unlink_name(status: WorkOrderStatus) -> Option<&'static str> {
    match status {
        WorkOrderStatus::Planned => Some(UNLINK),
        WorkOrderStatus::Released => Some(UNLINK_RELEASED),
        WorkOrderStatus::InProgress => Some(UNLINK_IN_PROGRESS),
        WorkOrderStatus::OnHold => Some(UNLINK_ON_HOLD),
        _ => None,
    }
}

/// The system's twin of a user transition.
pub fn auto(name: &str) -> String {
    format!("auto_{name}")
}

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderInput {
    /// Every check that fails, gathered by the service (a BOM that is not
    /// released, no line); the guard refuses while any is left.
    #[serde(default)]
    pub problems: Vec<String>,
    /// The user who runs it, for the record.
    #[serde(default)]
    pub by: Option<i64>,
    /// `hold`: the reason, one of [`HOLD_REASONS`].
    #[serde(default)]
    pub hold_reason: Option<String>,
    /// `release`: the pick list, `(item, quantity in thousandths)`.
    #[serde(default)]
    pub pick: Vec<(i64, i64)>,
    /// `release`: the standard material cost of one unit, in fils, for the
    /// work order to keep.
    #[serde(default)]
    pub std_material_cost: Option<i64>,
    /// `release`: the standard cost of one unit with the hours of the line.
    #[serde(default)]
    pub std_unit_cost: Option<i64>,
}

/// The input of a run; the default without one.
pub(crate) fn input_of(ctx: &TransitionCtx) -> OrderInput {
    ctx.input_as::<OrderInput>().unwrap_or_default()
}

fn problems_guard(transition: Transition<WorkOrder>) -> Transition<WorkOrder> {
    transition.guard("ready", |_order: &WorkOrder, ctx| {
        let problems = input_of(ctx).problems;
        Box::pin(async move {
            if problems.is_empty() {
                Ok(())
            } else {
                Err(problems.join("\n"))
            }
        })
    })
}

const OPEN: [WorkOrderStatus; 4] = [
    WorkOrderStatus::Planned,
    WorkOrderStatus::Released,
    WorkOrderStatus::InProgress,
    WorkOrderStatus::OnHold,
];

fn cancelling(transition: Transition<WorkOrder>) -> Transition<WorkOrder> {
    transition
        .from(OPEN)
        .to(WorkOrderStatus::Cancelled)
        .guard("nothing_issued", |order: &WorkOrder, _ctx| {
            let started_work = order.has_work_in_progress();
            Box::pin(async move {
                if started_work {
                    Err(english("manufacturing.cancel.work_started", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .guard("reason", |order: &WorkOrder, ctx| {
            let needs = matches!(
                order.status,
                WorkOrderStatus::InProgress | WorkOrderStatus::OnHold
            );
            let given = ctx.reason().is_some_and(|r| !r.trim().is_empty());
            Box::pin(async move {
                if needs && !given {
                    Err(english("manufacturing.problems.reason", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|order: &WorkOrder, ctx, tx| {
            let id = order.id;
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            let none: Option<i64> = None;
            Box::pin(async move {
                set_order(
                    &mut *tx,
                    id,
                    &[
                        ("cancel_reason", &reason),
                        ("sales_order_id", &none),
                        ("sales_order_line_id", &none),
                        ("source", &WorkOrderSource::Stock),
                    ],
                )
                .await?;
                crate::pick_list::withdraw(&mut *tx, id).await
            })
        })
}

fn unlinking(transition: Transition<WorkOrder>, status: WorkOrderStatus) -> Transition<WorkOrder> {
    transition
        .from([status])
        .to(status)
        .guard("reason", |_order: &WorkOrder, ctx| {
            let given = ctx.reason().is_some_and(|r| !r.trim().is_empty());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("manufacturing.problems.reason", &[]))
                }
            })
        })
        .guard("linked", |order: &WorkOrder, _ctx| {
            let linked = order.source == WorkOrderSource::SalesOrder;
            Box::pin(async move {
                if linked {
                    Ok(())
                } else {
                    Err(english("manufacturing.unlink.not_linked", &[]))
                }
            })
        })
        .effect(|order: &WorkOrder, ctx, tx| {
            let id = order.id;
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            Box::pin(async move {
                set_order(
                    &mut *tx,
                    id,
                    &[
                        ("source", &WorkOrderSource::Stock),
                        ("unlink_reason", &reason),
                        ("unlinked_at", &Some(renox::db::now())),
                    ],
                )
                .await
            })
        })
}

fn aborting(transition: Transition<WorkOrder>) -> Transition<WorkOrder> {
    transition
        .from([
            WorkOrderStatus::Released,
            WorkOrderStatus::InProgress,
            WorkOrderStatus::OnHold,
        ])
        .to(WorkOrderStatus::Aborted)
        .guard("abortable", |order: &WorkOrder, ctx| {
            let verdict = abort::guard(order, ctx);
            Box::pin(async move { verdict })
        })
        .effect(|order: &WorkOrder, ctx, tx| {
            let order = order.clone();
            let input = abort::input_of(ctx);
            Box::pin(async move { abort::effects(tx, &order, &input).await })
        })
}

impl StateMachine for WorkOrder {
    type State = WorkOrderStatus;
    const COLUMN: renox::db::Col<WorkOrder, WorkOrderStatus> = WorkOrder::STATUS;

    fn machine() -> Machine<Self> {
        use WorkOrderStatus::{
            Aborted, Cancelled, Closed, Completed, InProgress, OnHold, Planned, Released,
        };
        let mut machine = Machine::new()
            .initial([Planned])
            .state(Planned)
            .tone("neutral")
            .state(Released)
            .tone("info")
            .state(InProgress)
            .tone("info")
            .state(OnHold)
            .tone("warning")
            .state(Completed)
            .tone("success")
            .state(Closed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .state(Aborted)
            .tone("error")
            .terminal()
            .transition(problems_guard(
                Transition::new(RELEASE)
                    .label("Release")
                    .tone("primary")
                    .from([Planned])
                    .to(Released)
                    .effect(|order: &WorkOrder, ctx, tx| {
                        let id = order.id;
                        let input = input_of(ctx);
                        Box::pin(async move {
                            set_order(
                                &mut *tx,
                                id,
                                &[
                                    ("std_material_cost", &input.std_material_cost.unwrap_or(0)),
                                    ("std_unit_cost", &input.std_unit_cost.unwrap_or(0)),
                                ],
                            )
                            .await?;
                            crate::pick_list::generate(&mut *tx, id, &input.pick).await
                        })
                    }),
            ))
            .transition(
                Transition::new(START)
                    .label("Start")
                    .tone("primary")
                    .from([Released])
                    .to(InProgress)
                    .effect(|order: &WorkOrder, _ctx, tx| {
                        let id = order.id;
                        Box::pin(async move {
                            set_order(&mut *tx, id, &[("started_at", &Some(renox::db::now()))])
                                .await
                        })
                    }),
            )
            .transition(
                Transition::new(HOLD)
                    .label("Hold")
                    .from([InProgress])
                    .to(OnHold)
                    .guard("reason", |_order: &WorkOrder, ctx| {
                        let ok = input_of(ctx)
                            .hold_reason
                            .is_some_and(|r| HOLD_REASONS.contains(&r.as_str()));
                        Box::pin(async move {
                            if ok {
                                Ok(())
                            } else {
                                Err(english("manufacturing.hold.reason", &[]))
                            }
                        })
                    })
                    .effect(|order: &WorkOrder, ctx, tx| {
                        let id = order.id;
                        let reason = input_of(ctx).hold_reason;
                        Box::pin(async move {
                            set_order(&mut *tx, id, &[("hold_reason", &reason)]).await
                        })
                    }),
            )
            .transition(
                Transition::new(RESUME)
                    .label("Resume")
                    .tone("primary")
                    .from([OnHold])
                    .to(InProgress)
                    .effect(|order: &WorkOrder, _ctx, tx| {
                        let id = order.id;
                        let none: Option<String> = None;
                        Box::pin(
                            async move { set_order(&mut *tx, id, &[("hold_reason", &none)]).await },
                        )
                    }),
            )
            .transition(
                Transition::new(COMPLETE)
                    .label("Complete")
                    .tone("primary")
                    .from([InProgress])
                    .to(Completed)
                    .guard("reported", |order: &WorkOrder, _ctx| {
                        let reported = order.good_milli > 0;
                        Box::pin(async move {
                            if reported {
                                Ok(())
                            } else {
                                Err(english("manufacturing.complete.nothing_reported", &[]))
                            }
                        })
                    })
                    .effect(|order: &WorkOrder, _ctx, tx| {
                        let id = order.id;
                        Box::pin(async move {
                            set_order(&mut *tx, id, &[("stopped_at", &Some(renox::db::now()))])
                                .await
                        })
                    }),
            );
        for (name, system) in [(CLOSE, false), (AUTO_CLOSE, true)] {
            let transition = problems_guard(
                Transition::new(name)
                    .label("Close")
                    .from([Completed])
                    .to(Closed)
                    .effect(|order: &WorkOrder, _ctx, tx| {
                        let id = order.id;
                        Box::pin(async move { crate::cost::close(&mut *tx, id).await })
                    }),
            );
            machine = machine.transition(if system {
                transition.system()
            } else {
                transition
            });
        }
        for (name, system) in [(CANCEL, false), (AUTO_CANCEL, true)] {
            let transition = cancelling(Transition::new(name).label("Cancel").tone("danger"));
            machine = machine.transition(if system {
                transition.system()
            } else {
                transition
            });
        }
        for status in [Planned, Released, InProgress, OnHold] {
            let name = unlink_name(status).unwrap_or(UNLINK);
            machine = machine.transition(unlinking(
                Transition::new(name).label("Unlink from the order"),
                status,
            ));
            machine = machine.transition(
                unlinking(
                    Transition::new(auto(name)).label("Unlink from the order"),
                    status,
                )
                .system(),
            );
        }
        machine
            .transition(aborting(
                Transition::new(ABORT).label("Abort").tone("danger"),
            ))
            .transition(
                aborting(Transition::new(approved(ABORT)).label("Abort (approved)")).system(),
            )
    }
}

/// The work order in the kernel's registry, with the plant as the scope where
/// approvers are looked for and `abort` under the action of `MFG.ACL.12`.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<WorkOrder>(SUBJECT_TYPE)
        .action(ABORT, abort::ACTION)
        .scope(|order: &WorkOrder| order.scope())
        .build()
}

/// Registers the machine with the kernel and with `rnx state:diagram`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<WorkOrder>();
    for policy in abort::policies() {
        policy.register();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_of_the_spec_is_declared_with_a_tone() {
        let machine = WorkOrder::machine();
        let names: Vec<&str> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "planned",
                "released",
                "in_progress",
                "on_hold",
                "completed",
                "closed",
                "cancelled",
                "aborted"
            ]
        );
        assert!(machine.states().iter().all(|s| s.tone.is_some()));
        let terminal: Vec<&str> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["closed", "cancelled", "aborted"]);
    }

    #[test]
    fn every_transition_of_the_spec_is_declared() {
        let machine = WorkOrder::machine();
        let open = ["planned", "released", "in_progress", "on_hold"];
        let table: [(&str, &[&str], &str, bool); 11] = [
            (RELEASE, &["planned"], "released", false),
            (START, &["released"], "in_progress", false),
            (HOLD, &["in_progress"], "on_hold", false),
            (RESUME, &["on_hold"], "in_progress", false),
            (COMPLETE, &["in_progress"], "completed", false),
            (CLOSE, &["completed"], "closed", false),
            (AUTO_CLOSE, &["completed"], "closed", true),
            (CANCEL, &open, "cancelled", false),
            (AUTO_CANCEL, &open, "cancelled", true),
            (
                ABORT,
                &["released", "in_progress", "on_hold"],
                "aborted",
                false,
            ),
            (
                "abort_approved",
                &["released", "in_progress", "on_hold"],
                "aborted",
                true,
            ),
        ];
        for (name, from, to, system) in &table {
            let t = machine.find(name).expect(name);
            assert_eq!(t.from_states(), *from, "{name}");
            assert_eq!(t.to_state(), Some(*to), "{name}");
            assert_eq!(t.is_system(), *system, "{name}");
        }
        for state in open {
            let name = unlink_name(state.parse_status()).unwrap();
            let t = machine.find(name).expect(name);
            assert_eq!(t.from_states(), [state]);
            assert_eq!(t.to_state(), Some(state));
            let twin = machine.find(&auto(name)).expect("the system twin");
            assert!(twin.is_system());
        }
        assert_eq!(machine.transitions().len(), table.len() + 8);
    }

    trait ParseStatus {
        fn parse_status(&self) -> WorkOrderStatus;
    }

    impl ParseStatus for &str {
        fn parse_status(&self) -> WorkOrderStatus {
            match *self {
                "planned" => WorkOrderStatus::Planned,
                "released" => WorkOrderStatus::Released,
                "in_progress" => WorkOrderStatus::InProgress,
                _ => WorkOrderStatus::OnHold,
            }
        }
    }

    #[test]
    fn a_started_work_order_is_in_progress_or_has_work_on_it() {
        let mut order = WorkOrder::default();
        assert!(!order.started());
        order.status = WorkOrderStatus::OnHold;
        assert!(order.started());
        order.status = WorkOrderStatus::Released;
        order.has_issue = true;
        assert!(order.started() && order.has_work_in_progress());
    }
}
