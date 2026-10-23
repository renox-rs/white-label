//! The 08 Auto client's values for the shared scheduling, HR, ticketing and
//! document mechanisms (08 §4.4, §7.1 *Booking*, §7.1 settings, §9; base for
//! BL-AUT-01 .. BL-AUT-18).
//!
//! The mechanisms live in the shared modules (`wl-scheduling`, `wl-hr`,
//! `wl-documents`, `wl-ticketing`) and the vertical reaches them only through
//! `wl-kernel`. The tables are `auto::seed` and `auto::reasons`; this file
//! turns them into:
//!
//! - the document kinds registered with wl-documents ([`register_documents`]):
//!   the police accident report (the repair permit, a legal block of
//!   `job_card.approve` and `insurance_claim.approve`, override `never`,
//!   `AUT.ACL.20`), the claim documents (driving licence, Mulkiya, Emirates ID,
//!   personal data, `AUT.ACL.18`), the customer trade licence and the fleet
//!   contract (reminders only, `DOC.RUL.01`), kept by the responsible roles of
//!   08 §7.1 *Responsible role per document kind*;
//! - the scheduling settings ([`scheduling_for`]): the workshop reason codes,
//!   the workshop manager as the manager told of an open conflict
//!   (`SCH.RUL.06`, H = 2), eligibility D = 30 (`SCH.RUL.05`), reminders 24
//!   hours before, no deposit;
//! - the HR settings ([`hr`]): the workshop manager is the line manager of a
//!   technician;
//! - the seed ([`AutoSeed`], command `auto:install`): the service types
//!   every workshop starts with, each requiring check-in (it opens the job
//!   card), and the ticket categories (warranty, vehicle incident, complaint).
//!   It adds what the database lacks and keeps what the client changed.
//!
//! Decisions: the police report is the one legal block of 08 (§9: "no
//! override"); the `blocks` entries name the transitions of the job card and
//! the claim, and the providers that answer them come with BL-AUT-07.

use std::sync::Arc;

use auto::permissions::{
    ACCOUNTANT_ROLE, BRANCH_MANAGER_ROLE, INSURANCE_COORDINATOR_ROLE, OWNER_ROLE,
    WORKSHOP_MANAGER_ROLE,
};
use auto::seed::{
    self, CUSTOMER_LICENCE_KIND, DRIVING_LICENCE_KIND, EMIRATES_ID_KIND, FLEET_CONTRACT_KIND,
    FLEET_CONTRACT_SUBJECT_TYPE, INSURANCE_CLAIM_SUBJECT_TYPE, JOB_CARD_SUBJECT_TYPE,
    LOOK_AHEAD_DAYS, MULKIYA_KIND, POLICE_REPORT_KIND, REMINDER_DAYS,
};
use renox::prelude::*;
use wl_documents::registrations::{OTHER, TRADEMARK, VerticalSettings, register_vertical};
use wl_documents::{KindSeed, Sensitivity, register_kind};
use wl_kernel::Manifest;
use wl_kernel::documents::BlockMode;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_scheduling::{ServiceType, WlScheduling};
use wl_ticketing::config::InvoiceOn;
use wl_ticketing::{Category, Priority, SlaPolicy};

/// The client app these values are for.
pub const AUTO: &str = "auto";

/// The transition `job_card.approve` (BL-AUT-07).
pub const GUARDED_JOB_CARD_APPROVE: &str = "job_card.approve";
/// The transition `insurance_claim.approve` (BL-AUT-07).
pub const GUARDED_CLAIM_APPROVE: &str = "insurance_claim.approve";
/// The transition `insurance_claim.submit_estimate` (BL-AUT-07).
pub const GUARDED_CLAIM_SUBMIT_ESTIMATE: &str = "insurance_claim.submit_estimate";

