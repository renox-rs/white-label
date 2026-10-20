//! The follow-up tasks of a reversal that cannot reopen its invoice
//! (wl-invoicing §4 `unallocate`, #70 A7).
//!
//! A refused unallocation opens one task for the `accountant` role:
//!
//! - `reversal_after_write_off`: the payment came before the write-off, so
//!   the invoice stays as it is and the accountant writes off more or
//!   pursues the party;
//! - `reversal_refused`: any other refusal (for example an amount above
//!   what is allocated from the payment);
//! - `bill_reversal_refused`: the refusal of a reversal on a supplier bill
//!   (#75 A8), for example an amount above what is applied to the bill.
//!
//! The record is the module's own because ticketing may be off. It is unique
//! per (kind, unallocation key), so a repeated delivery opens none twice.
//! The accountants get the bell notice [`REVERSAL_TASK`] after the task is
//! stored.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest, fill};

use crate::allocate::amount_text;
use crate::invoice::Invoice;
use crate::supplier_bill::SupplierBill;
use crate::texts::english;

/// The notification type of a new task.
pub const REVERSAL_TASK: &str = "invoicing.reversal_task";

/// The role that handles the tasks.
pub const ACCOUNTANT: &str = "accountant";

/// The module that sends the notice.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the type belongs to.
const GROUP: &str = "invoices";

/// What a task is about.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TaskKind {
    /// The reversed payment came before the invoice was written off.
    #[default]
    ReversalAfterWriteOff,
    /// The reversal was refused for another reason.
    ReversalRefused,
    /// The reversal of a payment on a supplier bill was refused.
    BillReversalRefused,
    /// A bank account has no balance, or only an old one, for the cash-flow
    /// forecast (`FIN.RUL.25`): import a statement or enter a balance.
    StatementBalanceStale,
    /// The ASP rejected an e-invoice transmission (`FIN.RUL.16`): fix the
    /// cause outside the document and resubmit, or credit the invoice.
    EinvoiceRejected,
    /// An e-invoice transmission is not sent H hours after `issue`
    /// (`FIN.RUL.16`).
    EinvoiceStuck,
    /// A delivery was put under review while a draft invoice bills its
    /// lines (#69): check the review before issuing the draft.
    DeliveryDisputedBeforeIssue,
    /// The delivery whose review flagged this invoice was reversed (#69):
    /// resolve the dispute, usually `credited` for the undelivered lines.
    DeliveryReversed,
    /// The delivery whose review flagged this invoice was lost (#69): resolve
    /// the dispute, usually `credited` for the undelivered lines.
    DeliveryLost,
}

impl TaskKind {
    /// The kind of task for the refusal of a reversal on `invoice`.
    pub fn of(invoice: &Invoice) -> Self {
        if invoice.status == InvoiceStatus::WrittenOff || invoice.write_off_amount.is_some() {
            TaskKind::ReversalAfterWriteOff
        } else {
            TaskKind::ReversalRefused
        }
    }

    /// The key of the task's message text.
    pub(crate) fn text_key(self) -> &'static str {
        match self {
            TaskKind::ReversalAfterWriteOff => "wl_invoicing.task.reversal_after_write_off",
            TaskKind::ReversalRefused => "wl_invoicing.task.reversal_refused",
            TaskKind::BillReversalRefused => "wl_invoicing.task.bill_reversal_refused",
            TaskKind::StatementBalanceStale => "wl_invoicing.task.statement_balance_stale",
            TaskKind::EinvoiceRejected => "wl_invoicing.einvoice.task.rejected",
            TaskKind::EinvoiceStuck => "wl_invoicing.einvoice.task.stuck",
            TaskKind::DeliveryDisputedBeforeIssue => "wl_invoicing.delivery.task.before_issue",
            TaskKind::DeliveryReversed => "wl_invoicing.delivery.task.reversed",
            TaskKind::DeliveryLost => "wl_invoicing.delivery.task.lost",
        }
    }
}

/// Whether a task still needs someone.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TaskStatus {
    /// Waiting for the accountant.
    #[default]
    Open,
    /// Handled.
    Done,
}

