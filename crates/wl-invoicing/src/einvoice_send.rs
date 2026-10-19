//! Queueing and sending e-invoice transmissions (wl-invoicing §4 *E-invoice
//! transmission*, §5.12 steps 1 to 4, `FIN.INV.01`, `FIN.RUL.16`).
//!
//! - **Queue:** after an invoice or a credit note `issue` commits, a listener
//!   ([`register`]) calls [`after_issue`]. When e-invoicing applies to the
//!   issuing entity on the document's own issue date (a credit note follows its
//!   own date, not its invoice's) and the recipient is in scope
//!   ([`EInvoiceScope`](crate::EInvoiceScope)), a transmission is queued with a
//!   payload built from the issued document ([`crate::einvoice_payload`]) and
//!   sent at once. Nothing here can block `issue`: a failure to queue is
//!   caught up by the scheduled run ([`sweep`]), which queues the documents
//!   issued in the last days that have no transmission.
//! - **Send:** [`send_one`] hands the stored payload to the ASP adapter of the
//!   entity. The ASP's reference is stored and `submit` runs; an ASP that
//!   cannot be reached (or an entity with no adapter) leaves the transmission
//!   `queued` with its error and its next try, measured from the start of the
//!   attempt: +5 min, +30 min, +2 h, +6 h, then every 6 h until sent
//!   ([`InvoicingConfig::einvoice_retry_after`](crate::InvoicingConfig)). A
//!   queued transmission is never dropped. The calls report to the integration
//!   health of the kernel (`asp.<entity code>`, `PLT.RUL.05`), so the screens
//!   it affects show the banner while the connection is down.
//! - **Answers:** [`respond`] runs `accept` or `reject` for an answer of the
//!   ASP to the attempt it names (an answer to an older attempt changes
//!   nothing); the scheduled run polls the transmissions `submitted`.
//! - **Resubmit and supersede:** [`resubmit`] and [`supersede_for`].
//!
//! The scheduled run [`TASK`] (every minute) queues what was missed, sends what
//! is due, polls what waits for an answer and raises the tasks and the outage
//! notice of `FIN.RUL.16` ([`crate::einvoice_tasks`]).

use renox::chrono::{Days, NaiveDate};
use renox::db::Json;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned, TransitionError};
use wl_kernel::documents::today;
use wl_kernel::integrations;
use wl_kernel::parties::{PartyKind, directory};
use wl_kernel::{LegalEntity, Subject};

use crate::credit_note::CreditNote;
use crate::credit_note_machine::{ISSUE as NOTE_ISSUE, ISSUE_APPROVED, ISSUE_SYSTEM};
use crate::einvoice::{
    ACCEPT, CREDIT_NOTE, EInvoiceTransmission, INVOICE, REJECT, RESUBMIT, RejectInput,
    ResubmitInput, SUBMIT, SUPERSEDE, SubmitInput, SupersedeInput, TransmissionStatus,
};
use crate::einvoice_adapter::{AspAnswer, AspError, Submission, adapter_of, scope_of};
use crate::einvoice_payload::{self as payload, Document};
use crate::invoice::Invoice;
use crate::issue::ISSUE as INVOICE_ISSUE;
use crate::config::InvoicingConfig;
use crate::config_of;
#[cfg(test)]
use crate::texts;
use wl_kernel::invoicing::{CreditNoteStatus, InvoiceStatus};

/// The scheduled task that runs [`run_due`] every minute.
pub const TASK: &str = "invoicing:einvoices";

/// How many days back the scheduled run looks for issued documents that have
/// no transmission.
const SWEEP_DAYS: u64 = 3;

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The key of the integration connection that carries the transmissions of
/// the entity with this code.
pub fn connection_key(entity_code: &str) -> String {
    format!("asp.{}", entity_code.trim().to_lowercase())
}

