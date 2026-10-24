//! Registering cheques, and the module's [`Cheques`] (wl-payments §5.3 step
//! 1, §6): how a vertical hands the register the cheques it received with an
//! agreement, and reads them back.
//!
//! A cheque is registered `received` (a received cheque held, or an issued
//! one handed to the payee) with its dates and the documents it pays. A
//! post-dated schedule ([`ChequeSchedule`]) registers all the cheques of a
//! lease or none. The kernel passes the acting user as an id (`None` = the
//! system), and the calling module has authorized its user already.
//!
//! Refused (422): an amount of 0 or less, a blank bank or number, a currency
//! that is not an ISO code, an unknown party, an inactive legal entity, a
//! document linked twice, and a cheque whose number is already registered for
//! the same party and bank (a lease's cheques entered twice).

use renox::prelude::*;
use wl_kernel::cheques::{
    ChequeDocument, ChequeFuture, ChequeId, ChequeSchedule, ChequeStatus, ChequeSummary, Cheques,
    OpenCheque, RegisterCheque, ReturnCheques, ReturnedCheques,
};
use wl_kernel::invoicing::invoicer;
use wl_kernel::payments::{AllocationTarget, Direction};
use wl_kernel::{LegalEntity, PartyId, Subject};

use crate::cheque::{Cheque, ChequeLink};

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// `request` with the currency in capitals and the bank and number trimmed.
fn normalised(request: &RegisterCheque) -> RegisterCheque {
    RegisterCheque {
        bank: request.bank.trim().to_owned(),
        number: request.number.trim().to_owned(),
        currency: request.currency.trim().to_ascii_uppercase(),
        ..request.clone()
    }
}

/// Why `request` cannot be registered, if it cannot, before the database is
/// asked.
pub(crate) fn request_problem(request: &RegisterCheque) -> Option<String> {
    if request.amount <= 0 {
        return Some("The cheque amount must be above 0.".to_owned());
    }
    if request.bank.trim().is_empty() {
        return Some("The cheque's bank is required.".to_owned());
    }
    if request.number.trim().is_empty() {
        return Some("The cheque number is required.".to_owned());
    }
    let currency = request.currency.trim();
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Some(format!(
            "`{}` is not a currency code like AED.",
            request.currency
        ));
    }
    let mut seen: Vec<&Subject> = Vec::new();
    for document in &request.linked {
        if seen.contains(&document) {
            return Some(format!("{document} is linked twice."));
        }
        seen.push(document);
    }
    None
}

/// Checks `request` against the database: the entity is active, the party is
/// found and the number is not registered yet.
async fn check(state: &AppState, request: &RegisterCheque) -> Result<()> {
    if let Some(problem) = request_problem(request) {
        return Err(refused(problem));
    }
    let entity = LegalEntity::find(&state.db, request.legal_entity_id).await?;
    if !entity.as_ref().is_some_and(LegalEntity::is_active) {
        return Err(refused("The legal entity is not active."));
    }
    let party = wl_kernel::parties::directory(state)
        .find(&state.db, request.party)
        .await?;
    if party.is_none() {
        return Err(refused("The party was not found."));
    }
    let same = Cheque::query()
        .where_(Cheque::PARTY_ID.eq(request.party.0))
        .where_(Cheque::BANK.eq(request.bank.trim()))
        .where_(Cheque::NUMBER.eq(request.number.trim()))
        .where_(Cheque::STATUS.is_in([
            ChequeStatus::Received,
            ChequeStatus::DueForDeposit,
            ChequeStatus::Deposited,
            ChequeStatus::Bounced,
            ChequeStatus::LegalCase,
        ]))
        .count(&state.db)
        .await?;
    if same > 0 {
        return Err(refused(format!(
            "Cheque {} of {} is already registered for this party.",
            request.number.trim(),
            request.bank.trim()
        )));
    }
    // The cheque of an approved owner drawing: the owner party, the approved
    // amount (`FIN.ACL.21`).
    crate::drawings::check_cheque(state, request).await?;
    Ok(())
}

