//! Drafting credit notes (wl-invoicing §5.12): a partial credit note is
//! drafted against an invoice, changed and discarded while it is a `draft`.
//! A draft has no number and no effect on any balance; `issue` (its own
//! transition) makes it count.
//!
//! The credit on one invoice line is limited to its net minus what issued
//! credit notes already credited on it (the "remaining credit",
//! [`vat::remaining_credit`]). Drafts do not count against it, so several
//! drafts can be prepared and the first one issued wins.

use std::collections::HashMap;

use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Outcome, StateMachine, TransitionBuilder, TransitionError};
use wl_kernel::invoicing::{CreditLine, CreditNoteStatus, InvoiceStatus};
use wl_kernel::{LegalEntity, Subject};

use crate::allocate::allocate_in;
use crate::allocation::Allocation;
use crate::credit_note::{CreditNote, CreditNoteKind};
use crate::credit_note_line::CreditNoteLine;
use crate::credit_note_machine::{APPLY, ISSUE, ISSUE_SYSTEM, IssueInput};
use crate::credit_note_movement::{CreditNoteMovement, MovementKind};
use crate::invoice::Invoice;
use crate::line::InvoiceLine;
use crate::restore::ALLOCATION_SUBJECT_TYPE;
use crate::vat::{self, VatTotals};

/// The invoice states a credit note can be drafted against.
const CREDITABLE: [InvoiceStatus; 4] = [
    InvoiceStatus::Issued,
    InvoiceStatus::PartiallyPaid,
    InvoiceStatus::Overdue,
    InvoiceStatus::Paid,
];

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The invoice lines to credit, with the net on each, or why the credit
/// cannot be drafted: the invoice is not in a state that can be credited,
/// the reason is blank, there are no lines, a line is unknown, repeated or
/// not above 0, or its net is above the line's remaining credit.
/// `credited` is the net issued credit notes already credited, by invoice
/// line id.
pub fn plan<'a>(
    invoice_status: InvoiceStatus,
    reason: &str,
    requested: &[CreditLine],
    invoice_lines: &'a [InvoiceLine],
    credited: &HashMap<i64, i64>,
) -> std::result::Result<Vec<(&'a InvoiceLine, i64)>, String> {
    if !CREDITABLE.contains(&invoice_status) {
        return Err(format!(
            "an invoice that is {invoice_status} cannot be credited"
        ));
    }
    if reason.trim().is_empty() {
        return Err("give a reason for the credit note".to_owned());
    }
    if requested.is_empty() {
        return Err("a credit note needs at least one line".to_owned());
    }
    plan_lines(requested, invoice_lines, credited)
}

/// The lines of [`plan`] alone: each one known, once, above 0 and within
/// the line's remaining credit.
pub fn plan_lines<'a>(
    requested: &[CreditLine],
    invoice_lines: &'a [InvoiceLine],
    credited: &HashMap<i64, i64>,
) -> std::result::Result<Vec<(&'a InvoiceLine, i64)>, String> {
    let mut planned: Vec<(&InvoiceLine, i64)> = Vec::with_capacity(requested.len());
    for credit in requested {
        let position = credit.invoice_line;
        let Some(line) = invoice_lines.iter().find(|l| l.position == position) else {
            return Err(format!("the invoice has no line {position}"));
        };
        if planned.iter().any(|(l, _)| l.id == line.id) {
            return Err(format!("line {position} is credited twice"));
        }
        if credit.net <= 0 {
            return Err(format!("line {position} must credit more than 0"));
        }
        let remaining = vat::remaining_credit(
            line.net_amount,
            credited.get(&line.id).copied().unwrap_or(0),
        );
        if credit.net > remaining {
            return Err(format!(
                "line {position} can be credited by {remaining} fils at most"
            ));
        }
        planned.push((line, credit.net));
    }
    Ok(planned)
}

