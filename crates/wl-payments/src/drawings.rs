//! Owner drawings (wl-payments §3 *Payment* purpose, §4 `record`, §5.3 step 5,
//! `FIN.ACL.21`, `FIN.ACL.23`; wl-invoicing §5.16).
//!
//! When the owner takes money out of the business the accountant saves an
//! outgoing payment to the drawings category's owner party with purpose
//! *owner drawings*. [`request`] checks it (the category is a drawings
//! category with an owner party, the payee is that party, the amount is in
//! AED, the method details are complete) and keeps its details in a
//! [`PaymentDrawing`]; **no payment exists yet**. It asks wl-invoicing, through
//! the kernel's [`OwnerDrawings`](wl_kernel::drawings::OwnerDrawings), for the
//! approval: a request to a holder of `drawings.approve` other than the
//! requester (`PLT.APR.01`), or, for the entity's only holder, an approval at
//! once, flagged *self-approved* (`FIN.ACL.23`).
//!
//! Only the final approval records the payment ([`record_approved`], run on
//! `OwnerDrawingDecided` or at once for a sole owner): with the details
//! entered, purpose *owner drawings*, the drawings category, the payment dated
//! as entered, allocated to no document and never to a cost object. The
//! `record` transition has a guard `drawing_approved` that only this module
//! satisfies, so no other path records an outgoing drawing. A rejection, a
//! withdrawal or an expiry records nothing (the request becomes `rejected`
//! with the reason). A drawing to be paid by cheque waits, `approved`, for its
//! issued cheque ([`check_cheque`]): the cheque must be registered to the
//! owner party for the approved amount, and its `clear` records the payment
//! with the purpose ([`for_cheque`]).
//!
//! A **repayment** by the owner is an incoming payment with the same purpose
//! ([`repay`]): allowed only from the category's owner party, it needs no
//! approval and reduces the drawings of its month.
//!
//! Budget vs actual reads the payments through [`PaymentDrawings`], a kernel
//! [`DrawingPaymentSource`]; reversing a drawing (`reverse`, reason *entered
//! in error*) needs no approval and tells the holders of `drawings.approve`
//! ([`register_types`]). A drawing marked on a bank statement line (§5.7
//! step 3, [`crate::bank_statement_flow::mark_drawing`]) is requested with
//! [`request_for_line`]; the line stays `suggested` until the approval
//! records the payment and matches it, or the rejection, withdrawal or expiry
//! puts it back to `unmatched`.

use renox::auth::permissions::{Scope, users_with_permission_in};
use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::{Deserialize, Serialize};
use wl_kernel::drawings::{
    DRAWING_REQUEST_SUBJECT_TYPE, DrawingCategory, DrawingFuture, DrawingPayment,
    DrawingPaymentSource, DrawingRequest as AskForDrawing, DrawingStart, OwnerDrawingDecided,
    PURPOSE, owner_drawings,
};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::payments::{
    Direction, PaymentMethod, PaymentReversed, PaymentStatus, RecordPayment, ReversalReason,
};
use wl_kernel::{LegalEntity, PartyId, Subject};

use crate::cheque::Cheque;
use crate::followup::money_text;
use crate::payment::{Payment, details_json};
use crate::record::{self, RecordInput};
use crate::texts::english;

/// The permission that approves a drawing (wl-invoicing).
const DRAWINGS_APPROVE: &str = "drawings.approve";

/// The notification type of a reversed drawing.
pub const DRAWING_REVERSED: &str = "payments.drawing_reversed";

/// Where a drawing request stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DrawingStatus {
    /// Waiting for a holder of `drawings.approve`.
    #[default]
    Requested,
    /// Approved; the payment waits for its cheque (or could not be recorded:
    /// see the reason).
    Approved,
    /// The payment is recorded.
    Recorded,
    /// Refused, withdrawn or expired: nothing was recorded.
    Rejected,
}