/// The police accident report as wl-documents' seed: a document of the job
/// card or of its claim that blocks the approval of both, a legal block
/// (override `never`) kept by the insurance coordinator (§7.1 settings).
pub fn police_report_kind() -> KindSeed {
    KindSeed::new(POLICE_REPORT_KIND, "Police accident report")
        .name_ar("تقرير حادث الشرطة")
        .subject_type(JOB_CARD_SUBJECT_TYPE)
        .subject_type(INSURANCE_CLAIM_SUBJECT_TYPE)
        .numbered()
        .sensitivity(Sensitivity::Personal)
        .responsible_role(INSURANCE_COORDINATOR_ROLE)
        .legal_block()
        .blocks(GUARDED_JOB_CARD_APPROVE, BlockMode::Block)
        .blocks(GUARDED_CLAIM_APPROVE, BlockMode::Block)
        .blocks(GUARDED_CLAIM_SUBMIT_ESTIMATE, BlockMode::Block)
}

/// A document of a claim whose view is audited (`AUT.ACL.18`): personal
/// data, kept by the insurance coordinator.
fn claim_document(key: &str, name_en: &str, name_ar: &str) -> KindSeed {
    KindSeed::new(key, name_en)
        .name_ar(name_ar)
        .subject_type(INSURANCE_CLAIM_SUBJECT_TYPE)
        .subject_type(JOB_CARD_SUBJECT_TYPE)
        .numbered()
        .sensitivity(Sensitivity::Personal)
        .responsible_role(INSURANCE_COORDINATOR_ROLE)
}

/// The driver's licence of a claim.
pub fn driving_licence_kind() -> KindSeed {
    claim_document(DRIVING_LICENCE_KIND, "Driving licence", "رخصة القيادة").expires()
}

/// The vehicle registration card (Mulkiya) of a claim.
pub fn mulkiya_kind() -> KindSeed {
    claim_document(
        MULKIYA_KIND,
        "Vehicle registration (Mulkiya)",
        "ملكية المركبة",
    )
    .expires()
}

/// The Emirates ID of a claim's driver or owner.
pub fn emirates_id_kind() -> KindSeed {
    claim_document(EMIRATES_ID_KIND, "Emirates ID", "الهوية الإماراتية").expires()
}

/// The trade licence of a B2B customer: an expiring document of the party,
/// kept by the accountant; reminders only, no `blocks`.
pub fn customer_licence_kind() -> KindSeed {
    KindSeed::new(CUSTOMER_LICENCE_KIND, "Customer trade licence")
        .name_ar("الرخصة التجارية للعميل")
        .subject_type(PARTY_SUBJECT_TYPE)
        .expires()
        .numbered()
        .reminders(REMINDER_DAYS)
        .look_ahead(LOOK_AHEAD_DAYS)
        .responsible_role(ACCOUNTANT_ROLE)
}

/// The signed fleet contract: an expiring document of the contract, kept by
/// the owner; reminders only, no `blocks`.
pub fn fleet_contract_kind() -> KindSeed {
    KindSeed::new(FLEET_CONTRACT_KIND, "Fleet service contract")
        .name_ar("عقد خدمة الأسطول")
        .subject_type(FLEET_CONTRACT_SUBJECT_TYPE)
        .expires()
        .reminders(REMINDER_DAYS)
        .look_ahead(LOOK_AHEAD_DAYS)
        .responsible_role(OWNER_ROLE)
}

/// Registers the 08 kinds with wl-documents (only for the Auto client), and
/// turns on the two registration kinds 08 uses (*Trademark registration* and
/// *Other certification*, `DOC.EXP.01`), renewed by the owner, with no
/// `blocks`.
pub fn register_documents(client_app: &str) {
    if client_app != AUTO {
        return;
    }
    register_kind(police_report_kind());
    register_kind(driving_licence_kind());
    register_kind(mulkiya_kind());
    register_kind(emirates_id_kind());
    register_kind(customer_licence_kind());
    register_kind(fleet_contract_kind());
    register_vertical(
        AUTO,
        VerticalSettings::new()
            .enabled(&[TRADEMARK, OTHER])
            .role(TRADEMARK, OWNER_ROLE)
            .role(OTHER, OWNER_ROLE),
    );
}

/// Gives the accountant `invoices.issue` in an Auto deployment: the split
/// billing of an insurance claim (`AUT.INS.04`) and the monthly invoice of a
/// fleet contract (`FIN.USG.01`) are issued by the accountant (08 §3.1). The
/// roles table is shared by every vertical, so this is a grant of this
/// client only (see `roles::grant_extra`).
pub fn grant_roles(client_app: &str) {
    if client_app != AUTO {
        return;
    }
    super::roles::grant_extra(ACCOUNTANT_ROLE, &[auto::permissions::INVOICES_ISSUE]);
}