/// The net that issued credit notes credited, by invoice line id.
pub async fn credited_by_line<'c>(
    db: impl renox::db::Executor<'c>,
    invoice_id: i64,
) -> Result<HashMap<i64, i64>> {
    let mut conn = db.into_conn();
    let issued: Vec<i64> = CreditNote::query()
        .where_(CreditNote::INVOICE_ID.eq(invoice_id))
        .where_(CreditNote::STATUS.is_in([CreditNoteStatus::Issued, CreditNoteStatus::Applied]))
        .get(conn.reborrow())
        .await?
        .into_iter()
        .map(|n| n.id)
        .collect();
    if issued.is_empty() {
        return Ok(HashMap::new());
    }
    let lines = CreditNoteLine::query()
        .where_(CreditNoteLine::CREDIT_NOTE_ID.is_in(issued))
        .get(conn.reborrow())
        .await?;
    let mut by_line = HashMap::new();
    for line in lines {
        *by_line.entry(line.invoice_line_id).or_insert(0) += line.net_amount;
    }
    Ok(by_line)
}

/// The lines of credit note `credit_note_id`, in order.
pub async fn lines_of(db: &Db, credit_note_id: i64) -> Result<Vec<CreditNoteLine>> {
    CreditNoteLine::query()
        .where_(CreditNoteLine::CREDIT_NOTE_ID.eq(credit_note_id))
        .order_by(CreditNoteLine::POSITION)
        .get(db)
        .await
}

/// The totals per VAT category of credit note `credit_note_id`, from its
/// stored lines.
pub async fn totals_of(db: &Db, credit_note_id: i64) -> Result<VatTotals> {
    let lines = lines_of(db, credit_note_id).await?;
    let amounts: Vec<_> = lines.iter().map(CreditNoteLine::amounts).collect();
    Ok(VatTotals::of(&amounts))
}

/// The lines for `planned` on credit note `credit_note_id`, and their totals.
fn build(
    credit_note_id: i64,
    planned: &[(&InvoiceLine, i64)],
    rate_micro: Option<i64>,
) -> (Vec<CreditNoteLine>, VatTotals) {
    let lines: Vec<CreditNoteLine> = planned
        .iter()
        .zip(1..)
        .map(|((line, net), position)| {
            CreditNoteLine::of(credit_note_id, position, line, *net, rate_micro)
        })
        .collect();
    let amounts: Vec<_> = lines.iter().map(CreditNoteLine::amounts).collect();
    let totals = VatTotals::of(&amounts);
    (lines, totals)
}

/// Plans `lines` on `invoice`, from its stored lines and the credit issued.
async fn plan_for<'c>(
    db: impl renox::db::Executor<'c>,
    invoice: &Invoice,
    lines: &[CreditLine],
    reason: &str,
) -> Result<(Vec<InvoiceLine>, Vec<(i64, i64)>)> {
    let mut conn = db.into_conn();
    let invoice_lines = crate::invoicer::lines_of(conn.reborrow(), invoice.id).await?;
    let credited = credited_by_line(conn.reborrow(), invoice.id).await?;
    let planned = plan(invoice.status, reason, lines, &invoice_lines, &credited)
        .map_err(refused)?
        .into_iter()
        .map(|(line, net)| (line.id, net))
        .collect();
    Ok((invoice_lines, planned))
}

/// The planned `(invoice line id, net)` pairs resolved against
/// `invoice_lines`.
fn resolve<'a>(
    invoice_lines: &'a [InvoiceLine],
    planned: &[(i64, i64)],
) -> Vec<(&'a InvoiceLine, i64)> {
    planned
        .iter()
        .filter_map(|(id, net)| {
            invoice_lines
                .iter()
                .find(|l| l.id == *id)
                .map(|l| (l, *net))
        })
        .collect()
}

/// A credit note and its lines, built but not yet stored.
pub(crate) struct Prepared {
    /// The invoice as it is now.
    pub invoice: Invoice,
    /// The credit note, a `draft` with its totals set.
    pub note: CreditNote,
    /// Its lines, to be given the note's id on insert.
    pub lines: Vec<CreditNoteLine>,
}

