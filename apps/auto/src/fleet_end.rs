//! What follows the end of a fleet contract (08 §4.5 step 6, §7.1 *Fleet
//! contract* `end` and `terminate`, BL-AUT-13, BL-AUT-12).
//!
//! The machine ([`crate::fleet_contract_machine`]) records the end; the
//! effects that reach other modules are done here, after the commit, by the
//! listener of the transitions [`END`], [`TERMINATE`], [`TERMINATE_APPROVED`],
//! [`TERMINATE_SYSTEM`] and [`TERMINATE_BY_DECISION`] ([`wind_down`]), once and
//! in this order:
//!
//! 1. the bookings the service plans asked for **after the last day of
//!    service** are cancelled as the system (reason `fleet_contract_ended`; a
//!    booking on the last day itself stays) and every plan is stopped;
//! 2. the **final consolidated invoice** is drafted: from the day after the
//!    period invoiced last to the last day of service, with the jobs delivered
//!    by now ([`crate::fleet_billing::invoice_final`]);
//! 3. the **early termination fee** is invoiced to the fleet when the contract
//!    has one, unless the owner's approval waived it;
//! 4. the **fleet contact is told** that the contract ended;
//! 5. for a termination, [`FleetContractTerminated`] is emitted.
//!
//! Open job cards are not stopped: they finish, and a job card delivered after
//! the contract ended is billed on its own invoice at the rates it was priced
//! at ([`crate::billing::bill`]). A step that fails leaves the contract
//! without `wound_down_at`; the daily run tries it again ([`retry_all`]) and
//! each step is safe to repeat.
//!
//! A termination approved **after** its effective date passed, or one whose
//! effective date was before the day of the request, ends at once;
//! one whose effective date is today or later keeps its state until the first
//! daily run after it ([`after_scheduled`]).

use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::Transitioned;
use wl_kernel::approvals::{ApprovalDecided, ApprovalEnd};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::notify::{
    self, Category, GROUP_BOOKINGS, NotificationTypeDef, NotifyChannel, NotifyRequest, notifier,
};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::PartyId;

use crate::fleet_billing::{self, last_day};
use crate::fleet_contract::{
    FLEET_CONTRACT_SUBJECT_TYPE, FleetContract, FleetContractTerminated, FleetStatus,
};
use crate::fleet_contract_machine::{
    END, SCHEDULE_TERMINATION_APPROVED, SCHEDULE_TERMINATION_RENEWAL_APPROVED,
    SCHEDULE_TERMINATION_SUSPENDED_APPROVED, TERMINATE, TERMINATE_APPROVED, TERMINATE_BY_DECISION,
    TERMINATE_SYSTEM,
};
use crate::fleet_contracts::{due_transition, run_system};
use crate::fleet_invoice::{FleetInvoice, KIND_FINAL};
use crate::fleet_services;
use crate::permissions;
use crate::reasons::FLEET_CONTRACT_ENDED;
use crate::texts::english;

/// The notification type of the notice that a fleet contract ended (08 §7.1
/// *Notification types registered by 08*, group *Booking confirmations and
/// reminders*).
pub const NOTICE_TYPE: &str = "auto.fleet_contract_ended";

/// The notification type of the notice that a fleet contract was terminated:
/// a statutory notice (08 marks it so), whose give-up marks the contact invalid
/// and gives the Owner / GM the `statutory_notice` task.
pub const TERMINATION_TYPE: &str = "auto.fleet_termination_notice";

/// The notification type of the notice that the contract ended: an email, the
/// group's default reaction (the contact is marked invalid, no task); the
/// notice is also kept as the fleet account manager's task when it cannot be
/// queued at all.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE_TYPE, "auto")
        .group(GROUP_BOOKINGS)
        .category(Category::Transactional)
        .channels(&[NotifyChannel::Email])
        .template(
            NotifyChannel::Email,
            "en",
            Some("{{ title }}"),
            "{{ body }}",
        )
        .template(
            NotifyChannel::Email,
            "ar",
            Some("{{ title }}"),
            "{{ body }}",
        )
}

/// The notification type of the notice that the contract was terminated: a
/// statutory notice, an email; a message that gives up marks the contact
/// invalid and gives the Owner / GM the `statutory_notice` task.
pub fn termination_type() -> NotificationTypeDef {
    NotificationTypeDef::new(TERMINATION_TYPE, "auto")
        .group("statutory")
        .statutory(true)
        .category(Category::Transactional)
        .staff_role(permissions::OWNER_ROLE)
        .channels(&[NotifyChannel::Email])
        .template(
            NotifyChannel::Email,
            "en",
            Some("{{ title }}"),
            "{{ body }}",
        )
        .template(
            NotifyChannel::Email,
            "ar",
            Some("{{ title }}"),
            "{{ body }}",
        )
}

