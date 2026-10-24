//! The supplier warranty claim services (08 §4.6 step 4, §7.1 *Supplier
//! warranty claim*, `AUT.WAR.02`): raising a claim from a warranty ticket that
//! was closed as accepted, recording the supplier's answer, recovering it
//! when the supplier credits or replaces the part, and writing it off.
//!
//! Every status change goes through the machine
//! ([`crate::supplier_claim_machine`]); `raise` is how a claim comes to be.
//!
//! Decisions:
//!
//! - The customer's warranty claim is a ticket of category `warranty`
//!   (wl-ticketing). The claim names the ticket and the sale or job card it is
//!   about; the kernel's `TaskCreator::tickets_of` finds it among the warranty
//!   tickets of that record and says whether it is `closed`. The ticket's
//!   custom field *warranty decision* is not readable through the kernel, so
//!   the purchaser confirms that the decision is *accepted* ([`NewSupplierClaim::decision_accepted`],
//!   see `DEFERRED.md`).
//! - The warranty terms (period, mileage, labour included) are stored per
//!   brand or supplier in the catalogue (08 §7.1); there is no catalogue yet,
//!   so the terms are given when the claim is raised and kept on it. Whether
//!   the warranty applies is [`crate::supplier_terms::check`]; a claim out of
//!   terms is refused with the term that failed.
//! - One claim per ticket.
//! - The supplier's answer (accepted or rejected) is recorded on the claim and
//!   changes no state: the claim ends when the supplier credits or replaces
//!   the part (`recover`) or the purchaser writes it off.
//! - A Return to supplier of wl-inventory (BL-STK-03, not built) is linked by
//!   its id ([`link_return`]); its credit (`SupplierReturnCredited`) recovers
//!   the claim by itself unless the credit was a manual *supplier refused*.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use wl_kernel::approvals::submit::REQUEST_KEY;
use wl_kernel::approvals::{ApprovalPolicy, announce};
use wl_kernel::documents::today;
use wl_kernel::inventory::SupplierReturnCredited;
use wl_kernel::parties::directory;
use wl_kernel::tickets::task_creator;
use wl_kernel::{PartyId, Subject};

use crate::db::refused;
use crate::estimate::Estimate;
use crate::job_card::JobCard;
use crate::labour_line::LineStatus;
use crate::permissions;
use crate::supplier_claim::{SupplierAnswer, SupplierClaim, SupplierClaimStatus};
use crate::supplier_claim_machine::{
    RECOVER, RECOVER_SYSTEM, SupplierInput, WRITE_OFF, WRITE_OFF_ACTION,
};
use crate::supplier_terms::{self, Facts, OutOfTerms, Terms};
use crate::texts::english;

/// The name of the history row of a claim that came to be.
pub const RAISE: &str = "raise";

/// The key of the ticket category of a customer's warranty claim.
pub const WARRANTY_CATEGORY: &str = "warranty";

/// The subject type of a Return to supplier (wl-inventory).
pub const SUPPLIER_RETURN_SUBJECT_TYPE: &str = "inventory.supplier_return";

/// A claim to raise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSupplierClaim {
    /// The warranty ticket.
    pub ticket_id: i64,
    /// The sale or job card the ticket is about.
    pub about: Subject,
    /// The ticket's *warranty decision* is accepted (the purchaser confirms
    /// it; the field is not readable through the kernel).
    pub decision_accepted: bool,
    /// The supplier (a party).
    pub supplier_party_id: i64,
    /// The failed part.
    pub part_description: String,
    /// Its item code.
    pub item_code: Option<String>,
    /// Its brand.
    pub brand: Option<String>,
    /// The quantity in thousandths.
    pub quantity_milli: i64,
    /// What the part cost, in fils.
    pub part_cost: i64,
    /// The day it was bought from the supplier.
    pub purchase_date: NaiveDate,
    /// The warranty period in months.
    pub warranty_months: i64,
    /// The warranty mileage in kilometres, when the terms set one.
    pub warranty_km: Option<i64>,
    /// Whether the terms include the labour of the replacement.
    pub labour_included: bool,
    /// The labour of the replacement, when it is not taken from a job card.
    pub labour_amount: Option<i64>,
    /// The job card the part was fitted on.
    pub fitted_job_card_id: Option<i64>,
    /// The job card of the replacement.
    pub replacement_job_card_id: Option<i64>,
    /// The mileage when the part was fitted, when not taken from the job card.
    pub fitted_odometer: Option<i64>,
    /// The day it failed.
    pub failure_date: NaiveDate,
    /// The mileage at the failure.
    pub failure_odometer: Option<i64>,
    /// Whether the failed part was kept.
    pub part_kept: bool,
}