/// Plans `lines` on invoice `invoice_id` as it is now and builds the draft
/// credit note of `kind` with its lines. The legal entity, currency and
/// rate are the invoice's. Refusals are those of [`draft`].
pub(crate) async fn prepare<'c>(
    db: impl renox::db::Executor<'c>,
    invoice_id: i64,
    lines: &[CreditLine],
    reason: &str,
    kind: CreditNoteKind,
    created_by: Option<i64>,
) -> Result<Prepared> {
    let mut conn = db.into_conn();
    // The invoice as it is now, not as the caller loaded it.
    let invoice = Invoice::find_or_404(conn.reborrow(), invoice_id).await?;
    let (invoice_lines, planned) = plan_for(conn.reborrow(), &invoice, lines, reason).await?;
    let mut note = CreditNote {
        invoice_id: invoice.id,
        legal_entity_id: invoice.legal_entity_id,
        party_id: invoice.party_id,
        currency: invoice.currency.clone(),
        exchange_rate_micro: invoice.exchange_rate_micro,
        reason: reason.trim().to_owned(),
        kind,
        created_by,
        status: CreditNoteStatus::Draft,
        ..Default::default()
    };
    let (new_lines, totals) = build(0, &resolve(&invoice_lines, &planned), note.aed_rate());
    note.set_totals(&totals);
    Ok(Prepared {
        invoice,
        note,
        lines: new_lines,
    })
}

/// Inserts `prepared` in `tx`: the credit note, then its lines.
pub(crate) async fn insert_in(
    tx: &mut renox::db::Transaction,
    prepared: Prepared,
) -> Result<(CreditNote, usize)> {
    let Prepared {
        mut note,
        mut lines,
        ..
    } = prepared;
    note.insert(&mut *tx).await?;
    for line in &mut lines {
        line.credit_note_id = note.id;
        line.insert(&mut *tx).await?;
    }
    // Findable as a draft from the commit on (BL-AGT-04).
    wl_kernel::search::index_in(&mut *tx, &note.subject()).await?;
    Ok((note, lines.len()))
}

/// Drafts a credit note against `invoice` for `lines` (the net on each
/// invoice line, by position) with its lines and totals in one
/// transaction. It is a `draft`: no number, no remaining balance, and the
/// invoice is not touched. The legal entity, currency and rate are the
/// invoice's.
///
/// Refused (422) for an invoice that is `draft`, `credited` or
/// `written_off`, a blank reason, no lines, an unknown or repeated line, a
/// net of 0 or less, or a net above the line's remaining credit.
pub async fn draft(
    db: &Db,
    invoice: &Invoice,
    lines: &[CreditLine],
    reason: &str,
    created_by: Option<i64>,
) -> Result<CreditNote> {
    let prepared = prepare(
        db,
        invoice.id,
        lines,
        reason,
        CreditNoteKind::Partial,
        created_by,
    )
    .await?;
    let mut tx = db.begin().await?;
    let (note, _) = insert_in(&mut tx, prepared).await?;
    tx.commit().await?;
    Ok(note)
}