/// What winding a contract down came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WoundDown {
    /// The bookings cancelled.
    pub cancelled: usize,
    /// The final consolidated invoice, when one was drafted.
    pub final_invoice: Option<i64>,
    /// The invoice of the early termination fee, when there is one.
    pub fee_invoice: Option<i64>,
    /// The last day of service is not over: the last invoice, the fee and the
    /// notice wait for the first daily run after it.
    pub deferred: bool,
}

/// Does everything that follows the end of contract `id` (see the module
/// doc). A contract that is not `ended`, or that was wound down already, is
/// left as it is (`None`).
///
/// # Errors
///
/// The database's, the scheduling contract's and the invoicer's errors; a
/// failed step is tried again by the next call.
pub async fn wind_down(state: &AppState, id: i64) -> Result<Option<WoundDown>> {
    let contract = FleetContract::find_or_404(&state.db, id).await?;
    if contract.status != FleetStatus::Ended || contract.wound_down_at.is_some() {
        return Ok(None);
    }
    let last = last_day(&contract);
    let terminated = contract.termination_reason.is_some();
    let mut done = WoundDown {
        cancelled: fleet_services::cancel_bookings_after(
            state,
            id,
            FLEET_CONTRACT_ENDED,
            Some(last),
        )
        .await?,
        ..WoundDown::default()
    };
    fleet_services::stop_all_plans(&state.db, id).await?;
    if last >= today() {
        // An owner's decision ends a contract on its last day of service: the
        // period is not over, so the invoices follow the first run after it.
        done.deferred = true;
        return Ok(Some(done));
    }
    fleet_billing::invoice_final(state, &contract, last).await?;
    done.final_invoice = FleetInvoice::of_contract(&state.db, id)
        .await?
        .into_iter()
        .find(|invoice| invoice.kind == KIND_FINAL)
        .and_then(|invoice| invoice.invoice_id);
    if terminated {
        done.fee_invoice = invoice_fee(state, &contract, last).await?;
    }
    tell_contact(state, &contract, terminated, last).await?;
    FleetContract::query()
        .where_(FleetContract::ID.eq(id))
        .update(
            &state.db,
            &[("wound_down_at", &Some(now())), ("updated_at", &now())],
        )
        .await?;
    if terminated {
        let ended = FleetContract::find_or_404(&state.db, id).await?;
        state
            .emit(FleetContractTerminated {
                contract: ended.subject(),
                number: ended.number(),
                fleet_party_id: ended.fleet_party_id,
                reason: ended.termination_reason.clone(),
                effective_date: ended.effective_date,
                final_invoice: done.final_invoice,
                fee_invoice: done.fee_invoice,
            })
            .await?;
    }
    Ok(Some(done))
}

