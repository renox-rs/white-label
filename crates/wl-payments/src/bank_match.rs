//! The match engine of the bank reconciliation (wl-payments §5.7 points 2 to
//! 4, `FIN.BNK.01`, BL-PAY-29).
//!
//! - **Suggestions** ([`suggest_import`], run after `parse`). Each line is
//!   compared, in this order, with: a payment already recorded (same entity,
//!   amount, method transfer, dated within 3 days, not on a line yet:
//!   *already recorded*); a deposited cheque of the amount (*cheque*); an
//!   open invoice of the bank account's entity of exactly the amount whose
//!   number is in the reference or description (*reference*); an open
//!   invoice of exactly the amount of a party whose name is like the payer's
//!   (*payer name*); the oldest open invoices of such a party adding up to
//!   the amount (*oldest invoices*). Below these there is no suggestion and
//!   the line stays `unmatched`. Money out is only suggested as a payment
//!   already recorded (matching it to unpaid bills is later). A line with a
//!   suggestion is `suggested`; invoices of another entity are never offered
//!   (the search is the entity's, through the kernel's
//!   [`OpenInvoiceSearch`](wl_kernel::invoice_search::OpenInvoiceSearch)).
//! - **Confirm** ([`confirm`]) takes the suggestion; **match manually**
//!   ([`match_manually`]) names the party and the invoices with the amount of
//!   each (one line to several invoices; what is left of the line stays on
//!   the party's credit). Both make the line `matched` and record nothing.
//! - **At `complete`** ([`record_confirmed`]): a confirmed line to invoices
//!   records one transfer for the whole line and allocates it; a line
//!   matched to a deposited cheque clears the cheque through its `clear`
//!   (which records the cheque's payment: no second one); a line matched to a
//!   payment already recorded only links it.
//! - **Reverse a match** ([`reverse_match`]): the payment recorded from a
//!   confirmed line is reversed, reason *entered in error*, note *bank match
//!   wrong*. Where the client has an approval policy on `payments.reverse`
//!   (01 `PRP.ACL.16`) this is a request that a property manager who did not
//!   ask approves; the allocations are removed and the invoices reopen then.

use std::collections::{BTreeMap, BTreeSet};

use renox::chrono::Days;
use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::TransitionError;
use wl_kernel::PartyId;
use wl_kernel::invoice_search::invoice_search;
use wl_kernel::invoicing::{InvoiceId, InvoiceSummary, invoicer};
use wl_kernel::payments::{
    AllocationTarget, Direction, PaymentId, PaymentMethod, PaymentStatus, RecordPayment,
    ReversalReason,
};

use crate::bank_statement::{BankStatementImport, ImportStatus};
use crate::bank_statement_line::{BankStatementLine, LineStatus};
use crate::cheque::Cheque;
use crate::payment::Payment;
use crate::permissions;

/// A line matched to invoices: its payment is recorded at `complete`.
pub const KIND_INVOICES: &str = "invoices";
/// A line matched to a payment (already recorded, or recorded at `complete`).
pub const KIND_PAYMENT: &str = "payment";
/// A line matched to a deposited cheque, cleared at `complete`.
pub const KIND_CHEQUE: &str = "cheque";

/// The note of a reversed match.
pub const REVERSE_NOTE: &str = "bank match wrong";

/// How many days apart a payment already recorded may be dated.
const RECORDED_DAYS: u64 = 3;

/// What the engine proposes for a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// [`KIND_INVOICES`], [`KIND_PAYMENT`] or [`KIND_CHEQUE`].
    pub kind: &'static str,
    /// The party.
    pub party: Option<i64>,
    /// The targets as `(id, amount)`.
    pub targets: Vec<(i64, i64)>,
    /// Why.
    pub reason: &'static str,
    /// How sure, 0 to 100.
    pub score: i64,
}

/// The words of `text` in lower case, letters and digits only.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Words a company name carries that say nothing about who it is.
const FILLER: [&str; 10] = [
    "llc", "l", "c", "fze", "fzco", "fz", "co", "company", "est", "the",
];

/// Whether the payer text `text` names `party_name`: at least half of the
/// name's significant words (and at least one) appear in it.
pub fn similar_name(party_name: &str, text: &str) -> bool {
    let name: Vec<String> = words(party_name)
        .into_iter()
        .filter(|w| !FILLER.contains(&w.as_str()) && w.len() > 1)
        .collect();
    if name.is_empty() {
        return false;
    }
    let seen: BTreeSet<String> = words(text).into_iter().collect();
    let found = name.iter().filter(|w| seen.contains(*w)).count();
    found * 2 >= name.len() && found > 0
}