/// Changes the lines and the reason of draft `credit_note`, replacing its
/// lines and totals in one transaction. Same refusals as [`draft`], and 422
/// when it is no longer a `draft`.
pub async fn change(
    db: &Db,
    credit_note: &CreditNote,
    lines: &[CreditLine],
    reason: &str,
) -> Result<CreditNote> {
    let mut note = CreditNote::find_or_404(db, credit_note.id).await?;
    if note.status != CreditNoteStatus::Draft {
        return Err(refused("only a draft credit note can be changed"));
    }
    let invoice = Invoice::find_or_404(db, note.invoice_id).await?;
    let (invoice_lines, planned) = plan_for(db, &invoice, lines, reason).await?;
    let (mut new_lines, totals) =
        build(note.id, &resolve(&invoice_lines, &planned), note.aed_rate());
    note.reason = reason.trim().to_owned();
    note.set_totals(&totals);

    let mut tx = db.begin().await?;
    note.save(&mut tx).await?;
    let old_lines = CreditNoteLine::query()
        .where_(CreditNoteLine::CREDIT_NOTE_ID.eq(note.id))
        .get(&mut tx)
        .await?;
    for mut old in old_lines {
        old.delete(&mut tx).await?;
    }
    for line in &mut new_lines {
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(note)
}

/// Deletes draft `credit_note` and its lines. Refused (422) when it is no
/// longer a `draft`: an issued credit note is never deleted.
pub async fn discard(db: &Db, credit_note: &CreditNote) -> Result<()> {
    let mut note = CreditNote::find_or_404(db, credit_note.id).await?;
    if note.status != CreditNoteStatus::Draft {
        return Err(refused("only a draft credit note can be discarded"));
    }
    let mut tx = db.begin().await?;
    let old_lines = CreditNoteLine::query()
        .where_(CreditNoteLine::CREDIT_NOTE_ID.eq(note.id))
        .get(&mut tx)
        .await?;
    for mut old in old_lines {
        old.delete(&mut tx).await?;
    }
    note.delete(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Who runs a transition: a signed-in user (their permission is checked) or
/// the system, optionally on behalf of a user id (kept in the history
/// row's `meta` as `ran_by`).
#[derive(Clone, Copy)]
pub enum By<'a> {
    /// A signed-in user.
    User(&'a AuthUser),
    /// The system, for a caller that has only a user id, or none.
    System(Option<i64>),
}

impl By<'_> {
    /// The user's id, if any.
    pub fn id(&self) -> Option<i64> {
        match self {
            By::User(user) => Some(user.id),
            By::System(id) => *id,
        }
    }
}

/// `record`'s transition `user_name` for a user, or `system_name` for the
/// system, with `meta` (an object) and `ran_by` added to it.
pub(crate) fn run_of<'r, M: StateMachine>(
    record: &'r M,
    by: &By<'_>,
    user_name: &str,
    system_name: &str,
    mut meta: Value,
) -> TransitionBuilder<'r, M> {
    if let Some(id) = by.id() {
        meta["ran_by"] = json!(id);
    }
    match by {
        By::User(user) => record.transition(user_name).by(user).meta(meta),
        By::System(_) => record.transition(system_name).by_system().meta(meta),
    }
}

/// The outcomes of one service call, to emit after its commit.
#[derive(Default)]
pub(crate) struct Outcomes {
    pub notes: Vec<Outcome<CreditNote>>,
    pub invoices: Vec<Outcome<Invoice>>,
}

impl Outcomes {
    /// Queues them on the caller's `after` (a transaction-aware contract
    /// call, BL-KRN-11): they are emitted when the caller runs it after its
    /// commit, and never on a rollback.
    pub fn queue_on(self, after: &mut wl_kernel::after_commit::AfterCommit) {
        for outcome in self.notes {
            after.push(move |state: AppState| async move { outcome.emit(&state).await });
        }
        for outcome in self.invoices {
            after.push(move |state: AppState| async move { outcome.emit(&state).await });
        }
    }

    /// Emits them all (after the commit).
    pub async fn emit(self, state: &AppState) -> std::result::Result<(), TransitionError> {
        for outcome in self.notes {
            outcome.emit(state).await.map_err(TransitionError::Failed)?;
        }
        for outcome in self.invoices {
            outcome.emit(state).await.map_err(TransitionError::Failed)?;
        }
        Ok(())
    }
}

/// What the issue guards need for `invoice`, whose issuing entity is
/// looked up in `db`.
pub(crate) async fn issue_input<'c>(
    db: impl renox::db::Executor<'c>,
    invoice: &Invoice,
    lines: usize,
    line_problem: Option<String>,
) -> Result<IssueInput> {
    let entity = match invoice.legal_entity_id {
        Some(id) => LegalEntity::find(db, id).await?,
        None => None,
    };
    Ok(IssueInput {
        invoice_status: Some(invoice.status),
        invoice_entity: invoice.legal_entity_id,
        entity_active: entity.as_ref().is_some_and(LegalEntity::is_active),
        lines,
        line_problem,
    })
}