/// An owner drawing waiting to be paid, with the details the accountant
/// entered. Its status mirrors the approval wl-invoicing keeps for it
/// (`invoicing.drawing_approval`), which owns the lifecycle; this module moves
/// it only when the kernel announces the decision.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_payments_drawing_requests", index(status))]
pub struct PaymentDrawing {
    /// The primary key.
    pub id: i64,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// The code of the drawings category.
    pub category: String,
    /// The category's owner party, the payee.
    pub party_id: i64,
    /// How the money moves.
    pub method: PaymentMethod,
    /// The method's details: transfer reference; cheque number, bank and date.
    pub method_details: Json<Value>,
    /// The amount in AED fils.
    pub amount: i64,
    /// The currency; always `AED`.
    pub currency: String,
    /// The day the money moves.
    pub payment_date: NaiveDate,
    /// The bank's reference.
    pub reference: Option<String>,
    /// The bank account the money moves through.
    pub bank_account: Option<String>,
    /// The user who saved it.
    pub requested_by: Option<i64>,
    /// The user who gave the last yes.
    pub approved_by: Option<i64>,
    /// Whether the sole owner approved it alone (`FIN.ACL.23`).
    pub self_approved: bool,
    /// Why it was not approved, or why it could not be recorded.
    pub reason: Option<String>,
    /// The payment recorded for it.
    pub payment_id: Option<i64>,
    /// The cheque registered for it.
    pub cheque_id: Option<i64>,
    /// The bank statement line it was marked on, if any (§5.7 step 3).
    pub statement_line_id: Option<i64>,
    /// Where it stands.
    pub status: DrawingStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PaymentDrawing {
    /// The request as a [`Subject`]: `payments.drawing_request:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, self.id)
    }

    /// The payee.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The outgoing payment this request records.
    pub fn as_payment(&self) -> RecordPayment {
        let mut request =
            RecordPayment::new(self.party(), self.legal_entity_id, self.method, self.amount)
                .outgoing()
                .currency(&self.currency)
                .date(self.payment_date)
                .method_details(self.method_details.0.clone());
        if let Some(reference) = &self.reference {
            request = request.reference(reference);
        }
        if let Some(account) = &self.bank_account {
            request = request.bank_account(account);
        }
        if let Some(user) = self.requested_by {
            request = request.collected_by(user);
        }
        request
    }
}

/// What the accountant enters for a drawing or a repayment.
#[derive(Debug, Clone)]
pub struct DrawingInput {
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The code of the drawings category.
    pub category: String,
    /// The payee of a drawing, the payer of a repayment: must be the
    /// category's owner party.
    pub party: PartyId,
    /// How the money moves.
    pub method: PaymentMethod,
    /// The method's details.
    pub method_details: Value,
    /// The amount in AED fils.
    pub amount: i64,
    /// The currency; only `AED` is accepted.
    pub currency: String,
    /// The day the money moves; today when `None`.
    pub date: Option<NaiveDate>,
    /// The bank's reference.
    pub reference: Option<String>,
    /// The bank account.
    pub bank_account: Option<String>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Why `input` cannot be a drawing or a repayment of `category`, if it cannot:
/// the category must have an owner party, the payee must be that party, the
/// amount above 0 and the currency AED.
pub fn input_problem(category: &DrawingCategory, input: &DrawingInput) -> Option<&'static str> {
    match category.owner_party {
        None => return Some("The drawings category has no owner party yet: set it first."),
        Some(owner) if owner != input.party => {
            return Some("An owner drawing is paid to the category's owner party.");
        }
        Some(_) => {}
    }
    if input.amount <= 0 {
        return Some("The amount must be above 0.");
    }
    if !input.currency.trim().eq_ignore_ascii_case("AED") {
        return Some("Owner drawings are recorded in AED.");
    }
    None
}

/// The drawings category `code`, or a 422.
async fn category_of(state: &AppState, code: &str) -> Result<DrawingCategory> {
    owner_drawings(state)
        .category(&state.db, code)
        .await?
        .ok_or_else(|| refused("Choose an owner drawings category."))
}

/// The payment `input` makes, outgoing or incoming.
fn payment_of(input: &DrawingInput, direction: Direction, by: Option<i64>) -> RecordPayment {
    let mut request = RecordPayment::new(
        input.party,
        input.legal_entity_id,
        input.method,
        input.amount,
    )
    .currency("AED")
    .method_details(input.method_details.clone());
    if direction == Direction::Out {
        request = request.outgoing();
    }
    if let Some(date) = input.date {
        request = request.date(date);
    }
    if let Some(reference) = &input.reference {
        request = request.reference(reference);
    }
    if let Some(account) = &input.bank_account {
        request = request.bank_account(account);
    }
    if let Some(user) = by {
        request = request.collected_by(user);
    }
    request
}

/// Marks `payment` as an owner drawings payment of `category`.
fn mark(payment: &mut Payment, category: &str, request: Option<i64>, self_approved: bool) {
    payment.purpose = PURPOSE.to_owned();
    payment.drawing_category = Some(category.to_owned());
    payment.drawing_request_id = request;
    payment.self_approved = self_approved;
}