/// What `issue` needs to know about e-invoicing for an invoice (the guards of
/// the invoice's `issue`).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IssueFacts {
    /// Whether e-invoicing applies to the entity on the issue date.
    pub applies: bool,
    /// Whether the recipient is in the mandate (`EInvoiceScope`).
    pub in_scope: bool,
    /// The recipient identifier the party has, if any.
    pub recipient_id: Option<String>,
}

/// The e-invoicing facts for issuing `invoice` today.
///
/// # Errors
///
/// A database error.
pub async fn issue_facts(state: &AppState, invoice: &Invoice) -> Result<IssueFacts> {
    let config = config_of(state);
    let Some(entity_id) = invoice.legal_entity_id else {
        return Ok(IssueFacts::default());
    };
    let Some(entity) = LegalEntity::find(&state.db, entity_id).await? else {
        return Ok(IssueFacts::default());
    };
    if !config.einvoice_applies(&entity.code, today()) {
        return Ok(IssueFacts::default());
    }
    let kind = directory(state)
        .find(&state.db, invoice.party())
        .await?
        .map(|p| p.kind);
    let in_scope = scope_of(state)
        .in_scope(state, &entity, &invoice.subject(), kind)
        .await?;
    let recipient_id = directory(state)
        .billing_identity(&state.db, invoice.party())
        .await?
        .and_then(|identity| identity.einvoice_recipient_id)
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty());
    Ok(IssueFacts {
        applies: true,
        in_scope,
        recipient_id,
    })
}

/// Reports a successful call of the entity's ASP to the integration health,
/// when the connection is registered.
async fn health_ok(state: &AppState, entity_code: &str) -> Result {
    let key = connection_key(entity_code);
    if integrations::get(&key).is_some() {
        integrations::report_success(state, &key).await?;
    }
    Ok(())
}

/// Reports a failed call of the entity's ASP to the integration health.
async fn health_failed(state: &AppState, entity_code: &str, message: &str) -> Result {
    let key = connection_key(entity_code);
    if integrations::get(&key).is_some() {
        integrations::report_failure(state, &key, message).await?;
    }
    Ok(())
}

/// Queues a transmission for `document` when e-invoicing applies to its
/// entity on its issue date and the recipient is in scope, and returns it;
/// `None` when it does not apply, is out of scope, or the document already has
/// a transmission that is not superseded.
///
/// # Errors
///
/// A database error.
pub async fn queue_document(
    state: &AppState,
    document: &Document,
) -> Result<Option<EInvoiceTransmission>> {
    let config = config_of(state);
    let Some(entity_id) = document.legal_entity_id() else {
        return Ok(None);
    };
    let Some(entity) = LegalEntity::find(&state.db, entity_id).await? else {
        return Ok(None);
    };
    let Some(issued) = document.issue_date() else {
        return Ok(None);
    };
    if !config.einvoice_applies(&entity.code, issued) {
        return Ok(None);
    }
    let subject = document.subject();
    let existing = EInvoiceTransmission::of_document(&state.db, &subject).await?;
    if existing
        .iter()
        .any(|t| t.status != TransmissionStatus::Superseded)
    {
        return Ok(None);
    }
    let kind: Option<PartyKind> = directory(state)
        .find(&state.db, document.party())
        .await?
        .map(|p| p.kind);
    if !scope_of(state)
        .in_scope(state, &entity, &subject, kind)
        .await?
    {
        return Ok(None);
    }
    let built = payload::build(state, document).await?;
    let at = renox::db::now();
    let mut row = EInvoiceTransmission {
        legal_entity_id: entity.id,
        document_type: subject.subject_type.clone(),
        document_id: subject.subject_id,
        adapter: config.einvoice_of(&entity.code).and_then(|e| e.adapter.clone()),
        recipient_id: built.recipient_id,
        payload: Json(built.payload),
        payload_hash: built.hash,
        attempt: 1,
        queued_at: at,
        next_try_at: Some(at),
        status: TransmissionStatus::Queued,
        ..Default::default()
    };
    row.insert(&state.db).await?;
    Ok(Some(row))
}