/// The unsaved `received` cheque for `request`, registered by `by`.
fn new_cheque(request: &RegisterCheque, by: Option<i64>) -> Cheque {
    Cheque {
        direction: request.direction,
        legal_entity_id: request.legal_entity_id,
        party_id: request.party.0,
        bank_account: request
            .bank_account
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned),
        bank: request.bank.clone(),
        number: request.number.clone(),
        cheque_date: request.date,
        amount: request.amount,
        currency: request.currency.clone(),
        agreement_type: request.agreement.as_ref().map(|a| a.subject_type.clone()),
        agreement_id: request.agreement.as_ref().map(|a| a.subject_id),
        collected_by: request.collected_by,
        field_collected: request.field_collected && request.direction == Direction::In,
        open_amount: request.amount,
        recorded_by: by,
        drawing_request_id: request.drawing,
        status: ChequeStatus::Received,
        ..Default::default()
    }
}

/// Inserts the cheque for `request` and its links in `tx`.
async fn insert_in(
    tx: &mut renox::db::Transaction,
    request: &RegisterCheque,
    by: Option<i64>,
) -> Result<Cheque> {
    let mut cheque = new_cheque(request, by);
    cheque.insert(&mut *tx).await?;
    for (position, document) in request.linked.iter().enumerate() {
        let mut link = ChequeLink {
            cheque_id: cheque.id,
            position: i32::try_from(position).unwrap_or(i32::MAX),
            subject_type: document.subject_type.clone(),
            subject_id: document.subject_id,
            ..Default::default()
        };
        link.insert(&mut *tx).await?;
    }
    Ok(cheque)
}

/// Registers `request` as a `received` cheque, by user `by` (`None` = the
/// system).
///
/// # Errors
///
/// 422 as described in the module's documentation; nothing is written then.
pub async fn register(
    state: &AppState,
    by: Option<i64>,
    request: &RegisterCheque,
) -> Result<Cheque> {
    let request = normalised(request);
    check(state, &request).await?;
    let mut tx = state.db.begin().await?;
    let cheque = insert_in(&mut tx, &request, by).await?;
    if let Some(id) = request.drawing {
        // The drawing knows its cheque, so a second one is refused.
        if let Some(mut row) = crate::drawings::PaymentDrawing::find(&mut tx, id).await? {
            row.cheque_id = Some(cheque.id);
            row.save(&mut tx).await?;
        }
    }
    tx.commit().await?;
    // A holder who collects cheques in the field hands them over in a
    // handover of the day, counted by someone else (`FIN.ACL.20`).
    if let (true, Some(user_id)) = (cheque.field_collected, cheque.collected_by) {
        crate::cash_cheques::ensure_handover(state, user_id, Some(cheque.legal_entity_id)).await?;
    }
    Ok(cheque)
}

/// Registers every cheque of `schedule` (received, with the schedule's
/// agreement) in one transaction, or none.
///
/// # Errors
///
/// 422 when one cheque is refused, a number repeats in the schedule, or the
/// schedule has no cheque; nothing is written then.
pub async fn register_schedule(
    state: &AppState,
    by: Option<i64>,
    schedule: &ChequeSchedule,
) -> Result<Vec<Cheque>> {
    if schedule.cheques.is_empty() {
        return Err(refused("The schedule has no cheque."));
    }
    let mut requests: Vec<RegisterCheque> = Vec::new();
    for item in &schedule.cheques {
        let mut request = RegisterCheque::new(
            schedule.party,
            schedule.legal_entity_id,
            &schedule.bank,
            &item.number,
            item.date,
            item.amount,
        )
        .currency(&schedule.currency)
        .agreement(schedule.agreement.clone());
        request.linked = item.linked.clone();
        let request = normalised(&request);
        if requests.iter().any(|r| r.number == request.number) {
            return Err(refused(format!(
                "Cheque {} is in the schedule twice.",
                request.number
            )));
        }
        check(state, &request).await?;
        requests.push(request);
    }
    let mut tx = state.db.begin().await?;
    let mut cheques = Vec::new();
    for request in &requests {
        cheques.push(insert_in(&mut tx, request, by).await?);
    }
    tx.commit().await?;
    Ok(cheques)
}

/// The summary of `cheque` with its links.
async fn summary_of(db: &Db, cheque: &Cheque) -> Result<ChequeSummary> {
    Ok(cheque.summary(cheque.linked(db).await?))
}