/// The early termination fee, invoiced to the fleet once: none when the
/// contract has no fee or the owner's approval waived it.
async fn invoice_fee(
    state: &AppState,
    contract: &FleetContract,
    last: renox::chrono::NaiveDate,
) -> Result<Option<i64>> {
    if contract.termination_fee_invoice_id.is_some() {
        return Ok(contract.termination_fee_invoice_id);
    }
    if contract.early_termination_fee <= 0 || contract.termination_fee_waived {
        return Ok(None);
    }
    let description = {
        let number = contract.number();
        english("auto.fleet.fee_line", &[("contract", &number)])
    };
    let draft = InvoiceDraft::new(PartyId(contract.fleet_party_id), contract.subject())
        .date_of_supply(last)
        .line(
            DraftLine::new(
                &description,
                contract.early_termination_fee,
                VatCategory::Standard,
            )
            .origin(contract.subject()),
        );
    let books = invoicer(state);
    let Some(invoice) = books.draft(&state.db, draft).await? else {
        return Ok(None);
    };
    FleetContract::query()
        .where_(FleetContract::ID.eq(contract.id))
        .update(
            &state.db,
            &[
                ("termination_fee_invoice_id", &Some(invoice.0)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    // The invoicing module issues a draft as the system only where it is set
    // to; otherwise the accountant issues it.
    let _issued = books.issue(state, invoice, None).await;
    Ok(Some(invoice.0))
}

/// Tells the fleet contact (the fleet itself when the contract names none)
/// that the contract ended, once. When the message cannot be queued the fleet
/// account manager gets a task to call.
async fn tell_contact(
    state: &AppState,
    contract: &FleetContract,
    terminated: bool,
    last: renox::chrono::NaiveDate,
) -> Result {
    if contract.contact_notified_at.is_some() {
        return Ok(());
    }
    let number = contract.number();
    let (title, body) = {
        let key = if terminated { "terminated" } else { "ended" };
        let params: [(&str, &dyn std::fmt::Display); 3] = [
            ("contract", &number),
            ("title", &contract.title),
            ("date", &last),
        ];
        (
            english(&format!("auto.fleet.notice_{key}_title"), &params),
            english(&format!("auto.fleet.notice_{key}_body"), &params),
        )
    };
    let party = PartyId(contract.contact_party_id.unwrap_or(contract.fleet_party_id));
    let type_key = if terminated {
        TERMINATION_TYPE
    } else {
        NOTICE_TYPE
    };
    let request = NotifyRequest::new(type_key, contract.subject())
        .subject_label(&number)
        .to_party(party)
        .data(json!({ "title": title, "body": body }));
    let queued = notifier(state).notify(state, request).await.is_ok();
    if !queued {
        let (task_title, task_body) = {
            let params: [(&str, &dyn std::fmt::Display); 1] = [("contract", &number)];
            (
                english("auto.fleet.notice_task_title", &params),
                english("auto.fleet.notice_task_body", &params),
            )
        };
        let task = RoleTask::new(
            contract.subject(),
            permissions::FLEET_ACCOUNT_MANAGER_ROLE,
            &task_title,
            &format!("auto.fleet_notice:{}", contract.id),
        )
        .description(&task_body);
        role_tasks::open(state, task).await?;
    }
    FleetContract::query()
        .where_(FleetContract::ID.eq(contract.id))
        .update(
            &state.db,
            &[("contact_notified_at", &Some(now())), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// A scheduled termination was approved: when its effective date is already
/// before today the contract ends now (the first daily run after the
/// effective date would have ended it); otherwise it keeps its state.
///
/// # Errors
///
/// The machine's refusal.
pub async fn after_scheduled(state: &AppState, id: i64) -> Result {
    let contract = FleetContract::find_or_404(&state.db, id).await?;
    let Some((name, input)) = due_transition(&contract, today()) else {
        return Ok(());
    };
    if name != TERMINATE_SYSTEM {
        return Ok(());
    }
    let reason = contract
        .termination_reason
        .clone()
        .unwrap_or_else(|| name.to_owned());
    run_system(state, &contract, name, Some(reason.as_str()), input).await?;
    Ok(())
}

/// Tries again the winding down of the contracts that ended and were not wound
/// down (a step failed): the daily run calls it. Answers the contracts done;
/// the first error is returned after all were tried.
///
/// # Errors
///
/// The first error of the contracts.
pub async fn retry_all(state: &AppState) -> Result<Vec<i64>> {
    let waiting = FleetContract::query()
        .where_(FleetContract::STATUS.eq(FleetStatus::Ended))
        .where_(FleetContract::WOUND_DOWN_AT.is_null())
        .order_by(FleetContract::ID)
        .get(&state.db)
        .await?;
    let mut done = Vec::new();
    let mut failure = None;
    for contract in waiting {
        match wind_down(state, contract.id).await {
            Ok(Some(wound)) if !wound.deferred => done.push(contract.id),
            Ok(Some(_) | None) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(done), Err)
}

/// Records who approved the termination (the owner), after the approval.
///
/// # Errors
///
/// The database's errors.
pub async fn on_decided(state: &AppState, event: &ApprovalDecided) -> Result {
    if event.subject.subject_type != FLEET_CONTRACT_SUBJECT_TYPE
        || event.decision != ApprovalEnd::Approved
    {
        return Ok(());
    }
    let approver = event.approvers.last().copied();
    FleetContract::query()
        .where_(FleetContract::ID.eq(event.subject.subject_id))
        .update(
            &state.db,
            &[
                ("termination_approved_by", &approver),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Registers the notification type and the listeners.
pub(crate) fn register(app: &mut Registry) {
    notify::register_type(notice_type());
    notify::register_type(termination_type());
    app.listen(
        |event: Transitioned<FleetContract>, state: AppState| async move {
            match event.transition.as_str() {
                END | TERMINATE | TERMINATE_APPROVED | TERMINATE_SYSTEM | TERMINATE_BY_DECISION => {
                    wind_down(&state, event.key).await.map(|_| ())
                }
                SCHEDULE_TERMINATION_APPROVED
                | SCHEDULE_TERMINATION_SUSPENDED_APPROVED
                | SCHEDULE_TERMINATION_RENEWAL_APPROVED => {
                    after_scheduled(&state, event.key).await
                }
                _ => Ok(()),
            }
        },
    );
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        on_decided(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::notify::GiveUpReaction;

    #[test]
    fn the_notice_is_an_email_in_the_bookings_group() {
        let def = notice_type();
        assert_eq!(def.channels, [NotifyChannel::Email]);
        assert!(def.template_for(NotifyChannel::Email, "ar").is_some());
        assert_eq!(def.group, GROUP_BOOKINGS);
        assert!(!def.statutory);
    }

    #[test]
    fn aut_18_the_termination_notice_is_statutory_and_tells_the_owner_on_give_up() {
        let def = termination_type();
        assert!(def.statutory);
        assert_eq!(def.staff_role.as_deref(), Some("owner"));
        let reactions = wl_kernel::notify::default_give_up(&def).reactions;
        assert_eq!(
            reactions,
            [
                GiveUpReaction::MarkContactInvalid,
                GiveUpReaction::StatutoryNotice
            ]
        );
    }
}