/// Saves an outgoing owner drawing by `by`: keeps its details, asks for the
/// approval and, when it is given at once (the sole owner, `FIN.ACL.23`),
/// records the payment. Otherwise **nothing is recorded** until the final
/// approval.
///
/// # Errors
///
/// 422 for an unknown or ownerless category, a payee that is not the owner
/// party, an amount of 0 or less, a currency other than AED, a method that is
/// not enabled, an inactive entity or an unknown party, incomplete method
/// details (a cheque's are given when it is registered), or when the approval
/// cannot be asked for.
pub async fn request(
    state: &AppState,
    by: &AuthUser,
    input: &DrawingInput,
) -> Result<PaymentDrawing> {
    request_with(state, by, input, None).await
}

/// Like [`request`] for an outgoing bank statement line marked as an owner
/// drawing (§5.7 step 3): the request remembers the line, which is matched to
/// the payment when the drawing is recorded and put back to `unmatched` with
/// the reason when it is not approved.
///
/// # Errors
///
/// As [`request`].
pub(crate) async fn request_for_line(
    state: &AppState,
    by: &AuthUser,
    input: &DrawingInput,
    line_id: i64,
) -> Result<PaymentDrawing> {
    request_with(state, by, input, Some(line_id)).await
}

async fn request_with(
    state: &AppState,
    by: &AuthUser,
    input: &DrawingInput,
    statement_line_id: Option<i64>,
) -> Result<PaymentDrawing> {
    let category = category_of(state, &input.category).await?;
    if let Some(problem) = input_problem(&category, input) {
        return Err(refused(problem));
    }
    let pay = payment_of(input, Direction::Out, Some(by.id));
    let (payment, gathered) = record::prepare(state, &pay, Some(by.id))
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    if !gathered.method_enabled {
        return Err(refused(format!(
            "The payment method {} is not enabled.",
            input.method.as_str()
        )));
    }
    if !gathered.entity_active {
        return Err(refused("The legal entity is not active."));
    }
    if !gathered.party_found {
        return Err(refused("The party was not found."));
    }
    if input.method != PaymentMethod::Cheque
        && let Some(problem) = record::details_problem(&payment)
    {
        return Err(refused(problem));
    }
    let mut row = PaymentDrawing {
        legal_entity_id: input.legal_entity_id,
        category: category.code.clone(),
        party_id: input.party.0,
        method: input.method,
        method_details: details_json(input.method_details.clone()),
        amount: input.amount,
        currency: "AED".to_owned(),
        payment_date: payment.payment_date,
        reference: payment.reference.clone(),
        bank_account: payment.bank_account.clone(),
        requested_by: Some(by.id),
        statement_line_id,
        status: DrawingStatus::Requested,
        ..Default::default()
    };
    row.insert(&state.db).await?;
    let started = owner_drawings(state)
        .request(
            state,
            AskForDrawing {
                legal_entity_id: row.legal_entity_id,
                category: category.code.clone(),
                source: row.subject(),
                amount: row.amount,
                requested_by: by.id,
            },
        )
        .await;
    match started {
        Ok(DrawingStart::Pending) => Ok(row),
        Ok(DrawingStart::Approved { self_approved }) => {
            approved(state, row, Some(by.id), self_approved).await
        }
        Err(error) => {
            // No approval was asked for: the saved details are given up.
            row.status = DrawingStatus::Rejected;
            row.reason = Some(error.to_string());
            row.save(&state.db).await?;
            Err(error)
        }
    }
}

/// The approval is given: record the payment, or wait for the cheque of a
/// drawing paid by cheque.
async fn approved(
    state: &AppState,
    mut row: PaymentDrawing,
    approver: Option<i64>,
    self_approved: bool,
) -> Result<PaymentDrawing> {
    row.approved_by = approver;
    row.self_approved = self_approved;
    row.status = DrawingStatus::Approved;
    row.save(&state.db).await?;
    if row.method == PaymentMethod::Cheque {
        return Ok(row);
    }
    match record_approved(state, &row).await {
        Ok(()) => PaymentDrawing::find_or_404(&state.db, row.id).await,
        Err(error) => {
            // The approval stands; the accountant sees why it is not paid.
            row.reason = Some(error.to_string());
            row.save(&state.db).await?;
            Ok(row)
        }
    }
}