/// The part of `open` fils each linked document takes, in link order. An
/// invoice takes up to its balance (`Some`); a document whose balance this
/// module cannot read (`None`: a supplier bill, a charge) takes what is left,
/// and the ones after it nothing.
pub(crate) fn apply_to_documents(
    open: i64,
    documents: Vec<(Subject, Option<i64>)>,
) -> Vec<ChequeDocument> {
    let mut left = open.max(0);
    let mut applied = Vec::new();
    for (document, balance) in documents {
        let amount = match balance {
            Some(balance) => left.min(balance.max(0)),
            None => left,
        };
        left -= amount;
        applied.push(ChequeDocument { document, amount });
    }
    applied
}

/// The cheques of `legal_entity_id` in `direction` that are not cleared yet,
/// oldest cheque date first, each with the documents it pays.
pub(crate) async fn not_cleared(
    state: &AppState,
    legal_entity_id: i64,
    direction: Direction,
) -> Result<Vec<OpenCheque>> {
    let cheques = Cheque::query()
        .where_(Cheque::LEGAL_ENTITY_ID.eq(legal_entity_id))
        .where_(Cheque::DIRECTION.eq(direction))
        .where_(Cheque::STATUS.is_in([
            ChequeStatus::Received,
            ChequeStatus::DueForDeposit,
            ChequeStatus::Deposited,
        ]))
        .order_by(Cheque::CHEQUE_DATE)
        .order_by(Cheque::ID)
        .get(&state.db)
        .await?;
    open_with_documents(state, cheques).await
}

/// The received cheques of `legal_entity_id` that bounced and are still owed,
/// oldest cheque date first, each with the documents it pays.
pub(crate) async fn bounced(state: &AppState, legal_entity_id: i64) -> Result<Vec<OpenCheque>> {
    let cheques = Cheque::query()
        .where_(Cheque::LEGAL_ENTITY_ID.eq(legal_entity_id))
        .where_(Cheque::DIRECTION.eq(Direction::In))
        .where_(Cheque::STATUS.is_in([ChequeStatus::Bounced, ChequeStatus::LegalCase]))
        .order_by(Cheque::CHEQUE_DATE)
        .order_by(Cheque::ID)
        .get(&state.db)
        .await?;
    open_with_documents(state, cheques).await
}

/// `cheques` with the documents each pays and the part applied to it.
async fn open_with_documents(state: &AppState, cheques: Vec<Cheque>) -> Result<Vec<OpenCheque>> {
    let mut open = Vec::new();
    for cheque in cheques {
        let linked = cheque.linked(&state.db).await?;
        let mut balances = Vec::new();
        for document in &linked {
            let balance = match AllocationTarget::from_subject(document) {
                Some(AllocationTarget::Invoice(id)) => Some(
                    invoicer(state)
                        .find(&state.db, id)
                        .await?
                        .map_or(0, |invoice| invoice.balance),
                ),
                _ => None,
            };
            balances.push((document.clone(), balance));
        }
        let documents = apply_to_documents(cheque.open_amount, balances);
        open.push(OpenCheque {
            cheque: cheque.summary(linked),
            documents,
        });
    }
    Ok(open)
}

