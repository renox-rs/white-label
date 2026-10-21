//! The 05 Clinics client's values for the shared scheduling, HR and document
//! mechanisms (05 §4.5, §7.1, §7.2 *Settings for the shared negative flows*;
//! BL-CLN-12, BL-CLN-01).
//!
//! The mechanisms live in the shared modules (`wl-scheduling`, `wl-hr`,
//! `wl-documents`) and the vertical reaches them only through `wl-kernel`.
//! The tables are `clinics::seed` and `clinics::reasons`; this file turns
//! them into:
//!
//! - the document kinds registered with wl-documents ([`register_documents`]):
//!   the practitioner licence and the branch privileges, which block
//!   `booking.book` and `booking.check_in` with override `never`
//!   (`SCH.ACL.04`), remind at 90, 60, 30 and 7 days and look 30 days ahead;
//!   the clinic manager of the branch renews them;
//! - the scheduling settings ([`scheduling_for`]): the reason codes 05 adds
//!   (`coverage_not_confirmed`, initiated by the business) and the clinic
//!   manager as the manager told of an open conflict (`SCH.RUL.06`);
//! - the HR settings ([`hr`]): the clinic manager is the line manager of a
//!   practitioner, told when a qualification expires (`HRM.RUL.10`);
//! - the seed ([`ClinicsSeed`], command `clinics:install`): the service types
//!   every clinic starts with, each needing the licence and the privileges
//!   (`SCH.RUL.05`). It adds what the database lacks and keeps what the
//!   client changed.
//!
//! Decisions: the two kinds are not marked compliance-critical in
//! wl-documents (and so not `legal_block`): the owner is told by `CLN.RUL.01`
//! in one message with the number of appointments, not a second time by
//! `DOC.RUL.03` (§7.2), and `DOC.RUL.03` is a compliance-critical rule
//! owned by the owner role all the same. Whether the law makes them legal
//! blocks is `⚠ open: lawyer (L-19)`; the override mode is `never` as the
//! spec says. `booking.start` is not listed in the kinds' blocks: wl-scheduling
//! asks the qualification again at `start` by itself.

use std::sync::Arc;

use clinics::permissions::CLINIC_MANAGER_ROLE;
use clinics::seed::{
    self, LICENCE_KIND, LOOK_AHEAD_DAYS, PRIVILEGES_KIND, REMINDER_DAYS, REQUIRED_QUALIFICATIONS,
};
use renox::prelude::*;
use wl_documents::{KindSeed, register_kind};
use wl_kernel::Manifest;
use wl_kernel::documents::{BlockMode, OverrideMode};
use wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;
use wl_scheduling::eligibility::{GUARDED_BOOK, GUARDED_CHECK_IN};
use wl_scheduling::{ServiceType, WlScheduling};

/// The client app these values are for.
pub const CLINICS: &str = "clinics";

/// The kind `key` of a practitioner: an expiring document of the employee that
/// blocks `booking.book` and `booking.check_in`, never overridden.
fn practitioner_kind(key: &str, name_en: &str, name_ar: &str) -> KindSeed {
    KindSeed::new(key, name_en)
        .name_ar(name_ar)
        .subject_type(EMPLOYEE_SUBJECT_TYPE)
        .expires()
        .numbered()
        .reminders(REMINDER_DAYS)
        .look_ahead(LOOK_AHEAD_DAYS)
        .responsible_role(CLINIC_MANAGER_ROLE)
        .override_mode(OverrideMode::Never)
        .blocks(GUARDED_BOOK, BlockMode::Block)
        .blocks(GUARDED_CHECK_IN, BlockMode::Block)
}

/// The practitioner licence as wl-documents' seed.
pub fn licence_kind() -> KindSeed {
    practitioner_kind(LICENCE_KIND, "Practitioner licence", "رخصة مزاولة المهنة")
}

/// The branch privileges as wl-documents' seed.
pub fn privileges_kind() -> KindSeed {
    practitioner_kind(
        PRIVILEGES_KIND,
        "Branch privileges",
        "صلاحيات الفرع",
    )
}

