//! The Property client's values for the shared compliance, document, notice,
//! sales and staff mechanisms (01 Property §4.5, §7.2, §9; BL-PRP-27).
//!
//! The tables are `property::seed`; the modules keep their settings as builder
//! calls on the module and as seeds, as for the money settings
//! ([`super::negative_flow`]). This file turns the tables into:
//!
//! - the document kinds registered with wl-documents ([`register_documents`]),
//!   among them the Civil Defence approval as a legal block, and the
//!   registration kinds on for 01;
//! - the ticketing settings ([`ticketing_for`]): no-show after 2 hours, a
//!   visit missed after 3 days, the coordinator and the property manager;
//! - the sales, HR and notification settings ([`sales_for`], [`hr_for`]);
//! - the seed ([`PropertySeed`], command `property:install`): the ticket
//!   categories, the lead sources 01 uses, the onboarding checklists and the
//!   SOP library. It adds what the database lacks and keeps what the client
//!   changed, so a re-run changes nothing.
//!
//! Where a value is the module's own default (no-show hours, the SOP reminder
//! days, the onboarding reminder days), the settings page says so.

use std::sync::Arc;

use property::seed::{
    self, BlockSpec, CATEGORIES, KINDS, KindSpec, Mode, ONBOARDING, OnboardingSpec, SOPS,
};
use renox::prelude::*;
use wl_documents::registrations::{OTHER, TRADEMARK, VerticalSettings, register_vertical};
use wl_documents::{KindSeed, Sensitivity, Sop, SopReader, register_kind};
use wl_hr::{OnboardingTemplate, StepInput, TemplateInput};
use wl_invoicing::BudgetCategory;
use wl_kernel::Manifest;
use wl_kernel::documents::{BlockCondition, BlockMode};
use wl_sales::{LeadSource, WlSales};
use wl_ticketing::config::InvoiceOn;
use wl_ticketing::{Category, Priority, SlaPolicy, WlTicketing};

use super::property_work;
use super::roles::{ACCOUNTANT, PROPERTY_MANAGER};

/// The client app these values are for.
const PROPERTY: &str = "property";

/// The document kind `spec` as wl-documents' seed.
pub fn kind_seed(spec: &KindSpec) -> KindSeed {
    let mut seed = KindSeed::new(spec.key, spec.name_en)
        .name_ar(spec.name_ar)
        .responsible_role(spec.role);
    for subject in spec.subjects {
        seed = seed.subject_type(subject);
    }
    if spec.expires {
        seed = seed.expires();
    }
    if spec.numbered {
        seed = seed.numbered();
    }
    if !spec.reminders.is_empty() {
        seed = seed.reminders(spec.reminders);
    }
    if !spec.also_notify.is_empty() {
        seed = seed.also_notify(spec.also_notify);
    }
    if spec.personal {
        seed = seed.sensitivity(Sensitivity::Personal);
    }
    if spec.notify_party {
        seed = seed.notify_party();
    }
    if spec.critical {
        seed = seed.compliance_critical();
    }
    if spec.legal_block {
        seed = seed.legal_block();
    }
    if spec.look_ahead > 0 {
        seed = seed.look_ahead(spec.look_ahead);
    }
    for block in spec.blocks {
        seed = with_block(seed, block);
    }
    seed
}

/// `seed` with the entry `block`.
fn with_block(seed: KindSeed, block: &BlockSpec) -> KindSeed {
    let mode = match block.mode {
        Mode::Block => BlockMode::Block,
        Mode::Flag => BlockMode::Flag,
    };
    match block.condition {
        Some((field, values)) => seed.blocks_when(
            block.guarded,
            mode,
            BlockCondition::new(field, values.iter().copied()),
        ),
        None => seed.blocks(block.guarded, mode),
    }
}

/// Registers the Property kinds with wl-documents, and turns on the two
/// registration kinds 01 uses (*Trademark registration*, *Other
/// certification*; the responsible role is the owner role, the company admin
/// by default).
pub fn register_documents(client_app: &str) {
    if client_app != PROPERTY {
        return;
    }
    for spec in KINDS {
        register_kind(kind_seed(spec));
    }
    register_vertical(
        PROPERTY,
        VerticalSettings::new().enabled(&[TRADEMARK, OTHER]),
    );
    property_work::register();
}

