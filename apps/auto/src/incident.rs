//! A vehicle incident (08 §4.4, `AUT.ACL.22`, BL-AUT-06): damage while the
//! vehicle is in the workshop's care or on a test drive. It links the ticket
//! of the category `vehicle_incident` to the job card and keeps the branch
//! manager's liability decision.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of an incident.
pub const INCIDENT_SUBJECT_TYPE: &str = "auto.incident";
/// The ticket category of an incident.
pub const INCIDENT_CATEGORY: &str = "vehicle_incident";
/// Liability: the workshop caused the damage.
pub const WORKSHOP: &str = "workshop";
/// Liability: the damage was there before (the check-in) or is the customer's.
pub const NOT_WORKSHOP: &str = "not_workshop";

/// A vehicle incident.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_incidents", index(job_card_id))]
pub struct Incident {
    /// The primary key.
    pub id: i64,
    /// The job card the vehicle is in the care of.
    #[model(references = "auto_job_cards")]
    pub job_card_id: i64,
    /// The vehicle.
    pub vehicle_id: i64,
    /// The ticket (wl-ticketing), when ticketing is on.
    pub ticket_id: Option<i64>,
    /// The ticket's number.
    pub ticket_number: Option<String>,
    /// What happened.
    pub description: String,
    /// The damage came from a road accident (on a test drive or while moving
    /// the vehicle).
    pub road_accident: bool,
    /// The user who reported it.
    pub reported_by: i64,
    /// [`WORKSHOP`] or [`NOT_WORKSHOP`], once decided.
    pub liability: Option<String>,
    /// Why.
    pub liability_reason: Option<String>,
    /// The branch manager (or the owner) who decided.
    pub decided_by: Option<i64>,
    /// When it was decided.
    pub decided_at: Option<DateTime>,
    /// The internal job card that repairs the damage, for [`WORKSHOP`].
    pub internal_job_card_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Incident {
    /// The number people read: `INC-00003`.
    pub fn number(&self) -> String {
        format!("INC-{:05}", self.id)
    }

    /// The incident as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(INCIDENT_SUBJECT_TYPE, self.id)
    }

    /// Whether the liability is decided.
    pub fn is_decided(&self) -> bool {
        self.liability.is_some()
    }

    /// The incidents of job card `job_card_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_job_card(db: &Db, job_card_id: i64) -> Result<Vec<Incident>> {
        Incident::query()
            .where_(Incident::JOB_CARD_ID.eq(job_card_id))
            .order_by(Incident::ID)
            .get(db)
            .await
    }

    /// The incident behind ticket `ticket_id`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_ticket(db: &Db, ticket_id: i64) -> Result<Option<Incident>> {
        Incident::query()
            .where_(Incident::TICKET_ID.eq(Some(ticket_id)))
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_incident_is_numbered_and_decided_once_it_has_a_liability() {
        let mut incident = Incident {
            id: 3,
            ..Incident::default()
        };
        assert_eq!(incident.number(), "INC-00003");
        assert!(!incident.is_decided());
        incident.liability = Some(WORKSHOP.to_owned());
        assert!(incident.is_decided());
        assert_eq!(incident.subject().to_string(), "auto.incident:3");
    }
}