/// What the guards of issuing draft `note` need, from the stored invoice,
/// lines and credit already issued.
async fn gather(db: &Db, note: &CreditNote) -> Result<IssueInput> {
    let invoice = Invoice::find_or_404(db, note.invoice_id).await?;
    let invoice_lines = crate::invoicer::lines_of(db, invoice.id).await?;
    let credited = credited_by_line(db, invoice.id).await?;
    let stored = lines_of(db, note.id).await?;
    let requested: Vec<CreditLine> = stored
        .iter()
        .filter_map(|line| {
            invoice_lines
                .iter()
                .find(|l| l.id == line.invoice_line_id)
                .map(|l| CreditLine::new(l.position, line.net_amount))
        })
        .collect();
    let problem = if requested.len() == stored.len() {
        plan_lines(&requested, &invoice_lines, &credited).err()
    } else {
        Some("a line of the credit note is no longer on the invoice".to_owned())
    };
    issue_input(db, &invoice, stored.len(), problem).await
}

/// Lowers the remaining of credit note `note` (as loaded) by `amount`, only
/// from the remaining that was loaded, so two uses at once never both pass.
/// 422 when it moved or is too low.
pub(crate) async fn lower_remaining_in(
    tx: &mut renox::db::Transaction,
    note: &CreditNote,
    amount: i64,
) -> Result<()> {
    if amount <= 0 || amount > note.remaining {
        return Err(refused(
            "the amount is above the credit note's remaining balance",
        ));
    }
    let moved = renox::db::sql(format!(
        "UPDATE {} SET remaining = remaining - ?, updated_at = ? WHERE id = ? AND remaining = ?",
        CreditNote::TABLE
    ))
    .bind(amount)
    .bind(renox::db::now())
    .bind(note.id)
    .bind(note.remaining)
    .execute(&mut *tx)
    .await?;
    if moved == 0 {
        return Err(refused("The credit note changed; try again."));
    }
    Ok(())
}

/// Runs `issue` on draft `note` in `tx`, then reloads it.
pub(crate) async fn issue_in(
    tx: &mut renox::db::Transaction,
    note: &CreditNote,
    input: &IssueInput,
    by: &By<'_>,
    outcomes: &mut Outcomes,
) -> std::result::Result<CreditNote, TransitionError> {
    let outcome = run_of(note, by, ISSUE, ISSUE_SYSTEM, json!({}))
        .input(input)
        .run_in(&mut *tx)
        .await?;
    outcomes.notes.push(outcome);
    Ok(CreditNote::find_or_404(&mut *tx, note.id).await?)
}

/// Runs `apply` on `note` (its remaining is 0) in `tx`, then reloads it.
pub(crate) async fn apply_in(
    tx: &mut renox::db::Transaction,
    note: &CreditNote,
    by: &By<'_>,
    outcomes: &mut Outcomes,
) -> std::result::Result<CreditNote, TransitionError> {
    // Always the system: a user does not apply a credit note.
    let mut meta = json!({});
    if let Some(id) = by.id() {
        meta["ran_by"] = json!(id);
    }
    let outcome = note
        .transition(APPLY)
        .by_system()
        .meta(meta)
        .run_in(&mut *tx)
        .await?;
    outcomes.notes.push(outcome);
    Ok(CreditNote::find_or_404(&mut *tx, note.id).await?)
}

/// Uses `amount` of `note` (as loaded) on `invoice` in `tx`: lowers the
/// remaining, allocates to the invoice with the credit note as the source,
/// and writes the `invoice_allocated` movement (its target the allocation).
async fn use_in(
    tx: &mut renox::db::Transaction,
    note: &CreditNote,
    invoice: &Invoice,
    amount: i64,
    by: &By<'_>,
    outcomes: &mut Outcomes,
) -> std::result::Result<(), TransitionError> {
    lower_remaining_in(tx, note, amount).await?;
    let source = note.subject();
    let moved = allocate_in(tx, invoice, &source, amount, by.id()).await?;
    outcomes.invoices.push(moved);
    let allocation = Allocation::query()
        .where_(Allocation::INVOICE_ID.eq(invoice.id))
        .where_(Allocation::SOURCE_TYPE.eq(source.subject_type.clone()))
        .where_(Allocation::SOURCE_ID.eq(source.subject_id))
        .order_by_desc(Allocation::ID)
        .first(&mut *tx)
        .await?
        .ok_or_else(|| Error::from(renox::anyhow::anyhow!("the allocation was not written")))?;
    CreditNoteMovement::new(
        note.id,
        MovementKind::InvoiceAllocated,
        Some(&Subject::new(ALLOCATION_SUBJECT_TYPE, allocation.id)),
        amount,
        None,
        by.id(),
    )
    .insert(&mut *tx)
    .await?;
    Ok(())
}