/// The ticketing module with the Property client's settings: a vendor that has
/// not started 2 hours after the visit time is a no-show (`WRK.RUL.19`), a
/// planned visit is missed 3 days after its due date (`WRK.RUL.14`), the
/// coordinator is told, and the property manager for the compliance-critical
/// categories. The reassignment reasons `vendor_no_show`, `contract_ended` and
/// `vendor_document_expired` are the module's own defaults.
pub fn ticketing_for(client_app: &str) -> WlTicketing {
    let module = WlTicketing::new();
    match client_app {
        PROPERTY => module
            .no_show_hours(seed::NO_SHOW_HOURS)
            .visit_missed_days(seed::VISIT_MISSED_DAYS)
            .coordinator_role(seed::COORDINATOR_ROLE)
            .compliance_manager_role(PROPERTY_MANAGER),
        // 04 `ACC.RUL.04`: the nightly overdue run under the 04 rule id,
        // telling the assignee, the manager and the partner; a started task
        // is not waived but listed for the manager (§7 *waive a started
        // task*: off); `close_missed` takes a reason from the §7.1 list.
        "accounting" => {
            let mut module = module
                .overdue_rule_id(accounting::tasks::OVERDUE_RULE)
                .task_manager_role(accounting::permissions::MANAGER_ROLE)
                .task_overdue_role(accounting::permissions::PARTNER_ROLE)
                .waive_started_tasks(false)
                .waiting_label("Waiting on client");
            for (code, name, needs_text) in accounting::tasks::missed_reason_names() {
                module = if needs_text {
                    module.missed_reason_with_text(code, &name)
                } else {
                    module.missed_reason(code, &name)
                };
            }
            module
        }
        _ => module,
    }
}

/// The sales module with the Property client's settings: a cost entry that
/// takes a campaign over its budget needs a manager (`SAL.ACL.07`), and no
/// lead source requires a campaign.
pub fn sales_for(client_app: &str) -> WlSales {
    let module = WlSales::new();
    match client_app {
        PROPERTY => module.over_budget_approval(true).campaign_required_for(&[]),
        super::construction_seed::CONSTRUCTION => super::construction_seed::sales(module),
        "accounting" => super::accounting_seed::sales_for(client_app),
        super::clinics_seed::CLINICS => super::clinics_seed::sales(module),
        _ => module,
    }
}

/// The HR module with the Property client's settings: the line manager of a
/// staff member is the property manager of their building, and the owner role
/// of the client is the second person for waiving and reopening a step.
pub fn hr_for(client_app: &str, owner_role: &str) -> wl_hr::WlHr {
    let module = wl_hr::WlHr::new();
    match client_app {
        PROPERTY => module.manager_role(PROPERTY_MANAGER).owner_role(owner_role),
        super::construction_seed::CONSTRUCTION => super::construction_seed::hr(module, owner_role),
        "accounting" => super::accounting_seed::hr_for(client_app, owner_role),
        super::clinics_seed::CLINICS => super::clinics_seed::hr(module, owner_role),
        _ => module,
    }
}

/// The attendance module with the client's settings: HR and the HSE officer
/// are the roles of the roles module; the Property client has no construction
/// crews, so the defaults of the module apply.
pub fn attendance_for(client_app: &str) -> wl_attendance::WlAttendance {
    if client_app == super::construction_seed::CONSTRUCTION {
        return super::construction_seed::attendance(wl_attendance::WlAttendance::new());
    }
    wl_attendance::WlAttendance::new().hr_role(super::roles::HR)
}

/// What [`install`] stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Ticket categories created.
    pub categories: usize,
    /// Lead sources turned off.
    pub sources_off: usize,
    /// Onboarding checklists created.
    pub checklists: usize,
    /// SOPs created.
    pub sops: usize,
    /// Budget categories given the accountant as owner role.
    pub budget_categories: usize,
}

impl Installed {
    /// Everything created or changed.
    pub fn total(&self) -> usize {
        self.categories + self.sources_off + self.checklists + self.sops + self.budget_categories
    }
}

/// Stores what the Property client's modules lack, for the modules the
/// manifest enables.
///
/// # Errors
///
/// The database's errors.
pub async fn install(state: &AppState) -> Result<Installed> {
    let manifest = state.provided::<Arc<Manifest>>();
    let on = |name: &str| manifest.as_ref().is_some_and(|m| m.is_enabled(name));
    let mut installed = Installed::default();
    if on("ticketing") {
        installed.categories = install_categories(&state.db).await?;
    }
    if on("sales") {
        installed.sources_off = turn_off_unused_sources(&state.db).await?;
    }
    if on("hr") {
        installed.checklists = install_checklists(&state.db).await?;
    }
    if on("documents") {
        installed.sops = install_sops(&state.db).await?;
    }
    if on("invoicing") {
        installed.budget_categories = install_budget_categories(&state.db).await?;
    }
    Ok(installed)
}

