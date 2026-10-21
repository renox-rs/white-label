//! The vertical's static tables (05 §7.2 *Settings for the shared negative
//! flows*, §4.5). The host turns them into the document kinds and the service
//! types it installs (`host/src/app/clinics_seed.rs`); the app depends on no
//! crate that owns those tables.

/// The practitioner licence (DHA, DoH, Sharjah Health Authority or MOHAP):
/// a document kind of the practitioner and a qualification of wl-hr
/// (`HRM.QUA.01`).
pub const LICENCE_KIND: &str = "practitioner_licence";

/// The practitioner's privileges at one branch (`HRM.QUA.01`).
pub const PRIVILEGES_KIND: &str = "branch_privileges";

/// The qualification kinds every service type of the clinic requires of the
/// person who delivers it, as wl-scheduling stores them
/// (`required_qualifications`, comma separated; `SCH.ACL.04`,
/// `SCH.RUL.05`).
pub const REQUIRED_QUALIFICATIONS: &str = "practitioner_licence,branch_privileges";

/// A service type the clinic starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceSeed {
    /// The stable code (`consultation`).
    pub code: &'static str,
    /// The name in English.
    pub name_en: &'static str,
    /// The name in Arabic.
    pub name_ar: &'static str,
    /// How long it takes.
    pub minutes: i64,
}

/// The service types the host installs where the database has none of that
/// code (05 §4.1): each checks the patient in, tracks the visit and needs the
/// licence and the privileges ([`REQUIRED_QUALIFICATIONS`]). Prices, deposits
/// and the self-pay price list are the clinic's own (`SAL.CAT.01`).
pub const SERVICE_TYPES: &[ServiceSeed] = &[
    ServiceSeed {
        code: "consultation",
        name_en: "Consultation",
        name_ar: "استشارة",
        minutes: 30,
    },
    ServiceSeed {
        code: "follow_up",
        name_en: "Follow-up visit",
        name_ar: "زيارة متابعة",
        minutes: 20,
    },
    ServiceSeed {
        code: "procedure",
        name_en: "Procedure",
        name_ar: "إجراء",
        minutes: 60,
    },
];

/// The days before the expiry of a licence or of privileges that `DOC.RUL.01`
/// reminds (180 days are for the facility licence, not for a practitioner).
pub const REMINDER_DAYS: &[i64] = &[90, 60, 30, 7];

/// The days ahead `DOC.RUL.02` lists the bookings a lapsing licence affects
/// (the same 30 days as `SCH.RUL.05`).
pub const LOOK_AHEAD_DAYS: i64 = 30;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_types_have_unique_codes() {
        let mut codes: Vec<&str> = SERVICE_TYPES.iter().map(|s| s.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), SERVICE_TYPES.len());
    }

    #[test]
    fn every_service_requires_the_licence_and_the_privileges() {
        assert_eq!(
            REQUIRED_QUALIFICATIONS.split(',').collect::<Vec<_>>(),
            [LICENCE_KIND, PRIVILEGES_KIND]
        );
    }
}