/// Moves the link of `cheque` from `from` to `to`, keeping its place in the
/// order the documents are paid in (`Cheques::relink`).
///
/// # Errors
///
/// 422 for a cheque that is not `received` or `due_for_deposit`, one not
/// linked to `from`, or one linked to `to` already; 404 for an unknown cheque.
async fn relink(state: &AppState, cheque: ChequeId, from: &Subject, to: &Subject) -> Result {
    let found = Cheque::find(&state.db, cheque.0)
        .await?
        .ok_or(Error::NotFound)?;
    if !matches!(
        found.status,
        ChequeStatus::Received | ChequeStatus::DueForDeposit
    ) {
        return Err(refused(format!(
            "Cheque {} is {} and cannot be relinked.",
            found.number,
            found.status.as_str()
        )));
    }
    let links = found.links(&state.db).await?;
    if links.iter().any(|link| &link.subject() == to) {
        return Err(refused(format!(
            "Cheque {} is linked to {to} already.",
            found.number
        )));
    }
    let Some(link) = links.iter().find(|link| &link.subject() == from) else {
        return Err(refused(format!(
            "Cheque {} is not linked to {from}.",
            found.number
        )));
    };
    ChequeLink::query()
        .where_(ChequeLink::ID.eq(link.id))
        .update(
            &state.db,
            &[
                ("subject_type", &to.subject_type),
                ("subject_id", &to.subject_id),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// Marks the `received` and `due_for_deposit` cheques of `ids` as held as
/// evidence (`Cheques::hold_as_evidence`); returns the ones marked, in the
/// order given. Any other cheque is left as it is.
///
/// # Errors
///
/// 422 for a blank note; the database's errors.
async fn hold_as_evidence(state: &AppState, ids: &[ChequeId], note: &str) -> Result<Vec<ChequeId>> {
    let note = note.trim();
    if note.is_empty() {
        return Err(refused(
            "The reason a cheque is held as evidence is required.",
        ));
    }
    let mut marked = Vec::new();
    for id in ids {
        let Some(found) = Cheque::find(&state.db, id.0).await? else {
            continue;
        };
        if !matches!(
            found.status,
            ChequeStatus::Received | ChequeStatus::DueForDeposit
        ) {
            continue;
        }
        Cheque::query()
            .where_(Cheque::ID.eq(found.id))
            .update(
                &state.db,
                &[
                    ("evidence_note", &Some(note.to_owned())),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        marked.push(*id);
    }
    Ok(marked)
}

/// Registers and reads `wl_payments_cheques` for the kernel contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChequeBook;

impl Cheques for ChequeBook {
    fn register<'a>(
        &'a self,
        state: &'a AppState,
        cheque: RegisterCheque,
        by: Option<i64>,
    ) -> ChequeFuture<'a, ChequeSummary> {
        Box::pin(async move {
            let cheque = register(state, by, &cheque).await?;
            summary_of(&state.db, &cheque).await
        })
    }

    fn register_schedule<'a>(
        &'a self,
        state: &'a AppState,
        schedule: ChequeSchedule,
        by: Option<i64>,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>> {
        Box::pin(async move {
            let cheques = register_schedule(state, by, &schedule).await?;
            let mut summaries = Vec::new();
            for cheque in &cheques {
                summaries.push(summary_of(&state.db, cheque).await?);
            }
            Ok(summaries)
        })
    }

    fn find<'a>(&'a self, db: &'a Db, id: ChequeId) -> ChequeFuture<'a, Option<ChequeSummary>> {
        Box::pin(async move {
            match Cheque::find(db, id.0).await? {
                Some(cheque) => Ok(Some(summary_of(db, &cheque).await?)),
                None => Ok(None),
            }
        })
    }

    fn of_agreement<'a>(
        &'a self,
        db: &'a Db,
        agreement: &'a Subject,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>> {
        Box::pin(async move {
            let mut summaries = Vec::new();
            for cheque in Cheque::of_agreement(db, agreement).await? {
                summaries.push(summary_of(db, &cheque).await?);
            }
            Ok(summaries)
        })
    }

    fn open_bounced_amount<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        currency: &'a str,
    ) -> ChequeFuture<'a, i64> {
        Box::pin(Cheque::owed_amount(db, party, currency))
    }

    fn return_undeposited<'a>(
        &'a self,
        state: &'a AppState,
        request: ReturnCheques,
        by: Option<i64>,
    ) -> ChequeFuture<'a, Option<ReturnedCheques>> {
        Box::pin(async move { crate::cheque_return::return_undeposited(state, &request, by).await })
    }

    fn uncleared_amount<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        currency: &'a str,
    ) -> ChequeFuture<'a, i64> {
        Box::pin(Cheque::uncleared_amount(db, party, currency))
    }

    fn not_cleared<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
        direction: Direction,
    ) -> ChequeFuture<'a, Vec<OpenCheque>> {
        Box::pin(not_cleared(state, legal_entity_id, direction))
    }

    fn hold_as_evidence<'a>(
        &'a self,
        state: &'a AppState,
        cheques: Vec<ChequeId>,
        note: String,
    ) -> ChequeFuture<'a, Vec<ChequeId>> {
        Box::pin(async move { hold_as_evidence(state, &cheques, &note).await })
    }

    fn relink<'a>(
        &'a self,
        state: &'a AppState,
        cheque: ChequeId,
        from: Subject,
        to: Subject,
    ) -> ChequeFuture<'a, ()> {
        Box::pin(async move { relink(state, cheque, &from, &to).await })
    }

    fn bounced<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
    ) -> ChequeFuture<'a, Vec<OpenCheque>> {
        Box::pin(bounced(state, legal_entity_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;

    fn request() -> RegisterCheque {
        let date = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        RegisterCheque::new(PartyId(7), 2, "ENBD", "1042", date, 2_000_000)
    }

    #[test]
    fn a_good_request_has_no_problem() {
        assert_eq!(request_problem(&request()), None);
    }

    #[test]
    fn the_amount_bank_number_and_currency_are_checked() {
        let mut r = request();
        r.amount = 0;
        assert!(request_problem(&r).is_some());
        let mut r = request();
        r.bank = "  ".into();
        assert!(request_problem(&r).is_some());
        let mut r = request();
        r.number = String::new();
        assert!(request_problem(&r).is_some());
        let mut r = request();
        r.currency = "DIRHAM".into();
        assert!(request_problem(&r).is_some());
        let mut r = request();
        r.currency = " aed ".into();
        assert_eq!(request_problem(&r), None);
        assert_eq!(normalised(&r).currency, "AED");
    }

    #[test]
    fn a_document_is_linked_once() {
        let doc = Subject::new("invoicing.invoice", 3);
        let r = request().link(doc.clone()).link(doc);
        assert!(request_problem(&r).is_some());
    }

    #[test]
    fn a_new_cheque_starts_received_with_the_whole_amount_open() {
        let r = normalised(&request().agreement(Subject::new("property.lease", 9)));
        let cheque = new_cheque(&r, Some(4));
        assert_eq!(cheque.status, ChequeStatus::Received);
        assert_eq!(cheque.open_amount, 2_000_000);
        assert_eq!(cheque.honoured_amount, 0);
        assert_eq!(cheque.presentations, 0);
        assert_eq!(cheque.recorded_by, Some(4));
        assert_eq!(
            cheque.agreement().map(|a| a.to_string()).as_deref(),
            Some("property.lease:9")
        );
    }

    #[test]
    fn only_a_received_cheque_can_be_a_field_cheque() {
        let received = new_cheque(&normalised(&request().collected_by(9, true)), None);
        assert!(received.field_collected);
        let issued = new_cheque(&normalised(&request().issued().collected_by(9, true)), None);
        assert!(!issued.field_collected);
    }

    #[test]
    fn an_invoice_takes_up_to_its_balance_and_the_rest_goes_on() {
        let one = Subject::new("invoicing.invoice", 1);
        let two = Subject::new("invoicing.invoice", 2);
        let applied = apply_to_documents(
            1_000,
            vec![(one.clone(), Some(400)), (two.clone(), Some(900))],
        );
        assert_eq!(
            applied[0],
            ChequeDocument {
                document: one,
                amount: 400
            }
        );
        assert_eq!(
            applied[1],
            ChequeDocument {
                document: two,
                amount: 600
            }
        );
    }

    #[test]
    fn a_document_of_unknown_balance_takes_what_is_left_once() {
        let bill = Subject::new("invoicing.supplier_bill", 3);
        let charge = Subject::new("invoicing.charge", 4);
        let applied = apply_to_documents(1_000, vec![(bill.clone(), None), (charge.clone(), None)]);
        assert_eq!(applied[0].amount, 1_000);
        assert_eq!(applied[1].amount, 0);
    }

    #[test]
    fn a_paid_invoice_takes_nothing() {
        let invoice = Subject::new("invoicing.invoice", 1);
        let applied = apply_to_documents(500, vec![(invoice, Some(0))]);
        assert_eq!(applied[0].amount, 0);
    }

    #[test]
    fn the_book_is_a_cheques_contract() {
        fn takes(_: impl Cheques) {}
        takes(ChequeBook);
    }
}