/// The ticket categories of 01: fire and life safety and lifts
/// (compliance-critical, their visits only reassigned at the end of a
/// contract, high priority) and the three the earlier issues fall back from.
async fn install_categories(db: &Db) -> Result<usize> {
    let policy = SlaPolicy::by_key(db, wl_ticketing::seed::STANDARD_POLICY).await?;
    let mut created = 0;
    for spec in CATEGORIES {
        if Category::by_key(db, spec.key).await?.is_some() {
            continue;
        }
        Category::create(
            db,
            Category {
                key: spec.key.to_owned(),
                name_en: spec.name_en.to_owned(),
                name_ar: spec.name_ar.to_owned(),
                default_priority: if spec.critical {
                    Priority::High
                } else {
                    Priority::Normal
                },
                sla_policy_id: policy.as_ref().map(|p| p.id),
                requires_signoff: false,
                active: true,
                compliance_critical: spec.critical,
                contract_end_reassign_only: spec.critical,
                // A chargeable job is invoiced to the tenant when the ticket
                // closes, not when it is resolved (BL-PRP-26 item 7).
                chargeable: spec.chargeable,
                invoice_on: if spec.chargeable {
                    InvoiceOn::Close
                } else {
                    InvoiceOn::default()
                },
                ..Category::default()
            },
        )
        .await?;
        created += 1;
    }
    Ok(created)
}

