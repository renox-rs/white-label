//! The job card services (08 §4.4, §7.1 *Job card*, `AUT.JOB.01`,
//! `AUT.JOB.02`): checking a vehicle in, recording the intake, assigning the
//! technician, drafting the estimate lines, sending the estimate and the
//! customer's answer, the work done per line, the QC and the delivery.
//!
//! Every status change goes through the machine ([`crate::job_card_machine`]);
//! the services check the permission and who may act (a technician works only
//! the job cards assigned to them, `AUT.ACL.08`), work out what a guard needs
//! from the database and pass it in the run's input, and do after the commit
//! what a transition's effect may not: the invoice and the booking on delivery
//! ([`crate::billing`]).
//!
//! Decisions:
//!
//! - A job card opened from a booking's check-in has no intake yet; the advisor
//!   records the vehicle's condition ([`record_intake`]) before the estimate
//!   goes out, so the condition is documented before any work (consumer
//!   protection, 08 §2). A walk-in is checked in with its intake at the desk
//!   ([`check_in`]).
//! - Lines drafted after the first approval are *added work*: they go to the
//!   customer in a new round by `send_estimate` from `in_progress`
//!   (`AUT.JOB.02`); the job card cannot be delivered while any line waits for
//!   an answer.
//! - The customer's approval is the consent to the cost, the time and the
//!   warranty of 08 §2: it records the name, the way (link, signature,
//!   recorded by the advisor), the promised day and the warranty days (at least
//!   15).

use renox::auth::permissions::users_with_role;
use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::scheduling::{BookingId, booking_actions};

use crate::approval::{EstimateApproval, METHODS, MIN_WARRANTY_DAYS};
use crate::db::refused;
use crate::documents::{GUARDED_JOB_CARD_APPROVE, police_verdict};
use crate::estimate::Estimate;
use crate::job_card::{FUEL_LEVELS, JobCard, JobCardStatus, QC_KEYS, QcCheck};
use crate::job_card_machine::{
    APPROVE, APPROVE_SYSTEM, DECLINE, DECLINE_ADDED, DECLINE_ADDED_SYSTEM, DECLINE_SYSTEM,
    DELIVER, FAIL_QC, FINISH_WORK, JobInput, PASS_QC, SEND_ESTIMATE, START_INSPECTION,
    WAIT_PARTS,
};
use crate::labour_line::{LabourLine, LabourPricing, LineStatus, WorkStatus};
use crate::labour_operation::LabourOperation;
use crate::part_line::{PartCondition, PartLine};
use crate::permissions;
use crate::texts::english;
use crate::vehicle::Vehicle;
use crate::vehicles::ensure_odometer;

/// The name of the history row of a job card that came to be.
pub const CHECK_IN: &str = "check_in";

/// A job card to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJobCard {
    /// The vehicle.
    pub vehicle_id: i64,
    /// The customer's complaint.
    pub complaint: String,
    /// The odometer reading at check-in, in kilometres.
    pub odometer_in: i64,
    /// The fuel level, one of [`FUEL_LEVELS`].
    pub fuel_level: Option<String>,
    /// The condition of the vehicle at intake.
    pub intake_condition: Option<String>,
    /// The customer who acknowledged the recorded condition.
    pub intake_acknowledged_by: Option<String>,
    /// The customer's signature on the condition.
    pub intake_signature: Option<String>,
    /// Whether the damage is from an accident.
    pub accident_damage: bool,
    /// The police accident report number.
    pub police_report_no: Option<String>,
    /// The inspection fee agreed at check-in, in fils.
    pub inspection_fee: i64,
    /// The daily storage fee agreed at check-in, in fils.
    pub storage_fee_daily: i64,
    /// The technician assigned, a user holding the technician role.
    pub technician_user_id: Option<i64>,
    /// The booking that was checked in.
    pub booking_id: Option<i64>,
    /// The workshop (a wl-scheduling location).
    pub workshop: Option<Subject>,
    /// The customer billed; the vehicle's owner when `None`.
    pub customer_party_id: Option<i64>,
}

impl NewJobCard {
    /// A job card for `vehicle_id` with this complaint.
    pub fn new(vehicle_id: i64, complaint: &str) -> Self {
        NewJobCard {
            vehicle_id,
            complaint: complaint.to_owned(),
            odometer_in: 0,
            fuel_level: None,
            intake_condition: None,
            intake_acknowledged_by: None,
            intake_signature: None,
            accident_damage: false,
            police_report_no: None,
            inspection_fee: 0,
            storage_fee_daily: 0,
            technician_user_id: None,
            booking_id: None,
            workshop: None,
            customer_party_id: None,
        }
    }

    /// With this odometer reading.
    #[must_use]
    pub fn odometer(mut self, km: i64) -> Self {
        self.odometer_in = km;
        self
    }

    /// With this fuel level.
    #[must_use]
    pub fn fuel(mut self, level: &str) -> Self {
        self.fuel_level = Some(level.to_owned());
        self
    }

    /// With this condition at intake.
    #[must_use]
    pub fn condition(mut self, condition: &str) -> Self {
        self.intake_condition = Some(condition.to_owned());
        self
    }

    /// The customer `name` acknowledged the condition.
    #[must_use]
    pub fn acknowledged_by(mut self, name: &str) -> Self {
        self.intake_acknowledged_by = Some(name.to_owned());
        self
    }

    /// The customer signed the condition.
    #[must_use]
    pub fn signature(mut self, signature: &str) -> Self {
        self.intake_signature = Some(signature.to_owned());
        self
    }

