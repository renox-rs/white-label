//! Resources, service types and availability (wl-scheduling §3, `SCH.RES.01`).
//!
//! A [`Resource`] is something or someone that can be booked. A
//! [`ServiceType`] says what is booked and which resources deliver it
//! ([`ServiceResource`]). The availability of a resource is its
//! [`AvailabilityRule`]s, less its [`AvailabilityBlock`]s, less its bookings
//! ([`crate::slots`]).
//!
//! Decision: working hours are stored as minutes after local midnight
//! (Asia/Dubai, no daylight saving), so a rule has no time-zone arithmetic.
//! A rule without a service type applies to every service type of the
//! resource.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// What kind of thing a resource is (wl-scheduling §3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ResourceType {
    /// A practitioner, tutor or technician.
    #[default]
    Person,
    /// A room.
    Room,
    /// A workshop bay.
    Bay,
    /// A piece of equipment.
    Equipment,
    /// An online meeting.
    Online,
}

/// Something or someone that can be booked.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_scheduling_resources", index(active))]
pub struct Resource {
    /// The primary key.
    pub id: i64,
    /// The name shown to staff and parties.
    pub name: String,
    /// Person, room, bay, equipment or online.
    pub resource_type: ResourceType,
    /// The type of the location (a branch, workshop or campus), if any.
    pub location_type: Option<String>,
    /// The id of the location.
    pub location_id: Option<i64>,
    /// The employee (`wl-hr`) behind a person.
    pub employee_id: Option<i64>,
    /// The asset (`wl-inventory`) behind a bay or a piece of equipment.
    pub asset_id: Option<i64>,
    /// The role a person works in here (`practitioner`), asked of a
    /// role-bound qualification such as a licence.
    pub role: Option<String>,
    /// Bookings that may overlap: 1, or N for a group slot.
    pub capacity: i64,
    /// Whether it can be booked.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The subject type of a resource.
pub const RESOURCE_SUBJECT_TYPE: &str = "scheduling.resource";

impl Resource {
    /// An active resource named `name` of `resource_type`, with capacity 1.
    pub fn new(name: &str, resource_type: ResourceType) -> Self {
        Resource {
            name: name.to_owned(),
            resource_type,
            capacity: 1,
            active: true,
            ..Resource::default()
        }
    }

    /// With this location (a branch, workshop or campus).
    pub fn at(mut self, location: &Subject) -> Self {
        self.location_type = Some(location.subject_type.clone());
        self.location_id = Some(location.subject_id);
        self
    }

    /// With this capacity (seats of a group slot).
    pub fn capacity(mut self, capacity: i64) -> Self {
        self.capacity = capacity.max(1);
        self
    }

    /// For this employee.
    pub fn employee(mut self, employee: i64) -> Self {
        self.employee_id = Some(employee);
        self
    }

    /// Working in this role (`practitioner`).
    pub fn role(mut self, role: &str) -> Self {
        self.role = Some(role.to_owned());
        self
    }

    /// The resource as a [`Subject`]: `scheduling.resource:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(RESOURCE_SUBJECT_TYPE, self.id)
    }

    /// The location, if the resource has one.
    pub fn location(&self) -> Option<Subject> {
        match (&self.location_type, self.location_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }
}

/// What is booked.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_scheduling_service_types")]
pub struct ServiceType {
    /// The primary key.
    pub id: i64,
    /// The stable code, e.g. `consultation`.
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// How long the service takes.
    pub duration_minutes: i64,
    /// Time held before the start, which counts as occupied.
    pub buffer_before_minutes: i64,
    /// Time held after the end, which counts as occupied.
    pub buffer_after_minutes: i64,
    /// Seats per slot.
    pub capacity: i64,
    /// The resource types one booking needs, one of each, comma separated.
    pub required_types: String,
    /// A booking starts `requested` and staff accept it.
    pub requires_acceptance: bool,
    /// Parties may book it themselves.
    pub self_bookable: bool,
    /// It may be delivered online.
    pub online_allowed: bool,
    /// The party is checked in before the service.
    pub check_in_required: bool,
    /// The service is tracked as `in_progress`.
    pub track_in_progress: bool,
    /// Hours before the start until which the party may cancel.
    pub cancel_cutoff_hours: i64,
    /// Hours before the start until which the party may reschedule.
    pub reschedule_cutoff_hours: i64,
    /// The booking deposit in fils; 0 for none.
    pub deposit_amount: i64,
    /// Hours before the start inside which a party cancellation forfeits the
    /// deposit.
    pub deposit_forfeit_hours: i64,
    /// After check-in, the rest of the deposit is forfeited (else refunded).
    pub walk_out_forfeits: bool,
    /// The prepayment of a self-booking in fils; 0 for none.
    pub prepayment_amount: i64,
    /// The kinds of qualification (`practitioner_licence`) the person who
    /// delivers it must hold, comma separated; checked with
    /// `qualification_valid` (`SCH.ACL.04`, `SCH.RUL.05`).
    pub required_qualifications: String,
    /// The kinds of document the resource (its employee or asset) must hold
    /// valid, comma separated; checked with `document_valid`.
    pub required_documents: String,
    /// Whether it can be booked.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ServiceType {
    /// An active service type with the defaults of wl-scheduling §7: one
    /// person, no acceptance, cut-offs of 24 hours, no deposit.
    pub fn new(code: &str, name_en: &str, duration_minutes: i64) -> Self {
        ServiceType {
            code: code.to_owned(),
            name_en: name_en.to_owned(),
            duration_minutes,
            capacity: 1,
            required_types: "person".to_owned(),
            self_bookable: true,
            cancel_cutoff_hours: 24,
            reschedule_cutoff_hours: 24,
            deposit_forfeit_hours: 24,
            walk_out_forfeits: true,
            active: true,
            ..ServiceType::default()
        }
    }