/// A follow-up task. Amounts are fils of the invoice's currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_tasks",
    unique(kind, key),
    index(invoice_id),
    index(bill_id)
)]
pub struct InvoicingTask {
    /// The primary key.
    pub id: i64,
    /// What it is about.
    pub kind: TaskKind,
    /// The invoice; `None` for a task of a supplier bill.
    pub invoice_id: Option<i64>,
    /// The supplier bill; `None` for a task of an invoice.
    pub bill_id: Option<i64>,
    /// The type of the money's source.
    pub source_type: String,
    /// The id of the money's source.
    pub source_id: i64,
    /// The amount of the refused reversal, in fils.
    pub amount: i64,
    /// The idempotency key of the refused unallocation.
    pub key: String,
    /// What the accountant should do, in words.
    pub message: String,
    /// The role that handles it.
    pub role: String,
    /// `open` or `done`.
    pub status: TaskStatus,
    /// The user who closed it.
    pub done_by: Option<i64>,
    /// When it was closed.
    pub done_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl InvoicingTask {
    /// Where the money came from.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }
}

/// The values the task message and notice texts are filled with.
fn data(invoice: &Invoice, source: &Subject, amount: i64, reason: &str) -> Value {
    let party_name = invoice
        .party_snapshot
        .as_ref()
        .map(|snapshot| snapshot.0.legal_name_en.clone())
        .unwrap_or_default();
    json!({
        "number": invoice.number.clone().unwrap_or_default(),
        "party_name": party_name,
        "source": source.to_string(),
        "amount": amount_text(&invoice.currency, amount),
        "reason": reason,
        "link": format!("/invoices/{}", invoice.id),
    })
}

/// The message of a task of `kind`; `reason` is the refusal's text.
pub fn message(
    kind: TaskKind,
    invoice: &Invoice,
    source: &Subject,
    amount: i64,
    reason: &str,
) -> String {
    fill(
        english(kind.text_key()),
        &data(invoice, source, amount, reason),
    )
}

/// Opens the task for the refusal `reason` of a reversal of `amount` from
/// `source` on `invoice`, keyed on the unallocation `key`, and tells the
/// accountants. `None` when that kind of task already exists for the key
/// (a repeated delivery): nothing is written or sent then.
///
/// # Errors
///
/// A database or notifier error.
pub async fn open(
    state: &AppState,
    invoice: &Invoice,
    source: &Subject,
    amount: i64,
    key: &str,
    reason: &str,
) -> Result<Option<InvoicingTask>> {
    let kind = TaskKind::of(invoice);
    let mut task = InvoicingTask {
        kind,
        invoice_id: Some(invoice.id),
        source_type: source.subject_type.clone(),
        source_id: source.subject_id,
        amount,
        key: key.to_owned(),
        message: message(kind, invoice, source, amount, reason),
        role: ACCOUNTANT.to_owned(),
        status: TaskStatus::Open,
        ..Default::default()
    };
    match task.insert(&state.db).await {
        Ok(()) => {}
        Err(error) if error.is_unique_violation() => return Ok(None),
        Err(error) => return Err(error),
    }
    let data = data(invoice, &task.source(), task.amount, "");
    notify_accountants(state, invoice.subject(), data, &task).await?;
    Ok(Some(task))
}

/// The values the supplier bill task message and notice are filled with.
fn bill_data(
    bill: &SupplierBill,
    supplier: &str,
    source: &Subject,
    amount: i64,
    reason: &str,
) -> Value {
    json!({
        "number": bill.supplier_invoice_number,
        "party_name": supplier,
        "source": source.to_string(),
        "amount": amount_text(&bill.currency, amount),
        "reason": lower_first(reason),
        "link": format!("/supplier-bills/{}", bill.id),
    })
}

/// `text` with its first letter in lower case, to follow "refused: ".
fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The message of a task for a refused reversal on a supplier bill;
/// `reason` is the refusal's text.
pub fn bill_message(bill: &SupplierBill, source: &Subject, amount: i64, reason: &str) -> String {
    fill(
        english(TaskKind::BillReversalRefused.text_key()),
        &bill_data(bill, "", source, amount, reason),
    )
}

/// Opens the task for the refusal `reason` of a reversal of `amount` from
/// `source` on supplier bill `bill`, keyed on the unallocation `key`, and
/// tells the accountants. `None` when the task already exists for the key.
///
/// # Errors
///
/// A database or notifier error.
pub async fn open_for_bill(
    state: &AppState,
    bill: &SupplierBill,
    source: &Subject,
    amount: i64,
    key: &str,
    reason: &str,
) -> Result<Option<InvoicingTask>> {
    let kind = TaskKind::BillReversalRefused;
    let supplier = wl_kernel::parties::directory(state)
        .billing_identity(&state.db, bill.supplier())
        .await?
        .map(|identity| identity.legal_name_en)
        .unwrap_or_default();
    let mut task = InvoicingTask {
        kind,
        bill_id: Some(bill.id),
        source_type: source.subject_type.clone(),
        source_id: source.subject_id,
        amount,
        key: key.to_owned(),
        message: bill_message(bill, source, amount, reason),
        role: ACCOUNTANT.to_owned(),
        status: TaskStatus::Open,
        ..Default::default()
    };
    match task.insert(&state.db).await {
        Ok(()) => {}
        Err(error) if error.is_unique_violation() => return Ok(None),
        Err(error) => return Err(error),
    }
    let data = bill_data(bill, &supplier, source, amount, "");
    notify_accountants(state, bill.subject(), data, &task).await?;
    Ok(Some(task))
}

/// The open tasks of supplier bill `bill_id`, oldest first.
pub async fn open_of_bill(db: &Db, bill_id: i64) -> Result<Vec<InvoicingTask>> {
    InvoicingTask::query()
        .where_(InvoicingTask::BILL_ID.eq(bill_id))
        .where_(InvoicingTask::STATUS.eq(TaskStatus::Open))
        .order_by(InvoicingTask::ID)
        .get(db)
        .await
}

/// The open tasks of `invoice_id`, oldest first.
pub async fn open_of(db: &Db, invoice_id: i64) -> Result<Vec<InvoicingTask>> {
    InvoicingTask::query()
        .where_(InvoicingTask::INVOICE_ID.eq(invoice_id))
        .where_(InvoicingTask::STATUS.eq(TaskStatus::Open))
        .order_by(InvoicingTask::ID)
        .get(db)
        .await
}

/// One notice to the holders of the `accountant` role (global scope, once
/// each, by id).
async fn notify_accountants(
    state: &AppState,
    subject: Subject,
    mut data: Value,
    task: &InvoicingTask,
) -> Result {
    let mut ids: Vec<i64> = users_with_role_in(&state.db, &task.role, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    data["message"] = Value::String(task.message.clone());
    let mut notice = NotifyRequest::new(REVERSAL_TASK, subject).data(data);
    for id in ids {
        notice = notice.to_user(id);
    }
    notify::notifier(state).notify(state, notice).await?;
    Ok(())
}

/// The notification type [`REVERSAL_TASK`]: staff, transactional, by e-mail
/// (and the bell).
pub fn notification_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REVERSAL_TASK, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.task.subject")),
            english("wl_invoicing.task.body"),
        )
}

