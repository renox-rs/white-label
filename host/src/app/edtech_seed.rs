//! The 10 EdTech client's values for the shared scheduling, HR, ticketing and
//! document mechanisms (10 §4.5, §7.1 *Booking*, §7.2, §9; base for
//! BL-EDU-07, BL-EDU-08 and BL-EDU-17).
//!
//! The mechanisms live in the shared modules (`wl-scheduling`, `wl-hr`,
//! `wl-documents`, `wl-ticketing`) and the vertical reaches them only through
//! `wl-kernel`. The tables are `edtech::seed` and `edtech::reasons`; this file
//! turns them into:
//!
//! - the document kinds registered with wl-documents ([`register_documents`]):
//!   the centre permit and the course approval (block `batch.open` and
//!   `certificate.issue`, override `never`, 60 days look-ahead, renewed by the
//!   centre manager and the academic coordinator), the police clearance and
//!   the private tutor permit or freelance visa of a tutor (block
//!   `batch.open`, `batch.change_tutor`, `session.change_tutor`,
//!   `session.reschedule`, `booking.book` and `booking.reassign`, flag
//!   `session.submit_attendance`, 30 days look-ahead), the student's ID and
//!   the guardian's consent;
//! - the scheduling settings ([`scheduling_for`]): the tutor reason codes, the
//!   centre manager as the manager told of an open conflict (`SCH.RUL.06`,
//!   H = 2), eligibility D = 30 (`SCH.RUL.05`), no deposit;
//! - the HR settings ([`hr`]): the centre manager is the line manager of a
//!   tutor;
//! - the seed ([`EdtechSeed`], command `edtech:install`): the service types
//!   every centre starts with and the ticket categories. It adds what the
//!   database lacks and keeps what the client changed.
//!
//! Decisions: the kinds are not marked compliance-critical in wl-documents
//! (and so not `legal_block`): `EDU.RUL.13` tells the coordinator, the centre
//! manager and the owner in one message, and which kinds are legal blocks is
//! `⚠ open: lawyer (L-19)`. The `blocks` entries of the batch, session and
//! certificate transitions are the kinds' statement of intent: the providers
//! that answer them come with BL-EDU-07 (`batch.open`, `certificate.issue`)
//! and BL-EDU-08 (the tutor transitions). The course approval blocks
//! `certificate.issue` only for a certificate that states it (the condition
//! `states_approval`).

use std::sync::Arc;

use edtech::permissions::{CENTRE_MANAGER_ROLE, COORDINATOR_ROLE, COUNSELLOR_ROLE};
use edtech::seed::{
    self, CENTRE_LOOK_AHEAD_DAYS, CENTRE_PERMIT_KIND, COURSE_APPROVAL_KIND, COURSE_SUBJECT_TYPE,
    GUARDIAN_CONSENT_KIND, POLICE_CLEARANCE_KIND, REMINDER_DAYS, REQUIRED_QUALIFICATIONS,
    STUDENT_ID_KIND, STUDENT_ID_REMINDER_DAYS, TUTOR_LOOK_AHEAD_DAYS, TUTOR_PERMIT_KIND,
};
use renox::prelude::*;
use wl_documents::registrations::{
    LEGAL_ENTITY_SUBJECT, OTHER, TRADEMARK, VerticalSettings, register_vertical,
};
use wl_documents::{KindSeed, Sensitivity, register_kind};
use wl_kernel::Manifest;
use wl_kernel::documents::{BlockCondition, BlockMode, OverrideMode};
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;
use wl_scheduling::{ServiceType, WlScheduling};
use wl_ticketing::config::InvoiceOn;
use wl_ticketing::{Category, Priority, SlaPolicy};

/// The client app these values are for.
pub const EDTECH: &str = "edtech";

/// The transition `batch.open` (BL-EDU-07, BL-EDU-08).
pub const GUARDED_BATCH_OPEN: &str = "batch.open";
/// The transition `batch.change_tutor` (BL-EDU-08).
pub const GUARDED_BATCH_CHANGE_TUTOR: &str = "batch.change_tutor";
/// The transition `session.change_tutor` (BL-EDU-08).
pub const GUARDED_SESSION_CHANGE_TUTOR: &str = "session.change_tutor";
/// The transition `session.reschedule` (BL-EDU-08).
pub const GUARDED_SESSION_RESCHEDULE: &str = "session.reschedule";
/// The transition `session.submit_attendance` (BL-EDU-11, flag).
pub const GUARDED_SESSION_SUBMIT_ATTENDANCE: &str = "session.submit_attendance";
/// The transition `certificate.issue` (BL-EDU-07).
pub const GUARDED_CERTIFICATE_ISSUE: &str = "certificate.issue";
/// The transition `booking.book` of wl-scheduling.
pub const GUARDED_BOOKING_BOOK: &str = "booking.book";
/// The transition `booking.reassign` of wl-scheduling.
pub const GUARDED_BOOKING_REASSIGN: &str = "booking.reassign";