/// Records the payment of the approved request `row`, in one transaction, by
/// the system with the requester's id in `recorded_by`; `PaymentReceived`
/// follows the commit. This is the only way an outgoing drawing is recorded.
///
/// # Errors
///
/// A guard of `record` (a method that is no longer enabled, an inactive
/// legal entity) or a database error; nothing is written then.
pub async fn record_approved(state: &AppState, row: &PaymentDrawing) -> Result<()> {
    // Money out to a blocked owner party waits under `PTY.ACL.03`: the
    // approval stands, the payment is not recorded until the party is free.
    if let Some(message) = money_out_refusal(state, row.party()).await? {
        return Err(refused(message));
    }
    let pay = row.as_payment();
    let (mut payment, mut input): (Payment, RecordInput) =
        record::prepare(state, &pay, row.requested_by)
            .await
            .map_err(renox::state_machine::TransitionError::into_error)?;
    mark(&mut payment, &row.category, Some(row.id), row.self_approved);
    input.drawing_approved = true;
    let mut tx = state.db.begin().await?;
    let (payment, outcome) = record::record_in(&mut tx, None, payment, &input)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    let mut done = PaymentDrawing::find_or_404(&mut tx, row.id).await?;
    done.status = DrawingStatus::Recorded;
    done.payment_id = Some(payment.id);
    done.reason = None;
    done.save(&mut tx).await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    // The statement line it was marked on is matched to the payment.
    crate::bank_statement_flow::drawing_recorded(state, &done).await?;
    Ok(())
}

/// Whether money may go out to `party`: the neutral refusal text when the
/// party is blocked (`PTY.ACL.03`). Only a question: nothing is written.
async fn money_out_refusal(state: &AppState, party: PartyId) -> Result<Option<String>> {
    use wl_kernel::block_guard::{ActionKind, GuardOutcome, guard};
    let mut tx = state.db.begin().await?;
    let outcome = guard(state)
        .check_in(&mut tx, None, party, ActionKind::MoneyOut)
        .await?;
    drop(tx);
    Ok(match outcome {
        GuardOutcome::Allowed => None,
        GuardOutcome::Blocked(refusal) => Some(refusal.message_en()),
    })
}

/// Records an incoming repayment by the owner, by `by`: purpose *owner
/// drawings*, from the category's owner party only, with no approval.
///
/// # Errors
///
/// 422 as for [`request`]; the guards of `record`.
pub async fn repay(state: &AppState, by: &AuthUser, input: &DrawingInput) -> Result<Payment> {
    let category = category_of(state, &input.category).await?;
    if let Some(problem) = input_problem(&category, input) {
        return Err(refused(problem));
    }
    let pay = payment_of(input, Direction::In, Some(by.id));
    let (mut payment, gathered) = record::prepare(state, &pay, Some(by.id))
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    mark(&mut payment, &category.code, None, false);
    let mut tx = state.db.begin().await?;
    let (payment, outcome) = record::record_in(&mut tx, Some(by), payment, &gathered)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    tx.commit().await?;
    outcome.emit(state).await?;
    Payment::find_or_404(&state.db, payment.id).await
}

/// The decision on a pending request is announced: record the payment on a
/// yes, keep the reason on a no. A request that is no longer `requested` (a
/// repeated delivery) is left as it is.
///
/// # Errors
///
/// A database error.
pub async fn decided(state: &AppState, event: &OwnerDrawingDecided) -> Result<()> {
    if event.source.subject_type != DRAWING_REQUEST_SUBJECT_TYPE {
        return Ok(());
    }
    let Some(mut row) = PaymentDrawing::find(&state.db, event.source.subject_id).await? else {
        return Ok(());
    };
    if row.status != DrawingStatus::Requested {
        return Ok(());
    }
    if event.approved {
        // `FIN.ACL.21`: the user who saved the drawing never approves it (the
        // sole owner's own approval is `DrawingStart::Approved`, not this).
        if event.approved_by.is_some() && event.approved_by == row.requested_by {
            return Err(refused("The requester cannot approve their own drawing."));
        }
        approved(state, row, event.approved_by, false).await?;
    } else {
        row.status = DrawingStatus::Rejected;
        row.reason = event.reason.clone();
        row.save(&state.db).await?;
        // The statement line it was marked on goes back to `unmatched`.
        crate::bank_statement_flow::drawing_rejected(state, &row).await?;
    }
    Ok(())
}