/// After the commit of the `issue` of `document`: queues its transmission
/// and sends it at once. An ASP that does not answer is not an error.
///
/// # Errors
///
/// A database error.
pub async fn after_issue(state: &AppState, document: Document) -> Result<()> {
    if let Some(row) = queue_document(state, &document).await? {
        send_one(state, row.id).await?;
    }
    Ok(())
}

/// Notes a failed try of `t`: one more failed try, the error and the next
/// try, measured from the start of the attempt. `unreachable` says the ASP
/// itself did not answer, which the integration health hears about.
async fn failed_try(
    state: &AppState,
    config: &InvoicingConfig,
    t: &EInvoiceTransmission,
    message: &str,
    entity_code: &str,
    unreachable: bool,
) -> Result {
    let tries = t.tries + 1;
    let next = t.queued_at + config.einvoice_retry_after(tries);
    EInvoiceTransmission::query()
        .where_(EInvoiceTransmission::ID.eq(t.id))
        .where_(EInvoiceTransmission::STATUS.eq(TransmissionStatus::Queued))
        .update(
            &state.db,
            &[
                ("tries", &tries),
                ("last_error", &Some(message.to_owned())),
                ("next_try_at", &Some(next)),
                ("updated_at", &Some(renox::db::now())),
            ],
        )
        .await?;
    if unreachable {
        health_failed(state, entity_code, message).await?;
    }
    Ok(())
}

/// Sends the `queued` transmission `id` to the ASP of its entity. Returns
/// whether the ASP took it (`submitted`). The ASP being unreachable, or the
/// entity having no adapter, leaves it `queued` with the next try set.
///
/// # Errors
///
/// A database error.
pub async fn send_one(state: &AppState, id: i64) -> Result<bool> {
    let Some(mut t) = EInvoiceTransmission::find(&state.db, id).await? else {
        return Ok(false);
    };
    if t.status != TransmissionStatus::Queued {
        return Ok(false);
    }
    let config = config_of(state);
    let Some(entity) = LegalEntity::find(&state.db, t.legal_entity_id).await? else {
        return Ok(false);
    };
    let adapter_name = config
        .einvoice_of(&entity.code)
        .and_then(|e| e.adapter.clone());
    let Some(adapter) = adapter_name.as_deref().and_then(|n| adapter_of(state, n)) else {
        failed_try(
            state,
            &config,
            &t,
            "No ASP adapter is configured for the entity.",
            &entity.code,
            false,
        )
        .await?;
        return Ok(false);
    };
    // A document queued before its party had an identifier reads it again.
    if t.recipient_id.as_deref().is_none_or(|r| r.trim().is_empty()) {
        if let Some(document) =
            Document::load(&state.db, &Subject::new(&t.document_type, t.document_id)).await?
        {
            let built = payload::build(state, &document).await?;
            if built.recipient_id.is_some() {
                EInvoiceTransmission::query()
                    .where_(EInvoiceTransmission::ID.eq(t.id))
                    .update(
                        &state.db,
                        &[
                            ("payload", &Json(built.payload.clone())),
                            ("payload_hash", &built.hash),
                            ("recipient_id", &built.recipient_id),
                        ],
                    )
                    .await?;
                t.payload = Json(built.payload);
                t.payload_hash = built.hash;
                t.recipient_id = built.recipient_id;
            }
        }
    }
    let Some(recipient) = t
        .recipient_id
        .clone()
        .filter(|r| !r.trim().is_empty())
    else {
        failed_try(
            state,
            &config,
            &t,
            "The recipient has no e-invoicing identifier.",
            &entity.code,
            false,
        )
        .await?;
        return Ok(false);
    };
    let submission = Submission {
        key: format!("{}-{}", t.id, t.attempt),
        transmission: t.subject(),
        document: t.document(),
        recipient_id: recipient,
        payload: t.payload.0.clone(),
        hash: t.payload_hash.clone(),
    };
    match adapter.submit(state, &submission).await {
        Ok(reference) => {
            let input = SubmitInput {
                adapter_configured: true,
                reference,
            };
            match t.transition(SUBMIT).by_system().input(&input).run(state).await {
                Ok(_) => {}
                // Another worker moved it first.
                Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => return Ok(false),
                Err(error) => return Err(error.into_error()),
            }
            health_ok(state, &entity.code).await?;
            Ok(true)
        }
        Err(error) => {
            failed_try(state, &config, &t, &error.to_string(), &entity.code, true).await?;
            Ok(false)
        }
    }
}