/// The centre permit as wl-documents' seed: an expiring document of the legal
/// entity that blocks `batch.open` and `certificate.issue`, never overridden
/// (§9); the centre manager renews it.
pub fn centre_permit_kind() -> KindSeed {
    KindSeed::new(CENTRE_PERMIT_KIND, "Centre permit")
        .name_ar("ترخيص المركز")
        .subject_type(LEGAL_ENTITY_SUBJECT)
        .expires()
        .numbered()
        .reminders(REMINDER_DAYS)
        .look_ahead(CENTRE_LOOK_AHEAD_DAYS)
        .responsible_role(CENTRE_MANAGER_ROLE)
        .override_fixed()
        .blocks(GUARDED_BATCH_OPEN, BlockMode::Block)
        .blocks(GUARDED_CERTIFICATE_ISSUE, BlockMode::Block)
}

/// The course approval as wl-documents' seed: an expiring document of the
/// course that blocks `batch.open`, and `certificate.issue` when the
/// certificate states the approval; the academic coordinator renews it.
pub fn course_approval_kind() -> KindSeed {
    KindSeed::new(COURSE_APPROVAL_KIND, "Course approval")
        .name_ar("اعتماد الدورة")
        .subject_type(COURSE_SUBJECT_TYPE)
        .expires()
        .numbered()
        .reminders(REMINDER_DAYS)
        .look_ahead(CENTRE_LOOK_AHEAD_DAYS)
        .responsible_role(COORDINATOR_ROLE)
        .override_fixed()
        .blocks(GUARDED_BATCH_OPEN, BlockMode::Block)
        .blocks_when(
            GUARDED_CERTIFICATE_ISSUE,
            BlockMode::Block,
            BlockCondition::new("states_approval", ["true"]),
        )
}

/// The `blocks` of the two tutor kinds (§9): block the assignments, flag the
/// attendance of a session taught without it.
fn tutor_blocks(kind: KindSeed) -> KindSeed {
    kind.blocks(GUARDED_BATCH_OPEN, BlockMode::Block)
        .blocks(GUARDED_BATCH_CHANGE_TUTOR, BlockMode::Block)
        .blocks(GUARDED_SESSION_CHANGE_TUTOR, BlockMode::Block)
        .blocks(GUARDED_SESSION_RESCHEDULE, BlockMode::Block)
        .blocks(GUARDED_BOOKING_BOOK, BlockMode::Block)
        .blocks(GUARDED_BOOKING_REASSIGN, BlockMode::Block)
        .blocks(GUARDED_SESSION_SUBMIT_ATTENDANCE, BlockMode::Flag)
}

/// The police clearance (good-conduct certificate) of a tutor: best practice,
/// override `never` by default; the client may switch it to approval by the
/// owner (BL-EDU-17).
pub fn police_clearance_kind() -> KindSeed {
    tutor_blocks(
        KindSeed::new(POLICE_CLEARANCE_KIND, "Police clearance")
            .name_ar("شهادة حسن السيرة والسلوك")
            .subject_type(EMPLOYEE_SUBJECT_TYPE)
            .expires()
            .numbered()
            .reminders(REMINDER_DAYS)
            .look_ahead(TUTOR_LOOK_AHEAD_DAYS)
            .responsible_role(COORDINATOR_ROLE)
            .override_mode(OverrideMode::Never),
    )
}

/// The MoHRE private tutor permit or the freelance visa of a freelance tutor:
/// as the police clearance, but the centre cannot switch the override.
pub fn tutor_permit_kind() -> KindSeed {
    tutor_blocks(
        KindSeed::new(TUTOR_PERMIT_KIND, "Private tutor permit or freelance visa")
            .name_ar("تصريح المدرس الخصوصي أو تأشيرة العمل الحر")
            .subject_type(EMPLOYEE_SUBJECT_TYPE)
            .expires()
            .numbered()
            .reminders(REMINDER_DAYS)
            .look_ahead(TUTOR_LOOK_AHEAD_DAYS)
            .responsible_role(COORDINATOR_ROLE)
            .override_fixed(),
    )
}