/// Why `request` cannot register a cheque for an approved drawing, if it
/// cannot: the drawing must be approved and paid by cheque, the cheque issued
/// to the category's owner party for the approved amount, with no document
/// linked. The cheque date is free.
///
/// # Errors
///
/// 422 with the reason; a database error.
pub async fn check_cheque(
    state: &AppState,
    request: &wl_kernel::cheques::RegisterCheque,
) -> Result<()> {
    let Some(id) = request.drawing else {
        return Ok(());
    };
    let Some(row) = PaymentDrawing::find(&state.db, id).await? else {
        return Err(refused("The owner drawing was not found."));
    };
    if request.direction != Direction::Out {
        return Err(refused(
            "The cheque of an owner drawing is an issued cheque.",
        ));
    }
    if row.status != DrawingStatus::Approved || row.method != PaymentMethod::Cheque {
        return Err(refused(
            "Only an approved owner drawing paid by cheque takes a cheque.",
        ));
    }
    if row.cheque_id.is_some() {
        return Err(refused("The owner drawing has its cheque already."));
    }
    if request.legal_entity_id != row.legal_entity_id {
        return Err(refused(
            "The cheque is in another legal entity than the drawing.",
        ));
    }
    if request.party != row.party() {
        return Err(refused(
            "The cheque goes to the category's owner party; another payee needs a new drawing request.",
        ));
    }
    if request.amount != row.amount || !request.currency.eq_ignore_ascii_case(&row.currency) {
        return Err(refused(
            "The cheque is for the approved amount; another amount needs a new drawing request.",
        ));
    }
    if !request.linked.is_empty() {
        return Err(refused("An owner drawing cheque pays no document."));
    }
    Ok(())
}

/// The drawing request the issued `cheque` pays, if any.
pub async fn for_cheque(db: &Db, cheque: &Cheque) -> Result<Option<PaymentDrawing>> {
    match cheque.drawing_request_id {
        Some(id) => PaymentDrawing::find(db, id).await,
        None => Ok(None),
    }
}

/// Marks `payment` as the drawings payment `row` stands for, when its cheque
/// clears.
pub(crate) fn mark_cheque_payment(payment: &mut Payment, row: &PaymentDrawing) {
    mark(payment, &row.category, Some(row.id), row.self_approved);
}

/// The drawings payments for budget vs actual (a kernel
/// [`DrawingPaymentSource`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct PaymentDrawings;

impl DrawingPaymentSource for PaymentDrawings {
    fn payments<'a>(
        &'a self,
        db: &'a Db,
        entities: &'a [i64],
        from: NaiveDate,
        to: NaiveDate,
    ) -> DrawingFuture<'a, Vec<DrawingPayment>> {
        Box::pin(async move {
            if entities.is_empty() {
                return Ok(Vec::new());
            }
            let rows = Payment::query()
                .where_(Payment::PURPOSE.eq(PURPOSE))
                .where_(Payment::LEGAL_ENTITY_ID.is_in(entities.to_vec()))
                .where_(Payment::STATUS.eq(PaymentStatus::Received))
                .where_(Payment::PAYMENT_DATE.gte(from))
                .where_(Payment::PAYMENT_DATE.lte(to))
                .order_by(Payment::PAYMENT_DATE)
                .order_by(Payment::ID)
                .get(db)
                .await?;
            Ok(rows
                .into_iter()
                .map(|p| DrawingPayment {
                    payment: p.payment_id(),
                    legal_entity_id: p.legal_entity_id,
                    category: p
                        .drawing_category
                        .clone()
                        .unwrap_or_else(|| crate::expense_category::OWNER_DRAWINGS.to_owned()),
                    direction: p.direction,
                    amount: p.amount,
                    date: p.payment_date,
                    party: p.party(),
                    self_approved: p.self_approved,
                })
                .collect())
        })
    }
}

/// The type of the notice that a recorded drawing was reversed.
pub fn reversed_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DRAWING_REVERSED, "payments")
        .group("drawings")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.drawings.reversed.subject")),
            english("wl_payments.drawings.reversed.body"),
        )
}

/// Registers the notification type.
pub(crate) fn register_types() {
    notify::register_type(reversed_type());
}