impl NewSupplierClaim {
    /// A claim of `supplier_party_id` for `part_description` (cost `part_cost`
    /// fils, bought on `purchase_date` with `warranty_months` of warranty) that
    /// failed on `failure_date`, from warranty ticket `ticket_id` about
    /// `about`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ticket_id: i64,
        about: Subject,
        supplier_party_id: i64,
        part_description: &str,
        part_cost: i64,
        purchase_date: NaiveDate,
        warranty_months: i64,
        failure_date: NaiveDate,
    ) -> Self {
        NewSupplierClaim {
            ticket_id,
            about,
            decision_accepted: true,
            supplier_party_id,
            part_description: part_description.to_owned(),
            item_code: None,
            brand: None,
            quantity_milli: 1_000,
            part_cost,
            purchase_date,
            warranty_months,
            warranty_km: None,
            labour_included: false,
            labour_amount: None,
            fitted_job_card_id: None,
            replacement_job_card_id: None,
            fitted_odometer: None,
            failure_date,
            failure_odometer: None,
            part_kept: true,
        }
    }

    /// The warranty also sets a mileage.
    #[must_use]
    pub fn mileage(mut self, km: i64) -> Self {
        self.warranty_km = Some(km);
        self
    }

    /// The part was fitted on job card `id` at `odometer` kilometres and failed
    /// at `failure_odometer`.
    #[must_use]
    pub fn fitted_on(mut self, id: i64, odometer: i64, failure_odometer: i64) -> Self {
        self.fitted_job_card_id = Some(id);
        self.fitted_odometer = Some(odometer);
        self.failure_odometer = Some(failure_odometer);
        self
    }

    /// The terms include the labour of the replacement, `amount` fils.
    #[must_use]
    pub fn with_labour(mut self, amount: i64) -> Self {
        self.labour_included = true;
        self.labour_amount = Some(amount);
        self
    }

    /// The terms include the labour of the replacement on job card `id`.
    #[must_use]
    pub fn with_labour_of(mut self, id: i64) -> Self {
        self.labour_included = true;
        self.replacement_job_card_id = Some(id);
        self
    }

    /// The part was not kept (the customer discarded it).
    #[must_use]
    pub fn not_kept(mut self) -> Self {
        self.part_kept = false;
        self
    }

    /// The warranty decision of the ticket is not accepted.
    #[must_use]
    pub fn rejected_decision(mut self) -> Self {
        self.decision_accepted = false;
        self
    }

    /// With this item code and brand.
    #[must_use]
    pub fn item(mut self, code: &str, brand: &str) -> Self {
        self.item_code = Some(code.trim().to_uppercase()).filter(|c| !c.is_empty());
        self.brand = Some(brand.trim().to_owned()).filter(|b| !b.is_empty());
        self
    }
}

/// The text a failed term is refused with.
fn out_of_terms_key(reason: OutOfTerms) -> &'static str {
    match reason {
        OutOfTerms::Period => "auto.supplier.problem.period",
        OutOfTerms::Mileage => "auto.supplier.problem.mileage",
        OutOfTerms::MileageUnknown => "auto.supplier.problem.mileage_unknown",
    }
}

/// The approved labour of job card `id` before VAT, in fils.
async fn labour_of(db: &Db, id: i64) -> Result<i64> {
    let estimate = Estimate::load(db, id).await?;
    Ok(estimate
        .labour
        .iter()
        .filter(|line| line.approval == LineStatus::Approved)
        .map(crate::labour_line::LabourLine::price)
        .sum())
}