/// The student's Emirates ID or passport: an expiring document of the party,
/// kept by the front desk; the image is personal data (`EDU.ACL.09`).
pub fn student_id_kind() -> KindSeed {
    KindSeed::new(STUDENT_ID_KIND, "Student Emirates ID or passport")
        .name_ar("الهوية الإماراتية أو جواز سفر الطالب")
        .subject_type(PARTY_SUBJECT_TYPE)
        .expires()
        .numbered()
        .reminders(STUDENT_ID_REMINDER_DAYS)
        .sensitivity(Sensitivity::Personal)
        .portal_upload()
        .responsible_role(COUNSELLOR_ROLE)
        .notify_party()
}

/// The guardian's consent to a minor's data and photos (`DOC.SIG.01`,
/// `PTY.CNS.01`): a document of the party, no expiry.
pub fn guardian_consent_kind() -> KindSeed {
    KindSeed::new(GUARDIAN_CONSENT_KIND, "Guardian consent")
        .name_ar("موافقة ولي الأمر")
        .subject_type(PARTY_SUBJECT_TYPE)
        .sensitivity(Sensitivity::Personal)
        .portal_upload()
        .responsible_role(COUNSELLOR_ROLE)
}

/// Registers the 10 kinds with wl-documents (only for the EdTech client), and
/// turns on the two registration kinds 10 uses (*Trademark registration* and
/// *Other certification*, `DOC.EXP.01`), renewed by the owner, with no
/// `blocks`.
pub fn register_documents(client_app: &str) {
    if client_app != EDTECH {
        return;
    }
    register_kind(centre_permit_kind());
    register_kind(course_approval_kind());
    register_kind(police_clearance_kind());
    register_kind(tutor_permit_kind());
    register_kind(student_id_kind());
    register_kind(guardian_consent_kind());
    register_vertical(
        EDTECH,
        VerticalSettings::new()
            .enabled(&[TRADEMARK, OTHER])
            .role(TRADEMARK, edtech::permissions::OWNER_ROLE)
            .role(OTHER, edtech::permissions::OWNER_ROLE),
    );
}

/// The scheduling module with the EdTech client's values (§7.1 *Booking*,
/// §7.2): D = 30 days for `SCH.RUL.05`, H = 2 hours for `SCH.RUL.06`,
/// reminders 24 and 2 hours before, the centre manager told of an open
/// conflict, and the tutor reason codes. Check-in is not used and no deposit
/// is taken by default. Any other client keeps the module's defaults.
pub fn scheduling_for(client_app: &str) -> WlScheduling {
    let mut module = WlScheduling::new();
    if client_app == EDTECH {
        module = module
            .look_ahead_days(seed::BOOKING_LOOK_AHEAD_DAYS)
            .escalation_hours(seed::ESCALATION_HOURS)
            .reminder_hours(seed::REMINDER_HOURS)
            .manager_role(CENTRE_MANAGER_ROLE)
            .walk_out_forfeits(false);
        for (code, initiated_by) in edtech::reasons::SCHEDULING_REASON_CODES {
            module = module.reason_code(code, *initiated_by);
        }
    }
    module
}

/// The HR module with the EdTech client's values: the centre manager is the
/// line manager of a tutor and the owner role of the client the second person
/// for waiving and reopening a step.
pub fn hr(module: wl_hr::WlHr, owner_role: &str) -> wl_hr::WlHr {
    module
        .manager_role(CENTRE_MANAGER_ROLE)
        .owner_role(owner_role)
}

/// What [`install`] stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Service types created.
    pub service_types: usize,
    /// Ticket categories created.
    pub categories: usize,
}

impl Installed {
    /// Everything created.
    pub fn total(&self) -> usize {
        self.service_types + self.categories
    }
}

/// Stores what the EdTech client's modules lack, for the modules the manifest
/// enables: the service types (by code), each without check-in, not
/// self-bookable yet (`SCH.SLF.01` is Later for 10), and the ticket
/// categories (by key). A row the client has already keeps its values.
///
/// # Errors
///
/// The database's errors.
pub async fn install(state: &AppState) -> Result<Installed> {
    let manifest = state.provided::<Arc<Manifest>>();
    let on = |name: &str| manifest.as_ref().is_some_and(|m| m.is_enabled(name));
    let mut installed = Installed::default();
    if on("scheduling") {
        installed.service_types = install_service_types(state).await?;
    }
    if on("ticketing") {
        installed.categories = install_categories(&state.db).await?;
    }
    Ok(installed)
}