/// Whether `text` carries the invoice number `number` (case and spacing
/// ignored).
pub fn mentions(number: &str, text: &str) -> bool {
    let squash = |s: &str| {
        s.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_uppercase()
    };
    let number = squash(number);
    !number.is_empty() && squash(text).contains(&number)
}

/// The oldest open invoices, in order, whose balances add up to `amount`
/// exactly, as `(id, balance)`; `None` when no prefix does.
pub fn oldest_covering(invoices: &[(i64, i64)], amount: i64) -> Option<Vec<(i64, i64)>> {
    let mut sum = 0;
    let mut taken = Vec::new();
    for (id, balance) in invoices {
        sum += balance;
        taken.push((*id, *balance));
        if sum == amount {
            return Some(taken);
        }
        if sum > amount {
            return None;
        }
    }
    None
}

/// The text the payer is read from: the counterparty and the description.
fn payer_text(line: &BankStatementLine) -> String {
    format!("{} {}", line.counterparty, line.description)
}

/// The references of `line`: the reference and the description.
fn reference_text(line: &BankStatementLine) -> String {
    format!("{} {}", line.reference, line.description)
}

/// A payment already recorded that `line` may be: same entity, direction,
/// amount, a transfer dated within 3 days, received, on no line yet, and not
/// taken by another line (`taken`).
async fn recorded_payment(
    db: &Db,
    line: &BankStatementLine,
    taken: &BTreeSet<i64>,
) -> Result<Option<Payment>> {
    let (direction, amount) = if line.amount >= 0 {
        (Direction::In, line.amount)
    } else {
        (Direction::Out, -line.amount)
    };
    let from = line.line_date - Days::new(RECORDED_DAYS);
    let to = line.line_date + Days::new(RECORDED_DAYS);
    let found = Payment::query()
        .where_(Payment::LEGAL_ENTITY_ID.eq(line.legal_entity_id))
        .where_(Payment::DIRECTION.eq(direction))
        .where_(Payment::AMOUNT.eq(amount))
        .where_(Payment::METHOD.eq(PaymentMethod::Transfer))
        .where_(Payment::STATUS.eq(PaymentStatus::Received))
        .where_(Payment::STATEMENT_LINE_ID.is_null())
        .where_(Payment::PAYMENT_DATE.gte(from))
        .where_(Payment::PAYMENT_DATE.lte(to))
        .order_by(Payment::PAYMENT_DATE)
        .order_by(Payment::ID)
        .get(db)
        .await?;
    Ok(found.into_iter().find(|p| !taken.contains(&p.id)))
}

/// A deposited cheque of `line`'s amount, deposited (or dated) on or before
/// the line, not taken by another line.
async fn deposited_cheque(
    db: &Db,
    line: &BankStatementLine,
    taken: &BTreeSet<i64>,
) -> Result<Option<Cheque>> {
    let found = Cheque::query()
        .where_(Cheque::LEGAL_ENTITY_ID.eq(line.legal_entity_id))
        .where_(Cheque::DIRECTION.eq(Direction::In))
        .where_(Cheque::STATUS.eq(wl_kernel::cheques::ChequeStatus::Deposited))
        .where_(Cheque::OPEN_AMOUNT.eq(line.amount))
        .order_by(Cheque::CHEQUE_DATE)
        .order_by(Cheque::ID)
        .get(db)
        .await?;
    Ok(found.into_iter().find(|c| {
        !taken.contains(&c.id) && c.deposit_date.unwrap_or(c.cheque_date) <= line.line_date
    }))
}

/// What the other lines of the import already took, by kind.
#[derive(Debug, Default)]
struct Taken {
    payments: BTreeSet<i64>,
    cheques: BTreeSet<i64>,
    invoices: BTreeSet<i64>,
}

fn taken_by(lines: &[BankStatementLine], except: i64) -> Taken {
    let mut taken = Taken::default();
    for other in lines.iter().filter(|l| l.id != except) {
        let pairs = [
            (other.suggestion_kind.as_deref(), &other.suggestion_targets),
            (other.match_kind.as_deref(), &other.match_targets),
        ];
        for (kind, targets) in pairs {
            let ids = targets.iter().flat_map(|t| t.0.iter().map(|(id, _)| *id));
            match kind {
                Some(KIND_PAYMENT) => taken.payments.extend(ids),
                Some(KIND_CHEQUE) => taken.cheques.extend(ids),
                Some(KIND_INVOICES) => taken.invoices.extend(ids),
                _ => {}
            }
        }
    }
    taken
}