/// The budget categories of 01 (`FIN.BUD.01`, BL-PRP-26 item 26): the
/// accountant owns every category that has no owner role yet, so `FIN.RUL.24`
/// warns the accountant of the line (the module's own owner roles, such as the
/// marketing manager of *marketing*, are kept). The mapping to the client's
/// export accounts (`FIN.EXT.01`) is the client's: it depends on the chart of
/// accounts of its accounting tool.
async fn install_budget_categories(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for mut category in BudgetCategory::listed(db, true).await? {
        if category.owner_role.is_none() && !category.is_drawings() {
            category.owner_role = Some(ACCOUNTANT.to_owned());
            category.save(db).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Turns off the lead sources 01 does not use (the module's *aggregator*).
async fn turn_off_unused_sources(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for mut source in LeadSource::ordered(db).await? {
        let used = seed::LEAD_SOURCES.contains(&source.code.as_str());
        if source.seeded && !used && source.active {
            source.active = false;
            source.save(db).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// The step of an onboarding checklist as wl-hr's input.
fn step_input(step: &seed::StepSpec) -> StepInput {
    let (title, owner, due_days, kind) = *step;
    if kind == "sops" {
        StepInput::sops(title, owner, due_days)
    } else if let Some(template) = kind.strip_prefix("checklist:") {
        StepInput::checklist(title, owner, due_days, template)
    } else {
        StepInput::task(title, owner, due_days)
    }
}

/// The checklist `spec` as wl-hr's input: for its job role, completion blocks
/// nothing (the role is active before it is complete).
fn template_input(spec: &OnboardingSpec) -> TemplateInput {
    let mut input = TemplateInput::for_role(spec.name, spec.role);
    for step in spec.steps {
        input = input.step(step_input(step));
    }
    input
}

/// Stores the checklists whose name is not there yet.
async fn install_checklists(db: &Db) -> Result<usize> {
    let mut created = 0;
    for spec in &ONBOARDING {
        let found = OnboardingTemplate::query()
            .where_(OnboardingTemplate::NAME_EN.eq(spec.name))
            .count(db)
            .await?;
        if found > 0 {
            continue;
        }
        match wl_hr::template::create(db, template_input(spec)).await? {
            Ok(_) => created += 1,
            Err(refused) => {
                return Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    refused.message(),
                ));
            }
        }
    }
    Ok(created)
}

/// Stores the SOPs of the library whose code is not there yet, as drafts with
/// their owner role and required readers. A version is attached by the owner;
/// the acknowledgements start when it is accepted (`DOC.RUL.04`).
async fn install_sops(db: &Db) -> Result<usize> {
    let mut created = 0;
    for spec in &SOPS {
        let found = Sop::query()
            .where_(Sop::CODE.eq(spec.code))
            .count(db)
            .await?;
        if found > 0 {
            continue;
        }
        let sop = Sop::create(
            db,
            Sop {
                code: spec.code.to_owned(),
                title_en: spec.title.to_owned(),
                category: spec.category.to_owned(),
                owner_role: spec.owner_role.to_owned(),
                review_interval_months: wl_documents::sops::service::DEFAULT_REVIEW_INTERVAL_MONTHS,
                ack_due_days: wl_documents::sops::service::DEFAULT_ACK_DUE_DAYS,
                ..Sop::default()
            },
        )
        .await?;
        for role in spec.readers {
            SopReader::create(
                db,
                SopReader {
                    sop_id: sop.id,
                    role_key: (*role).to_owned(),
                    ..SopReader::default()
                },
            )
            .await?;
        }
        created += 1;
    }
    Ok(created)
}

/// The module that seeds the Property client's values: the seeder runs after
/// the shared modules' own (it is mounted after them), and the command
/// `property:install` runs the same install.
pub struct PropertySeed;

impl Module for PropertySeed {
    fn name(&self) -> &'static str {
        "property_seed"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "property:install",
            "Store the Property ticket categories, onboarding checklists and SOP library, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = install(&state).await?;
                println!("{} Property defaults created.", installed.total());
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
    fn every_kind_becomes_a_seed_with_its_blocks() {
        for spec in KINDS {
            let seed = kind_seed(spec);
            assert_eq!(seed.key, spec.key);
            assert_eq!(seed.blocks.len(), spec.blocks.len());
            assert_eq!(seed.responsible_role.as_deref(), Some(spec.role));
        }
    }

    #[test]
    fn the_civil_defence_seed_is_a_legal_block() {
        let spec = KINDS
            .iter()
            .find(|k| k.key == seed::CIVIL_DEFENCE_APPROVAL)
            .expect("the Civil Defence approval");
        let seed = kind_seed(spec);
        assert!(seed.legal_block && seed.compliance_critical);
        assert_eq!(
            seed.override_mode,
            wl_kernel::documents::OverrideMode::Never
        );
        assert_eq!(seed.look_ahead_days, 30);
        assert!(seed.blocks.iter().all(|b| b.mode == BlockMode::Block));
    }

    /// The ticketing values are read by running the rules of the module under
    /// the Property client: a lifts visit is reported as missed on the third
    /// day after its due date and not on the second (`WRK.RUL.14`, it raises
    /// the compliance flag `visit_missed`), and a vendor that has not started
    /// is a no-show at two hours and not at one hour and 59 minutes
    /// (`WRK.RUL.19`). The module has no getters: only its behaviour shows
    /// that the values are in force.
    #[renox::test]
    async fn the_ticketing_settings_are_the_property_values() {
        use renox::chrono::{Days, Duration};
        use renox::testing::TestApp;
        use wl_kernel::documents::today;
        use wl_kernel::tickets::{Assignee, OpenTicket};
        use wl_kernel::{PartyKind, Subject};
        use wl_ticketing::flags::ComplianceFlag;
        use wl_ticketing::{NoShow, Ticket, Tickets, no_show, visit_check};

        let app = TestApp::new(crate::app()).await;
        app.kernel()
            .call("wl:install", Vec::<String>::new())
            .await
            .unwrap();
        app.kernel().seed().await.unwrap();
        let vendor = wl_parties::create(
            app.state(),
            wl_parties::NewParty {
                kind: PartyKind::Company,
                legal_name_en: "LiftCo LLC".into(),
                company: Some(wl_parties::CompanyDetails::default()),
                source: "back_office".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .party_id();
        let building = Subject::new("property.building_system", 4);
        let due = today();
        let visit = Tickets::open_at(
            app.state(),
            OpenTicket::new(building.clone(), seed::LIFTS, "Quarterly service")
                .planned_due(due)
                .assign_to(Assignee::party(vendor)),
            renox::db::now(),
        )
        .await
        .unwrap();

        // Two days after the due date: not missed yet. Three: missed.
        let second_day = due + Days::new(seed::VISIT_MISSED_DAYS as u64 - 1);
        visit_check::run(app.state(), second_day).await.unwrap();
        assert!(ComplianceFlag::open(app.db()).await.unwrap().is_empty());
        let third_day = due + Days::new(seed::VISIT_MISSED_DAYS as u64);
        visit_check::run(app.state(), third_day).await.unwrap();
        let flags = ComplianceFlag::open(app.db()).await.unwrap();
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].subject(), building);

        // The same ticket with a visit time: a no-show two hours after it.
        let at = renox::db::now();
        Ticket::where_eq("id", visit.id.0)
            .update(app.db(), &[("visit_at", &Some(at))])
            .await
            .unwrap();
        let early = at + Duration::hours(seed::NO_SHOW_HOURS) - Duration::minutes(1);
        assert_eq!(no_show::run(app.state(), early).await.unwrap().recorded, 0);
        let on_time = at + Duration::hours(seed::NO_SHOW_HOURS);
        assert_eq!(
            no_show::run(app.state(), on_time).await.unwrap().recorded,
            1
        );
        assert_eq!(
            NoShow::query().count(app.db()).await.unwrap(),
            1,
            "recorded once"
        );
    }

    #[test]
    fn every_checklist_step_is_valid_for_wl_hr() {
        for spec in &ONBOARDING {
            let input = template_input(spec);
            assert_eq!(input.steps.len(), spec.steps.len());
            for step in &input.steps {
                assert!(
                    wl_hr::template::is_valid_owner(&step.owner),
                    "{}",
                    step.owner
                );
            }
        }
    }
}