/// After `issue`: allocates the lower of the credit note's remaining and the
/// invoice's balance to the invoice (through [`allocate_in`], the credit
/// note the source), and applies the credit note when nothing is left. The
/// part above the balance, or everything for a `paid` invoice, stays as the
/// party's credit.
pub(crate) async fn allocate_step(
    tx: &mut renox::db::Transaction,
    note: CreditNote,
    by: &By<'_>,
    outcomes: &mut Outcomes,
) -> std::result::Result<CreditNote, TransitionError> {
    let invoice = Invoice::find_or_404(&mut *tx, note.invoice_id).await?;
    let open = [
        InvoiceStatus::Issued,
        InvoiceStatus::PartiallyPaid,
        InvoiceStatus::Overdue,
    ]
    .contains(&invoice.status);
    let amount = if open {
        note.remaining.min(invoice.balance)
    } else {
        0
    };
    let mut note = note;
    if amount > 0 {
        use_in(tx, &note, &invoice, amount, by, outcomes).await?;
        note = CreditNote::find_or_404(&mut *tx, note.id).await?;
    }
    if note.remaining == 0 {
        note = apply_in(tx, &note, by, outcomes).await?;
    }
    Ok(note)
}

/// Issues draft credit note `note` as `by`, in one transaction: `issue`
/// (the number, the snapshots, the issue date), then the allocation to its
/// invoice and `apply` when the whole gross was allocated. The events are
/// emitted after the commit.
///
/// # Errors
///
/// `Forbidden` without `invoices.credit`, `NotFrom` when it is not a draft,
/// `Guard` (422) for an invoice that cannot be credited, a blank reason, no
/// lines, a line above its remaining credit or an entity that differs or is
/// not active. Nothing is written then and no number is used.
pub async fn issue(
    state: &AppState,
    by: By<'_>,
    note: &CreditNote,
) -> std::result::Result<CreditNote, TransitionError> {
    let note = CreditNote::find_or_404(&state.db, note.id).await?;
    let input = gather(&state.db, &note).await?;
    let mut outcomes = Outcomes::default();
    let mut tx = state.db.begin().await?;
    let issued = issue_in(&mut tx, &note, &input, &by, &mut outcomes).await?;
    allocate_step(&mut tx, issued, &by, &mut outcomes).await?;
    tx.commit().await?;
    outcomes.emit(state).await?;
    Ok(CreditNote::find_or_404(&state.db, note.id).await?)
}

/// Drafts and issues, in one transaction, a credit note for `lines` of
/// invoice `invoice_id` (the Invoicer's `credit_partly`), then allocates it
/// as [`issue`] does. Refusals are those of [`draft`] and [`issue`]; a
/// refused issue leaves no draft behind.
pub async fn credit_partly(
    state: &AppState,
    invoice_id: i64,
    lines: &[CreditLine],
    reason: &str,
    by: By<'_>,
) -> std::result::Result<CreditNote, TransitionError> {
    let mut outcomes = Outcomes::default();
    let mut tx = state.db.begin().await?;
    let done = credit_partly_in(&mut tx, invoice_id, lines, reason, &by, &mut outcomes).await?;
    tx.commit().await?;
    outcomes.emit(state).await?;
    Ok(done)
}