/// The ASP answered the attempt `attempt` of transmission `id`: `accept` or
/// `reject` runs. A pending answer, an answer to an older attempt or to a
/// transmission that is no longer `submitted` changes nothing (`None`).
///
/// # Errors
///
/// A database error.
pub async fn respond(
    state: &AppState,
    id: i64,
    attempt: i64,
    answer: AspAnswer,
) -> Result<Option<EInvoiceTransmission>> {
    let Some(t) = EInvoiceTransmission::find(&state.db, id).await? else {
        return Ok(None);
    };
    if t.attempt != attempt || t.status != TransmissionStatus::Submitted {
        return Ok(None);
    }
    let run = match answer {
        AspAnswer::Pending => return Ok(None),
        AspAnswer::Accepted => t.transition(ACCEPT).by_system().run(state).await,
        AspAnswer::Rejected(errors) => {
            let input = RejectInput {
                errors: errors
                    .into_iter()
                    .map(|AspError { code, message }| (code, message))
                    .collect(),
            };
            t.transition(REJECT).by_system().input(&input).run(state).await
        }
    };
    match run {
        Ok(_) => EInvoiceTransmission::find(&state.db, id).await,
        Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => Ok(None),
        Err(error) => Err(error.into_error()),
    }
}

/// Sends the `queued` transmissions whose try is due. Returns how many the
/// ASP took.
async fn send_due(state: &AppState) -> Result<usize> {
    let now = renox::db::now();
    let due = EInvoiceTransmission::query()
        .where_(EInvoiceTransmission::STATUS.eq(TransmissionStatus::Queued))
        .order_by(EInvoiceTransmission::ID)
        .get(&state.db)
        .await?;
    let mut sent = 0;
    for t in due {
        if t.next_try_at.is_some_and(|at| at > now) {
            continue;
        }
        if send_one(state, t.id).await? {
            sent += 1;
        }
    }
    Ok(sent)
}

/// Asks the ASP what became of the `submitted` transmissions. Returns how many
/// were answered (accepted or rejected).
async fn poll_submitted(state: &AppState) -> Result<usize> {
    let config = config_of(state);
    let waiting = EInvoiceTransmission::query()
        .where_(EInvoiceTransmission::STATUS.eq(TransmissionStatus::Submitted))
        .order_by(EInvoiceTransmission::ID)
        .get(&state.db)
        .await?;
    let mut answered = 0;
    for t in waiting {
        let Some(entity) = LegalEntity::find(&state.db, t.legal_entity_id).await? else {
            continue;
        };
        let Some(reference) = t.asp_reference.clone() else {
            continue;
        };
        let adapter = config
            .einvoice_of(&entity.code)
            .and_then(|e| e.adapter.clone())
            .and_then(|name| adapter_of(state, &name));
        let Some(adapter) = adapter else {
            continue;
        };
        match adapter.poll(state, &reference).await {
            Ok(answer) => {
                health_ok(state, &entity.code).await?;
                if respond(state, t.id, t.attempt, answer).await?.is_some() {
                    answered += 1;
                }
            }
            Err(error) => health_failed(state, &entity.code, &error.to_string()).await?,
        }
    }
    Ok(answered)
}

