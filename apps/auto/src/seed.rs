//! The vertical's static tables (08 §4.4, §7.1 *Booking*, §7.1 settings, §9).
//! The host turns them into the document kinds, the service types and the
//! ticket categories it installs (`host/src/app/auto_seed.rs`); the app
//! depends on no crate that owns those tables.

/// The subject type of a vehicle (`auto.vehicle:7`).
pub const VEHICLE_SUBJECT_TYPE: &str = "auto.vehicle";

/// The subject type of a job card (`auto.job_card:7`).
pub const JOB_CARD_SUBJECT_TYPE: &str = "auto.job_card";

/// The subject type of an insurance claim (`auto.insurance_claim:7`), built
/// with the claims.
pub const INSURANCE_CLAIM_SUBJECT_TYPE: &str = "auto.insurance_claim";

/// The subject type of a fleet contract (`auto.fleet_contract:7`), built with
/// the fleet contracts.
pub const FLEET_CONTRACT_SUBJECT_TYPE: &str = "auto.fleet_contract";

/// The police accident report: the repair permit of the traffic authority
/// (Federal Decree-Law 14 of 2024, Art. 28), a document of the job card or of
/// its insurance claim (`AUT.ACL.20`). It blocks `job_card.approve` and
/// `insurance_claim.approve`, never overridden.
pub const POLICE_REPORT_KIND: &str = "police_report";

/// The driver's licence of the claim (`AUT.ACL.18`).
pub const DRIVING_LICENCE_KIND: &str = "claim_driving_licence";

/// The vehicle registration card (Mulkiya) of the claim (`AUT.ACL.18`).
pub const MULKIYA_KIND: &str = "claim_mulkiya";

/// The Emirates ID of the claim's driver or owner (`AUT.ACL.18`).
pub const EMIRATES_ID_KIND: &str = "claim_emirates_id";

/// The trade licence of a B2B customer: reminders only (`DOC.RUL.01`).
pub const CUSTOMER_LICENCE_KIND: &str = "customer_trade_licence";

/// The signed fleet service contract: reminders only (`DOC.RUL.01`).
pub const FLEET_CONTRACT_KIND: &str = "fleet_contract_document";

/// The days before the expiry of a customer licence or a fleet contract that
/// `DOC.RUL.01` reminds.
pub const REMINDER_DAYS: &[i64] = &[90, 60, 30, 7];

/// The look-ahead of the expiring kinds in days.
pub const LOOK_AHEAD_DAYS: i64 = 30;

/// `SCH.RUL.06`: hours before the start at which an open conflict goes to the
/// workshop manager.
pub const ESCALATION_HOURS: i64 = 2;

/// `SCH.RUL.05`: days ahead that bookings are checked for eligibility.
pub const BOOKING_LOOK_AHEAD_DAYS: i64 = 30;

/// `SCH.RUL.01`: hours before a workshop booking that the customer is
/// reminded.
pub const REMINDER_HOURS: &[i64] = &[24];

/// A service type the workshop starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceSeed {
    /// The stable code (`periodic_service`).
    pub code: &'static str,
    /// The name in English.
    pub name_en: &'static str,
    /// The name in Arabic.
    pub name_ar: &'static str,
    /// How long it takes.
    pub minutes: i64,
}

/// The service types the host installs where the database has none of that
/// code (§4.4, §7.1 *Booking*): a periodic service, a diagnosis and an
/// accident estimate. Check-in is required (it opens the job card); prices
/// and the bays are the workshop's own.
pub const SERVICE_TYPES: &[ServiceSeed] = &[
    ServiceSeed {
        code: "periodic_service",
        name_en: "Periodic service",
        name_ar: "صيانة دورية",
        minutes: 120,
    },
    ServiceSeed {
        code: "diagnosis",
        name_en: "Diagnosis",
        name_ar: "فحص وتشخيص",
        minutes: 60,
    },
    ServiceSeed {
        code: "accident_estimate",
        name_en: "Accident damage estimate",
        name_ar: "تقدير أضرار الحادث",
        minutes: 45,
    },
];

/// A ticket category the workshop starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategorySpec {
    /// The stable key.
    pub key: &'static str,
    /// The name in English.
    pub name_en: &'static str,
    /// The name in Arabic.
    pub name_ar: &'static str,
    /// The role a ticket of the category goes to.
    pub assignee_role: Option<&'static str>,
}

/// The ticket categories of 08: the customer warranty claim (`WRK.TKT.01`),
/// the vehicle incident (§4.4) and the customer complaint.
pub const CATEGORIES: &[CategorySpec] = &[
    CategorySpec {
        key: "warranty",
        name_en: "Warranty claim",
        name_ar: "مطالبة ضمان",
        assignee_role: Some(crate::permissions::SERVICE_ADVISOR_ROLE),
    },
    CategorySpec {
        key: "vehicle_incident",
        name_en: "Vehicle incident",
        name_ar: "حادث مركبة",
        assignee_role: Some(crate::permissions::BRANCH_MANAGER_ROLE),
    },
    CategorySpec {
        key: "customer_complaint",
        name_en: "Customer complaint",
        name_ar: "شكوى عميل",
        assignee_role: Some(crate::permissions::BRANCH_MANAGER_ROLE),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_are_distinct() {
        let mut keys: Vec<&str> = SERVICE_TYPES.iter().map(|s| s.code).collect();
        keys.extend(CATEGORIES.iter().map(|c| c.key));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }

    #[test]
    fn the_subject_types_are_the_apps() {
        for subject in [
            VEHICLE_SUBJECT_TYPE,
            JOB_CARD_SUBJECT_TYPE,
            INSURANCE_CLAIM_SUBJECT_TYPE,
            FLEET_CONTRACT_SUBJECT_TYPE,
        ] {
            assert!(subject.starts_with("auto."), "{subject}");
        }
    }
}