    /// The resource types one booking needs.
    pub fn required(&self) -> Vec<ResourceType> {
        self.required_types
            .split(',')
            .filter_map(|name| {
                ResourceType::ALL
                    .iter()
                    .copied()
                    .find(|kind| kind.as_str() == name.trim())
            })
            .collect()
    }

    /// The qualification kinds the delivering person must hold.
    pub fn qualification_kinds(&self) -> Vec<String> {
        kinds(&self.required_qualifications)
    }

    /// The document kinds the resource must hold.
    pub fn document_kinds(&self) -> Vec<String> {
        kinds(&self.required_documents)
    }

    /// Whether the service takes a deposit.
    pub fn has_deposit(&self) -> bool {
        self.deposit_amount > 0
    }
}

/// The kinds of a comma separated list, trimmed and without blanks.
fn kinds(list: &str) -> Vec<String> {
    list.split(',')
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A resource that can deliver a service type.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_scheduling_service_resources")]
pub struct ServiceResource {
    /// The primary key.
    pub id: i64,
    /// The service type.
    #[model(references = "wl_scheduling_service_types")]
    pub service_type_id: i64,
    /// The resource.
    #[model(references = "wl_scheduling_resources")]
    pub resource_id: i64,
}

/// A recurring working pattern of a resource.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_scheduling_availability_rules",
    index(resource_id, weekday)
)]
pub struct AvailabilityRule {
    /// The primary key.
    pub id: i64,
    /// The resource.
    #[model(references = "wl_scheduling_resources")]
    pub resource_id: i64,
    /// 1 = Monday .. 7 = Sunday.
    pub weekday: i64,
    /// The start, in minutes after local midnight.
    pub from_minute: i64,
    /// The end, in minutes after local midnight (at most 1440).
    pub to_minute: i64,
    /// The service type it applies to; every one when absent.
    pub service_type_id: Option<i64>,
    /// The first day it applies.
    pub valid_from: Option<NaiveDate>,
    /// The last day it applies.
    pub valid_to: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
}

impl AvailabilityRule {
    /// A rule for `weekday` (1 = Monday) from minute `from` to minute `to`.
    pub fn new(resource_id: i64, weekday: i64, from: i64, to: i64) -> Self {
        AvailabilityRule {
            resource_id,
            weekday,
            from_minute: from,
            to_minute: to,
            ..AvailabilityRule::default()
        }
    }

    /// Whether the rule applies on `day` to `service_type_id`.
    pub fn applies(&self, day: NaiveDate, service_type_id: i64) -> bool {
        self.valid_from.is_none_or(|from| from <= day)
            && self.valid_to.is_none_or(|to| day <= to)
            && self.service_type_id.is_none_or(|id| id == service_type_id)
    }
}

/// Time a resource is not bookable.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_scheduling_availability_blocks",
    index(resource_id, starts_at)
)]
pub struct AvailabilityBlock {
    /// The primary key.
    pub id: i64,
    /// The resource.
    #[model(references = "wl_scheduling_resources")]
    pub resource_id: i64,
    /// The start.
    pub starts_at: DateTime,
    /// The end.
    pub ends_at: DateTime,
    /// Why, e.g. `sick`.
    pub reason: String,
    /// `manual`, `leave`, `roster`, `session` or `holiday`.
    pub source: String,
    /// The record that caused it (a leave request, a roster).
    pub source_ref: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
}

impl AvailabilityBlock {
    /// A manual block of `resource_id` from `starts_at` to `ends_at`.
    pub fn manual(resource_id: i64, starts_at: DateTime, ends_at: DateTime, reason: &str) -> Self {
        AvailabilityBlock {
            resource_id,
            starts_at,
            ends_at,
            reason: reason.to_owned(),
            source: "manual".to_owned(),
            ..AvailabilityBlock::default()
        }
    }
}

/// Lets `resource_id` deliver `service_type_id`; once only.
pub async fn offer(db: &Db, service_type_id: i64, resource_id: i64) -> Result {
    let exists = ServiceResource::query()
        .where_(ServiceResource::SERVICE_TYPE_ID.eq(service_type_id))
        .where_(ServiceResource::RESOURCE_ID.eq(resource_id))
        .exists(db)
        .await?;
    if !exists {
        ServiceResource {
            service_type_id,
            resource_id,
            ..ServiceResource::default()
        }
        .save(db)
        .await?;
    }
    Ok(())
}

/// The active resources that can deliver `service_type_id`, by id.
pub async fn eligible_resources(db: &Db, service_type_id: i64) -> Result<Vec<Resource>> {
    let ids = ServiceResource::query()
        .where_(ServiceResource::SERVICE_TYPE_ID.eq(service_type_id))
        .get(db)
        .await?
        .into_iter()
        .map(|row| row.resource_id)
        .collect::<Vec<_>>();
    Resource::query()
        .where_(Resource::ID.is_in(ids))
        .where_(Resource::ACTIVE.eq(true))
        .order_by(Resource::ID)
        .get(db)
        .await
}
