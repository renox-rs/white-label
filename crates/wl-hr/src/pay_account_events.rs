//! What follows a pay account's transitions, after the commit (wl-hr §4
//! *Pay account*, §5.7, §6, issues #209 to #211). Listeners, not effects: a
//! notice that cannot be sent does not undo the change of the account.
//!
//! - **Request**: the approvers (`pay_accounts.approve`) are told ([`requested`]).
//! - **`approve`**: the change notice goes to the worker's mobile number on
//!   record before the request (and the e-mail address on record, where
//!   known), naming the last 4 digits, the date the account takes effect and a
//!   signed link to stop it. It goes through the kernel `Notifier`; the number
//!   comes from the row ([`PayAccount::notice_to`]), never from the request.
//!   With no number on record the notice is undelivered at once and HR gets a
//!   task to reach the worker.
//! - **`MessageGaveUp`** on that SMS: the account records *notice undelivered*,
//!   stays `cooling` and HR gets a task ([`on_gave_up`]). The type's own
//!   give-up reaction is `none`: this module reacts.
//! - **`reject`**: the requester is told the reason.
//! - **`stop`, `stop_by_worker`**: a confidential security task for the MD role
//!   (a ticket through the kernel `TaskCreator`) and the MD told.
//! - **`activate`, `waive_cooling_approved`**: `PayAccountChanged`, and the
//!   payroll lines the old account failed or held are released
//!   ([`crate::pay_account_use::on_activated`]).
//! - **`ApprovalDecided`** of a waiver is written to the audit log.

use std::time::Duration;

use renox::auth::permissions::{users_with_permission, users_with_role};
use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::Transitioned;
use wl_kernel::notify::{MessageGaveUp, NotifyChannel, NotifyRecipient};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::workforce::{PAY_ACCOUNT_SUBJECT_TYPE, PayAccountChanged};
use wl_kernel::{ApprovalDecided, ApprovalEnd, documents::today};

use crate::config::config_of;
use crate::employee::Employee;
use crate::notices;
use crate::pay_account::{
    ACTIVATE, APPROVE, PayAccount, PayAccountStatus, REJECT, STOP, STOP_BY_WORKER,
    WAIVE_COOLING_APPROVED,
};
use crate::permissions;
use crate::texts::english;

/// The source key of the task to reach the worker of account `id`.
pub fn reach_key(id: i64) -> String {
    format!("hr.pay_account_notice:{id}")
}

/// The source key of the security task of the stopped account `id`.
pub fn stop_key(id: i64) -> String {
    format!("hr.pay_account_stopped:{id}")
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<PayAccount>, state: AppState| async move {
            on_transitioned(&state, &event).await
        },
    );
    app.listen(
        |event: MessageGaveUp, state: AppState| async move { on_gave_up(&state, &event).await },
    );
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        if event.subject.subject_type != PAY_ACCOUNT_SUBJECT_TYPE {
            return Ok(());
        }
        audit_decided(&state, &event).await
    });
}

async fn on_transitioned(state: &AppState, event: &Transitioned<PayAccount>) -> Result {
    match event.transition.as_str() {
        APPROVE => approved(state, event.key).await,
        REJECT => rejected(state, event.key).await,
        STOP | STOP_BY_WORKER => stopped(state, event.key).await,
        ACTIVATE | WAIVE_COOLING_APPROVED => changed(state, event.key).await,
        _ => Ok(()),
    }
}

/// The approvers of a new request are told: the holders of
/// `pay_accounts.approve`, not the requester.
pub(crate) async fn requested(
    state: &AppState,
    employee: &Employee,
    account: &PayAccount,
) -> Result {
    let users: Vec<i64> = users_with_permission(&state.db, permissions::PAY_ACCOUNTS_APPROVE)
        .await?
        .iter()
        .map(|u| u.id)
        .filter(|id| Some(*id) != account.requested_by)
        .collect();
    notices::pay_account_requested(state, employee, account, &users).await
}

