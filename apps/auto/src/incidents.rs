//! Vehicle incidents and the liability decision (08 §4.4, `AUT.ACL.22`,
//! BL-AUT-06).
//!
//! Damage while the vehicle is in the workshop's care or on a test drive is
//! reported on the job card. It opens a ticket of the category
//! `vehicle_incident` (wl-ticketing, through the kernel's task creator) linked
//! to the job card, with the *road accident* answer. Only the branch manager
//! (a holder of `job_cards.liability`; the owner when the branch manager was
//! involved) decides the liability, and not an involved person: the reporter,
//! the job card's advisor and technician, and every driver of a test drive of
//! the job card.
//!
//! - **Workshop:** an internal job card is opened for the same vehicle (the
//!   workshop pays; nothing on it reaches a customer invoice), flagged accident
//!   damage when the incident was a road accident, so that its `approve` needs
//!   the police report ([`crate::documents`]).
//! - **Not workshop:** no internal job card; the ticket is resolved with the
//!   reason (pre-existing per the check-in photos, or the customer's); the
//!   advisor informs the customer and may add the repair to the customer's job
//!   card as a normal estimate.
//!
//! Decisions: the decision is taken here, on the job card, not on the ticket:
//! the ticket is resolved by this service as its source (`resolve_from_source`),
//! and a user's own `resolve` of a ticket of the category is refused by the
//! kernel's [`ResolveRule`] registered in [`register`]. The check-in photos
//! next to the incident photos are deferred (no photo attachments yet).

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::Subject;
use wl_kernel::tickets::{
    OpenTicket, ResolveRule, TicketFuture, TicketId, register_resolve_rule, task_creator,
};

use crate::db::refused;
use crate::incident::{INCIDENT_CATEGORY, Incident, NOT_WORKSHOP, WORKSHOP};
use crate::job_card::{JobCard, JobCardStatus};
use crate::job_cards::may_work;
use crate::permissions;
use crate::test_drive::TestDrive;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// What is reported.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewIncident {
    /// What happened.
    pub description: String,
    /// Whether the damage came from a road accident.
    pub road_accident: bool,
}

impl NewIncident {
    /// An incident described by `description`.
    pub fn new(description: &str) -> Self {
        NewIncident {
            description: description.trim().to_owned(),
            road_accident: false,
        }
    }

    /// The damage came from a road accident.
    #[must_use]
    pub fn road_accident(mut self) -> Self {
        self.road_accident = true;
        self
    }
}