/// Queues the documents issued in the last days that have no transmission,
/// for each entity with a go-live date: a safety net for a document whose
/// transmission could not be queued at `issue`. Returns how many it queued.
pub async fn sweep(state: &AppState) -> Result<usize> {
    let config = config_of(state);
    let since: NaiveDate = today()
        .checked_sub_days(Days::new(SWEEP_DAYS))
        .unwrap_or_else(today);
    let mut queued = 0;
    for entry in &config.einvoice {
        let from = entry.go_live.max(since);
        if from > today() {
            continue;
        }
        let Some(entity) = LegalEntity::query()
            .where_(LegalEntity::CODE.eq(entry.entity_code.as_str()))
            .first(&state.db)
            .await?
        else {
            continue;
        };
        let known: std::collections::BTreeSet<(String, i64)> =
            EInvoiceTransmission::query()
                .where_(EInvoiceTransmission::LEGAL_ENTITY_ID.eq(entity.id))
                .get(&state.db)
                .await?
                .into_iter()
                .map(|t| (t.document_type, t.document_id))
                .collect();
        let invoices = Invoice::query()
            .where_(Invoice::LEGAL_ENTITY_ID.eq(entity.id))
            .where_(Invoice::STATUS.ne(InvoiceStatus::Draft))
            .where_(Invoice::ISSUE_DATE.gte(from))
            .order_by(Invoice::ID)
            .get(&state.db)
            .await?;
        for invoice in invoices {
            if known.contains(&(INVOICE.to_owned(), invoice.id)) {
                continue;
            }
            if queue_document(state, &Document::Invoice(invoice)).await?.is_some() {
                queued += 1;
            }
        }
        let notes = CreditNote::query()
            .where_(CreditNote::LEGAL_ENTITY_ID.eq(entity.id))
            .where_(CreditNote::STATUS.ne(CreditNoteStatus::Draft))
            .where_(CreditNote::ISSUE_DATE.gte(from))
            .order_by(CreditNote::ID)
            .get(&state.db)
            .await?;
        for note in notes {
            if known.contains(&(CREDIT_NOTE.to_owned(), note.id)) {
                continue;
            }
            if queue_document(state, &Document::CreditNote(note)).await?.is_some() {
                queued += 1;
            }
        }
    }
    Ok(queued)
}

/// What a scheduled run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Documents queued that `issue` had missed.
    pub queued: usize,
    /// Transmissions the ASP took.
    pub sent: usize,
    /// Transmissions the ASP answered.
    pub answered: usize,
    /// Tasks raised.
    pub tasks: usize,
}

/// The scheduled run: queues what was missed, sends what is due, polls what
/// waits for an answer and raises the tasks of `FIN.RUL.16`.
///
/// # Errors
///
/// A database error.
pub async fn run_due(state: &AppState) -> Result<Report> {
    let queued = sweep(state).await?;
    let sent = send_due(state).await?;
    let answered = poll_submitted(state).await?;
    let tasks = crate::einvoice_tasks::run_stuck(state).await?;
    Ok(Report {
        queued,
        sent,
        answered,
        tasks,
    })
}

/// `user` sends the rejected transmission `id` again (`resubmit`), after the
/// cause outside the issued document was fixed: the payload is rebuilt from
/// the same document with the party data read again, the attempt count rises
/// and the transmission is `queued`. `fixed` is the user's confirmation. It is
/// sent at once.
///
/// # Errors
///
/// `Forbidden` without `einvoices.manage`; `NotFrom` unless `rejected`;
/// `Guard` (422) without the confirmation or when the party still has no
/// recipient identifier.
pub async fn resubmit(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    fixed: bool,
) -> std::result::Result<EInvoiceTransmission, TransitionError> {
    let failed = TransitionError::Failed;
    let t = EInvoiceTransmission::find_or_404(&state.db, id)
        .await
        .map_err(failed)?;
    let document = Document::load(&state.db, &t.document())
        .await
        .map_err(TransitionError::Failed)?;
    let built = match &document {
        Some(document) => Some(
            payload::build(state, document)
                .await
                .map_err(TransitionError::Failed)?,
        ),
        None => None,
    };
    let config = config_of(state);
    let entity = LegalEntity::find(&state.db, t.legal_entity_id)
        .await
        .map_err(TransitionError::Failed)?;
    let adapter = entity
        .and_then(|entity| config.einvoice_of(&entity.code).and_then(|e| e.adapter.clone()));
    let input = ResubmitInput {
        cause_fixed: fixed,
        payload: built.as_ref().map(|b| b.payload.clone()).unwrap_or_default(),
        hash: built.as_ref().map(|b| b.hash.clone()).unwrap_or_default(),
        recipient_id: built.and_then(|b| b.recipient_id),
        adapter,
    };
    t.transition(RESUBMIT).by(user).input(&input).run(state).await?;
    send_one(state, id).await.map_err(TransitionError::Failed)?;
    EInvoiceTransmission::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)
}