/// [`credit_partly`] inside the caller's transaction `tx` (the Invoicer's
/// `credit_partly_in`, BL-KRN-11): every read and write is in `tx`, the
/// number comes from the series in `tx`, and the outcomes are left in
/// `outcomes` to emit after the caller's commit. Nothing is committed.
pub(crate) async fn credit_partly_in(
    tx: &mut renox::db::Transaction,
    invoice_id: i64,
    lines: &[CreditLine],
    reason: &str,
    by: &By<'_>,
    outcomes: &mut Outcomes,
) -> std::result::Result<CreditNote, TransitionError> {
    let prepared = prepare(
        &mut *tx,
        invoice_id,
        lines,
        reason,
        CreditNoteKind::Partial,
        by.id(),
    )
    .await?;
    let input = issue_input(&mut *tx, &prepared.invoice, prepared.lines.len(), None).await?;
    let (note, _) = insert_in(tx, prepared).await?;
    let issued = issue_in(tx, &note, &input, by, outcomes).await?;
    allocate_step(tx, issued, by, outcomes).await
}

/// Uses `amount` of the remaining balance of issued credit note `note` on
/// `invoice`, another open invoice of the same party and currency, as `by`
/// (a user needs `invoices.credit`), through the invoice's `allocate`. The
/// credit note is `applied` when nothing is left. In one transaction; the
/// events are emitted after the commit.
///
/// # Errors
///
/// `Forbidden` without `invoices.credit`; a 422 when the credit note is not
/// `issued`, the invoice is not `issued`, `partially_paid` or `overdue`, the
/// party or currency differ, or the amount is not above 0, or is above the
/// remaining balance or the invoice's balance. Nothing is written then.
pub async fn use_on_invoice(
    state: &AppState,
    by: By<'_>,
    note: &CreditNote,
    invoice: &Invoice,
    amount: i64,
) -> std::result::Result<CreditNote, TransitionError> {
    if let By::User(user) = &by
        && !user.has_permission_anywhere(crate::permissions::CREDIT)
    {
        return Err(Error::Forbidden.into());
    }
    let note = CreditNote::find_or_404(&state.db, note.id).await?;
    let invoice = Invoice::find_or_404(&state.db, invoice.id).await?;
    if note.status != CreditNoteStatus::Issued {
        return Err(refused("only an issued credit note can be used").into());
    }
    let open = [
        InvoiceStatus::Issued,
        InvoiceStatus::PartiallyPaid,
        InvoiceStatus::Overdue,
    ]
    .contains(&invoice.status);
    if !open {
        return Err(refused("the invoice is not open for payment").into());
    }
    if invoice.party_id != note.party_id || invoice.currency != note.currency {
        return Err(refused("the invoice is of another party or currency").into());
    }
    if amount <= 0 || amount > note.remaining || amount > invoice.balance {
        return Err(refused(
            "the amount must be above 0 and within the credit note's remaining balance and the invoice's balance",
        )
        .into());
    }
    let mut outcomes = Outcomes::default();
    let mut tx = state.db.begin().await?;
    use_in(&mut tx, &note, &invoice, amount, &by, &mut outcomes).await?;
    let used = CreditNote::find_or_404(&mut tx, note.id).await?;
    if used.remaining == 0 {
        apply_in(&mut tx, &used, &by, &mut outcomes).await?;
    }
    tx.commit().await?;
    outcomes.emit(state).await?;
    Ok(CreditNote::find_or_404(&state.db, note.id).await?)
}

/// What other modules see of the credit notes of invoice `invoice_id`,
/// oldest first.
pub async fn for_invoice(db: &Db, invoice_id: i64) -> Result<Vec<CreditNote>> {
    CreditNote::query()
        .where_(CreditNote::INVOICE_ID.eq(invoice_id))
        .order_by(CreditNote::ID)
        .get(db)
        .await
}

