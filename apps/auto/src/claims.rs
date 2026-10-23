//! The insurance repair claim services (08 §4.7, §7.1 *Insurance repair
//! claim*, `AUT.INS.01` .. `AUT.INS.04`): registering a claim, the survey, the
//! estimate and the insurer's approval, supplementaries, the repair that
//! completes it, and the split billing to the insurer and the customer.
//!
//! Every status change goes through the machine ([`crate::claim_machine`]);
//! the services check the permission, work out what a guard needs from the
//! database and pass it in the run's input, and do after the commit what a
//! transition's effect may not: the job card follows the claim (the insurer's
//! approval is the job card's approval, run by the system, for the approved
//! lines only), and the payer bill and the invoice are made through the
//! kernel's `PayerBills` and `Invoicer`.
//!
//! Decisions:
//!
//! - A claim **funds** a job card: the estimate is prepared on the job card
//!   and sent (`send_estimate`), and the claim's estimate is that round. The
//!   insurer's approval names the lines it approves and the amount for each;
//!   the lines of the round it did not approve are declined on the job card
//!   (`AUT.ACL.15`: work is limited to the approved lines; anything else is a
//!   new round the customer approves as a private item).
//! - A job card that a claim funds is not billed on delivery; the claim's
//!   `invoice` bills it ([`crate::billing::bill`] answers `None`). A claim
//!   that ended without the insurer paying (rejected, withdrawn, voided)
//!   funds nothing, so the job card is billed to the customer as any other.
//! - `invoice` makes the customer's invoice (the excess and the private
//!   items) first and the insurer's payer bill after it, and keeps their ids
//!   on the claim as it goes, so that a run that fails halfway is run again
//!   without billing twice.
//! - The claim settles by itself ([`settle_if_paid`]) when the payer bill is
//!   paid or closed and the customer's invoice is paid; the events of
//!   wl-invoicing wake it.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    DraftLine, InvoiceDraft, InvoiceId, InvoicePaid, InvoiceStatus, VatCategory, invoicer,
};
use wl_kernel::parties::directory;
use wl_kernel::payer_bills::{
    NewPayerBill, NewPayerBillLine, PayerBillClosed, PayerBillId, PayerBillStatus,
    PayerRemittanceRecorded, payer_bills,
};
use wl_kernel::{PartyId, Subject};

use crate::approval::EstimateApproval;
use crate::billing::{labour_draft_line, part_draft_line};
use crate::claim::{ClaimStatus, INSURANCE_CLAIM_SUBJECT_TYPE, InsuranceClaim, RepairType};
use crate::claim_approval::{ApprovalKind, ApprovalOutcome, ApprovedLine, ClaimApproval};
use crate::claim_machine::{
    APPROVE, APPROVE_SUPPLEMENTARY, BOOK_SURVEY, CANCEL, COMPLETE, ClaimInput, DECLARE_TOTAL_LOSS,
    DECLINE_SUPPLEMENTARY, INVOICE, REJECT, REQUEST_SUPPLEMENTARY, SETTLE, SUBMIT_ESTIMATE, VOID,
};
use crate::claim_split::insurer_share;
use crate::db::refused;
use crate::estimate::Estimate;
use crate::insurer::Insurer;
use crate::insurers;
use crate::job_card::{JobCard, JobCardStatus};
use crate::job_card_machine::{APPROVE_SYSTEM, DECLINE_ADDED_SYSTEM, JobInput};
use crate::job_cards::run_system as run_job_system;
use crate::labour_line::{LabourLine, LineStatus};
use crate::part_line::PartLine;
use crate::permissions;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// The name of the history row of a claim that came to be.
pub const REGISTER: &str = "register";

/// A claim to register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewClaim {
    /// The insurer.
    pub insurer_id: i64,
    /// The vehicle.
    pub vehicle_id: i64,
    /// The job card the claim funds.
    pub job_card_id: Option<i64>,
    /// The policy number.
    pub policy_no: String,
    /// The insurer's claim number.
    pub claim_no: String,
    /// The day of the accident.
    pub accident_date: NaiveDate,
    /// The police accident report number.
    pub police_report_no: Option<String>,
    /// The date of the police report.
    pub police_report_date: Option<NaiveDate>,
    /// Agency or non-agency repair.
    pub repair_type: RepairType,
    /// The excess the customer pays, in fils.
    pub excess: i64,
    /// The policyholder, when it is not the owner of the vehicle.
    pub customer_party_id: Option<i64>,
}

impl NewClaim {
    /// A claim of insurer `insurer_id` for `vehicle_id`, policy `policy_no`,
    /// claim number `claim_no` and an accident on `accident_date`.
    pub fn new(
        insurer_id: i64,
        vehicle_id: i64,
        policy_no: &str,
        claim_no: &str,
        accident_date: NaiveDate,
    ) -> Self {
        NewClaim {
            insurer_id,
            vehicle_id,
            job_card_id: None,
            policy_no: policy_no.to_owned(),
            claim_no: claim_no.to_owned(),
            accident_date,
            police_report_no: None,
            police_report_date: None,
            repair_type: RepairType::NonAgency,
            excess: 0,
            customer_party_id: None,
        }
    }

    /// Funding job card `id`.
    #[must_use]
    pub fn job_card(mut self, id: i64) -> Self {
        self.job_card_id = Some(id);
        self
    }

    /// With this police report.
    #[must_use]
    pub fn police_report(mut self, no: &str, date: Option<NaiveDate>) -> Self {
        self.police_report_no = Some(no.to_owned());
        self.police_report_date = date;
        self
    }