/// Tells the holders of `drawings.approve` that the drawings payment of the
/// event was reversed (`reverse` is the correction of a drawing recorded in
/// error and needs no approval).
async fn tell_reversed(state: &AppState, event: &PaymentReversed) -> Result<()> {
    let Some(payment) = Payment::find(&state.db, event.payment.0).await? else {
        return Ok(());
    };
    if payment.purpose != PURPOSE {
        return Ok(());
    }
    let holders = users_with_permission_in(
        &state.db,
        DRAWINGS_APPROVE,
        &Scope::of_id::<LegalEntity>(payment.legal_entity_id),
    )
    .await?;
    if holders.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(DRAWING_REVERSED, payment.subject()).data(json!({
        "amount": money_text(&payment.currency, payment.amount),
        "reason": reason_text(event.reason),
        "link": format!("/payments/{}", payment.id),
    }));
    for user in holders {
        request = request.to_user(user.id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The reason of a reversal in words.
fn reason_text(reason: ReversalReason) -> String {
    reason.as_str().replace('_', " ")
}

/// Registers the listeners: the decision on a drawing request and the
/// reversal of a drawings payment.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: OwnerDrawingDecided, state: AppState| async move {
        decided(&state, &event).await
    });
    app.listen(|event: PaymentReversed, state: AppState| async move {
        tell_reversed(&state, &event).await
    });
    wl_kernel::drawings::register_drawing_payments("wl-payments", PaymentDrawings);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category(owner: Option<i64>) -> DrawingCategory {
        DrawingCategory {
            code: "owner_drawings".into(),
            name_en: "Owner drawings".into(),
            owner_party: owner.map(PartyId),
        }
    }

    fn input(party: i64, amount: i64, currency: &str) -> DrawingInput {
        DrawingInput {
            legal_entity_id: 1,
            category: "owner_drawings".into(),
            party: PartyId(party),
            method: PaymentMethod::Transfer,
            method_details: json!({}),
            amount,
            currency: currency.into(),
            date: None,
            reference: Some("TRX-1".into()),
            bank_account: None,
        }
    }

    #[test]
    fn a_drawing_goes_to_the_owner_party_in_aed() {
        let owner = category(Some(7));
        assert_eq!(input_problem(&owner, &input(7, 1_500_000, "AED")), None);
        assert!(input_problem(&owner, &input(8, 1_500_000, "AED")).is_some());
        assert!(input_problem(&owner, &input(7, 0, "AED")).is_some());
        assert!(input_problem(&owner, &input(7, 100, "USD")).is_some());
        assert!(input_problem(&category(None), &input(7, 100, "AED")).is_some());
    }

    #[test]
    fn the_request_records_an_outgoing_payment_with_the_details_entered() {
        let row = PaymentDrawing {
            id: 3,
            legal_entity_id: 2,
            party_id: 7,
            method: PaymentMethod::Transfer,
            amount: 1_500_000,
            currency: "AED".into(),
            payment_date: NaiveDate::from_ymd_opt(2027, 3, 10).unwrap(),
            reference: Some("TRX-9".into()),
            bank_account: Some("BA1".into()),
            requested_by: Some(4),
            ..Default::default()
        };
        let pay = row.as_payment();
        assert_eq!(pay.direction, Direction::Out);
        assert_eq!(
            (pay.party, pay.legal_entity_id, pay.amount),
            (PartyId(7), 2, 1_500_000)
        );
        assert_eq!(
            pay.date,
            Some(NaiveDate::from_ymd_opt(2027, 3, 10).unwrap())
        );
        assert_eq!(pay.reference.as_deref(), Some("TRX-9"));
        assert_eq!(pay.bank_account.as_deref(), Some("BA1"));
        assert_eq!(row.subject(), Subject::new("payments.drawing_request", 3));
    }

    #[test]
    fn marking_sets_the_purpose_the_category_and_the_flag() {
        let mut payment = Payment::default();
        mark(&mut payment, "owner_drawings", Some(3), true);
        assert_eq!(payment.purpose, PURPOSE);
        assert_eq!(payment.drawing_category.as_deref(), Some("owner_drawings"));
        assert_eq!(payment.drawing_request_id, Some(3));
        assert!(payment.self_approved);
    }

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let stored: Vec<_> = DrawingStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(stored, ["requested", "approved", "recorded", "rejected"]);
    }

    #[test]
    fn the_notification_type_is_transactional_with_an_english_email() {
        let def = reversed_type();
        assert_eq!(def.key, DRAWING_REVERSED);
        assert_eq!(def.category, Category::Transactional);
        assert_eq!(def.channels, [NotifyChannel::Email]);
    }
}