/// Reports an incident on job card `job_card_id` and opens its ticket.
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 in words for a
/// closed job card or a blank description.
pub async fn report(
    state: &AppState,
    by: &AuthUser,
    job_card_id: i64,
    new: NewIncident,
) -> Result<Incident> {
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    if !may_work(by, &card) {
        return Err(Error::Forbidden);
    }
    if !card.status.is_open() {
        return Err(refused(english("auto.incidents.problem.closed", &[])));
    }
    let description = new.description.trim().to_owned();
    if description.is_empty() {
        return Err(refused(english("auto.incidents.problem.description", &[])));
    }
    let mut incident = Incident {
        job_card_id: card.id,
        vehicle_id: card.vehicle_id,
        description: description.clone(),
        road_accident: new.road_accident,
        reported_by: by.id,
        ..Incident::default()
    };
    incident.save(&state.db).await?;
    let ticket = {
        let title = english(
            "auto.incidents.ticket_title",
            &[("job", &card.number()), ("incident", &incident.number())],
        );
        let road = english(
            if new.road_accident {
                "auto.incidents.road_yes"
            } else {
                "auto.incidents.road_no"
            },
            &[],
        );
        let body = english(
            "auto.incidents.ticket_body",
            &[("description", &description), ("road", &road)],
        );
        OpenTicket::new(card.subject(), INCIDENT_CATEGORY, &title)
            .description(&body)
            .requester_user(by.id)
            .source(incident.subject())
            .source_key(&format!("auto.incident:{}", incident.id))
    };
    if let Some(opened) = task_creator(state).open(state, ticket).await? {
        Incident::query()
            .where_(Incident::ID.eq(incident.id))
            .update(
                &state.db,
                &[
                    ("ticket_id", &Some(opened.id.0)),
                    ("ticket_number", &Some(opened.number.clone())),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    Incident::find_or_404(&state.db, incident.id).await
}

/// The users involved in `incident`: its reporter, the advisor and the
/// technician of the job card, and the drivers of its test drives.
///
/// # Errors
///
/// The database's errors.
pub async fn involved(db: &Db, incident: &Incident) -> Result<Vec<i64>> {
    let mut users = vec![incident.reported_by];
    if let Some(card) = JobCard::find(db, incident.job_card_id).await? {
        users.extend(card.advisor_user_id);
        users.extend(card.technician_user_id);
    }
    for drive in TestDrive::of_job_card(db, incident.job_card_id).await? {
        users.push(drive.driver_user_id);
    }
    users.sort_unstable();
    users.dedup();
    Ok(users)
}

/// The branch manager decides the liability of incident `id`.
///
/// # Errors
///
/// A 403 without `job_cards.liability`; a 422 in words for a liability that is
/// neither *workshop* nor *not workshop*, a blank reason, an incident decided
/// already, or an involved person deciding.
pub async fn decide(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    liability: &str,
    reason: &str,
) -> Result<Incident> {
    if !by.has_permission(permissions::JOB_CARDS_LIABILITY) {
        return Err(Error::Forbidden);
    }
    let incident = Incident::find_or_404(&state.db, id).await?;
    if incident.is_decided() {
        return Err(refused(english("auto.incidents.problem.decided", &[])));
    }
    if liability != WORKSHOP && liability != NOT_WORKSHOP {
        return Err(refused(english("auto.incidents.problem.liability", &[])));
    }
    let reason = reason.trim().to_owned();
    if reason.is_empty() {
        return Err(refused(english("auto.incidents.problem.reason", &[])));
    }
    if involved(&state.db, &incident).await?.contains(&by.id) {
        return Err(refused(english("auto.incidents.problem.involved", &[])));
    }
    let card = JobCard::find_or_404(&state.db, incident.job_card_id).await?;
    let vehicle = Vehicle::find_or_404(&state.db, incident.vehicle_id).await?;
    let stamp = now();
    let mut tx = state.db.begin().await?;
    let mut internal_id = None;
    if liability == WORKSHOP {
        let mut internal = JobCard {
            vehicle_id: vehicle.id,
            intake_plate: Some(crate::search::plate_key(&vehicle)),
            customer_party_id: card.customer_party_id,
            workshop_type: card.workshop_type.clone(),
            workshop_id: card.workshop_id,
            advisor_user_id: Some(by.id),
            complaint: english(
                "auto.incidents.internal_complaint",
                &[("description", &incident.description)],
            ),
            odometer_in: vehicle.odometer_km.max(card.odometer_in),
            intake_condition: Some(incident.description.clone()),
            intake_by: Some(by.id),
            intake_recorded_at: Some(stamp),
            accident_damage: incident.road_accident,
            internal: true,
            incident_id: Some(incident.id),
            created_by: Some(by.id),
            ..JobCard::default()
        };
        internal
            .insert_in_state(&mut tx, JobCardStatus::CheckedIn, "incident")
            .await?;
        wl_kernel::search::index_in(&mut tx, &internal.subject()).await?;
        internal_id = Some(internal.id);
    }
    Incident::query()
        .where_(Incident::ID.eq(incident.id))
        .update(
            &mut tx,
            &[
                ("liability", &Some(liability.to_owned())),
                ("liability_reason", &Some(reason.clone())),
                ("decided_by", &Some(by.id)),
                ("decided_at", &Some(stamp)),
                ("internal_job_card_id", &internal_id),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    tx.commit().await?;
    if let Some(ticket) = incident.ticket_id {
        let note = english(
            if liability == WORKSHOP {
                "auto.incidents.resolution_workshop"
            } else {
                "auto.incidents.resolution_not_workshop"
            },
            &[("reason", &reason)],
        );
        task_creator(state)
            .resolve_from_source(state, TicketId(ticket), &note)
            .await?;
    }
    Incident::find_or_404(&state.db, id).await
}

/// The kernel's rule for the category `vehicle_incident`: nobody resolves the
/// ticket from the ticket's own page; the liability is decided on the job card.
struct OnTheJobCard;

impl ResolveRule for OnTheJobCard {
    fn check<'a>(
        &'a self,
        _state: &'a AppState,
        _ticket: TicketId,
        _subject: &'a Subject,
        _user: i64,
    ) -> TicketFuture<'a, std::result::Result<(), String>> {
        Box::pin(async { Ok(Err(english("auto.incidents.problem.on_the_job_card", &[]))) })
    }
}

/// Registers the rule of the category.
pub(crate) fn register() {
    register_resolve_rule(INCIDENT_CATEGORY, OnTheJobCard);
}