    /// Repaired at the agency.
    #[must_use]
    pub fn agency(mut self) -> Self {
        self.repair_type = RepairType::Agency;
        self
    }

    /// With this excess in fils.
    #[must_use]
    pub fn excess(mut self, fils: i64) -> Self {
        self.excess = fils;
        self
    }

    /// For this policyholder.
    #[must_use]
    pub fn customer(mut self, party: i64) -> Self {
        self.customer_party_id = Some(party);
        self
    }
}

/// The text of a trimmed field, `None` when blank.
fn optional(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// Registers a claim (`AUT.INS.01`): the insurer, the policy and claim
/// number, the accident date, the police report, agency or non-agency repair
/// and the excess. The insurer's rate card version in force is pinned to the
/// claim.
///
/// # Errors
///
/// A 403 without `insurance.manage`; a 422 for an insurer that is unknown or
/// stopped, a vehicle that is not in the register, a blank policy or claim
/// number, a claim number the insurer has already, an accident in the future,
/// a police report dated before the accident or in the future, a negative
/// excess, or a job card that is for another vehicle, is closed or is funded
/// by another claim.
pub async fn register(state: &AppState, by: &AuthUser, new: NewClaim) -> Result<InsuranceClaim> {
    if !by.has_permission(permissions::INSURANCE_MANAGE) {
        return Err(Error::Forbidden);
    }
    let insurer = Insurer::find(&state.db, new.insurer_id)
        .await?
        .ok_or_else(|| refused(english("auto.claims.problem.insurer", &[])))?;
    if !insurer.active {
        return Err(refused(english(
            "auto.claims.problem.insurer_inactive",
            &[],
        )));
    }
    let vehicle = Vehicle::find(&state.db, new.vehicle_id)
        .await?
        .ok_or_else(|| refused(english("auto.claims.problem.vehicle", &[])))?;
    let policy_no = new.policy_no.trim().to_owned();
    let claim_no = new.claim_no.trim().to_owned();
    if policy_no.is_empty() {
        return Err(refused(english("auto.claims.problem.policy", &[])));
    }
    if claim_no.is_empty() {
        return Err(refused(english("auto.claims.problem.claim_no", &[])));
    }
    let taken = InsuranceClaim::query()
        .where_(InsuranceClaim::INSURER_ID.eq(insurer.id))
        .where_(InsuranceClaim::CLAIM_NO.eq(claim_no.as_str()))
        .count(&state.db)
        .await?;
    if taken > 0 {
        return Err(refused(english("auto.claims.problem.claim_taken", &[])));
    }
    let day = today();
    if new.accident_date > day {
        return Err(refused(english("auto.claims.problem.accident_date", &[])));
    }
    if let Some(report_date) = new.police_report_date
        && (report_date < new.accident_date || report_date > day)
    {
        return Err(refused(english("auto.claims.problem.police_date", &[])));
    }
    if new.excess < 0 {
        return Err(refused(english("auto.claims.problem.excess", &[])));
    }
    let mut customer = new.customer_party_id.unwrap_or(vehicle.owner_party_id);
    if let Some(job_id) = new.job_card_id {
        let card = JobCard::find(&state.db, job_id)
            .await?
            .ok_or_else(|| refused(english("auto.claims.problem.job_card", &[])))?;
        if card.vehicle_id != vehicle.id {
            return Err(refused(english(
                "auto.claims.problem.job_card_vehicle",
                &[],
            )));
        }
        if card.status.is_closed() {
            return Err(refused(english("auto.claims.problem.job_card_closed", &[])));
        }
        if InsuranceClaim::funding(&state.db, card.id).await?.is_some() {
            return Err(refused(english("auto.claims.problem.job_card_linked", &[])));
        }
        if new.customer_party_id.is_none() {
            customer = card.customer_party_id;
        }
    }
    let card_version = insurers::rate_card_of(state, &insurer, day).await?;
    let mut claim = InsuranceClaim {
        insurer_id: insurer.id,
        customer_party_id: customer,
        vehicle_id: vehicle.id,
        job_card_id: new.job_card_id,
        policy_no,
        claim_no,
        accident_date: new.accident_date,
        police_report_no: optional(&new.police_report_no),
        police_report_date: new.police_report_date,
        repair_type: new.repair_type,
        excess: new.excess,
        rate_card_id: card_version.as_ref().map(|v| v.rate_card),
        rate_card_version: card_version.as_ref().map(|v| v.version),
        created_by: Some(by.id),
        ..InsuranceClaim::default()
    };
    let mut tx = state.db.begin().await?;
    claim
        .insert_in_state(&mut tx, ClaimStatus::Registered, REGISTER)
        .await?;
    tx.commit().await?;
    // The accident and its police report are the job card's too
    // (`AUT.ACL.20`: the report is attached to the job card or its claim).
    if let Some(job_id) = new.job_card_id {
        mark_accident(state, job_id, &claim).await?;
    }
    InsuranceClaim::find_or_404(&state.db, claim.id).await
}

/// Flags the job card as an accident repair and gives it the claim's police
/// report when it has none.
async fn mark_accident(state: &AppState, job_card_id: i64, claim: &InsuranceClaim) -> Result {
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    let report = card
        .police_report_no
        .clone()
        .filter(|no| !no.trim().is_empty())
        .or_else(|| claim.police_report_no.clone());
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            &state.db,
            &[
                ("accident_damage", &true),
                ("police_report_no", &report),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Records the police report on claim `id` (`AUT.ACL.20`).
///
/// # Errors
///
/// A 403 without `insurance.manage`; a 422 for a blank number, a date before
/// the accident or in the future, or a claim that is over.
pub async fn record_police_report(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    no: &str,
    date: Option<NaiveDate>,
) -> Result<InsuranceClaim> {
    if !by.has_permission(permissions::INSURANCE_MANAGE) {
        return Err(Error::Forbidden);
    }
    let claim = InsuranceClaim::find_or_404(&state.db, id).await?;
    let no = no.trim();
    if no.is_empty() {
        return Err(refused(english("auto.claims.problem.police_no", &[])));
    }
    if claim.status.is_closed() {
        return Err(refused(english("auto.claims.problem.closed", &[])));
    }
    if let Some(date) = date
        && (date < claim.accident_date || date > today())
    {
        return Err(refused(english("auto.claims.problem.police_date", &[])));
    }
    InsuranceClaim::query()
        .where_(InsuranceClaim::ID.eq(claim.id))
        .update(
            &state.db,
            &[
                ("police_report_no", &Some(no.to_owned())),
                ("police_report_date", &date),
                ("updated_at", &now()),
            ],
        )
        .await?;
    if let Some(job_id) = claim.job_card_id {
        let fresh = InsuranceClaim::find_or_404(&state.db, claim.id).await?;
        mark_accident(state, job_id, &fresh).await?;
    }
    InsuranceClaim::find_or_404(&state.db, id).await
}

/// Runs the transition `name` of the user `by` on `claim`.
async fn run(
    state: &AppState,
    by: &AuthUser,
    claim: &InsuranceClaim,
    name: &str,
    reason: Option<&str>,
    input: ClaimInput,
) -> Result<InsuranceClaim> {
    let transition = claim.transition(name).by(by).input(&input);
    let _outcome = match reason {
        Some(reason) => transition.reason(reason).run(state).await,
        None => transition.run(state).await,
    }
    .map_err(TransitionError::into_error)?;
    InsuranceClaim::find_or_404(&state.db, claim.id).await
}

/// Runs the system transition `name` on `claim`.
async fn run_system(
    state: &AppState,
    claim: &InsuranceClaim,
    name: &str,
    input: ClaimInput,
) -> Result<InsuranceClaim> {
    let _outcome = claim
        .transition(name)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    InsuranceClaim::find_or_404(&state.db, claim.id).await
}

/// The input every run starts from.
fn input_for(by: &AuthUser) -> ClaimInput {
    ClaimInput {
        by: Some(by.id),
        on: Some(today()),
        ..ClaimInput::default()
    }
}

/// What a run of `name` by `by` would be refused for, without running it (the
/// access, the state and the guards).
///
/// # Errors
///
/// The refusal the run would give.
pub async fn dry_run(
    claim: &InsuranceClaim,
    name: &str,
    by: &AuthUser,
    input: &ClaimInput,
) -> std::result::Result<(), TransitionError> {
    let machine = InsuranceClaim::machine();
    let transition = machine.find(name)?;
    transition.check_access(claim, Some(by))?;
    transition.check_from(claim.status.as_str())?;
    let value = renox::serde_json::to_value(input).unwrap_or(renox::serde_json::Value::Null);
    let ctx = TransitionCtx::new(Some(by.clone())).with_input(value);
    transition.check_guards(claim, &ctx).await
}

/// The claim `id` for a user holding `permission`.
async fn load(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    permission: &str,
) -> Result<InsuranceClaim> {
    if !by.has_permission(permission) {
        return Err(Error::Forbidden);
    }
    InsuranceClaim::find_or_404(&state.db, id).await
}

/// `book_survey`: the surveyor is asked to inspect.
///
/// # Errors
///
/// A 403 without `insurance.manage`; a 422 without the policy and claim
/// numbers, or in another state.
pub async fn book_survey(state: &AppState, by: &AuthUser, id: i64) -> Result<InsuranceClaim> {
    let claim = load(state, by, id, permissions::INSURANCE_MANAGE).await?;
    run(state, by, &claim, BOOK_SURVEY, None, input_for(by)).await
}

/// The job card of `claim` and the estimate round waiting for an answer.
async fn pending_round_of(
    state: &AppState,
    claim: &InsuranceClaim,
) -> Result<(JobCard, EstimateApproval)> {
    let job_id = claim
        .job_card_id
        .ok_or_else(|| refused(english("auto.claims.problem.no_job_card", &[])))?;
    let card = JobCard::find_or_404(&state.db, job_id).await?;
    let round = EstimateApproval::pending_of(&state.db, card.id)
        .await?
        .filter(|_| card.status == JobCardStatus::AwaitingApproval)
        .ok_or_else(|| refused(english("auto.claims.problem.not_sent", &[])))?;
    Ok((card, round))
}

/// The lines the estimate round `round_id` sent to the customer.
async fn round_lines(db: &Db, round_id: i64) -> Result<(Vec<LabourLine>, Vec<PartLine>)> {
    let labour = LabourLine::query()
        .where_(LabourLine::APPROVAL_ID.eq(Some(round_id)))
        .where_(LabourLine::APPROVAL.eq(LineStatus::Sent))
        .get(db)
        .await?;
    let parts = PartLine::query()
        .where_(PartLine::APPROVAL_ID.eq(Some(round_id)))
        .where_(PartLine::APPROVAL.eq(LineStatus::Sent))
        .get(db)
        .await?;
    Ok((labour, parts))
}

/// `submit_estimate`: the estimate the advisor sent on the job card goes to
/// the insurer; the claim records what was asked and waits for the answer
/// (`AUT.INS.02`).
///
/// # Errors
///
/// A 403 without `insurance.manage`; a 422 without a job card whose estimate
/// is sent (waiting for an answer), without the policy and claim numbers, or
/// in another state.
pub async fn submit_estimate(state: &AppState, by: &AuthUser, id: i64) -> Result<InsuranceClaim> {
    let claim = load(state, by, id, permissions::INSURANCE_MANAGE).await?;
    let (_card, round) = pending_round_of(state, &claim).await?;
    let (labour, parts) = round_lines(&state.db, round.id).await?;
    let mut input = input_for(by);
    input.priced = Some(!(labour.is_empty() && parts.is_empty()) && round.net_total > 0);
    input.estimate_amount = Some(round.net_total);
    input.job_round_id = Some(round.id);
    run(state, by, &claim, SUBMIT_ESTIMATE, None, input).await
}

/// The insurer's answer to record: its reference, the written approval, the
/// lines it approved with an amount each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The insurer's approval reference or LPO number.
    pub reference: String,
    /// The insurer's written approval (a document reference).
    pub document: String,
    /// The approved lines; the description may be left blank.
    pub lines: Vec<ApprovedLine>,
    /// The excess, set when the estimate is approved.
    pub excess: Option<i64>,
}

impl Answer {
    /// An approval with this reference and document.
    pub fn new(reference: &str, document: &str) -> Self {
        Answer {
            reference: reference.to_owned(),
            document: document.to_owned(),
            lines: Vec::new(),
            excess: None,
        }
    }

    /// Approving `amount` fils of labour line `line_id`.
    #[must_use]
    pub fn labour(mut self, line_id: i64, amount: i64) -> Self {
        self.lines.push(ApprovedLine {
            kind: "labour".to_owned(),
            line_id,
            description: String::new(),
            amount,
        });
        self
    }

    /// Approving `amount` fils of part line `line_id`.
    #[must_use]
    pub fn part(mut self, line_id: i64, amount: i64) -> Self {
        self.lines.push(ApprovedLine {
            kind: "part".to_owned(),
            line_id,
            description: String::new(),
            amount,
        });
        self
    }

    /// With this excess in fils.
    #[must_use]
    pub fn excess(mut self, fils: i64) -> Self {
        self.excess = Some(fils);
        self
    }
}

/// Checks the approved `lines` against the lines of round `round_id` and fills
/// in their descriptions: a line must be one the round sent and cannot be
/// approved for more than its price.
async fn resolve_lines(
    db: &Db,
    round_id: i64,
    lines: &[ApprovedLine],
) -> Result<Vec<ApprovedLine>> {
    let (labour, parts) = round_lines(db, round_id).await?;
    let mut resolved = Vec::new();
    for line in lines {
        let (description, price) = match line.kind.as_str() {
            "labour" => labour
                .iter()
                .find(|l| l.id == line.line_id)
                .map(|l| (l.description.clone(), l.price())),
            "part" => parts
                .iter()
                .find(|l| l.id == line.line_id)
                .map(|l| (l.description.clone(), l.price())),
            _ => None,
        }
        .ok_or_else(|| refused(english("auto.claims.problem.line_unknown", &[])))?;
        if line.amount > price {
            return Err(refused(english(
                "auto.claims.problem.line_too_high",
                &[("line", &description)],
            )));
        }
        if resolved
            .iter()
            .any(|r: &ApprovedLine| r.kind == line.kind && r.line_id == line.line_id)
        {
            return Err(refused(english("auto.claims.problem.line_twice", &[])));
        }
        resolved.push(ApprovedLine {
            description,
            ..line.clone()
        });
    }
    Ok(resolved)
}

/// Declines the lines of round `round_id` that the insurer did not approve:
/// the job starts with the approved lines only (`AUT.ACL.15`).
async fn decline_unapproved(db: &Db, round_id: i64, approved: &[ApprovedLine]) -> Result {
    let (labour, parts) = round_lines(db, round_id).await?;
    let stamp = now();
    for line in labour.iter().filter(|l| {
        !approved
            .iter()
            .any(|a| a.kind == "labour" && a.line_id == l.id)
    }) {
        LabourLine::query()
            .where_(LabourLine::ID.eq(line.id))
            .update(
                db,
                &[("approval", &LineStatus::Declined), ("updated_at", &stamp)],
            )
            .await?;
    }
    for line in parts.iter().filter(|l| {
        !approved
            .iter()
            .any(|a| a.kind == "part" && a.line_id == l.id)
    }) {
        PartLine::query()
            .where_(PartLine::ID.eq(line.id))
            .update(
                db,
                &[("approval", &LineStatus::Declined), ("updated_at", &stamp)],
            )
            .await?;
    }
    Ok(())
}

/// The input of an approval run: the insurer's answer with its lines resolved.
async fn answer_input(
    state: &AppState,
    by: &AuthUser,
    claim: &InsuranceClaim,
    kind: ApprovalKind,
    answer: &Answer,
) -> Result<(ClaimInput, Option<i64>)> {
    let pending = ClaimApproval::pending(&state.db, claim.id, kind)
        .await?
        .ok_or_else(|| refused(english("auto.claims.problem.nothing_asked", &[])))?;
    let lines = match pending.job_round_id {
        Some(round_id) => resolve_lines(&state.db, round_id, &answer.lines).await?,
        None => answer.lines.clone(),
    };
    let mut input = input_for(by);
    input.reference = optional(&Some(answer.reference.clone()));
    input.document = optional(&Some(answer.document.clone()));
    input.lines = lines;
    input.excess = answer.excess;
    if answer.excess.is_some_and(|excess| excess < 0) {
        return Err(refused(english("auto.claims.problem.excess", &[])));
    }
    Ok((input, pending.job_round_id))
}

/// The job card follows the insurer's approval: the lines it did not approve
/// are declined and the round is approved as the system, in the insurer's
/// name.
async fn approve_job(
    state: &AppState,
    by: &AuthUser,
    claim: &InsuranceClaim,
    round_id: Option<i64>,
    lines: &[ApprovedLine],
) -> Result {
    let (Some(job_id), Some(round_id)) = (claim.job_card_id, round_id) else {
        return Ok(());
    };
    let card = JobCard::find_or_404(&state.db, job_id).await?;
    let pending = EstimateApproval::pending_of(&state.db, card.id).await?;
    if card.status != JobCardStatus::AwaitingApproval
        || pending.as_ref().map(|round| round.id) != Some(round_id)
    {
        return Ok(());
    }
    decline_unapproved(&state.db, round_id, lines).await?;
    let insurer = Insurer::find_or_404(&state.db, claim.insurer_id).await?;
    let name = directory(state)
        .display_name(&state.db, PartyId(insurer.party_id), "en")
        .await?
        .unwrap_or_else(|| claim.claim_no.clone());
    let input = JobInput {
        by: Some(by.id),
        on: Some(today()),
        approval_id: Some(round_id),
        decided_by_name: Some(name),
        method: Some("recorded".to_owned()),
        ..JobInput::default()
    };
    run_job_system(state, &card, APPROVE_SYSTEM, None, input).await?;
    Ok(())
}

/// `approve`: the insurer's approval or LPO, with the approved amount per
/// line (`AUT.INS.02`, `AUT.ACL.16`). The job card starts with the approved
/// lines only; the excess is set.
///
/// # Errors
///
/// A 403 without `insurance.manage`; a 422 without the reference, the written
/// approval or at least one approved line, for a line the estimate did not
/// send or an amount above its price, or in another state.
pub async fn approve(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    answer: Answer,
) -> Result<InsuranceClaim> {
    let claim = load(state, by, id, permissions::INSURANCE_MANAGE).await?;
    let (input, round_id) =
        answer_input(state, by, &claim, ApprovalKind::Estimate, &answer).await?;
    let lines = input.lines.clone();
    let approved = run(state, by, &claim, APPROVE, None, input).await?;
    approve_job(state, by, &approved, round_id, &lines).await?;
    InsuranceClaim::find_or_404(&state.db, id).await
}

/// `reject`: the insurer declined the claim; the customer is offered a private
/// estimate on the same job card.
///
/// # Errors
///
/// A 403 without `insurance.manage`; a 422 without the insurer's reason or in
/// another state.
pub async fn reject(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<InsuranceClaim> {
    let claim = load(state, by, id, permissions::INSURANCE_MANAGE).await?;
    run(state, by, &claim, REJECT, Some(reason), input_for(by)).await
}

/// `request_supplementary`: hidden damage found during strip-down goes back to
/// the insurer (`AUT.INS.03`). The advisor has sent the added work on the job
/// card; the claim asks the insurer for it, with the findings.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 without findings, without added
/// work sent on the job card, or in another state.
pub async fn request_supplementary(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    findings: &str,
) -> Result<InsuranceClaim> {
    let claim = load(state, by, id, permissions::JOB_CARDS_ADVISE).await?;
    let (_card, round) = pending_round_of(state, &claim).await?;
    let (labour, parts) = round_lines(&state.db, round.id).await?;
    let mut input = input_for(by);
    input.priced = Some(!(labour.is_empty() && parts.is_empty()));
    input.estimate_amount = Some(round.net_total);
    input.job_round_id = Some(round.id);
    input.findings = optional(&Some(findings.to_owned()));
    run(state, by, &claim, REQUEST_SUPPLEMENTARY, None, input).await
}

/// `approve_supplementary`: the insurer approved the hidden damage. The
/// approved amount goes up and the job card lines are released.
///
/// # Errors
///
/// As for [`approve`], on the supplementary.
pub async fn approve_supplementary(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    answer: Answer,
) -> Result<InsuranceClaim> {
    let claim = load(state, by, id, permissions::INSURANCE_MANAGE).await?;
    let (input, round_id) =
        answer_input(state, by, &claim, ApprovalKind::Supplementary, &answer).await?;
    let lines = input.lines.clone();
    let approved = run(state, by, &claim, APPROVE_SUPPLEMENTARY, None, input).await?;
    approve_job(state, by, &approved, round_id, &lines).await?;
    InsuranceClaim::find_or_404(&state.db, id).await
}

/// `decline_supplementary`: the insurer declined the hidden damage; the added
/// lines are declined on the job card (the advisor offers them to the customer
/// as private items on a new round, `AUT.ACL.15`).
///
/// # Errors
///
/// A 403 without `insurance.manage`; a 422 without the insurer's reason or in
/// another state.
pub async fn decline_supplementary(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<InsuranceClaim> {
    let claim = load(state, by, id, permissions::INSURANCE_MANAGE).await?;
    let pending = ClaimApproval::pending(&state.db, claim.id, ApprovalKind::Supplementary).await?;
    let declined = run(
        state,
        by,
        &claim,
        DECLINE_SUPPLEMENTARY,
        Some(reason),
        input_for(by),
    )
    .await?;
    if let (Some(job_id), Some(round_id)) = (
        declined.job_card_id,
        pending.and_then(|request| request.job_round_id),
    ) {
        let card = JobCard::find_or_404(&state.db, job_id).await?;
        let still_waiting = EstimateApproval::pending_of(&state.db, card.id)
            .await?
            .is_some_and(|round| round.id == round_id);
        if card.status == JobCardStatus::AwaitingApproval && still_waiting {
            let input = JobInput {
                by: Some(by.id),
                on: Some(today()),
                approval_id: Some(round_id),
                decided_by_name: Some(declined.claim_no.clone()),
                method: Some("recorded".to_owned()),
                ..JobInput::default()
            };
            run_job_system(state, &card, DECLINE_ADDED_SYSTEM, Some(reason), input).await?;
        }
    }
    InsuranceClaim::find_or_404(&state.db, id).await
}

/// `declare_total_loss`: the insurer declared the vehicle a total loss. The
/// notice is kept on the claim; the job card and the charges are the issue's
/// (BL-AUT-09).
///
/// # Errors
///
/// A 403 without `insurance.manage`; a 422 in another state.
pub async fn declare_total_loss(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    notice: &str,
) -> Result<InsuranceClaim> {
    let claim = load(state, by, id, permissions::INSURANCE_MANAGE).await?;
    let mut input = input_for(by);
    input.document = optional(&Some(notice.to_owned()));
    run(state, by, &claim, DECLARE_TOTAL_LOSS, None, input).await
}

/// `cancel`: the customer withdrew the claim; `document` is the written
/// withdrawal. What happens to the job card is the issue's (BL-AUT-08).
///
/// # Errors
///
/// A 403 without `insurance.manage`; a 422 without a reason or in another
/// state.
pub async fn cancel(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    document: Option<&str>,
) -> Result<InsuranceClaim> {
    let claim = load(state, by, id, permissions::INSURANCE_MANAGE).await?;
    let mut input = input_for(by);
    input.document = optional(&document.map(str::to_owned));
    run(state, by, &claim, CANCEL, Some(reason), input).await
}

/// `void`: registered in error. What happens to the job card is the issue's
/// (BL-AUT-10).
///
/// # Errors
///
/// A 403 without `insurance.manage`; a 422 without a reason or in another
/// state.
pub async fn void(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<InsuranceClaim> {
    let claim = load(state, by, id, permissions::INSURANCE_MANAGE).await?;
    run(state, by, &claim, VOID, Some(reason), input_for(by)).await
}

/// The job card passed QC (`complete`): the claim funding it is ready for
/// billing. A job card that no claim funds, or whose claim is not `approved`,
/// is left as it is.
///
/// # Errors
///
/// The machine's refusal.
pub async fn on_job_ready(state: &AppState, job_card_id: i64) -> Result {
    let Some(claim) = InsuranceClaim::funding(&state.db, job_card_id).await? else {
        return Ok(());
    };
    if claim.status != ClaimStatus::Approved {
        return Ok(());
    }
    let input = ClaimInput {
        on: Some(today()),
        ..ClaimInput::default()
    };
    run_system(state, &claim, COMPLETE, input).await?;
    Ok(())
}

/// Whether an open claim funds job card `job_card_id`, so that the job card is
/// not billed to the customer on delivery ([`crate::billing::bill`]).
///
/// # Errors
///
/// The database's errors.
pub async fn funds_job_card(db: &Db, job_card_id: i64) -> Result<bool> {
    Ok(InsuranceClaim::funding(db, job_card_id).await?.is_some())
}

/// A line of a request waiting for the insurer's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestLine {
    /// `labour` or `part`.
    pub kind: &'static str,
    /// The job card line.
    pub line_id: i64,
    /// What the line is.
    pub description: String,
    /// Its price before VAT, in fils.
    pub price: i64,
}

/// The lines of the request of `kind` that waits for the insurer's answer: the
/// lines of the job card's estimate round it is about.
///
/// # Errors
///
/// The database's errors.
pub async fn request_lines(db: &Db, claim_id: i64, kind: ApprovalKind) -> Result<Vec<RequestLine>> {
    let Some(round_id) = ClaimApproval::pending(db, claim_id, kind)
        .await?
        .and_then(|request| request.job_round_id)
    else {
        return Ok(Vec::new());
    };
    let (labour, parts) = round_lines(db, round_id).await?;
    let mut lines: Vec<RequestLine> = labour
        .iter()
        .map(|line| RequestLine {
            kind: "labour",
            line_id: line.id,
            description: line.description.clone(),
            price: line.price(),
        })
        .collect();
    lines.extend(parts.iter().map(|line| RequestLine {
        kind: "part",
        line_id: line.id,
        description: line.description.clone(),
        price: line.price(),
    }));
    Ok(lines)
}

/// The approved lines of every request the insurer approved.
pub fn approved_lines(approvals: &[ClaimApproval], kinds: &[ApprovalKind]) -> Vec<ApprovedLine> {
    approvals
        .iter()
        .filter(|a| a.outcome == ApprovalOutcome::Approved && kinds.contains(&a.kind))
        .flat_map(|a| a.lines.0.clone())
        .collect()
}

/// The lines of the job card the customer approved and the insurer did not:
/// the private items, as invoice lines.
fn private_lines(
    estimate: &Estimate,
    approved: &[ApprovedLine],
    origin: &Subject,
) -> Vec<DraftLine> {
    let mut lines = Vec::new();
    for line in estimate
        .labour
        .iter()
        .filter(|l| l.approval == LineStatus::Approved)
        .filter(|l| {
            !approved
                .iter()
                .any(|a| a.kind == "labour" && a.line_id == l.id)
        })
    {
        lines.push(labour_draft_line(line, origin.clone()));
    }
    for line in estimate
        .parts
        .iter()
        .filter(|l| l.approval == LineStatus::Approved)
        .filter(|l| {
            !approved
                .iter()
                .any(|a| a.kind == "part" && a.line_id == l.id)
        })
    {
        lines.push(part_draft_line(line, origin.clone()));
    }
    lines
}

/// Sets the ids of the payer bill and the customer invoice on the claim.
async fn keep_bills(
    state: &AppState,
    id: i64,
    payer_bill: Option<i64>,
    customer_invoice: Option<i64>,
) -> Result {
    InsuranceClaim::query()
        .where_(InsuranceClaim::ID.eq(id))
        .update(
            &state.db,
            &[
                ("payer_bill_id", &payer_bill),
                ("customer_invoice_id", &customer_invoice),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// `invoice`: the split billing (`AUT.INS.04`, `FIN.TPP.01`). The customer is
/// invoiced the excess and the private items; the insurer gets a payer bill
/// for the approved amount net of the excess, which issues its tax invoice
/// (the payer is the VAT recipient). From `total_loss` the insurer is billed
/// the charges it approved.
///
/// # Errors
///
/// A 403 without `invoices.issue`; a 422 in another state, from `total_loss`
/// without the insurer's approval of the charges, for an insurer without a
/// payer agreement, or when nothing was approved; the invoicer's and the
/// payer bills' errors.
pub async fn invoice(state: &AppState, by: &AuthUser, id: i64) -> Result<InsuranceClaim> {
    let claim = load(state, by, id, permissions::INVOICES_ISSUE).await?;
    let approvals = ClaimApproval::of_claim(&state.db, claim.id).await?;
    let from_total_loss = claim.status == ClaimStatus::TotalLoss;
    let kinds: &[ApprovalKind] = if from_total_loss {
        &[ApprovalKind::Charges]
    } else {
        &[ApprovalKind::Estimate, ApprovalKind::Supplementary]
    };
    let approved = approved_lines(&approvals, kinds);
    let mut input = input_for(by);
    input.charges_approved = Some(
        approvals
            .iter()
            .any(|a| a.kind == ApprovalKind::Charges && a.outcome == ApprovalOutcome::Approved),
    );
    dry_run(&claim, INVOICE, by, &input)
        .await
        .map_err(TransitionError::into_error)?;
    if approved.is_empty() {
        return Err(refused(english(
            "auto.claims.problem.nothing_approved",
            &[],
        )));
    }
    let insurer = Insurer::find_or_404(&state.db, claim.insurer_id).await?;
    let agreement = insurer
        .payer_agreement_id
        .ok_or_else(|| refused(english("auto.claims.problem.no_agreement", &[])))?;
    let excess = if from_total_loss { 0 } else { claim.excess };
    let share = insurer_share(&approved, excess);
    let origin = claim.subject();
    // The customer's invoice first: the excess and the private items.
    let mut customer_invoice = claim.customer_invoice_id;
    if customer_invoice.is_none() {
        let mut draft = InvoiceDraft::new(PartyId(claim.customer_party_id), origin.clone())
            .date_of_supply(today());
        let mut lines = 0;
        if share.excess_applied > 0 {
            draft = draft.line(
                DraftLine::new(
                    &english("auto.claims.excess_line", &[("claim", &claim.claim_no)]),
                    share.excess_applied,
                    VatCategory::Standard,
                )
                .origin(origin.clone()),
            );
            lines += 1;
        }
        if !from_total_loss && let Some(job_id) = claim.job_card_id {
            let estimate = Estimate::load(&state.db, job_id).await?;
            for line in private_lines(&estimate, &approved, &origin) {
                draft = draft.line(line);
                lines += 1;
            }
        }
        if lines > 0 {
            let books = invoicer(state);
            if let Some(made) = books.draft(&state.db, draft).await? {
                keep_bills(state, claim.id, claim.payer_bill_id, Some(made.0)).await?;
                customer_invoice = Some(made.0);
                let _issued = books.issue(state, made, None).await;
            }
        }
    }
    // Then the insurer's payer bill.
    let mut payer_bill = claim.payer_bill_id;
    if payer_bill.is_none() {
        let mut request = NewPayerBill::new(
            agreement,
            PartyId(claim.customer_party_id),
            origin.clone(),
            today(),
        )
        .payer_reference(&claim.claim_no);
        if let Some(invoice) = customer_invoice {
            request = request.customer_invoice(InvoiceId(invoice));
        }
        for line in &share.lines {
            request = request.line(
                NewPayerBillLine::new(&line.description, line.amount)
                    .vat_category(VatCategory::Standard),
            );
        }
        let made = payer_bills(state).create(state, request, Some(by)).await?;
        keep_bills(state, claim.id, Some(made.0), customer_invoice).await?;
        payer_bill = Some(made.0);
    }
    if let Some(bill) = payer_bill {
        let bills = payer_bills(state);
        let summary = bills.find(&state.db, PayerBillId(bill)).await?;
        if summary.is_none_or(|s| s.status == PayerBillStatus::Draft) {
            bills.issue(state, PayerBillId(bill), None).await?;
        }
    }
    run(state, by, &claim, INVOICE, None, input).await
}

/// The claim `id` when it is `invoiced` and every invoice of it is paid:
/// settles it as the system. Answers whether it settled.
///
/// # Errors
///
/// The machine's refusal.
pub async fn settle_if_paid(state: &AppState, id: i64) -> Result<bool> {
    let claim = InsuranceClaim::find_or_404(&state.db, id).await?;
    if claim.status != ClaimStatus::Invoiced {
        return Ok(false);
    }
    if let Some(bill) = claim.payer_bill_id {
        let paid = payer_bills(state)
            .find(&state.db, PayerBillId(bill))
            .await?
            .is_some_and(|s| matches!(s.status, PayerBillStatus::Paid | PayerBillStatus::Closed));
        if !paid {
            return Ok(false);
        }
    }
    if let Some(invoice) = claim.customer_invoice_id {
        let paid = invoicer(state)
            .find(&state.db, InvoiceId(invoice))
            .await?
            .is_some_and(|s| s.status == InvoiceStatus::Paid);
        if !paid {
            return Ok(false);
        }
    }
    let input = ClaimInput {
        on: Some(today()),
        ..ClaimInput::default()
    };
    run_system(state, &claim, SETTLE, input).await?;
    Ok(true)
}

/// The claim a subject names, when it is one.
fn claim_of(subject: &Subject) -> Option<i64> {
    (subject.subject_type == INSURANCE_CLAIM_SUBJECT_TYPE).then_some(subject.subject_id)
}

/// A payment or a remittance about `source`: settles the claim when it is one
/// and everything is paid.
async fn on_money(state: &AppState, source: &Subject) -> Result {
    if let Some(id) = claim_of(source) {
        settle_if_paid(state, id).await?;
    }
    Ok(())
}

/// Registers the listeners that settle a claim when its invoices are paid.
pub(crate) fn register_listeners(app: &mut Registry) {
    app.listen(|event: InvoicePaid, state: AppState| async move {
        on_money(&state, &event.source).await
    });
    app.listen(
        |event: PayerRemittanceRecorded, state: AppState| async move {
            on_money(&state, &event.source).await
        },
    );
    app.listen(|event: PayerBillClosed, state: AppState| async move {
        on_money(&state, &event.source).await
    });
}

/// The claims matching `text` (the number, the claim number, the policy number
/// or the insurer's claim, in any case) and `status` when given, newest
/// first.
///
/// # Errors
///
/// The database's errors.
pub async fn search(
    db: &Db,
    text: &str,
    status: Option<ClaimStatus>,
) -> Result<Vec<InsuranceClaim>> {
    let mut query = InsuranceClaim::query().order_by_desc(InsuranceClaim::ID);
    if let Some(status) = status {
        query = query.where_(InsuranceClaim::STATUS.eq(status));
    }
    let all = query.get(db).await?;
    let wanted = text.trim().to_lowercase();
    if wanted.is_empty() {
        return Ok(all);
    }
    Ok(all
        .into_iter()
        .filter(|claim| {
            let haystack =
                format!("{} {} {}", claim.number(), claim.claim_no, claim.policy_no).to_lowercase();
            wanted
                .split_whitespace()
                .all(|word| haystack.contains(word))
        })
        .collect())
}

/// The open job cards a claim can fund: of `vehicle_id` when given, matching
/// `text` (the number), that no claim funds, newest first, at most 20.
///
/// # Errors
///
/// The database's errors.
pub async fn linkable_job_cards(
    db: &Db,
    vehicle_id: Option<i64>,
    text: &str,
) -> Result<Vec<JobCard>> {
    let mut query = JobCard::query().order_by_desc(JobCard::ID);
    if let Some(vehicle) = vehicle_id {
        query = query.where_(JobCard::VEHICLE_ID.eq(vehicle));
    }
    let wanted = text.trim().to_lowercase();
    let mut found = Vec::new();
    for card in query.get(db).await? {
        if card.status.is_closed() || !card.number().to_lowercase().contains(&wanted) {
            continue;
        }
        if InsuranceClaim::funding(db, card.id).await?.is_some() {
            continue;
        }
        found.push(card);
        if found.len() == 20 {
            break;
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_claim_subject_names_a_claim() {
        assert_eq!(
            claim_of(&Subject::new(INSURANCE_CLAIM_SUBJECT_TYPE, 7)),
            Some(7)
        );
        assert_eq!(claim_of(&Subject::new("auto.job_card", 7)), None);
    }

    #[test]
    fn the_answer_builds_its_lines() {
        let answer = Answer::new("LPO-1", "email-1")
            .labour(3, 5_000)
            .part(4, 7_000);
        assert_eq!(answer.lines.len(), 2);
        assert_eq!(answer.lines[0].kind, "labour");
        assert_eq!(answer.lines[1].line_id, 4);
        assert_eq!(answer.excess, None);
        assert_eq!(answer.excess(30_000).excess, Some(30_000));
    }

    #[test]
    fn a_new_claim_is_a_non_agency_repair_without_an_excess() {
        let claim = NewClaim::new(
            1,
            2,
            "POL-1",
            "CLM-9",
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        );
        assert_eq!(claim.repair_type, RepairType::NonAgency);
        assert_eq!(claim.excess, 0);
        assert!(claim.agency().repair_type == RepairType::Agency);
    }
}