/// Registers [`REVERSAL_TASK`] with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notification_type());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoice() -> Invoice {
        Invoice {
            id: 4,
            number: Some("INV-PGP-2026-0004".into()),
            currency: "AED".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_written_off_invoice_gets_the_write_off_kind() {
        assert_eq!(TaskKind::of(&invoice()), TaskKind::ReversalRefused);
        let written_off = Invoice {
            status: InvoiceStatus::WrittenOff,
            ..invoice()
        };
        assert_eq!(TaskKind::of(&written_off), TaskKind::ReversalAfterWriteOff);
        let recovered = Invoice {
            status: InvoiceStatus::Paid,
            write_off_amount: Some(105_000),
            ..invoice()
        };
        assert_eq!(TaskKind::of(&recovered), TaskKind::ReversalAfterWriteOff);
    }

    #[test]
    fn the_message_names_the_payment_and_the_amount() {
        let source = Subject::new("payments.payment", 3);
        let text = message(
            TaskKind::ReversalAfterWriteOff,
            &invoice(),
            &source,
            50_000,
            "",
        );
        assert!(text.contains("payments.payment:3"), "{text}");
        assert!(text.contains("AED 500.00"), "{text}");
        assert!(text.contains("before the write-off"), "{text}");
        let refused = message(
            TaskKind::ReversalRefused,
            &invoice(),
            &source,
            90_000,
            "only AED 800.00 is allocated",
        );
        assert!(refused.contains("AED 900.00"), "{refused}");
        assert!(
            refused.contains("only AED 800.00 is allocated"),
            "{refused}"
        );
    }

    #[test]
    fn the_type_is_transactional_with_an_english_email() {
        let def = notification_type();
        assert_eq!(def.key, REVERSAL_TASK);
        assert_eq!(def.category, Category::Transactional);
        assert_eq!(def.channels, [NotifyChannel::Email]);
    }
}