/// The scheduling module with the Auto client's values (§7.1 *Booking*,
/// §7.2): D = 30 days for `SCH.RUL.05`, H = 2 hours for `SCH.RUL.06`, the
/// reminder 24 hours before (`SCH.RUL.01`), the workshop manager told of an
/// open conflict, and the workshop reason codes. Any other client keeps the
/// module's defaults.
pub fn scheduling_for() -> WlScheduling {
    let mut module = WlScheduling::new()
        .look_ahead_days(seed::BOOKING_LOOK_AHEAD_DAYS)
        .escalation_hours(seed::ESCALATION_HOURS)
        .reminder_hours(seed::REMINDER_HOURS)
        .manager_role(WORKSHOP_MANAGER_ROLE);
    for (code, initiated_by) in auto::reasons::SCHEDULING_REASON_CODES {
        module = module.reason_code(code, *initiated_by);
    }
    module
}

/// The HR module with the Auto client's values: the workshop manager is the
/// line manager of a technician and the owner role of the client the second
/// person for waiving and reopening a step; the branch manager is told of
/// the rest (`HRM.ONB.01`).
pub fn hr(module: wl_hr::WlHr, owner_role: &str) -> wl_hr::WlHr {
    module
        .manager_role(BRANCH_MANAGER_ROLE)
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

/// Stores what the Auto client's modules lack, for the modules the manifest
/// enables: the service types (by code), each requiring check-in and not
/// self-bookable yet (`SCH.SLF.01` is Later for 08), and the ticket
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
        // Check-in opens the job card (08 §7.1 *Booking*); a workshop service
        // takes a bay (`SCH.RES.01`).
        service.check_in_required = true;
        service.required_types = "bay".to_owned();
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

/// The module that seeds the Auto client's values: the seeder runs after the
/// shared modules' own (it is mounted after them), and the command
/// `auto:install` runs the same install.
pub struct AutoSeed;

impl Module for AutoSeed {
    fn name(&self) -> &'static str {
        "auto_seed"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "auto:install",
            "Store the Auto service types and ticket categories, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = install(&state).await?;
                println!("{} Auto defaults created.", installed.total());
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { install(&state).await.map(|_| ()) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::documents::OverrideMode;

    fn guarded(kind: &KindSeed) -> Vec<(&str, BlockMode)> {
        kind.blocks
            .iter()
            .map(|b| (b.guarded.as_str(), b.mode))
            .collect()
    }

    #[test]
    fn the_police_report_is_a_legal_block_of_the_approvals_and_never_overridden() {
        let kind = police_report_kind();
        assert_eq!(
            guarded(&kind),
            [
                ("job_card.approve", BlockMode::Block),
                ("insurance_claim.approve", BlockMode::Block),
                ("insurance_claim.submit_estimate", BlockMode::Block),
            ]
        );
        assert_eq!(kind.override_mode, OverrideMode::Never);
        assert!(kind.legal_block);
        assert_eq!(
            kind.responsible_role.as_deref(),
            Some("insurance_coordinator")
        );
        assert_eq!(
            kind.subject_types,
            ["auto.job_card", "auto.insurance_claim"]
        );
    }

    #[test]
    fn the_claim_documents_are_personal_data_of_the_coordinator() {
        for kind in [driving_licence_kind(), mulkiya_kind(), emirates_id_kind()] {
            assert_eq!(kind.sensitivity, Sensitivity::Personal);
            assert!(kind.blocks.is_empty());
            assert_eq!(
                kind.responsible_role.as_deref(),
                Some("insurance_coordinator")
            );
        }
    }

    #[test]
    fn the_licence_and_the_fleet_contract_only_remind() {
        for kind in [customer_licence_kind(), fleet_contract_kind()] {
            assert!(kind.blocks.is_empty());
            assert!(kind.has_expiry);
            assert_eq!(kind.reminder_offsets, [90, 60, 30, 7]);
        }
        assert_eq!(
            customer_licence_kind().responsible_role.as_deref(),
            Some("accountant")
        );
        assert_eq!(
            fleet_contract_kind().responsible_role.as_deref(),
            Some("owner")
        );
    }
}
