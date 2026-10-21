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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_service_requires_the_licence_and_the_privileges() {
        assert_eq!(
            REQUIRED_QUALIFICATIONS.split(',').collect::<Vec<_>>(),
            [LICENCE_KIND, PRIVILEGES_KIND]
        );
    }
}