    /// The damage is from an accident; the police report has this number
    /// (blank when it is not at hand yet).
    #[must_use]
    pub fn accident(mut self, report_no: &str) -> Self {
        self.accident_damage = true;
        self.police_report_no = Some(report_no.to_owned()).filter(|n| !n.trim().is_empty());
        self
    }

    /// With an inspection fee of `fils` agreed.
    #[must_use]
    pub fn inspection_fee(mut self, fils: i64) -> Self {
        self.inspection_fee = fils;
        self
    }

    /// With a daily storage fee of `fils` agreed.
    #[must_use]
    pub fn storage_fee(mut self, fils: i64) -> Self {
        self.storage_fee_daily = fils;
        self
    }

    /// Assigned to this technician.
    #[must_use]
    pub fn technician(mut self, user_id: i64) -> Self {
        self.technician_user_id = Some(user_id);
        self
    }

    /// Opened from this booking.
    #[must_use]
    pub fn booking(mut self, booking_id: i64) -> Self {
        self.booking_id = Some(booking_id);
        self
    }

    /// At this workshop.
    #[must_use]
    pub fn workshop(mut self, workshop: Subject) -> Self {
        self.workshop = Some(workshop);
        self
    }

    /// Billed to this party.
    #[must_use]
    pub fn customer(mut self, party_id: i64) -> Self {
        self.customer_party_id = Some(party_id);
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

/// The fuel level, checked.
///
/// # Errors
///
/// A 422 for a level that is not one of [`FUEL_LEVELS`].
pub fn clean_fuel(level: &Option<String>) -> Result<Option<String>> {
    let Some(level) = optional(level) else {
        return Ok(None);
    };
    let level = level.to_lowercase();
    if FUEL_LEVELS.contains(&level.as_str()) {
        Ok(Some(level))
    } else {
        Err(refused(english("auto.jobs.problem.fuel", &[])))
    }
}

/// Refuses a user who does not hold the technician role.
async fn ensure_technician(db: &Db, user_id: i64) -> Result {
    let holders = users_with_role(db, permissions::TECHNICIAN_ROLE).await?;
    if holders.iter().any(|user| user.id == user_id) {
        Ok(())
    } else {
        Err(refused(english("auto.jobs.problem.not_a_technician", &[])))
    }
}

/// Opens a job card at the desk: the vehicle is checked in with its complaint,
/// odometer, fuel level and condition (`AUT.JOB.01`, 08 §4.4 step 2). The
/// odometer reading becomes the vehicle's. When the job card comes from a
/// booking, the booking is linked to it so that its work done is asked about.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 for a vehicle that is not serviced
/// here, a blank complaint or condition, a fuel level that is not one of the
/// list, an odometer below the vehicle's last reading, a negative fee or a
/// technician who is not one.
pub async fn check_in(state: &AppState, by: &AuthUser, new: NewJobCard) -> Result<JobCard> {
    if !by.has_permission(permissions::JOB_CARDS_ADVISE) {
        return Err(Error::Forbidden);
    }
    let vehicle = Vehicle::find_or_404(&state.db, new.vehicle_id).await?;
    if !vehicle.active {
        return Err(refused(english("auto.jobs.problem.vehicle_inactive", &[])));
    }
    let complaint = new.complaint.trim().to_owned();
    if complaint.is_empty() {
        return Err(refused(english("auto.jobs.problem.complaint", &[])));
    }
    let condition = optional(&new.intake_condition)
        .ok_or_else(|| refused(english("auto.jobs.problem.condition", &[])))?;
    ensure_odometer(&vehicle, new.odometer_in)?;
    let fuel = clean_fuel(&new.fuel_level)?;
    if new.inspection_fee < 0 || new.storage_fee_daily < 0 {
        return Err(refused(english("auto.jobs.problem.fee", &[])));
    }
    if let Some(technician) = new.technician_user_id {
        ensure_technician(&state.db, technician).await?;
    }
    let stamp = now();
    let mut card = JobCard {
        vehicle_id: vehicle.id,
        customer_party_id: new.customer_party_id.unwrap_or(vehicle.owner_party_id),
        booking_id: new.booking_id,
        workshop_type: new.workshop.as_ref().map(|w| w.subject_type.clone()),
        workshop_id: new.workshop.as_ref().map(|w| w.subject_id),
        advisor_user_id: Some(by.id),
        technician_user_id: new.technician_user_id,
        complaint,
        odometer_in: new.odometer_in,
        fuel_level: fuel,
        intake_condition: Some(condition),
        intake_by: Some(by.id),
        intake_recorded_at: Some(stamp),
        intake_acknowledged_by: optional(&new.intake_acknowledged_by),
        intake_signature: optional(&new.intake_signature),
        accident_damage: new.accident_damage,
        police_report_no: optional(&new.police_report_no),
        inspection_fee: new.inspection_fee,
        storage_fee_daily: new.storage_fee_daily,
        created_by: Some(by.id),
        ..JobCard::default()
    };
    let mut tx = state.db.begin().await?;
    card.insert_in_state(&mut tx, JobCardStatus::CheckedIn, CHECK_IN)
        .await?;
    if new.odometer_in > vehicle.odometer_km {
        Vehicle::query()
            .where_(Vehicle::ID.eq(vehicle.id))
            .update(
                &mut tx,
                &[
                    ("odometer_km", &new.odometer_in),
                    ("odometer_at", &Some(stamp)),
                    ("updated_at", &stamp),
                ],
            )
            .await?;
    }
    tx.commit().await?;
    if let Some(booking) = card.booking_id {
        booking_actions(state)
            .link_work_done(state, BookingId(booking), card.subject())
            .await?;
    }
    // A vehicle under a fleet contract: the job is billed in its monthly
    // invoice.
    crate::fleet_billing::attach_job(state, &card).await?;
    JobCard::find_or_404(&state.db, card.id).await
}

/// The intake of a job card that came from a booking: the complaint, the
/// odometer, the fuel level and the condition of the vehicle, recorded by the
/// advisor with the customer's acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intake {
    /// The customer's complaint.
    pub complaint: String,
    /// The odometer reading in kilometres.
    pub odometer_in: i64,
    /// The fuel level.
    pub fuel_level: Option<String>,
    /// The condition of the vehicle.
    pub condition: String,
    /// The customer who acknowledged it.
    pub acknowledged_by: Option<String>,
    /// The customer's signature.
    pub signature: Option<String>,
    /// Whether the damage is from an accident.
    pub accident_damage: bool,
    /// The police report number.
    pub police_report_no: Option<String>,
    /// The inspection fee agreed, in fils.
    pub inspection_fee: i64,
    /// The daily storage fee agreed, in fils.
    pub storage_fee_daily: i64,
}

/// Records the intake of a job card that is `checked_in` or `inspecting`
/// (it can be corrected until the estimate goes out).
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 for a job card past the
/// inspection, or fields as for [`check_in`].
pub async fn record_intake(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    intake: Intake,
) -> Result<JobCard> {
    if !by.has_permission(permissions::JOB_CARDS_ADVISE) {
        return Err(Error::Forbidden);
    }
    let card = JobCard::find_or_404(&state.db, id).await?;
    if !matches!(
        card.status,
        JobCardStatus::CheckedIn | JobCardStatus::Inspecting
    ) {
        return Err(refused(english("auto.jobs.problem.intake_closed", &[])));
    }
    if card.accident_damage && !intake.accident_damage {
        // The flag is cleared with a reason and the workshop manager's
        // approval (`AUT.ACL.20`), not by correcting the intake.
        return Err(refused(english("auto.police.problem.clear_by_approval", &[])));
    }
    let vehicle = Vehicle::find_or_404(&state.db, card.vehicle_id).await?;
    let complaint = intake.complaint.trim().to_owned();
    if complaint.is_empty() {
        return Err(refused(english("auto.jobs.problem.complaint", &[])));
    }
    let condition = intake.condition.trim().to_owned();
    if condition.is_empty() {
        return Err(refused(english("auto.jobs.problem.condition", &[])));
    }
    ensure_odometer(&vehicle, intake.odometer_in)?;
    let fuel = clean_fuel(&intake.fuel_level)?;
    if intake.inspection_fee < 0 || intake.storage_fee_daily < 0 {
        return Err(refused(english("auto.jobs.problem.fee", &[])));
    }
    let stamp = now();
    let mut tx = state.db.begin().await?;
    JobCard::query()
        .where_(JobCard::ID.eq(id))
        .update(
            &mut tx,
            &[
                ("complaint", &complaint),
                ("odometer_in", &intake.odometer_in),
                ("fuel_level", &fuel),
                ("intake_condition", &Some(condition)),
                ("intake_by", &Some(by.id)),
                ("intake_recorded_at", &Some(stamp)),
                ("intake_acknowledged_by", &optional(&intake.acknowledged_by)),
                ("intake_signature", &optional(&intake.signature)),
                ("accident_damage", &intake.accident_damage),
                ("police_report_no", &optional(&intake.police_report_no)),
                ("inspection_fee", &intake.inspection_fee),
                ("storage_fee_daily", &intake.storage_fee_daily),
                ("advisor_user_id", &Some(by.id)),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    if intake.odometer_in > vehicle.odometer_km {
        Vehicle::query()
            .where_(Vehicle::ID.eq(vehicle.id))
            .update(
                &mut tx,
                &[
                    ("odometer_km", &intake.odometer_in),
                    ("odometer_at", &Some(stamp)),
                    ("updated_at", &stamp),
                ],
            )
            .await?;
    }
    tx.commit().await?;
    JobCard::find_or_404(&state.db, id).await
}

/// Assigns the technician `user_id` to an open job card.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 for a closed job card or a user
/// who does not hold the technician role.
pub async fn assign_technician(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    user_id: i64,
) -> Result<JobCard> {
    if !by.has_permission(permissions::JOB_CARDS_ADVISE) {
        return Err(Error::Forbidden);
    }
    let card = JobCard::find_or_404(&state.db, id).await?;
    if card.status.is_closed() {
        return Err(refused(english("auto.jobs.problem.closed", &[])));
    }
    ensure_technician(&state.db, user_id).await?;
    JobCard::query()
        .where_(JobCard::ID.eq(id))
        .update(
            &state.db,
            &[
                ("technician_user_id", &Some(user_id)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    JobCard::find_or_404(&state.db, id).await
}

/// Whether `by` may work the job card `card`: an advisor always, a technician
/// only the job cards assigned to them (`AUT.ACL.08`).
pub fn may_work(by: &AuthUser, card: &JobCard) -> bool {
    by.has_permission(permissions::JOB_CARDS_ADVISE)
        || (by.has_permission(permissions::JOB_CARDS_WORK)
            && card.technician_user_id == Some(by.id))
}

/// The job card `id` for a user who works it.
async fn load_for_work(state: &AppState, by: &AuthUser, id: i64) -> Result<JobCard> {
    let card = JobCard::find_or_404(&state.db, id).await?;
    if !may_work(by, &card) {
        return Err(Error::Forbidden);
    }
    Ok(card)
}

/// The job card `id` for a user holding `permission`.
async fn load(state: &AppState, by: &AuthUser, id: i64, permission: &str) -> Result<JobCard> {
    if !by.has_permission(permission) {
        return Err(Error::Forbidden);
    }
    JobCard::find_or_404(&state.db, id).await
}

/// Runs the transition `name` of the user `by` on `card`.
async fn run(
    state: &AppState,
    by: &AuthUser,
    card: &JobCard,
    name: &str,
    reason: Option<&str>,
    input: JobInput,
) -> Result<JobCard> {
    let transition = card.transition(name).by(by).input(&input);
    let _outcome = match reason {
        Some(reason) => transition.reason(reason).run(state).await,
        None => transition.run(state).await,
    }
    .map_err(TransitionError::into_error)?;
    JobCard::find_or_404(&state.db, card.id).await
}

/// Runs the system transition `name` on `card`.
pub(crate) async fn run_system(
    state: &AppState,
    card: &JobCard,
    name: &str,
    reason: Option<&str>,
    input: JobInput,
) -> Result<JobCard> {
    let transition = card.transition(name).by_system().input(&input);
    let _outcome = match reason {
        Some(reason) => transition.reason(reason).run(state).await,
        None => transition.run(state).await,
    }
    .map_err(TransitionError::into_error)?;
    JobCard::find_or_404(&state.db, card.id).await
}

/// The input every run starts from.
fn input_for(by: &AuthUser) -> JobInput {
    JobInput {
        by: Some(by.id),
        on: Some(today()),
        ..JobInput::default()
    }
}

/// `start_inspection`: the technician starts diagnosing.
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 without an
/// assigned technician or an intake, or in another state.
pub async fn start_inspection(state: &AppState, by: &AuthUser, id: i64) -> Result<JobCard> {
    let card = load_for_work(state, by, id).await?;
    run(state, by, &card, START_INSPECTION, None, input_for(by)).await
}

/// Records what the technician found.
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 for a closed one.
pub async fn record_findings(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    findings: &str,
) -> Result<JobCard> {
    let card = load_for_work(state, by, id).await?;
    if card.status.is_closed() {
        return Err(refused(english("auto.jobs.problem.closed", &[])));
    }
    JobCard::query()
        .where_(JobCard::ID.eq(id))
        .update(
            &state.db,
            &[
                ("findings", &optional(&Some(findings.to_owned()))),
                ("updated_at", &now()),
            ],
        )
        .await?;
    JobCard::find_or_404(&state.db, id).await
}

/// A labour line to draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLabour {
    /// What is done.
    pub description: String,
    /// Hourly or flat-rate.
    pub pricing: LabourPricing,
    /// The estimated hours (hourly) or the flat hours, in thousandths.
    pub hours_milli: i64,
    /// The hourly rate or the flat price, in fils.
    pub rate: i64,
    /// The menu operation it comes from.
    pub operation_id: Option<i64>,
}

impl NewLabour {
    /// `hours_milli` of `description` at `rate` fils an hour.
    pub fn hourly(description: &str, hours_milli: i64, rate: i64) -> Self {
        NewLabour {
            description: description.to_owned(),
            pricing: LabourPricing::Hourly,
            hours_milli,
            rate,
            operation_id: None,
        }
    }

    /// `description` at the flat price `price` for `flat_hours_milli`.
    pub fn flat(description: &str, flat_hours_milli: i64, price: i64) -> Self {
        NewLabour {
            description: description.to_owned(),
            pricing: LabourPricing::FlatRate,
            hours_milli: flat_hours_milli,
            rate: price,
            operation_id: None,
        }
    }

    /// The line of a menu operation: a flat-rate one at its flat price and
    /// hours, an hourly one for `hours_milli` hours at its rate.
    pub fn from_operation(operation: &LabourOperation, hours_milli: i64) -> Self {
        NewLabour {
            description: operation.name_en.clone(),
            pricing: operation.pricing,
            hours_milli: match operation.pricing {
                LabourPricing::FlatRate => operation.hours_milli,
                LabourPricing::Hourly => hours_milli,
            },
            rate: operation.rate,
            operation_id: Some(operation.id),
        }
    }
}

/// A part line to draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPart {
    /// The code in the stock catalogue, when it is a stocked part.
    pub item_code: Option<String>,
    /// What the part is.
    pub description: String,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The price of one unit in fils, before VAT.
    pub unit_price: i64,
    /// New, used or renovated.
    pub condition: PartCondition,
    /// Ordered for this job.
    pub special_order: bool,
    /// Where it comes from ([`crate::supply::SOURCES`]).
    pub source: String,
}

impl NewPart {
    /// `quantity_milli` of `description` at `unit_price` fils, new.
    pub fn new(description: &str, quantity_milli: i64, unit_price: i64) -> Self {
        NewPart {
            item_code: None,
            description: description.to_owned(),
            quantity_milli,
            unit_price,
            condition: PartCondition::New,
            special_order: false,
            source: crate::supply::STOCK.to_owned(),
        }
    }

    /// Coming from `source` ([`crate::supply::SOURCES`]).
    #[must_use]
    pub fn from_source(mut self, source: &str) -> Self {
        self.source = source.trim().to_owned();
        self
    }

    /// Of the stock item `code`.
    #[must_use]
    pub fn item(mut self, code: &str) -> Self {
        self.item_code = Some(code.trim().to_uppercase()).filter(|c| !c.is_empty());
        self
    }

    /// In this condition.
    #[must_use]
    pub fn condition(mut self, condition: PartCondition) -> Self {
        self.condition = condition;
        self
    }

    /// Ordered for this job.
    #[must_use]
    pub fn special_order(mut self) -> Self {
        self.special_order = true;
        self
    }
}

/// The job card for drafting lines: open to lines, and the user may work it.
async fn card_for_lines(state: &AppState, by: &AuthUser, id: i64) -> Result<JobCard> {
    let card = load_for_work(state, by, id).await?;
    crate::job_stops::ensure_not_pending(&state.db, card.id).await?;
    if !card.status.takes_lines() {
        return Err(refused(english("auto.jobs.problem.no_lines_now", &[])));
    }
    Ok(card)
}

/// The next line number of the job card.
async fn next_line_no(db: &Db, job_card_id: i64) -> Result<i64> {
    let estimate = Estimate::load(db, job_card_id).await?;
    let last = estimate
        .labour
        .iter()
        .map(|l| l.line_no)
        .chain(estimate.parts.iter().map(|l| l.line_no))
        .max()
        .unwrap_or(0);
    Ok(last + 1)
}

/// Drafts a labour line on the estimate. A line drafted after the first
/// approval is added work: it goes to the customer in a new round.
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 for a job card that
/// takes no lines now, a blank description, hours or a rate that are not above
/// zero.
pub async fn add_labour(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    new: NewLabour,
) -> Result<LabourLine> {
    let card = card_for_lines(state, by, id).await?;
    let description = new.description.trim().to_owned();
    if description.is_empty() {
        return Err(refused(english("auto.jobs.problem.line_description", &[])));
    }
    if new.hours_milli <= 0 || new.rate < 0 {
        return Err(refused(english("auto.jobs.problem.line_labour", &[])));
    }
    let mut line = LabourLine {
        job_card_id: card.id,
        operation_id: new.operation_id,
        description,
        pricing: new.pricing,
        hours_milli: new.hours_milli,
        rate: new.rate,
        approval: LineStatus::Draft,
        work: WorkStatus::NotDone,
        line_no: next_line_no(&state.db, card.id).await?,
        created_by: Some(by.id),
        ..LabourLine::default()
    };
    line.save(&state.db).await?;
    Ok(line)
}

/// Drafts a labour line from the menu operation `code`; an hourly operation
/// takes the hours given.
///
/// # Errors
///
/// As for [`add_labour`], and a 422 for a code that is not on the menu.
pub async fn add_operation(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    code: &str,
    hours_milli: i64,
) -> Result<LabourLine> {
    let operation = LabourOperation::by_code(&state.db, code)
        .await?
        .filter(|op| op.active)
        .ok_or_else(|| refused(english("auto.jobs.problem.no_operation", &[])))?;
    add_labour(
        state,
        by,
        id,
        NewLabour::from_operation(&operation, hours_milli),
    )
    .await
}

/// Drafts a part line on the estimate.
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 for a job card that
/// takes no lines now, a blank description, a quantity that is not above zero
/// or a negative price.
pub async fn add_part(state: &AppState, by: &AuthUser, id: i64, new: NewPart) -> Result<PartLine> {
    let card = card_for_lines(state, by, id).await?;
    let description = new.description.trim().to_owned();
    if description.is_empty() {
        return Err(refused(english("auto.jobs.problem.line_description", &[])));
    }
    if new.quantity_milli <= 0 || new.unit_price < 0 {
        return Err(refused(english("auto.jobs.problem.line_part", &[])));
    }
    if !crate::supply::SOURCES.contains(&crate::supply::clean_source(&new.source).as_str()) {
        return Err(refused(english("auto.supply.problem.source", &[])));
    }
    let mut line = PartLine {
        job_card_id: card.id,
        item_code: new.item_code.clone(),
        description,
        quantity_milli: new.quantity_milli,
        unit_price: new.unit_price,
        part_condition: new.condition,
        special_order: new.special_order,
        source: crate::supply::clean_source(&new.source),
        approval: LineStatus::Draft,
        line_no: next_line_no(&state.db, card.id).await?,
        created_by: Some(by.id),
        ..PartLine::default()
    };
    line.save(&state.db).await?;
    Ok(line)
}

/// Takes a drafted labour line off the estimate. A line already sent to the
/// customer stays: the round is answered first.
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 for a line that is
/// not a draft.
pub async fn remove_labour(state: &AppState, by: &AuthUser, line_id: i64) -> Result<()> {
    let line = LabourLine::find_or_404(&state.db, line_id).await?;
    load_for_work(state, by, line.job_card_id).await?;
    if line.approval != LineStatus::Draft {
        return Err(refused(english("auto.jobs.problem.line_sent", &[])));
    }
    LabourLine::query()
        .where_(LabourLine::ID.eq(line.id))
        .update(
            &state.db,
            &[("approval", &LineStatus::Removed), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// Takes a drafted part line off the estimate.
///
/// # Errors
///
/// As for [`remove_labour`].
pub async fn remove_part(state: &AppState, by: &AuthUser, line_id: i64) -> Result<()> {
    let line = PartLine::find_or_404(&state.db, line_id).await?;
    load_for_work(state, by, line.job_card_id).await?;
    if line.approval != LineStatus::Draft {
        return Err(refused(english("auto.jobs.problem.line_sent", &[])));
    }
    PartLine::query()
        .where_(PartLine::ID.eq(line.id))
        .update(
            &state.db,
            &[("approval", &LineStatus::Removed), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// What the advisor promises with the estimate (the customer's consent to the
/// time and the warranty).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Promise {
    /// The day the vehicle is promised for.
    pub promised_for: Option<NaiveDate>,
    /// The warranty in days; at least 15.
    pub warranty_days: Option<i64>,
}

/// `send_estimate`: the drafted lines go to the customer as a round to
/// approve on the link (`DOC.SIG.01`), from `inspecting` for the estimate and
/// from `in_progress` for added work.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 for a warranty below 15 days, a
/// promised day that has passed, no lines to send, a line without a price, a
/// missing intake, or another state.
pub async fn send_estimate(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    promise: Promise,
) -> Result<JobCard> {
    let card = load(state, by, id, permissions::JOB_CARDS_ADVISE).await?;
    if let Some(days) = promise.warranty_days
        && days < MIN_WARRANTY_DAYS
    {
        return Err(refused(english(
            "auto.jobs.problem.warranty",
            &[("days", &MIN_WARRANTY_DAYS)],
        )));
    }
    if let Some(day) = promise.promised_for
        && day < today()
    {
        return Err(refused(english("auto.jobs.problem.promised_past", &[])));
    }
    let estimate = Estimate::load(&state.db, card.id).await?;
    let mut input = input_for(by);
    input.priced = Some(estimate.drafts() > 0 && estimate.drafts_priced());
    input.promised_for = promise.promised_for;
    input.warranty_days = promise.warranty_days;
    run(state, by, &card, SEND_ESTIMATE, None, input).await
}

/// The estimate round the customer has not answered.
///
/// # Errors
///
/// The database's errors.
pub async fn pending_round(db: &Db, job_card_id: i64) -> Result<Option<EstimateApproval>> {
    EstimateApproval::pending_of(db, job_card_id).await
}

/// The advisor records the customer's approval, given in person or by phone:
/// `method` is `signature` (the customer signed on the desk) or `recorded`.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 for a blank name, a way that is
/// not one of the two, no round waiting for an answer, or another state.
pub async fn approve_recorded(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    name: &str,
    method: &str,
    signature: Option<&str>,
) -> Result<JobCard> {
    let card = load(state, by, id, permissions::JOB_CARDS_ADVISE).await?;
    let name = name.trim();
    if name.is_empty() {
        return Err(refused(english("auto.jobs.problem.approver", &[])));
    }
    if !METHODS.contains(&method) || method == "link" {
        return Err(refused(english("auto.jobs.problem.method", &[])));
    }
    let round = pending_round(&state.db, card.id)
        .await?
        .ok_or_else(|| refused(english("auto.jobs.problem.nothing_to_approve", &[])))?;
    let mut input = input_for(by);
    input.approval_id = Some(round.id);
    input.decided_by_name = Some(name.to_owned());
    input.method = Some(method.to_owned());
    input.signature = signature
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    input.police =
        Some(police_verdict(state, GUARDED_JOB_CARD_APPROVE, card.id, Some(by.id)).await?);
    run(state, by, &card, APPROVE, None, input).await
}

/// The round behind a customer's link, still waiting for an answer, and its
/// job card.
async fn round_of_link(state: &AppState, token: &str) -> Result<(EstimateApproval, JobCard)> {
    let round = EstimateApproval::by_token(&state.db, token)
        .await?
        .ok_or(Error::NotFound)?;
    let card = JobCard::find_or_404(&state.db, round.job_card_id).await?;
    if !round.is_pending() || card.status != JobCardStatus::AwaitingApproval {
        return Err(refused(english("auto.jobs.problem.link_answered", &[])));
    }
    Ok((round, card))
}

/// The customer approves on the link (`DOC.SIG.01`): the name, the signature
/// and the ticked consent to the cost, the time and the warranty.
///
/// # Errors
///
/// `NotFound` for a token nobody knows; a 422 for a round already answered, a
/// job card no longer waiting, a blank name or a consent not ticked.
pub async fn approve_by_link(
    state: &AppState,
    token: &str,
    name: &str,
    signature: Option<&str>,
    consent: bool,
) -> Result<JobCard> {
    let (round, card) = round_of_link(state, token).await?;
    let name = name.trim();
    if name.is_empty() {
        return Err(refused(english("auto.jobs.problem.approver", &[])));
    }
    if !consent {
        return Err(refused(english("auto.jobs.problem.consent", &[])));
    }
    let police = police_verdict(state, GUARDED_JOB_CARD_APPROVE, card.id, None).await?;
    let input = JobInput {
        on: Some(today()),
        approval_id: Some(round.id),
        decided_by_name: Some(name.to_owned()),
        method: Some("link".to_owned()),
        signature: signature
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        police: Some(police),
        ..JobInput::default()
    };
    run_system(state, &card, APPROVE_SYSTEM, None, input).await
}

/// The customer declines on the link: the whole estimate when nothing was
/// approved yet (the job card is `declined`), the added work otherwise (the
/// job goes on without it).
///
/// # Errors
///
/// As for [`approve_by_link`], without the consent.
pub async fn decline_by_link(
    state: &AppState,
    token: &str,
    name: &str,
    reason: &str,
) -> Result<JobCard> {
    let (round, card) = round_of_link(state, token).await?;
    let name = name.trim();
    if name.is_empty() {
        return Err(refused(english("auto.jobs.problem.approver", &[])));
    }
    let reason = if reason.trim().is_empty() {
        english("auto.jobs.customer_declined", &[])
    } else {
        reason.trim().to_owned()
    };
    let input = JobInput {
        on: Some(today()),
        approval_id: Some(round.id),
        decided_by_name: Some(name.to_owned()),
        method: Some("link".to_owned()),
        code: Some(crate::stop::ESTIMATE_DECLINED.to_owned()),
        ..JobInput::default()
    };
    let transition = if card.work_approved() {
        DECLINE_ADDED_SYSTEM
    } else {
        DECLINE_SYSTEM
    };
    run_system(state, &card, transition, Some(&reason), input).await
}

/// `decline`: the customer declines the estimate or leaves; the advisor
/// records it with a reason. From `awaiting_approval` after something was
/// approved it declines the added work only and the job goes on.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 without a reason or in another
/// state.
pub async fn decline(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<JobCard> {
    let card = load(state, by, id, permissions::JOB_CARDS_ADVISE).await?;
    let mut input = input_for(by);
    let round = pending_round(&state.db, card.id).await?;
    input.approval_id = round.map(|r| r.id);
    let estimate = Estimate::load(&state.db, card.id).await?;
    input.lines_recorded = Some(i64::try_from(estimate.recorded()).unwrap_or(i64::MAX));
    input.code = Some(crate::stop::CUSTOMER_LEFT.to_owned());
    let transition = if card.status == JobCardStatus::AwaitingApproval && card.work_approved() {
        DECLINE_ADDED
    } else {
        DECLINE
    };
    run(state, by, &card, transition, Some(reason), input).await
}

/// `void`: the job card was opened in error.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 without a reason or in another
/// state.
pub async fn void(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<JobCard> {
    let card = load(state, by, id, permissions::JOB_CARDS_ADVISE).await?;
    // The kernel's void: the reverse handler of the job card refuses a card
    // with lines or an open claim, and `RecordVoided` cancels the booking.
    match wl_kernel::voids::void(state, by, &card.subject(), reason)
        .await
        .map_err(TransitionError::into_error)?
    {
        wl_kernel::voids::VoidOutcome::Voided(_) => {}
        wl_kernel::voids::VoidOutcome::Pending(_) => {
            return Err(refused(english("auto.stop.problem.approval", &[])));
        }
    }
    JobCard::find_or_404(&state.db, id).await
}

/// `wait_parts`: the job waits for parts.
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 in another state.
pub async fn wait_parts(state: &AppState, by: &AuthUser, id: i64) -> Result<JobCard> {
    let card = load_for_work(state, by, id).await?;
    let estimate = Estimate::load(&state.db, card.id).await?;
    // The parts requested: approved lines whose part is not all issued.
    let wanted: Vec<&PartLine> = estimate
        .parts
        .iter()
        .filter(|line| line.approval == LineStatus::Approved && line.outstanding_milli() > 0)
        .collect();
    let sources: Vec<String> = wanted.iter().map(|line| line.source.clone()).collect();
    let (legal_date, source) = crate::supply::legal_date(&state.db, today(), &sources).await?;
    let mut input = input_for(by);
    input.parts_requested = Some(i64::try_from(wanted.len()).unwrap_or(i64::MAX));
    input.legal_date = Some(legal_date);
    input.wait_source = Some(source);
    run(state, by, &card, WAIT_PARTS, None, input).await
}

/// Records how much of the approved labour line `line_id` was done. `Done`
/// sets the hours to the estimate's (the flat hours for a flat-rate line);
/// `PartDone` needs the hours worked.
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 for a line the
/// customer has not approved, a job card not `in_progress`, or hours that are
/// not above zero for a part done.
pub async fn record_work(
    state: &AppState,
    by: &AuthUser,
    line_id: i64,
    work: WorkStatus,
    hours_done_milli: Option<i64>,
) -> Result<LabourLine> {
    let line = LabourLine::find_or_404(&state.db, line_id).await?;
    let card = load_for_work(state, by, line.job_card_id).await?;
    crate::job_stops::ensure_not_pending(&state.db, card.id).await?;
    if line.approval != LineStatus::Approved || card.status != JobCardStatus::InProgress {
        return Err(refused(english("auto.jobs.problem.work_not_now", &[])));
    }
    let hours = match work {
        WorkStatus::NotDone => 0,
        WorkStatus::Done => line.hours_milli.max(hours_done_milli.unwrap_or(0)),
        WorkStatus::PartDone => {
            let hours = hours_done_milli.unwrap_or(0);
            if hours <= 0 {
                return Err(refused(english("auto.jobs.problem.hours_done", &[])));
            }
            hours
        }
    };
    let finished = (work == WorkStatus::Done).then(now);
    LabourLine::query()
        .where_(LabourLine::ID.eq(line.id))
        .update(
            &state.db,
            &[
                ("work", &work),
                ("hours_done_milli", &hours),
                ("done_at", &finished),
                ("done_by", &Some(by.id)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    LabourLine::find_or_404(&state.db, line.id).await
}

/// `finish_work`: every approved labour line is done.
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 while a labour line
/// is not done, or in another state.
pub async fn finish_work(state: &AppState, by: &AuthUser, id: i64) -> Result<JobCard> {
    let card = load_for_work(state, by, id).await?;
    let estimate = Estimate::load(&state.db, card.id).await?;
    let mut input = input_for(by);
    input.labour_open = Some(estimate.labour_open());
    run(state, by, &card, FINISH_WORK, None, input).await
}

/// Ticks the checks of the QC checklist: the keys in `done` are done, the
/// others are not.
///
/// # Errors
///
/// A 403 without `job_cards.qc`; a 422 for a job card not in `quality_check`
/// or a key that is not a check.
pub async fn save_qc(state: &AppState, by: &AuthUser, id: i64, done: &[String]) -> Result<JobCard> {
    let card = load(state, by, id, permissions::JOB_CARDS_QC).await?;
    if card.status != JobCardStatus::QualityCheck {
        return Err(refused(english("auto.jobs.problem.qc_not_now", &[])));
    }
    if done.iter().any(|key| !QC_KEYS.contains(&key.as_str())) {
        return Err(refused(english("auto.jobs.problem.qc_key", &[])));
    }
    let checks: Vec<QcCheck> = QC_KEYS
        .iter()
        .map(|key| QcCheck {
            key: (*key).to_owned(),
            done: done.iter().any(|d| d == key),
        })
        .collect();
    JobCard::query()
        .where_(JobCard::ID.eq(id))
        .update(
            &state.db,
            &[
                ("qc_checks", &renox::db::Json(checks)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    JobCard::find_or_404(&state.db, id).await
}

/// `pass_qc`: the checklist is complete; the vehicle is ready.
///
/// # Errors
///
/// A 403 without `job_cards.qc`; a 422 while a check is not done.
pub async fn pass_qc(state: &AppState, by: &AuthUser, id: i64) -> Result<JobCard> {
    let card = load(state, by, id, permissions::JOB_CARDS_QC).await?;
    let mut input = input_for(by);
    input.test_drives_open = Some(crate::test_drives::open_count(&state.db, card.id).await?);
    let ready = run(state, by, &card, PASS_QC, None, input).await?;
    // The insurance claim the job card fills is ready for billing.
    crate::claims::on_job_ready(state, ready.id).await?;
    JobCard::find_or_404(&state.db, ready.id).await
}

/// `fail_qc`: the QC found something to put right; `findings` is the reason.
///
/// # Errors
///
/// A 403 without `job_cards.qc`; a 422 without findings.
pub async fn fail_qc(state: &AppState, by: &AuthUser, id: i64, findings: &str) -> Result<JobCard> {
    let card = load(state, by, id, permissions::JOB_CARDS_QC).await?;
    run(state, by, &card, FAIL_QC, Some(findings), input_for(by)).await
}

/// What the advisor records when the customer collects the vehicle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delivery {
    /// Who collected the vehicle.
    pub delivered_to: String,
    /// The odometer reading at delivery.
    pub odometer_out: Option<i64>,
    /// The date the next service falls due.
    pub next_service_date: Option<NaiveDate>,
    /// The odometer reading the next service falls due at.
    pub next_service_odometer: Option<i64>,
}

/// `deliver`: the customer collects the vehicle. The invoice is raised and the
/// booking completed after the commit ([`crate::billing`]); the next service
/// is recorded on the vehicle.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 for a blank recipient, a line still
/// waiting for the customer's answer, an incomplete QC, an odometer below the
/// check-in's, or another state.
pub async fn deliver(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    delivery: Delivery,
) -> Result<JobCard> {
    let card = load(state, by, id, permissions::JOB_CARDS_ADVISE).await?;
    let recipient = delivery.delivered_to.trim().to_owned();
    if recipient.is_empty() {
        return Err(refused(english("auto.jobs.problem.recipient", &[])));
    }
    if let Some(km) = delivery.odometer_out
        && km < card.odometer_in
    {
        return Err(refused(english(
            "auto.jobs.problem.odometer_out",
            &[("km", &card.odometer_in)],
        )));
    }
    let estimate = Estimate::load(&state.db, card.id).await?;
    let mut input = input_for(by);
    input.added_open = Some(i64::try_from(estimate.unanswered()).unwrap_or(i64::MAX));
    input.substitute_open = Some(crate::substitute::is_open(&state.db, card.id).await?);
    input.delivered_to = Some(recipient);
    input.odometer_out = delivery.odometer_out;
    input.next_service_date = delivery.next_service_date;
    input.next_service_odometer = delivery.next_service_odometer;
    let delivered = run(state, by, &card, DELIVER, None, input).await?;
    crate::billing::after_deliver(state, &delivered).await?;
    // The service plan that booked the visit moves on.
    crate::fleet_services::on_delivered(state, &delivered).await?;
    JobCard::find_or_404(&state.db, id).await
}

/// The job cards a user sees: every job card for a holder of
/// `job_cards.view`, only those assigned to them for a technician
/// (`AUT.ACL.08`); newest first.
///
/// # Errors
///
/// The database's errors.
pub async fn visible_to(state: &AppState, user: &AuthUser) -> Result<Vec<JobCard>> {
    let mut query = JobCard::query().order_by_desc(JobCard::ID);
    if !user.has_permission(permissions::JOB_CARDS_VIEW) {
        query = query.where_(JobCard::TECHNICIAN_USER_ID.eq(Some(user.id)));
    }
    query.get(&state.db).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fuel_level_is_one_of_the_five() {
        assert_eq!(
            clean_fuel(&Some(" Half ".to_owned())).unwrap(),
            Some("half".to_owned())
        );
        assert_eq!(clean_fuel(&None).unwrap(), None);
        assert_eq!(clean_fuel(&Some(" ".to_owned())).unwrap(), None);
        assert!(clean_fuel(&Some("overflowing".to_owned())).is_err());
    }

    #[test]
    fn a_new_job_card_is_built_with_its_options() {
        let new = NewJobCard::new(4, " Brake noise ")
            .odometer(52_000)
            .fuel("half")
            .condition("Scratch on the left door")
            .accident("")
            .inspection_fee(15_000)
            .technician(9)
            .booking(3);
        assert!(new.accident_damage);
        assert_eq!(new.police_report_no, None);
        assert_eq!(new.inspection_fee, 15_000);
        assert_eq!(new.technician_user_id, Some(9));
        assert_eq!(new.booking_id, Some(3));
    }

    #[test]
    fn a_menu_operation_makes_its_line() {
        let flat = LabourOperation {
            id: 5,
            name_en: "Oil change".to_owned(),
            pricing: LabourPricing::FlatRate,
            hours_milli: 1_000,
            rate: 12_000,
            ..LabourOperation::default()
        };
        let line = NewLabour::from_operation(&flat, 9_000);
        assert_eq!(line.hours_milli, 1_000, "a flat operation keeps its hours");
        assert_eq!(line.operation_id, Some(5));
        let hourly = LabourOperation {
            pricing: LabourPricing::Hourly,
            rate: 15_000,
            ..LabourOperation::default()
        };
        assert_eq!(NewLabour::from_operation(&hourly, 2_500).hours_milli, 2_500);
    }
}