/// `approve`: the notice to the worker, or the task when there is nobody to
/// send it to.
async fn approved(state: &AppState, id: i64) -> Result {
    let Some(account) = PayAccount::find(&state.db, id).await? else {
        return Ok(());
    };
    let (Some(number), Some(ends)) = (account.notice_to.clone(), account.cooling_ends_on) else {
        return reach_task(state, &account).await;
    };
    let seconds = (ends - today()).num_days().max(1).unsigned_abs() * 24 * 3600;
    let link = state.signed_url(
        "wl_hr.pay_account_stop",
        &[&account.id],
        Duration::from_secs(seconds),
    )?;
    let messages = notices::pay_account_change(state, &account, &number, &link, ends).await?;
    PayAccount::query()
        .where_(PayAccount::ID.eq(id))
        .update(
            &state.db,
            &[
                ("notice_sent_to", &Some(number)),
                ("notice_message", &messages.first().map(|m| m.0)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// A task for the HR role to reach the worker another way, once.
async fn reach_task(state: &AppState, account: &PayAccount) -> Result {
    let name = match Employee::find(&state.db, account.employee_id).await? {
        Some(employee) => notices::name_of(state, &employee).await,
        None => format!("#{}", account.employee_id),
    };
    let ends = account
        .cooling_ends_on
        .map(|day| day.to_string())
        .unwrap_or_default();
    let title = english(
        "wl_hr.pay_account.task_title",
        &[("employee", &name), ("last4", &account.last4)],
    );
    let description = english("wl_hr.pay_account.task_description", &[("date", &ends)]);
    let task = RoleTask::new(
        account.subject(),
        &config_of(state).hr_role,
        &title,
        &reach_key(account.id),
    )
    .description(&description);
    role_tasks::open(state, task).await?;
    Ok(())
}

/// `MessageGaveUp` of the change notice's SMS: *notice undelivered*.
pub(crate) async fn on_gave_up(state: &AppState, event: &MessageGaveUp) -> Result {
    if event.type_key != notices::PAY_ACCOUNT_CHANGE
        || event.subject.subject_type != PAY_ACCOUNT_SUBJECT_TYPE
    {
        return Ok(());
    }
    // A give-up of the e-mail copy does not say the worker was not told.
    let sms = matches!(
        event.recipient,
        NotifyRecipient::Address {
            channel: NotifyChannel::Sms,
            ..
        }
    );
    if !sms {
        return Ok(());
    }
    let Some(account) = PayAccount::find(&state.db, event.subject.subject_id).await? else {
        return Ok(());
    };
    if account.status != PayAccountStatus::Cooling || account.notice_undelivered_at.is_some() {
        return Ok(());
    }
    PayAccount::query()
        .where_(PayAccount::ID.eq(account.id))
        .update(
            &state.db,
            &[
                ("notice_undelivered_at", &Some(event.gave_up_at)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    reach_task(state, &account).await
}

/// `reject`: the requester is told why.
async fn rejected(state: &AppState, id: i64) -> Result {
    let Some(account) = PayAccount::find(&state.db, id).await? else {
        return Ok(());
    };
    let Some(requester) = account.requested_by else {
        return Ok(());
    };
    let Some(employee) = Employee::find(&state.db, account.employee_id).await? else {
        return Ok(());
    };
    notices::pay_account_rejected(state, &employee, &account, &[requester]).await
}

/// `stop`: a security task for the MD role and the MD told.
async fn stopped(state: &AppState, id: i64) -> Result {
    let Some(account) = PayAccount::find(&state.db, id).await? else {
        return Ok(());
    };
    let Some(employee) = Employee::find(&state.db, account.employee_id).await? else {
        return Ok(());
    };
    let config = config_of(state);
    let name = notices::name_of(state, &employee).await;
    let by = if account.stopped_by_worker {
        english("wl_hr.pay_account.stopped_by_worker", &[])
    } else {
        english("wl_hr.pay_account.stopped_by_hr", &[])
    };
    let title = english(
        "wl_hr.pay_account.security_title",
        &[("employee", &name), ("last4", &account.last4)],
    );
    let description = english(
        "wl_hr.pay_account.security_description",
        &[("by", &by), ("last4", &account.last4)],
    );
    let task = RoleTask::new(account.subject(), &config.md_role, &title, &stop_key(id))
        .description(&description)
        .confidential();
    role_tasks::open(state, task).await?;
    let md: Vec<i64> = users_with_role(&state.db, &config.md_role)
        .await?
        .iter()
        .map(|u| u.id)
        .collect();
    notices::pay_account_stopped(state, &employee, &account, &by, &md).await
}

/// `activate` and the MD's waiver: `PayAccountChanged`, and the lines the old
/// account failed or held are released.
async fn changed(state: &AppState, id: i64) -> Result {
    let Some(account) = PayAccount::find(&state.db, id).await? else {
        return Ok(());
    };
    state
        .emit(PayAccountChanged {
            employee: wl_kernel::EmployeeId(account.employee_id),
            last4: account.last4.clone(),
            approver: account.approved_by,
        })
        .await?;
    crate::pay_account_use::on_activated(state, &account).await
}

/// The audit entry of a waiver's end: the approval, or why it ended.
async fn audit_decided(state: &AppState, event: &ApprovalDecided) -> Result {
    let action = if event.decision == ApprovalEnd::Approved {
        "hr.pay_account_waive_approved".to_owned()
    } else {
        format!("hr.pay_account_waive_{}", event.decision.as_str())
    };
    let mut entry = renox::audit::Entry::new(&action)
        .subject(PayAccount::TABLE, event.subject.subject_id)
        .data(json!({
            "approval_request": event.request,
            "approvers": event.approvers,
        }));
    if let Some(user) = event.approvers.last() {
        entry = entry.user(*user);
    }
    renox::audit::record(&state.db, entry).await?;
    Ok(())
}