/// The sum of the remaining balances of `party_id`'s `issued` credit notes
/// in `currency`.
pub async fn open_credit(db: &Db, party_id: i64, currency: &str) -> Result<i64> {
    let notes = CreditNote::query()
        .where_(CreditNote::PARTY_ID.eq(party_id))
        .where_(CreditNote::CURRENCY.eq(currency))
        .where_(CreditNote::STATUS.eq(CreditNoteStatus::Issued))
        .get(db)
        .await?;
    Ok(notes.iter().map(|n| n.remaining).sum())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::invoicing::VatCategory;

    fn lines() -> Vec<InvoiceLine> {
        vec![
            InvoiceLine {
                id: 11,
                position: 1,
                net_amount: 100_000,
                vat_category: VatCategory::Standard,
                ..Default::default()
            },
            InvoiceLine {
                id: 12,
                position: 2,
                net_amount: 50_000,
                vat_category: VatCategory::ZeroRated,
                ..Default::default()
            },
        ]
    }

    fn check(
        status: InvoiceStatus,
        reason: &str,
        requested: &[CreditLine],
        credited: &HashMap<i64, i64>,
    ) -> std::result::Result<Vec<(i64, i64)>, String> {
        let lines = lines();
        plan(status, reason, requested, &lines, credited)
            .map(|p| p.into_iter().map(|(l, n)| (l.id, n)).collect())
    }

    #[test]
    fn a_credit_within_the_line_is_planned() {
        let none = HashMap::new();
        let request = [CreditLine::new(1, 20_000), CreditLine::new(2, 50_000)];
        assert_eq!(
            check(InvoiceStatus::Issued, " Damaged ", &request, &none),
            Ok(vec![(11, 20_000), (12, 50_000)])
        );
        for status in CREDITABLE {
            assert!(check(status, "x", &request, &none).is_ok());
        }
    }

    #[test]
    fn the_credit_is_limited_by_what_was_credited() {
        let credited = HashMap::from([(11, 80_000)]);
        let at_most = check(
            InvoiceStatus::Issued,
            "x",
            &[CreditLine::new(1, 20_000)],
            &credited,
        );
        assert_eq!(at_most, Ok(vec![(11, 20_000)]));
        let over = check(
            InvoiceStatus::Issued,
            "x",
            &[CreditLine::new(1, 20_001)],
            &credited,
        );
        assert!(over.unwrap_err().contains("20000 fils at most"));
    }

    #[test]
    fn bad_credits_are_refused() {
        let none = HashMap::new();
        let ok = [CreditLine::new(1, 100)];
        for status in [
            InvoiceStatus::Draft,
            InvoiceStatus::Credited,
            InvoiceStatus::WrittenOff,
        ] {
            assert!(
                check(status, "x", &ok, &none)
                    .unwrap_err()
                    .contains("cannot be credited")
            );
        }
        assert!(
            check(InvoiceStatus::Issued, "  ", &ok, &none)
                .unwrap_err()
                .contains("reason")
        );
        assert!(
            check(InvoiceStatus::Issued, "x", &[], &none)
                .unwrap_err()
                .contains("at least one line")
        );
        let unknown = check(InvoiceStatus::Issued, "x", &[CreditLine::new(3, 1)], &none);
        assert!(unknown.unwrap_err().contains("no line 3"));
        let twice = [CreditLine::new(1, 1), CreditLine::new(1, 2)];
        assert!(
            check(InvoiceStatus::Issued, "x", &twice, &none)
                .unwrap_err()
                .contains("twice")
        );
        for net in [0, -5] {
            let zero = [CreditLine::new(2, net)];
            assert!(
                check(InvoiceStatus::Issued, "x", &zero, &none)
                    .unwrap_err()
                    .contains("more than 0")
            );
        }
    }

    #[test]
    fn lines_and_totals_are_built_per_line() {
        let invoice_lines = lines();
        let planned = [(&invoice_lines[0], 20_000), (&invoice_lines[1], 5_000)];
        let (built, totals) = build(9, &planned, Some(vat::RATE_SCALE));
        assert_eq!(built.len(), 2);
        assert_eq!((built[0].credit_note_id, built[0].position), (9, 1));
        assert_eq!((built[1].position, built[1].invoice_line_id), (2, 12));
        assert_eq!(built[0].vat_amount, 1_000);
        assert_eq!(built[1].vat_amount, 0);
        assert_eq!(
            (totals.net, totals.vat, totals.gross),
            (25_000, 1_000, 26_000)
        );
        assert_eq!(totals.vat_aed, Some(1_000));
    }
}