/// Raises a claim to the supplier (`AUT.WAR.02`).
///
/// # Errors
///
/// A 403 without `supplier_claims.manage`; a 422 for a supplier that is not a
/// party, a ticket that is not a warranty ticket of that sale or job card, one
/// that is not closed or whose decision is not accepted, one that has a claim
/// already, a blank part, an amount or period that is not above zero, a failure
/// before the purchase or in the future, or a failure out of the supplier's
/// terms (the term that failed is named).
pub async fn raise(
    state: &AppState,
    by: &AuthUser,
    new: NewSupplierClaim,
) -> Result<SupplierClaim> {
    if !by.has_permission(permissions::SUPPLIER_CLAIMS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let description = new.part_description.trim().to_owned();
    if description.is_empty() {
        return Err(refused(english("auto.supplier.problem.part", &[])));
    }
    if new.part_cost <= 0 || new.quantity_milli <= 0 || new.warranty_months <= 0 {
        return Err(refused(english("auto.supplier.problem.amounts", &[])));
    }
    if new.warranty_km.is_some_and(|km| km <= 0) {
        return Err(refused(english("auto.supplier.problem.amounts", &[])));
    }
    if new.failure_date < new.purchase_date || new.failure_date > today() {
        return Err(refused(english("auto.supplier.problem.dates", &[])));
    }
    let found = directory(state)
        .display_name(&state.db, PartyId(new.supplier_party_id), "en")
        .await?;
    if found.is_none() {
        return Err(refused(english("auto.supplier.problem.supplier", &[])));
    }
    // The warranty ticket: of that record, closed, decided as accepted.
    let tickets = task_creator(state)
        .tickets_of(state, &new.about, WARRANTY_CATEGORY)
        .await?;
    let ticket = tickets
        .into_iter()
        .find(|ticket| ticket.id.0 == new.ticket_id)
        .ok_or_else(|| refused(english("auto.supplier.problem.ticket", &[])))?;
    if ticket.status != "closed" {
        return Err(refused(english("auto.supplier.problem.ticket_open", &[])));
    }
    if !new.decision_accepted {
        return Err(refused(english("auto.supplier.problem.not_accepted", &[])));
    }
    if SupplierClaim::of_ticket(&state.db, ticket.id.0)
        .await?
        .is_some()
    {
        return Err(refused(english("auto.supplier.problem.once", &[])));
    }
    // Where the part was fitted: the mileage at the fitting.
    let mut fitted_odometer = new.fitted_odometer;
    if let Some(card_id) = new.fitted_job_card_id {
        let card = JobCard::find(&state.db, card_id)
            .await?
            .ok_or_else(|| refused(english("auto.supplier.problem.job_card", &[])))?;
        fitted_odometer = fitted_odometer.or(Some(card.odometer_out.unwrap_or(card.odometer_in)));
    }
    // The terms: the period, and the mileage of a part that was fitted.
    let terms = Terms {
        months: new.warranty_months,
        km: new.warranty_km,
    };
    let facts = Facts {
        purchase_date: new.purchase_date,
        failure_date: new.failure_date,
        fitted_odometer,
        failure_odometer: new.failure_odometer,
    };
    if let Err(reason) = supplier_terms::check(terms, facts) {
        return Err(refused(english(out_of_terms_key(reason), &[])));
    }
    // What is claimed: the part, and the labour when the terms include it.
    let labour = match (new.labour_included, new.replacement_job_card_id) {
        (false, _) => 0,
        (true, Some(card_id)) => labour_of(&state.db, card_id).await?,
        (true, None) => new.labour_amount.unwrap_or(0),
    };
    if labour < 0 {
        return Err(refused(english("auto.supplier.problem.amounts", &[])));
    }
    let mut claim = SupplierClaim {
        ticket_id: ticket.id.0,
        ticket_number: ticket.number.clone(),
        about_type: Some(new.about.subject_type.clone()),
        about_id: Some(new.about.subject_id),
        supplier_party_id: new.supplier_party_id,
        part_description: description,
        item_code: new.item_code.clone(),
        brand: new.brand.clone(),
        quantity_milli: new.quantity_milli,
        part_cost: new.part_cost,
        labour_included: new.labour_included,
        labour_amount: labour,
        claim_amount: supplier_terms::claim_amount(new.part_cost, new.labour_included, labour),
        purchase_date: new.purchase_date,
        warranty_months: new.warranty_months,
        warranty_km: new.warranty_km,
        fitted_job_card_id: new.fitted_job_card_id,
        replacement_job_card_id: new.replacement_job_card_id,
        fitted_odometer,
        failure_date: new.failure_date,
        failure_odometer: new.failure_odometer,
        part_kept: new.part_kept,
        supplier_answer: SupplierAnswer::Pending,
        raised_at: Some(now()),
        created_by: Some(by.id),
        ..SupplierClaim::default()
    };
    let mut tx = state.db.begin().await?;
    claim
        .insert_in_state(&mut tx, SupplierClaimStatus::Raised, RAISE)
        .await?;
    tx.commit().await?;
    // The claim document goes to the supplier and the kept part onto a Return
    // to supplier.
    crate::supplier_rules::after_raise(state, claim.id).await?;
    SupplierClaim::find_or_404(&state.db, claim.id).await
}

/// Records the supplier's answer to a claim that is raised: its reference, its
/// document, and whether it accepted or rejected it. The state does not
/// change; the claim ends when the supplier credits it or it is written off.
///
/// # Errors
///
/// A 403 without `supplier_claims.manage`; a 422 for a claim that is not
/// raised, or an answer of pending, or a rejection without a note.
pub async fn record_answer(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    answer: SupplierAnswer,
    reference: Option<&str>,
    note: Option<&str>,
) -> Result<SupplierClaim> {
    if !by.has_permission(permissions::SUPPLIER_CLAIMS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let mut claim = SupplierClaim::find_or_404(&state.db, id).await?;
    if !claim.is_open() || answer == SupplierAnswer::Pending {
        return Err(refused(english("auto.supplier.problem.answer", &[])));
    }
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    if answer == SupplierAnswer::Rejected && note.is_none() {
        return Err(refused(english(
            "auto.supplier.problem.rejection_note",
            &[],
        )));
    }
    claim.supplier_answer = answer;
    claim.supplier_answer_note = note.map(str::to_owned);
    claim.supplier_reference = reference
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
        .or(claim.supplier_reference);
    claim.answered_at = Some(now());
    claim.save(&state.db).await?;
    SupplierClaim::find_or_404(&state.db, id).await
}

/// Records the claim document that was sent to the supplier (the PDF or the
/// email) and the supplier's reference, when it has one.
///
/// # Errors
///
/// A 403 without `supplier_claims.manage`; a 422 for a blank document.
pub async fn record_document(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    document: &str,
    reference: Option<&str>,
) -> Result<SupplierClaim> {
    if !by.has_permission(permissions::SUPPLIER_CLAIMS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let document = document.trim();
    if document.is_empty() {
        return Err(refused(english("auto.supplier.problem.document", &[])));
    }
    let mut claim = SupplierClaim::find_or_404(&state.db, id).await?;
    claim.claim_document = Some(document.to_owned());
    if let Some(reference) = reference.map(str::trim).filter(|r| !r.is_empty()) {
        claim.supplier_reference = Some(reference.to_owned());
    }
    claim.save(&state.db).await?;
    SupplierClaim::find_or_404(&state.db, id).await
}

/// Links the Return to supplier (`inventory.supplier_return:N`) of the kept
/// part: its credit recovers the claim.
///
/// # Errors
///
/// A 403 without `supplier_claims.manage`; a 422 for a claim that is not
/// raised, whose part was not kept, or a subject that is not a return.
pub async fn link_return(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    supplier_return: &Subject,
) -> Result<SupplierClaim> {
    if !by.has_permission(permissions::SUPPLIER_CLAIMS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let mut claim = SupplierClaim::find_or_404(&state.db, id).await?;
    if !claim.is_open()
        || !claim.part_kept
        || supplier_return.subject_type != SUPPLIER_RETURN_SUBJECT_TYPE
    {
        return Err(refused(english("auto.supplier.problem.return", &[])));
    }
    claim.supplier_return_id = Some(supplier_return.subject_id);
    claim.save(&state.db).await?;
    SupplierClaim::find_or_404(&state.db, id).await
}

/// `recover`: the supplier credited the claim or sent a replacement.
/// `credit_reference` is the supplier's credit note, or the receipt of the
/// replacement when `replacement` is set.
///
/// # Errors
///
/// A 403 without `supplier_claims.manage`; a 422 without a credit reference or
/// a replacement, or for a claim that is not raised.
pub async fn recover(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    credit_reference: Option<&str>,
    replacement: bool,
) -> Result<SupplierClaim> {
    if !by.has_permission(permissions::SUPPLIER_CLAIMS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let claim = SupplierClaim::find_or_404(&state.db, id).await?;
    let input = SupplierInput {
        by: Some(by.id),
        basis: Some(
            if replacement {
                "replacement"
            } else {
                "credit_note"
            }
            .to_owned(),
        ),
        credit_reference: credit_reference
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned),
        replacement_received: Some(replacement),
        ..SupplierInput::default()
    };
    let _outcome = claim
        .transition(RECOVER)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    SupplierClaim::find_or_404(&state.db, id).await
}

/// What a write-off came to.
#[derive(Debug, Clone)]
pub struct WriteOff {
    /// The claim: `written_off`, or as it was while the branch manager decides.
    pub claim: SupplierClaim,
    /// The approval request that waits for the branch manager.
    pub approval_request: Option<i64>,
}

/// `write_off`: the supplier refused and the cost is absorbed as warranty
/// expense. It waits for the approval of a branch manager other than the
/// purchaser who asked (`PLT.APR.01`, action `auto.supplier_claim_write_off`):
/// the claim stays `raised` meanwhile, and a rejected request leaves it so.
///
/// # Errors
///
/// A 403 without `supplier_claims.manage`; a 422 without a reason or for a
/// claim that is not raised.
pub async fn write_off(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<WriteOff> {
    if !by.has_permission(permissions::SUPPLIER_CLAIMS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let claim = SupplierClaim::find_or_404(&state.db, id).await?;
    let mut input = SupplierInput {
        by: Some(by.id),
        reason: Some(reason.trim().to_owned()),
        needs_approval: Some(1),
        ..SupplierInput::default()
    };
    // A policy taken away must not let a write-off through unapproved.
    let value = renox::serde_json::to_value(&input).unwrap_or(renox::serde_json::Value::Null);
    input.approval_ready = ApprovalPolicy::for_action(&state.db, WRITE_OFF_ACTION, &value)
        .await?
        .is_some();
    let outcome = claim
        .transition(WRITE_OFF)
        .by(by)
        .reason(reason)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    announce(state, &outcome).await?;
    let approval_request = match &outcome {
        Outcome::Pending(pending) => pending
            .data
            .get(REQUEST_KEY)
            .and_then(renox::serde_json::Value::as_i64),
        Outcome::Done(_) => None,
    };
    Ok(WriteOff {
        claim: SupplierClaim::find_or_404(&state.db, id).await?,
        approval_request,
    })
}

/// `SupplierReturnCredited`: the Return to supplier of a raised claim was
/// credited by a debit note, a replacement or a refund received (the supplier's
/// credit note or the replacement of 08 §7.1; a refund is the credit paid in
/// cash), so the claim is recovered as the system. A manual credit with the
/// reason *supplier refused*, or on any other basis, leaves the claim raised.
///
/// # Errors
///
/// The machine's refusal.
pub async fn on_return_credited(state: &AppState, event: &SupplierReturnCredited) -> Result {
    if event.supplier_return.subject_type != SUPPLIER_RETURN_SUBJECT_TYPE {
        return Ok(());
    }
    let Some(claim) = SupplierClaim::of_return(&state.db, event.supplier_return.subject_id).await?
    else {
        return Ok(());
    };
    if event.basis == "supplier_refused" {
        // The supplier refused: the credit is not a recovery. The claim stays
        // raised and the purchaser is asked to write it off.
        return crate::supplier_rules::ask_to_write_off(state, &claim).await;
    }
    if !matches!(
        event.basis.as_str(),
        "debit_note" | "replacement" | "refund_received"
    ) {
        // A basis the spec does not name is no recovery: the claim stays raised.
        return Ok(());
    }
    let reference = event
        .debit_note
        .map(|note| format!("debit note {note}"))
        .or_else(|| Some(event.basis.clone()));
    let input = SupplierInput {
        basis: Some(
            match event.basis.as_str() {
                "replacement" => "replacement",
                "refund_received" => "refund_received",
                _ => "credit_note",
            }
            .to_owned(),
        ),
        credit_reference: reference,
        replacement_received: Some(event.basis == "replacement"),
        ..SupplierInput::default()
    };
    let _outcome = claim
        .transition(RECOVER_SYSTEM)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// The id of the warranty ticket of `about` whose number is `number`
/// (`TKT-2026-0001`, in any case).
///
/// # Errors
///
/// A 422 when the sale or job card has no warranty ticket of that number.
pub async fn ticket_by_number(state: &AppState, about: &Subject, number: &str) -> Result<i64> {
    let wanted = number.trim().to_uppercase();
    let tickets = task_creator(state)
        .tickets_of(state, about, WARRANTY_CATEGORY)
        .await?;
    tickets
        .into_iter()
        .find(|ticket| ticket.number.to_uppercase() == wanted)
        .map(|ticket| ticket.id.0)
        .ok_or_else(|| refused(english("auto.supplier.problem.ticket", &[])))
}

/// Registers the listener that recovers a claim when its return is credited.
pub(crate) fn register_listeners(app: &mut Registry) {
    app.listen(
        |event: SupplierReturnCredited, state: AppState| async move {
            on_return_credited(&state, &event).await
        },
    );
}

/// The claims matching `text` (the number, the ticket number, the part) and
/// `status` when given, newest first.
///
/// # Errors
///
/// The database's errors.
pub async fn search(
    db: &Db,
    text: &str,
    status: Option<SupplierClaimStatus>,
) -> Result<Vec<SupplierClaim>> {
    let mut query = SupplierClaim::query().order_by_desc(SupplierClaim::ID);
    if let Some(status) = status {
        query = query.where_(SupplierClaim::STATUS.eq(status));
    }
    let all = query.get(db).await?;
    let wanted = text.trim().to_lowercase();
    if wanted.is_empty() {
        return Ok(all);
    }
    Ok(all
        .into_iter()
        .filter(|claim| {
            let haystack = format!(
                "{} {} {} {}",
                claim.number(),
                claim.ticket_number,
                claim.part_description,
                claim.item_code.clone().unwrap_or_default()
            )
            .to_lowercase();
            wanted
                .split_whitespace()
                .all(|word| haystack.contains(word))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_failed_term_has_its_own_text() {
        let mut keys = vec![
            out_of_terms_key(OutOfTerms::Period),
            out_of_terms_key(OutOfTerms::Mileage),
            out_of_terms_key(OutOfTerms::MileageUnknown),
        ];
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn a_new_claim_is_a_kept_part_with_an_accepted_decision() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let claim = NewSupplierClaim::new(
            5,
            Subject::new("auto.job_card", 3),
            9,
            "Alternator",
            45_000,
            day,
            12,
            day,
        );
        assert!(claim.part_kept && claim.decision_accepted);
        assert!(!claim.clone().not_kept().part_kept);
        assert!(!claim.clone().rejected_decision().decision_accepted);
        let fitted = claim.clone().fitted_on(3, 40_000, 58_000).mileage(20_000);
        assert_eq!(
            (
                fitted.fitted_odometer,
                fitted.failure_odometer,
                fitted.warranty_km
            ),
            (Some(40_000), Some(58_000), Some(20_000))
        );
        assert_eq!(
            claim.item(" alt-1 ", "Bosch").item_code.as_deref(),
            Some("ALT-1")
        );
    }
}