/// Registers the 05 kinds with wl-documents (only for the Clinics client).
pub fn register_documents(client_app: &str) {
    if client_app != CLINICS {
        return;
    }
    register_kind(licence_kind());
    register_kind(privileges_kind());
}

/// The scheduling module with the Clinics client's values: the 05 reason
/// codes and the clinic manager as the manager of the branch. Any other
/// client keeps the module's defaults.
pub fn scheduling_for(client_app: &str) -> WlScheduling {
    let mut module = WlScheduling::new();
    if client_app == CLINICS {
        module = module.manager_role(CLINIC_MANAGER_ROLE);
        for (code, initiated_by) in clinics::reasons::SCHEDULING_REASON_CODES {
            module = module.reason_code(code, *initiated_by);
        }
    }
    module
}

/// The HR module with the Clinics client's values: the clinic manager is the
/// line manager of a practitioner and the owner role of the client the second
/// person for waiving and reopening a step.
pub fn hr(module: wl_hr::WlHr, owner_role: &str) -> wl_hr::WlHr {
    module
        .manager_role(CLINIC_MANAGER_ROLE)
        .owner_role(owner_role)
}

/// What [`install`] stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Service types created.
    pub service_types: usize,
}

impl Installed {
    /// Everything created.
    pub fn total(&self) -> usize {
        self.service_types
    }
}

/// Stores the service types the database lacks (by code), each checking the
/// patient in, tracking the visit and needing the licence and the privileges.
/// A type the client has already keeps its values. Nothing is stored when
/// scheduling is off.
///
/// # Errors
///
/// The database's errors.
pub async fn install(state: &AppState) -> Result<Installed> {
    let mut installed = Installed::default();
    let on = state
        .provided::<Arc<Manifest>>()
        .is_some_and(|manifest| manifest.is_enabled("scheduling"));
    if !on {
        return Ok(installed);
    }
    for spec in seed::SERVICE_TYPES {
        let found = ServiceType::query()
            .where_(ServiceType::CODE.eq(spec.code))
            .first(&state.db)
            .await?;
        if found.is_some() {
            continue;
        }
        let mut service = ServiceType::new(spec.code, spec.name_en, spec.minutes);
        service.name_ar = Some(spec.name_ar.to_owned());
        service.check_in_required = true;
        service.track_in_progress = true;
        service.required_qualifications = REQUIRED_QUALIFICATIONS.to_owned();
        service.save(&state.db).await?;
        installed.service_types += 1;
    }
    Ok(installed)
}

/// The module that seeds the Clinics client's values: the seeder runs after
/// the shared modules' own (it is mounted after them), and the command
/// `clinics:install` runs the same install.
pub struct ClinicsSeed;

impl Module for ClinicsSeed {
    fn name(&self) -> &'static str {
        "clinics_seed"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "clinics:install",
            "Store the Clinics service types, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = install(&state).await?;
                println!("{} Clinics defaults created.", installed.total());
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { install(&state).await.map(|_| ()) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_practitioner_kinds_block_booking_and_check_in_and_are_never_overridden() {
        for kind in [licence_kind(), privileges_kind()] {
            let guarded: Vec<&str> = kind.blocks.iter().map(|b| b.guarded.as_str()).collect();
            assert_eq!(guarded, ["booking.book", "booking.check_in"]);
            assert_eq!(kind.override_mode, OverrideMode::Never);
            assert_eq!(kind.responsible_role.as_deref(), Some("clinic_manager"));
            assert!(kind.has_expiry && kind.has_number);
            assert_eq!(kind.look_ahead_days, 30);
            assert_eq!(kind.reminder_offsets, [90, 60, 30, 7]);
            assert_eq!(kind.subject_types, ["hr.employee"]);
        }
        assert_eq!(licence_kind().key, "practitioner_licence");
        assert_eq!(privileges_kind().key, "branch_privileges");
    }
}
