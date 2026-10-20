//! The tasks and notices of the e-invoice transmissions (wl-invoicing §4, §5.12
//! step 3, §6 `FIN.RUL.16`, events `EInvoiceAccepted` and `EInvoiceRejected`).
//!
//! - **A rejection** (`reject`) raises one task for the invoice owner and the
//!   accountant with the error codes and messages ([`on_rejected`]); the issued
//!   document, its balance and its reminders do not change. A transmission has
//!   one such task: a new rejection after a `resubmit` updates it and opens it
//!   again. The task closes when the transmission is `accepted` or
//!   `superseded`.
//! - **A transmission not sent** H hours after `issue` (default 24), still
//!   `queued` or `submitted`, raises one task with the waiting time
//!   ([`run_stuck`], never a second for the same transmission).
//! - **An outage:** when every transmission of an entity created in the last H
//!   hours is still `queued` and one has waited H hours, the company admin is
//!   told, once per outage (the occurrence key is the start of the oldest
//!   waiting transmission, `run_rule`).
//! - The kernel events [`EInvoiceAccepted`] and [`EInvoiceRejected`] follow
//!   the commit of `accept` and `reject`.
//!
//! The tasks are the module's own [`InvoicingTask`]s (ticketing may be off),
//! shown on the task list and opened at the e-invoice board.

use renox::chrono::Duration;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::Transitioned;
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::einvoice::{EInvoiceAccepted, EInvoiceRejected};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest, fill};
use wl_kernel::scopes::COMPANY_ADMIN_ROLE;
use wl_kernel::{LegalEntity, Subject};

use crate::budget_notices::holders_of;
use crate::config_of;
use crate::credit_note::CreditNote;
use crate::einvoice::{
    ACCEPT, CREDIT_NOTE, EInvoiceTransmission, INVOICE, REJECT, SUPERSEDE, TransmissionStatus,
};
use crate::invoice::Invoice;
use crate::task::{ACCOUNTANT, InvoicingTask, TaskKind, TaskStatus};
use crate::texts::english;

/// The rule, as the specification names it.
pub const RULE: &str = "FIN.RUL.16";

/// The notification type of a task for the invoice owner and the accountant.
pub const TASK_NOTICE: &str = "invoicing.einvoice_task";

/// The notification type of the company admin's outage notice.
pub const OUTAGE_NOTICE: &str = "invoicing.einvoice_outage";

/// The module that sends the notices.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the types belong to.
const GROUP: &str = "einvoices";

/// The type of the task notice: staff, transactional, by e-mail.
pub fn task_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(TASK_NOTICE, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.einvoice.task_notice.subject")),
            english("wl_invoicing.einvoice.task_notice.body"),
        )
}

/// The type of the outage notice: staff, transactional, by e-mail.
pub fn outage_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(OUTAGE_NOTICE, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.einvoice.outage.subject")),
            english("wl_invoicing.einvoice.outage.body"),
        )
}

/// The document of `t` as the task and the notices name it: its number, the
/// user who owns it and the invoice it is about (for the task list).
struct Named {
    number: String,
    owner: Option<i64>,
    invoice_id: Option<i64>,
}

async fn name_of(db: &Db, t: &EInvoiceTransmission) -> Result<Named> {
    Ok(match t.document_type.as_str() {
        INVOICE => {
            let invoice = Invoice::find(db, t.document_id).await?;
            Named {
                number: invoice
                    .as_ref()
                    .and_then(|i| i.number.clone())
                    .unwrap_or_else(|| t.document().to_string()),
                owner: invoice.as_ref().and_then(|i| i.owner_id),
                invoice_id: Some(t.document_id),
            }
        }
        CREDIT_NOTE => {
            let note = CreditNote::find(db, t.document_id).await?;
            Named {
                number: note
                    .as_ref()
                    .and_then(|n| n.number.clone())
                    .unwrap_or_else(|| t.document().to_string()),
                owner: note.as_ref().and_then(|n| n.created_by),
                invoice_id: note.map(|n| n.invoice_id),
            }
        }
        _ => Named {
            number: t.document().to_string(),
            owner: None,
            invoice_id: None,
        },
    })
}

/// The message of the rejection task of `t` for the document `number`.
pub fn rejected_message(number: &str, codes: &[String], messages: &[String]) -> String {
    fill(
        english("wl_invoicing.einvoice.task.rejected"),
        &json!({
            "number": number,
            "codes": codes.join(", "),
            "messages": messages.join(" "),
        }),
    )
}

/// The message of the task for a transmission not sent.
pub fn stuck_message(number: &str, hours: i64, error: Option<&str>) -> String {
    fill(
        english("wl_invoicing.einvoice.task.stuck"),
        &json!({
            "number": number,
            "hours": hours,
            "error": error.unwrap_or("no answer"),
        }),
    )
}