/// A credit note with the reason *e-invoice correction* was issued against
/// invoice `note.invoice_id`: its rejected transmission becomes `superseded`,
/// linked to the credit note (`supersede`). Returns how many it superseded.
///
/// # Errors
///
/// A database error.
pub async fn supersede_for(state: &AppState, note: &CreditNote) -> Result<usize> {
    if !EInvoiceTransmission::is_correction(&note.reason) {
        return Ok(0);
    }
    let invoice = Subject::new(INVOICE, note.invoice_id);
    let input = SupersedeInput {
        credit_note_id: note.id,
        reason: note.reason.clone(),
    };
    let mut superseded = 0;
    for t in EInvoiceTransmission::of_document(&state.db, &invoice).await? {
        if t.status != TransmissionStatus::Rejected {
            continue;
        }
        match t.transition(SUPERSEDE).by_system().input(&input).run(state).await {
            Ok(_) => superseded += 1,
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    Ok(superseded)
}

/// The user names, on the draft `draft`, the invoice `old_invoice_id` it
/// replaces after an e-invoice correction: the old invoice must have a
/// superseded transmission, and be the draft's party's and entity's.
///
/// # Errors
///
/// 422 for a draft that is not a draft, an unknown invoice, another party or
/// entity, or an invoice with no superseded transmission.
pub async fn set_replaces(db: &Db, draft: &Invoice, old_invoice_id: i64) -> Result<()> {
    if draft.status != InvoiceStatus::Draft {
        return Err(refused("Only a draft invoice can name the invoice it replaces."));
    }
    let Some(old) = Invoice::find(db, old_invoice_id).await? else {
        return Err(refused("The invoice to replace was not found."));
    };
    if old.id == draft.id
        || old.party_id != draft.party_id
        || old.legal_entity_id != draft.legal_entity_id
    {
        return Err(refused(
            "The invoice to replace is another party's or another entity's.",
        ));
    }
    let superseded = EInvoiceTransmission::of_document(db, &old.subject())
        .await?
        .iter()
        .any(|t| t.status == TransmissionStatus::Superseded);
    if !superseded {
        return Err(refused(
            "The invoice has no superseded e-invoice transmission: credit it with the reason \
             e-invoice correction first.",
        ));
    }
    Invoice::query()
        .where_(Invoice::ID.eq(draft.id))
        .update(db, &[("replaces_invoice_id", &Some(old.id))])
        .await?;
    Ok(())
}

/// When the new invoice `invoice` that names the invoice it replaces is
/// issued, the superseded transmission of the old one is linked to it too.
/// Returns how many it linked.
///
/// # Errors
///
/// A database error.
pub async fn link_replacement(state: &AppState, invoice: &Invoice) -> Result<usize> {
    let Some(old) = invoice.replaces_invoice_id else {
        return Ok(0);
    };
    let mut linked = 0;
    for t in EInvoiceTransmission::of_document(&state.db, &Subject::new(INVOICE, old)).await? {
        if t.status == TransmissionStatus::Superseded && t.replaced_by_invoice_id.is_none() {
            EInvoiceTransmission::query()
                .where_(EInvoiceTransmission::ID.eq(t.id))
                .update(
                    &state.db,
                    &[
                        ("replaced_by_invoice_id", &Some(invoice.id)),
                        ("updated_at", &Some(renox::db::now())),
                    ],
                )
                .await?;
            linked += 1;
        }
    }
    Ok(linked)
}

/// How many transmissions of the entities `entities` wait in the queue.
///
/// # Errors
///
/// A database error.
pub async fn queued_count(db: &Db, entities: &[i64]) -> Result<i64> {
    if entities.is_empty() {
        return Ok(0);
    }
    EInvoiceTransmission::query()
        .where_(EInvoiceTransmission::LEGAL_ENTITY_ID.is_in(entities.to_vec()))
        .where_(EInvoiceTransmission::STATUS.eq(TransmissionStatus::Queued))
        .count(db)
        .await
        .map(|n| i64::try_from(n).unwrap_or(i64::MAX))
}

/// How many transmissions of any entity wait in the queue.
///
/// # Errors
///
/// A database error.
pub async fn queued_count_all(db: &Db) -> Result<i64> {
    EInvoiceTransmission::query()
        .where_(EInvoiceTransmission::STATUS.eq(TransmissionStatus::Queued))
        .count(db)
        .await
        .map(|n| i64::try_from(n).unwrap_or(i64::MAX))
}

/// The text of the queue banner from the `template` (with `{{ count }}`), or
/// `None` when nothing is queued.
pub fn queue_banner(template: &str, count: i64) -> Option<String> {
    (count > 0).then(|| {
        wl_kernel::notify::fill(template, &renox::serde_json::json!({ "count": count }))
    })
}

/// Registers the listeners: a document issued is queued and sent, a credit
/// note that corrects supersedes the rejected transmission, and a transmission
/// that changed state gives its event and its task; the scheduled run and its
/// command.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Invoice>, state: AppState| async move {
        if event.transition != INVOICE_ISSUE {
            return Ok(());
        }
        let Some(invoice) = Invoice::find(&state.db, event.key).await? else {
            return Ok(());
        };
        // `issue` is never blocked: whatever fails here, the scheduled run
        // queues the document again.
        let _ = link_replacement(&state, &invoice).await;
        let _ = after_issue(&state, Document::Invoice(invoice)).await;
        Ok(())
    });
    app.listen(|event: Transitioned<CreditNote>, state: AppState| async move {
        let issued = [NOTE_ISSUE, ISSUE_SYSTEM, ISSUE_APPROVED];
        if !issued.contains(&event.transition.as_str()) {
            return Ok(());
        }
        let Some(note) = CreditNote::find(&state.db, event.key).await? else {
            return Ok(());
        };
        let _ = after_issue(&state, Document::CreditNote(note.clone())).await;
        let _ = supersede_for(&state, &note).await;
        Ok(())
    });
    crate::einvoice_tasks::register(app);
    app.schedule()
        .every_minute(TASK, |state: AppState| async move {
            run_due(&state).await.map(|_| ())
        });
    app.command(
        TASK,
        "Queue the e-invoice transmissions issue missed, send the ones due, poll the ones \
         waiting for the ASP and raise the tasks of FIN.RUL.16",
        |_args, state: AppState| async move {
            let report = run_due(&state).await?;
            println!(
                "{} queued, {} sent, {} answered, {} tasks.",
                report.queued, report.sent, report.answered, report.tasks
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_connection_of_an_entity_is_named_after_its_code() {
        assert_eq!(connection_key("PGP"), "asp.pgp");
        assert_eq!(connection_key(" Acme "), "asp.acme");
    }

    #[test]
    fn the_banner_counts_the_queue() {
        let template = texts::english("wl_invoicing.einvoice.banner");
        assert_eq!(queue_banner(template, 0), None);
        let text = queue_banner(template, 3).expect("a banner");
        assert!(text.contains('3'), "{text}");
    }

    #[test]
    fn the_issue_facts_default_to_not_applying() {
        let facts = IssueFacts::default();
        assert!(!facts.applies && !facts.in_scope);
        assert_eq!(facts.recipient_id, None);
    }
}