/// The suggestion for `line` of an import of `currency`, if any; `lines`
/// are the import's lines (what they took is not offered twice).
pub async fn suggest(
    state: &AppState,
    currency: &str,
    line: &BankStatementLine,
    lines: &[BankStatementLine],
) -> Result<Option<Suggestion>> {
    let db = &state.db;
    let taken = taken_by(lines, line.id);
    if let Some(payment) = recorded_payment(db, line, &taken.payments).await? {
        return Ok(Some(Suggestion {
            kind: KIND_PAYMENT,
            party: Some(payment.party_id),
            targets: vec![(payment.id, payment.amount)],
            reason: "already_recorded",
            score: 95,
        }));
    }
    if line.amount <= 0 {
        return Ok(None);
    }
    if let Some(cheque) = deposited_cheque(db, line, &taken.cheques).await? {
        return Ok(Some(Suggestion {
            kind: KIND_CHEQUE,
            party: Some(cheque.party_id),
            targets: vec![(cheque.id, cheque.open_amount)],
            reason: "cheque",
            score: 90,
        }));
    }
    let open: Vec<InvoiceSummary> = invoice_search(state)
        .open_of_entity(db, line.legal_entity_id, currency)
        .await?
        .into_iter()
        .filter(|i| !taken.invoices.contains(&i.id.0))
        .collect();
    let references = reference_text(line);
    if let Some(invoice) = open.iter().find(|i| {
        i.balance == line.amount
            && i.number
                .as_deref()
                .is_some_and(|n| mentions(n, &references))
    }) {
        return Ok(Some(Suggestion {
            kind: KIND_INVOICES,
            party: Some(invoice.party.0),
            targets: vec![(invoice.id.0, invoice.balance)],
            reason: "reference",
            score: 100,
        }));
    }
    // The parties whose name is like the payer's.
    let payer = payer_text(line);
    let directory = wl_kernel::parties::directory(state);
    let mut names: BTreeMap<i64, bool> = BTreeMap::new();
    for invoice in &open {
        if names.contains_key(&invoice.party.0) {
            continue;
        }
        let name = directory
            .display_name(db, invoice.party, "en")
            .await?
            .unwrap_or_default();
        names.insert(invoice.party.0, similar_name(&name, &payer));
    }
    let alike = |party: PartyId| names.get(&party.0).copied().unwrap_or(false);
    if let Some(invoice) = open
        .iter()
        .find(|i| i.balance == line.amount && alike(i.party))
    {
        return Ok(Some(Suggestion {
            kind: KIND_INVOICES,
            party: Some(invoice.party.0),
            targets: vec![(invoice.id.0, invoice.balance)],
            reason: "payer_name",
            score: 80,
        }));
    }
    for (party, _) in names.iter().filter(|(_, alike)| **alike) {
        let of_party: Vec<(i64, i64)> = open
            .iter()
            .filter(|i| i.party.0 == *party)
            .map(|i| (i.id.0, i.balance))
            .collect();
        if let Some(targets) = oldest_covering(&of_party, line.amount) {
            return Ok(Some(Suggestion {
                kind: KIND_INVOICES,
                party: Some(*party),
                targets,
                reason: "oldest_invoices",
                score: 60,
            }));
        }
    }
    Ok(None)
}

/// Computes the suggestions of the lines of import `import_id` left
/// `unmatched` (not a drawing): a line with one becomes `suggested`. Returns
/// how many were suggested.
pub async fn suggest_import(state: &AppState, import_id: i64) -> Result<usize> {
    let import = BankStatementImport::find_or_404(&state.db, import_id).await?;
    let mut lines = BankStatementLine::of_import(&state.db, import.id).await?;
    let mut suggested = 0;
    for i in 0..lines.len() {
        let line = lines[i].clone();
        if line.status != LineStatus::Unmatched || line.drawing_request_id.is_some() {
            continue;
        }
        let Some(found) = suggest(state, &import.currency, &line, &lines).await? else {
            continue;
        };
        let mut row = line;
        row.status = LineStatus::Suggested;
        row.suggestion_kind = Some(found.kind.to_owned());
        row.suggestion_party_id = found.party;
        row.suggestion_targets = Some(Json(found.targets));
        row.suggestion_reason = Some(found.reason.to_owned());
        row.suggestion_score = Some(found.score);
        row.save(&state.db).await?;
        lines[i] = row;
        suggested += 1;
    }
    Ok(suggested)
}

fn guard(message: &str) -> TransitionError {
    TransitionError::Guard(message.to_owned())
}