/// Tells the document's owner and the accountants of a new task.
async fn tell(state: &AppState, t: &EInvoiceTransmission, named: &Named, message: &str) -> Result {
    let mut ids = holders_of(&state.db, &[ACCOUNTANT.to_owned()]).await?;
    ids.extend(named.owner);
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(TASK_NOTICE, t.subject()).data(json!({
        "number": named.number,
        "message": message,
        "link": "/einvoices",
    }));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Opens, or updates and reopens, the task `kind` of transmission `t`, and
/// tells the owner and the accountants when it is new or was closed.
async fn raise(
    state: &AppState,
    t: &EInvoiceTransmission,
    kind: TaskKind,
    message: String,
) -> Result<bool> {
    let named = name_of(&state.db, t).await?;
    let document = t.document();
    let mut task = InvoicingTask {
        kind,
        invoice_id: named.invoice_id,
        source_type: document.subject_type.clone(),
        source_id: document.subject_id,
        amount: 0,
        key: t.id.to_string(),
        message: message.clone(),
        role: ACCOUNTANT.to_owned(),
        status: TaskStatus::Open,
        ..Default::default()
    };
    match task.insert(&state.db).await {
        Ok(()) => {}
        Err(error) if error.is_unique_violation() => {
            let Some(mut existing) = InvoicingTask::query()
                .where_(InvoicingTask::KIND.eq(kind))
                .where_(InvoicingTask::KEY.eq(t.id.to_string().as_str()))
                .first(&state.db)
                .await?
            else {
                return Ok(false);
            };
            // One task per transmission: a stuck one is raised once, a
            // rejection updates its task.
            if kind == TaskKind::EinvoiceStuck {
                return Ok(false);
            }
            existing.message = message.clone();
            existing.status = TaskStatus::Open;
            existing.done_by = None;
            existing.done_at = None;
            existing
                .save_only(&state.db, &["message", "status", "done_by", "done_at"])
                .await?;
        }
        Err(error) => return Err(error),
    }
    tell(state, t, &named, &message).await?;
    Ok(true)
}

/// A rejection: the task for the invoice owner and the accountant with the
/// error codes. Returns whether a task was opened or reopened.
///
/// # Errors
///
/// A database or notifier error.
pub async fn on_rejected(state: &AppState, t: &EInvoiceTransmission) -> Result<bool> {
    let number = name_of(&state.db, t).await?.number;
    let message = rejected_message(&number, &t.error_codes.0, &t.error_messages.0);
    raise(state, t, TaskKind::EinvoiceRejected, message).await
}

/// Closes the open tasks of transmission `t` (it was accepted or superseded).
///
/// # Errors
///
/// A database error.
pub async fn close_tasks(state: &AppState, t: &EInvoiceTransmission) -> Result<usize> {
    let open = InvoicingTask::query()
        .where_(InvoicingTask::KIND.is_in([TaskKind::EinvoiceRejected, TaskKind::EinvoiceStuck]))
        .where_(InvoicingTask::KEY.eq(t.id.to_string().as_str()))
        .where_(InvoicingTask::STATUS.eq(TaskStatus::Open))
        .get(&state.db)
        .await?;
    let mut closed = 0;
    for mut task in open {
        task.status = TaskStatus::Done;
        task.done_at = Some(renox::db::now());
        task.save_only(&state.db, &["status", "done_at"]).await?;
        closed += 1;
    }
    Ok(closed)
}

/// `FIN.RUL.16` for the transmissions not sent after H hours: one task each,
/// and the outage notice. Returns how many tasks it raised.
///
/// # Errors
///
/// A database or notifier error.
pub async fn run_stuck(state: &AppState) -> Result<usize> {
    let config = config_of(state);
    let hours = config.einvoice_task_hours;
    let cutoff = renox::db::now() - Duration::hours(hours);
    let waiting = EInvoiceTransmission::query()
        .where_(
            EInvoiceTransmission::STATUS
                .is_in([TransmissionStatus::Queued, TransmissionStatus::Submitted]),
        )
        .where_(EInvoiceTransmission::QUEUED_AT.lte(cutoff))
        .order_by(EInvoiceTransmission::ID)
        .get(&state.db)
        .await?;
    let mut raised = 0;
    for t in &waiting {
        let number = name_of(&state.db, t).await?.number;
        let message = stuck_message(&number, hours, t.last_error.as_deref());
        if raise(state, t, TaskKind::EinvoiceStuck, message).await? {
            raised += 1;
        }
    }
    let mut entities: Vec<i64> = waiting
        .iter()
        .filter(|t| t.status == TransmissionStatus::Queued)
        .map(|t| t.legal_entity_id)
        .collect();
    entities.sort_unstable();
    entities.dedup();
    for entity in entities {
        outage(state, entity, hours, cutoff).await?;
    }
    Ok(raised)
}

/// Tells the company admin that the ASP has been unreachable for all the
/// transmissions of `entity`, once per outage: every transmission created
/// since `cutoff` is still `queued`, and one has waited `hours` hours.
async fn outage(state: &AppState, entity: i64, hours: i64, cutoff: renox::db::DateTime) -> Result {
    let recent = EInvoiceTransmission::query()
        .where_(EInvoiceTransmission::LEGAL_ENTITY_ID.eq(entity))
        .where_(EInvoiceTransmission::QUEUED_AT.gt(cutoff))
        .get(&state.db)
        .await?;
    if recent
        .iter()
        .any(|t| t.status != TransmissionStatus::Queued)
    {
        return Ok(());
    }
    let Some(oldest) = EInvoiceTransmission::query()
        .where_(EInvoiceTransmission::LEGAL_ENTITY_ID.eq(entity))
        .where_(EInvoiceTransmission::STATUS.eq(TransmissionStatus::Queued))
        .order_by(EInvoiceTransmission::QUEUED_AT)
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    let Some(legal_entity) = LegalEntity::find(&state.db, entity).await? else {
        return Ok(());
    };
    let spec = RuleRunSpec::new(
        RULE,
        Subject::new("kernel.legal_entity", entity),
        format!("outage:{}", oldest.queued_at.timestamp()),
    );
    let code = legal_entity.code.clone();
    run_rule(state, spec, move |state| async move {
        let admins = holders_of(&state.db, &[COMPANY_ADMIN_ROLE.to_owned()]).await?;
        if admins.is_empty() {
            return Ok(());
        }
        let mut request =
            NotifyRequest::new(OUTAGE_NOTICE, Subject::new("kernel.legal_entity", entity)).data(
                json!({
                    "entity": code,
                    "hours": hours,
                    "link": "/einvoices",
                }),
            );
        for id in admins {
            request = request.to_user(id);
        }
        notify::notifier(&state).notify(&state, request).await?;
        Ok(())
    })
    .await?;
    Ok(())
}

/// Registers the notification types and the listener of the transmission's
/// transitions: the kernel events, the task of a rejection, the closing of
/// the tasks.
pub(crate) fn register(app: &mut Registry) {
    notify::register_type(task_notice_type());
    notify::register_type(outage_notice_type());
    app.listen(
        |event: Transitioned<EInvoiceTransmission>, state: AppState| async move {
            let name = event.transition.as_str();
            if ![ACCEPT, REJECT, SUPERSEDE].contains(&name) {
                return Ok(());
            }
            let Some(t) = EInvoiceTransmission::find(&state.db, event.key).await? else {
                return Ok(());
            };
            match name {
                ACCEPT => {
                    close_tasks(&state, &t).await?;
                    state
                        .emit(EInvoiceAccepted {
                            transmission: t.subject(),
                            document: t.document(),
                            legal_entity_id: t.legal_entity_id,
                            asp_reference: t.asp_reference.clone(),
                        })
                        .await
                }
                REJECT => {
                    on_rejected(&state, &t).await?;
                    state
                        .emit(EInvoiceRejected {
                            transmission: t.subject(),
                            document: t.document(),
                            legal_entity_id: t.legal_entity_id,
                            asp_reference: t.asp_reference.clone(),
                            error_codes: t.error_codes.0.clone(),
                        })
                        .await
                }
                _ => close_tasks(&state, &t).await.map(|_| ()),
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rejection_task_names_the_document_and_the_codes() {
        let text = rejected_message(
            "INV-PGP-2027-0001",
            &["BR-CO-15".to_owned(), "BR-S-08".to_owned()],
            &["Totals differ.".to_owned()],
        );
        assert!(text.contains("INV-PGP-2027-0001"), "{text}");
        assert!(text.contains("BR-CO-15, BR-S-08"), "{text}");
        assert!(text.contains("Totals differ."), "{text}");
    }

    #[test]
    fn the_stuck_task_names_the_waiting_time() {
        let text = stuck_message("INV-1", 24, Some("The ASP does not answer."));
        assert!(text.contains("24 hours"), "{text}");
        assert!(text.contains("The ASP does not answer."), "{text}");
        assert!(stuck_message("INV-1", 24, None).contains("no answer"));
    }

    #[test]
    fn the_notification_types_are_transactional_with_an_english_email() {
        for def in [task_notice_type(), outage_notice_type()] {
            assert_eq!(def.category, Category::Transactional);
            assert_eq!(def.channels, [NotifyChannel::Email]);
        }
        assert_eq!(task_notice_type().key, TASK_NOTICE);
    }
}