async fn install_service_types(state: &AppState) -> Result<usize> {
    let mut created = 0;
    for spec in seed::SERVICE_TYPES {
        let found = ServiceType::query()
            .where_(ServiceType::CODE.eq(spec.code))
            .first(&state.db)
            .await?;
        if found.is_some() {
            continue;
        }
        let mut service = ServiceType::new_in(state, spec.code, spec.name_en, spec.minutes);
        service.name_ar = Some(spec.name_ar.to_owned());
        service.self_bookable = false;
        service.required_qualifications = REQUIRED_QUALIFICATIONS.to_owned();
        service.save(&state.db).await?;
        created += 1;
    }
    Ok(created)
}

async fn install_categories(db: &Db) -> Result<usize> {
    let policy = SlaPolicy::by_key(db, wl_ticketing::seed::STANDARD_POLICY).await?;
    let mut created = 0;
    for spec in seed::CATEGORIES {
        if Category::by_key(db, spec.key).await?.is_some() {
            continue;
        }
        Category::create(
            db,
            Category {
                key: spec.key.to_owned(),
                name_en: spec.name_en.to_owned(),
                name_ar: spec.name_ar.to_owned(),
                default_priority: Priority::Normal,
                sla_policy_id: policy.as_ref().map(|p| p.id),
                requires_signoff: false,
                confidential: false,
                active: true,
                invoice_on: InvoiceOn::default(),
                default_assignee_role: spec.assignee_role.map(str::to_owned),
                ..Category::default()
            },
        )
        .await?;
        created += 1;
    }
    Ok(created)
}

/// The module that seeds the EdTech client's values: the seeder runs after the
/// shared modules' own (it is mounted after them), and the command
/// `edtech:install` runs the same install.
pub struct EdtechSeed;

impl Module for EdtechSeed {
    fn name(&self) -> &'static str {
        "edtech_seed"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "edtech:install",
            "Store the EdTech service types and ticket categories, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = install(&state).await?;
                println!("{} EdTech defaults created.", installed.total());
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { install(&state).await.map(|_| ()) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guarded(kind: &KindSeed) -> Vec<(&str, BlockMode)> {
        kind.blocks
            .iter()
            .map(|b| (b.guarded.as_str(), b.mode))
            .collect()
    }

    #[test]
    fn the_centre_permit_blocks_the_batch_and_the_certificate_and_is_never_overridden() {
        let kind = centre_permit_kind();
        assert_eq!(
            guarded(&kind),
            [
                ("batch.open", BlockMode::Block),
                ("certificate.issue", BlockMode::Block)
            ]
        );
        assert_eq!(kind.override_mode, OverrideMode::Never);
        assert!(kind.override_fixed);
        assert_eq!(kind.look_ahead_days, 60);
        assert_eq!(kind.responsible_role.as_deref(), Some("centre_manager"));
        assert_eq!(kind.subject_types, ["kernel.legal_entity"]);
    }

    #[test]
    fn the_course_approval_blocks_the_certificate_only_when_it_states_the_approval() {
        let kind = course_approval_kind();
        assert_eq!(kind.blocks.len(), 2);
        assert!(kind.blocks[0].condition.is_none());
        assert!(kind.blocks[1].condition.is_some());
        assert_eq!(kind.responsible_role.as_deref(), Some("academic_coordinator"));
        assert_eq!(kind.subject_types, ["edtech.course"]);
    }

    #[test]
    fn the_tutor_kinds_block_the_assignments_and_flag_the_attendance() {
        for kind in [police_clearance_kind(), tutor_permit_kind()] {
            let entries = guarded(&kind);
            assert_eq!(entries.len(), 7);
            assert!(entries.contains(&("session.submit_attendance", BlockMode::Flag)));
            assert!(entries.contains(&("batch.change_tutor", BlockMode::Block)));
            assert_eq!(kind.override_mode, OverrideMode::Never);
            assert_eq!(kind.look_ahead_days, 30);
            assert_eq!(kind.subject_types, ["hr.employee"]);
        }
        // Only the police clearance may be switched to approval.
        assert!(!police_clearance_kind().override_fixed);
        assert!(tutor_permit_kind().override_fixed);
    }
}