/// The line `id` of an import in review, for `user` with `bank.reconcile`.
async fn line_in_review(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<(BankStatementLine, BankStatementImport), TransitionError> {
    if !user.has_permission(permissions::BANK_RECONCILE) {
        return Err(TransitionError::Forbidden);
    }
    let line = BankStatementLine::find_or_404(&state.db, id).await?;
    let import = BankStatementImport::find_or_404(&state.db, line.import_id).await?;
    if import.status != ImportStatus::InReview {
        return Err(TransitionError::NotFrom(import.status.as_str().to_owned()));
    }
    Ok((line, import))
}

/// `user` confirms the suggestion of line `id`: it becomes `matched` to what
/// was suggested. A payment already recorded is linked to the line now;
/// nothing is recorded before `complete`.
///
/// # Errors
///
/// `Forbidden` without `bank.reconcile`; `NotFrom` unless the import is in
/// review; `Guard` (422) for a line without a suggestion.
pub async fn confirm(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<BankStatementLine, TransitionError> {
    let (mut line, _import) = line_in_review(state, user, id).await?;
    let Some(kind) = line.suggestion_kind.clone() else {
        return Err(guard("The line has no suggestion to confirm."));
    };
    if line.status != LineStatus::Suggested || line.drawing_request_id.is_some() {
        return Err(guard("The line is not suggested."));
    }
    let targets = line
        .suggestion_targets
        .clone()
        .map(|t| t.0)
        .unwrap_or_default();
    line.status = LineStatus::Matched;
    line.match_kind = Some(kind.clone());
    line.match_party_id = line.suggestion_party_id;
    line.match_targets = Some(Json(targets.clone()));
    line.confirmed_by = Some(user.id);
    line.match_id = match kind.as_str() {
        KIND_PAYMENT | KIND_CHEQUE => targets.first().map(|(id, _)| *id),
        _ => None,
    };
    line.save(&state.db).await?;
    if kind == KIND_PAYMENT
        && let Some(payment) = line.match_id
    {
        Payment::query()
            .where_(Payment::ID.eq(payment))
            .update(&state.db, &[("statement_line_id", &Some(line.id))])
            .await?;
    }
    Ok(line)
}

/// `user` matches the incoming line `id` to invoices of `party` by hand, as
/// `(invoice, amount)`: one line to several invoices. What the amounts leave
/// of the line stays on the party's credit.
///
/// # Errors
///
/// `Forbidden` without `bank.reconcile`; `NotFrom` unless the import is in
/// review; `Guard` (422) for a line that is matched, ignored, a drawing or
/// money out, no invoice, an invoice of another party or entity or not open,
/// an amount of 0 or less or above the invoice's balance, or amounts above
/// the line.
pub async fn match_manually(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    party: PartyId,
    allocations: &[(i64, i64)],
) -> std::result::Result<BankStatementLine, TransitionError> {
    let (mut line, import) = line_in_review(state, user, id).await?;
    if !matches!(line.status, LineStatus::Unmatched | LineStatus::Suggested)
        || line.drawing_request_id.is_some()
    {
        return Err(guard("The line is not open to match."));
    }
    if line.amount <= 0 {
        return Err(guard("Only money in is matched to invoices."));
    }
    if allocations.is_empty() {
        return Err(guard("Choose the invoices the line pays."));
    }
    let mut total = 0;
    for (invoice, amount) in allocations {
        let Some(found) = invoicer(state).find(&state.db, InvoiceId(*invoice)).await? else {
            return Err(guard("An invoice was not found."));
        };
        if found.party != party || found.legal_entity_id != Some(import.legal_entity_id) {
            return Err(guard(
                "An invoice is not the party's or not of the bank account's entity.",
            ));
        }
        if !found.status.is_open() || *amount <= 0 || *amount > found.balance {
            return Err(guard(
                "An amount is 0 or less, or above the invoice's balance.",
            ));
        }
        total += amount;
    }
    if total > line.amount {
        return Err(guard("The amounts are above the line."));
    }
    line.status = LineStatus::Matched;
    line.match_kind = Some(KIND_INVOICES.to_owned());
    line.match_party_id = Some(party.0);
    line.match_targets = Some(Json(allocations.to_vec()));
    line.match_id = None;
    line.confirmed_by = Some(user.id);
    line.save(&state.db).await?;
    Ok(line)
}

/// At `complete` of `import` by `user`: records and allocates the payment of
/// each line confirmed to invoices that has none, and clears each cheque a
/// line was matched to. Lines matched to a payment already recorded need
/// nothing.
///
/// # Errors
///
/// The refusals of recording, allocating and clearing.
pub(crate) async fn record_confirmed(
    state: &AppState,
    user: &AuthUser,
    import: &BankStatementImport,
) -> std::result::Result<(), TransitionError> {
    for mut line in BankStatementLine::of_import(&state.db, import.id).await? {
        if line.status != LineStatus::Matched {
            continue;
        }
        match line.match_kind.as_deref() {
            Some(KIND_INVOICES) if line.match_id.is_none() => {
                let Some(party) = line.match_party_id else {
                    continue;
                };
                let reference = if line.reference.trim().is_empty() {
                    format!("LINE-{}", line.id)
                } else {
                    line.reference.trim().to_owned()
                };
                let mut request = RecordPayment::new(
                    PartyId(party),
                    line.legal_entity_id,
                    PaymentMethod::Transfer,
                    line.amount,
                );
                request.currency = line.currency.clone();
                request.date = Some(line.line_date);
                request.reference = Some(reference.clone());
                request.method_details = json!({ "reference": reference });
                request.statement_line_id = Some(line.id);
                request.bank_account = Some(import.bank_account.clone());
                let payment = crate::record::record_for(state, Some(user.id), &request).await?;
                for (invoice, amount) in line.match_targets.clone().map(|t| t.0).unwrap_or_default()
                {
                    crate::allocate::allocate(
                        state,
                        PaymentId(payment.id),
                        AllocationTarget::Invoice(InvoiceId(invoice)),
                        amount,
                        Some(user.id),
                    )
                    .await
                    .map_err(TransitionError::Failed)?;
                }
                line.match_id = Some(payment.id);
                line.save(&state.db).await?;
            }
            Some(KIND_CHEQUE) => {
                let Some(cheque) = line.match_id else {
                    continue;
                };
                let cheque = Cheque::find_or_404(&state.db, cheque).await?;
                if cheque.status == wl_kernel::cheques::ChequeStatus::Deposited {
                    crate::cheque_deposit::clear(state, user, &cheque).await?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The payment recorded from the confirmed line `id`, if it has one.
pub async fn payment_of_line(db: &Db, id: i64) -> Result<Option<Payment>> {
    let Some(line) = BankStatementLine::find(db, id).await? else {
        return Ok(None);
    };
    if line.status != LineStatus::Matched || line.match_kind.as_deref() != Some(KIND_INVOICES) {
        return Ok(None);
    }
    match line.match_id {
        Some(payment) => Payment::find(db, payment).await,
        None => Ok(None),
    }
}

/// `user` reverses the match of the confirmed line `id` (*bank match wrong*):
/// the payment recorded from it is reversed, or, under the client's approval
/// policy on `payments.reverse` (01 `PRP.ACL.16`), a request waits for an
/// approver who did not ask. Returns the payment as it stands.
///
/// # Errors
///
/// `Guard` (422) for a line whose payment was not recorded from a confirmed
/// match; the refusals of `reverse`.
pub async fn reverse_match(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Payment, TransitionError> {
    let Some(payment) = payment_of_line(&state.db, id).await? else {
        return Err(guard(
            "The line has no payment recorded from a confirmed match.",
        ));
    };
    crate::reverse::reverse(
        state,
        Some(user),
        None,
        &payment,
        Some(ReversalReason::EnteredInError),
        Some(REVERSE_NOTE),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payer_named_like_the_party_is_similar() {
        assert!(similar_name(
            "Al Noor Trading LLC",
            "TRF FROM AL NOOR TRADING"
        ));
        assert!(similar_name("Tara Nasser", "IBFT TARA NASSER RENT"));
        assert!(!similar_name("Tara Nasser", "IBFT OMAR SALEH"));
        assert!(!similar_name("LLC", "LLC"));
    }

    #[test]
    fn the_invoice_number_is_found_in_the_reference() {
        assert!(mentions("INV-ACME-2026-0310", "inv-acme-2026-0310 rent"));
        assert!(!mentions("INV-ACME-2026-0310", "INV-ACME-2026-0311"));
        assert!(!mentions("", "anything"));
    }

    #[test]
    fn the_oldest_invoices_cover_the_amount_exactly_or_not_at_all() {
        let open = [(1, 450_000), (2, 450_000), (3, 100_000)];
        assert_eq!(
            oldest_covering(&open, 900_000),
            Some(vec![(1, 450_000), (2, 450_000)])
        );
        assert_eq!(oldest_covering(&open, 500_000), None);
        assert_eq!(oldest_covering(&open, 2_000_000), None);
    }
}
