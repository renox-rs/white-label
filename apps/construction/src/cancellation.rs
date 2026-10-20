//! Cancelling a project before work started (`cancel`, 02 Construction §7.1
//! *Project*; BL-CON-02). A project with work done always ends through
//! `terminate`, so `cancel` is possible only from `setup`, and only while no
//! attendance, no goods receipt and no IPA exists for the project.
//!
//! The MD (`projects.cancel`) runs [`cancel`] with a reason. The facts are
//! read first (attendance and goods receipts live in their modules, behind
//! the kernel's `AttendanceReview` and `ScopeStock`) and the transition's
//! guard refuses when one exists. After the commit the open purchase orders
//! of the project are **flagged for cancellation**: procurement gets one task
//! listing them, because the supplier must be told; nothing is cancelled
//! here.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::attendance::attendance_review;
use wl_kernel::inventory::{OpenOrder, scope_stock};

use crate::ipa;
use crate::permissions;
use crate::project::{CANCEL, Project};
use crate::tasks::{self, Task};
use crate::texts::english;

/// What a `cancel` run carries to its guard: the facts the caller read,
/// because a guard cannot look them up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelInput {
    /// Whether the facts were read; a run without them is refused.
    #[serde(default)]
    pub checked: bool,
    /// Attendance was recorded at a work location of the project.
    #[serde(default)]
    pub attendance: bool,
    /// A goods receipt was recorded at a store of the project.
    #[serde(default)]
    pub goods_receipt: bool,
    /// An IPA exists for the project.
    #[serde(default)]
    pub ipa: bool,
}

/// The input of a `cancel` run.
pub(crate) fn cancel_input(ctx: &TransitionCtx) -> CancelInput {
    ctx.input_as::<CancelInput>().unwrap_or_default()
}

/// Why the project cannot be cancelled with `input`, if it cannot: work was
/// recorded, or the facts were not read.
pub fn cancel_problem(input: &CancelInput) -> Option<String> {
    if !input.checked {
        return Some(english("construction.bad_input", &[]));
    }
    if input.attendance {
        return Some(english("construction.cancel.has_attendance", &[]));
    }
    if input.goods_receipt {
        return Some(english("construction.cancel.has_receipt", &[]));
    }
    if input.ipa {
        return Some(english("construction.cancel.has_ipa", &[]));
    }
    None
}

/// The effect of `cancel` inside its transaction: the reason is kept.
pub(crate) async fn cancel_effects(
    tx: &mut Transaction,
    id: i64,
    reason: Option<String>,
) -> Result {
    Project::query()
        .where_(Project::ID.eq(id))
        .update(&mut *tx, &[("cancel_reason", &reason)])
        .await?;
    Ok(())
}

/// A cancelled project and the orders flagged for cancellation.
#[derive(Debug)]
pub struct Cancelled {
    /// The project, `cancelled`.
    pub project: Project,
    /// The open purchase orders procurement was told to cancel.
    pub orders: Vec<OpenOrder>,
}

/// `user` cancels project `id` with `reason`.
///
/// # Errors
///
/// `Forbidden` without `projects.cancel`, `NotFrom` unless the project is in
/// `setup`, `ReasonRequired` without a reason, `Guard` (422) when attendance,
/// a goods receipt or an IPA exists. Nothing changes then.
pub async fn cancel(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<Cancelled, TransitionError> {
    let project = Project::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !user.has_permission_in(permissions::PROJECTS_CANCEL, &project.scope()) {
        return Err(TransitionError::Forbidden);
    }
    let subject = project.subject();
    let input = CancelInput {
        checked: true,
        attendance: attendance_review(state)
            .has_records(state, &subject)
            .await?,
        goods_receipt: scope_stock(state)
            .has_goods_receipts(state, &subject)
            .await?,
        ipa: ipa::count_of(&state.db, id).await? > 0,
    };
    let _outcome = project
        .transition(CANCEL)
        .by(user)
        .reason(reason)
        .input(&input)
        .run(state)
        .await?;
    // The open orders are flagged, not cancelled: the supplier must be told.
    let orders = scope_stock(state).open_orders(state, &subject).await?;
    let cancelled = Project::find_or_404(&state.db, id).await?;
    if !orders.is_empty() {
        let listed: Vec<String> = orders
            .iter()
            .map(|order| {
                order
                    .number
                    .clone()
                    .unwrap_or_else(|| order.order.to_string())
            })
            .collect();
        let name = cancelled.name.clone();
        tasks::open(
            state,
            &cancelled,
            Task {
                subject: subject.clone(),
                role: permissions::PROCUREMENT_ROLE,
                assignee: None,
                source_key: format!("construction.cancel_orders:{id}"),
                title: english("construction.cancel.orders_title", &[("project", &name)]),
                description: english(
                    "construction.cancel.orders_body",
                    &[("orders", &listed.join(", "))],
                ),
                due: None,
            },
        )
        .await?;
    }
    Ok(Cancelled {
        project: cancelled,
        orders,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked() -> CancelInput {
        CancelInput {
            checked: true,
            ..CancelInput::default()
        }
    }

    #[test]
    fn a_project_without_work_can_be_cancelled() {
        assert_eq!(cancel_problem(&checked()), None);
    }

    #[test]
    fn attendance_a_receipt_or_an_ipa_refuses_it() {
        let attendance = CancelInput {
            attendance: true,
            ..checked()
        };
        assert!(cancel_problem(&attendance).is_some_and(|p| p.contains("Attendance")));
        let receipt = CancelInput {
            goods_receipt: true,
            ..checked()
        };
        assert!(cancel_problem(&receipt).is_some_and(|p| p.contains("goods receipt")));
        let ipa = CancelInput {
            ipa: true,
            ..checked()
        };
        assert!(cancel_problem(&ipa).is_some_and(|p| p.contains("IPA")));
    }

    #[test]
    fn a_run_without_the_facts_is_refused() {
        assert!(cancel_problem(&CancelInput::default()).is_some());
    }
}
