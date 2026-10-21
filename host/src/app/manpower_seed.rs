//! The Manpower client's values for the shared modules (09 Manpower §3, §4,
//! §7.2, §8, §9; BL-MPW-14, BL-MPW-17, BL-MPW-20, BL-MPW-21).
//!
//! The values are `manpower::seed`; this file turns them into the owning
//! modules' own calls: the builders of the money, attendance, HR and sales
//! modules ([`invoicing`], [`payments`], [`attendance`], [`hr`], [`sales`]),
//! the document kinds with their `blocks` ([`register_documents`]), the
//! integration fallbacks ([`register_integrations`]) and one idempotent seed
//! ([`ManpowerSeed`], command `manpower:install`): the ticket categories
//! (conduct, absence, grievance), the lead sources, the onboarding checklists,
//! the SOP library, the budget categories and the give-up reactions of the
//! notification types of the shared modules. It adds what the database lacks
//! and keeps what the client changed, so a re-run changes nothing.
//!
//! Nothing here is a rule: the rules run in the modules with these values.
//! What the modules do not expose yet is listed in `manpower::seed`.

use std::sync::Arc;

use manpower::seed::{
    self, BUDGET_CATEGORIES, CATEGORIES, Guards, KindSpec, ONBOARDING, OnboardingSpec, Override,
    SOPS,
};
use renox::chrono::Weekday;
use renox::db::Json;
use renox::prelude::*;
use wl_documents::registrations::{OTHER, TRADEMARK, VerticalSettings, register_vertical};
use wl_documents::{KindSeed, Sop, SopReader, register_kind};
use wl_hr::{OnboardingTemplate, StepInput, TemplateInput};
use wl_invoicing::budget_category::{self, CategoryInput};
use wl_invoicing::{BudgetCategory, BudgetKind, MappingKind, WlInvoicing};
use wl_kernel::Manifest;
use wl_kernel::documents::{BlockCondition, BlockMode, OverrideMode};
use wl_notify::NotificationType;
use wl_payments::WlPayments;
use wl_sales::{LeadSource, WlSales};
use wl_ticketing::config::InvoiceOn;
use wl_ticketing::{Category, Priority, SlaPolicy};

/// The client app these values are for.
pub const MANPOWER: &str = "manpower";

/// `module` with the Manpower money settings: the MD approves every write-off
/// (limit AED 0), the client approver may dispute an invoice in the portal and a
/// dispute is escalated after 14 days, bad-debt relief on, the forecast alert
/// below AED 250,000 for the MD and the accountant at 06:00 over 13 weeks from
/// Monday with the due dates moved by each client's days late and disputed
/// amounts left out, the budget variance told to the accountant, and no drawing
/// approved by its requester.
pub fn invoicing(module: WlInvoicing, entities: &[String]) -> WlInvoicing {
    let mut module = module
        .write_off_approvers(&seed::WRITE_OFF_APPROVERS)
        .portal_disputes(seed::PORTAL_DISPUTES)
        .dispute_escalation_days(seed::DISPUTE_ESCALATION_DAYS)
        .bad_debt_relief(seed::BAD_DEBT_RELIEF)
        .cash_forecast_weeks(seed::CASH_FORECAST_WEEKS)
        .cash_week_start(Weekday::Mon)
        .cash_days_late(seed::CASH_DAYS_LATE)
        .cash_disputed_counted(seed::CASH_DISPUTED_COUNTED)
        .cash_alert_time(seed::CASH_ALERT_TIME)
        .cash_alert_roles(&seed::CASH_ALERT_ROLES)
        .budget_variance_percent(seed::BUDGET_VARIANCE_PERCENT)
        .budget_variance_role(seed::BUDGET_CATEGORY_OWNER)
        .drawings_self_approve(seed::DRAWINGS_SELF_APPROVE);
    for code in entities {
        module = module.cash_threshold(code, seed::CASH_THRESHOLD);
    }
    module
}

/// `module` with the Manpower cheque settings: two presentations, the chase at
/// 3 and 14 days, the accountant told of a bounce and deciding. No bounce fee
/// is set (`⚠ open: lawyer L-22`).
pub fn payments(module: WlPayments) -> WlPayments {
    module
        .cheque_presentations(seed::CHEQUE_PRESENTATIONS)
        .cheque_chase(seed::BOUNCE_REMIND_DAYS, seed::BOUNCE_DECIDE_DAYS)
        .cheque_manager(seed::CHEQUE_MANAGER_ROLE, true)
}

/// The attendance module with the Manpower settings: a supervisor enters
/// yesterday's paper attendance (one day back), 2 hours overtime a day and 144
/// hours in three weeks, the work permit and the residence visa checked, the
/// client's escalation at 6 days, HR told of a flag.
pub fn attendance(module: wl_attendance::WlAttendance) -> wl_attendance::WlAttendance {
    use wl_attendance::config::PayrollBasis;
    use wl_attendance::timesheet::ApproverKind;
    module
        // Salary never waits for the client: the basis is `attendance` and
        // `wait` is refused for an external approver.
        .payroll_basis(ApproverKind::External, PayrollBasis::Attendance)
        .expect("attendance is always accepted as the basis")
        .late_entry_days(seed::LATE_ENTRY_DAYS)
        .hr_role(seed::HR)
        .overtime_cap(seed::OVERTIME_PER_DAY, seed::OVERTIME_PER_WINDOW)
        .permit_check(true)
        .permit_kinds(&seed::PERMIT_KINDS)
        .escalation_days(seed::ESCALATION_DAYS)
}

/// The HR module with the Manpower settings: the owner role of the client is the
/// MD, told of a WPS rejection a day before the due date, absconding after more
/// than 7 days, a pay account cooling for 5 days, the settlement alert 10 days
/// after the last working day, and the payroll cut-off on day 25.
pub fn hr(module: wl_hr::WlHr, owner_role: &str) -> wl_hr::WlHr {
    module
        .owner_role(owner_role)
        .md_role(owner_role)
        .hr_role(seed::HR)
        .accountant_role(seed::ACCOUNTANT)
        .absence_days(seed::ABSENCE_DAYS)
        .wps_escalation_days(seed::WPS_ESCALATION_DAYS)
        .cooling_days(seed::COOLING_DAYS)
        .settlement_alert_days(seed::SETTLEMENT_ALERT_DAYS)
        .cut_off_day(seed::CUT_OFF_DAY)
}

/// The sales module with the Manpower settings: a cost entry over a campaign's
/// budget needs a manager, no source requires a campaign.
pub fn sales(module: WlSales) -> WlSales {
    module
        .over_budget_approval(seed::OVER_BUDGET_APPROVAL)
        .campaign_required_for(&[])
}

/// The document kind `spec` as wl-documents' seed.
pub fn kind_seed(spec: &KindSpec) -> KindSeed {
    let mut kind = KindSeed::new(spec.key, spec.name_en)
        .name_ar(spec.name_ar)
        .responsible_role(spec.role)
        .also_notify(spec.also_notify)
        .override_mode(match spec.override_mode {
            Override::Never => OverrideMode::Never,
            Override::WithApproval => OverrideMode::WithApproval,
        });
    for subject in spec.subjects {
        kind = kind.subject_type(subject);
    }
    if !spec.reminders.is_empty() {
        kind = kind.expires().numbered().reminders(spec.reminders);
    }
    if spec.look_ahead > 0 {
        kind = kind.look_ahead(spec.look_ahead);
    }
    if spec.guards != Guards::Nothing && spec.override_mode == Override::Never {
        kind = kind.legal_block();
    }
    let condition = BlockCondition::new("role", seed::SECURITY_ROLES);
    for guarded in seed::DEPLOYMENT_GUARDS
        .into_iter()
        .chain([seed::ATTENDANCE_ACCEPT])
    {
        let mode = if guarded == seed::ATTENDANCE_ACCEPT {
            BlockMode::Flag
        } else {
            BlockMode::Block
        };
        kind = match spec.guards {
            Guards::Nothing => kind,
            Guards::Everyone => kind.blocks(guarded, mode),
            Guards::SecurityRoles => kind.blocks_when(guarded, mode, condition.clone()),
        };
    }
    kind
}

/// Registers the Manpower kinds with wl-documents and turns on the two
/// registration kinds 09 uses with the MD as responsible role.
pub fn register_documents() {
    for spec in seed::KINDS.iter().chain([&seed::SIGNED_TIMESHEET]) {
        register_kind(kind_seed(spec));
    }
    let mut settings = VerticalSettings::new().enabled(&[TRADEMARK, OTHER]);
    for (key, role) in seed::REGISTRATION_KINDS {
        settings = settings.role(key, role);
    }
    register_vertical(MANPOWER, settings);
}

/// Registers the connections 09 gives a manual fallback (§8): the SMS gateway,
/// the bank or exchange house of WPS and the attendance import. (The ASP
/// connection of each legal entity is registered by wl-invoicing from the
/// entity's e-invoicing go-live date, which is the client's.)
pub fn register_integrations(client_app: &str) {
    use wl_kernel::integrations::{Delivery, IntegrationDef};
    if client_app != MANPOWER {
        return;
    }
    for (key, module, owner, screens, en, ar) in seed::CONNECTIONS {
        IntegrationDef::of(key, module, |_state, _call| {
            Box::pin(async { Ok(Delivery::Sent) })
        })
        .owner_role(owner)
        .screens(screens.iter().copied())
        .fallback(en, Some(ar))
        .register();
    }
}

/// Gives HR and the accountant `tickets.confidential.view` (and the right to
/// see tickets) in a manpower deployment: the *labour case* of a final
/// settlement and the worker's *grievance* are confidential, visible to HR, the
/// accountant and the MD (the company admin has it) and not to the account
/// manager or the site supervisor (`WRK.ACL.03`, 09 §3.2). The roles table is
/// shared by every vertical, where the accountant of an accounting firm must
/// not see its AML cases, so this is a grant of this client only.
pub fn grant_roles(client_app: &str) {
    if client_app != MANPOWER {
        return;
    }
    for role in [seed::HR, seed::ACCOUNTANT] {
        super::roles::grant_extra(
            role,
            &[
                wl_ticketing::permissions::VIEW,
                wl_ticketing::permissions::CONFIDENTIAL_VIEW,
            ],
        );
    }
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
    /// Budget categories created or given an owner role.
    pub budget_categories: usize,
    /// Notification types given their 09 reaction.
    pub notification_types: usize,
}

impl Installed {
    /// Everything created or changed.
    pub fn total(&self) -> usize {
        self.categories
            + self.sources_off
            + self.checklists
            + self.sops
            + self.budget_categories
            + self.notification_types
    }
}

/// Stores what the Manpower client's modules lack, for the modules the manifest
/// enables.
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
    if on("notify") {
        installed.notification_types = install_notification_types(&state.db).await?;
    }
    Ok(installed)
}

/// The ticket categories of 09: the conduct complaint, the absence case of HR
/// and the worker's grievance (confidential).
async fn install_categories(db: &Db) -> Result<usize> {
    let policy = SlaPolicy::by_key(db, wl_ticketing::seed::STANDARD_POLICY).await?;
    let mut created = 0;
    for spec in &CATEGORIES {
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
                confidential: spec.confidential,
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

/// The budget categories of 09: the visa and insurance costs, accommodation and
/// office are added, and the accountant owns every category that has no owner
/// role yet. The mapping to the client's export accounts is the client's.
async fn install_budget_categories(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for spec in &BUDGET_CATEGORIES {
        if BudgetCategory::by_code(db, spec.code).await?.is_some() {
            continue;
        }
        budget_category::save(
            db,
            None,
            CategoryInput {
                code: spec.code.to_owned(),
                name_en: spec.name_en.to_owned(),
                name_ar: Some(spec.name_ar.to_owned()),
                kind: BudgetKind::Expense,
                parent_code: None,
                owner_role: Some(seed::BUDGET_CATEGORY_OWNER.to_owned()),
                owner_party_id: None,
                mappings: spec
                    .cost_codes
                    .iter()
                    .map(|code| (MappingKind::CostCode, (*code).to_owned()))
                    .collect(),
                active: true,
            },
        )
        .await?;
        changed += 1;
    }
    for mut category in BudgetCategory::listed(db, true).await? {
        if category.owner_role.is_none() && !category.is_drawings() {
            category.owner_role = Some(seed::BUDGET_CATEGORY_OWNER.to_owned());
            category.save(db).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Turns off the seeded lead sources 09 does not use.
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

/// The checklist `spec` as wl-hr's input; it applies to each of its job roles.
fn template_input(spec: &OnboardingSpec) -> TemplateInput {
    let mut input = TemplateInput::for_role(spec.name, spec.role);
    input.job_roles = spec
        .role
        .split(',')
        .map(|role| role.trim().to_owned())
        .collect();
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

/// Stores the SOPs whose code is not there yet, as drafts with their owner
/// role and required readers.
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

/// Gives the notification types of the shared modules their 09 group, statutory
/// mark, give-up reactions and staff role (`seed::NOTIFICATION_TYPES`). A type
/// the client edited is kept as it is; the write leaves `updated_at` alone, so a
/// second run changes nothing. (The types 09 registers itself carry their own.)
async fn install_notification_types(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for spec in &seed::NOTIFICATION_TYPES {
        let Some(row) = NotificationType::by_key(db, spec.key).await? else {
            continue;
        };
        let same = row.group_key == spec.group
            && row.statutory == spec.statutory
            && row.give_up.0 == spec.give_up
            && row.staff_role.as_deref() == spec.role;
        if same || !row.is_unedited() {
            continue;
        }
        let give_up = Json(spec.give_up.to_vec());
        let role = spec.role.map(str::to_owned);
        NotificationType::query()
            .where_(NotificationType::ID.eq(row.id))
            .update(
                db,
                &[
                    ("group_key", &spec.group.to_owned()),
                    ("statutory", &spec.statutory),
                    ("give_up", &give_up),
                    ("staff_role", &role),
                ],
            )
            .await?;
        changed += 1;
    }
    Ok(changed)
}

/// The module that seeds the Manpower client's values: the seeder runs after
/// the shared modules' own (it is mounted after them), and the command
/// `manpower:install` runs the same install.
pub struct ManpowerSeed;

impl Module for ManpowerSeed {
    fn name(&self) -> &'static str {
        "manpower_seed"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "manpower:install",
            "Store the Manpower ticket categories, onboarding checklists, SOP library, budget categories and notification reactions, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = install(&state).await?;
                println!("{} Manpower defaults created.", installed.total());
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
    fn the_money_builders_carry_the_spec_values() {
        let entities = vec!["GMS".to_owned()];
        let module = invoicing(WlInvoicing::new(), &entities);
        let config = module.config();
        assert_eq!(config.write_off_approvers, ["admin"]);
        assert!(config.portal_disputes);
        assert_eq!(config.dispute_escalation_days, 14);
        assert_eq!(config.cash_threshold_of("GMS"), 25_000_000);
        assert_eq!(config.cash_forecast_weeks, 13);
        assert_eq!(config.cash_alert_roles, ["admin", "accountant"]);
        assert!(config.cash_days_late);
        assert!(!config.cash_disputed_counted);
        assert!(!config.drawings_self_approve);
        let module = payments(WlPayments::new());
        let config = module.config();
        assert_eq!(config.cheque_max_presentations, 2);
        assert_eq!(
            (config.cheque_reminder_days, config.cheque_decision_days),
            (3, 14)
        );
        assert!(config.bounce_fee.is_none(), "no bounce fee is seeded");
    }

    #[test]
    fn the_labour_builders_carry_the_spec_values() {
        let module = attendance(wl_attendance::WlAttendance::new());
        let config = module.config();
        assert_eq!(config.late_entry_days, 1);
        assert_eq!(config.overtime_day_minutes, 120);
        assert_eq!(config.overtime_window_minutes, 144 * 60);
        assert_eq!(config.escalation_days, 6);
        assert!(config.permit_check);
        assert_eq!(config.permit_kinds, ["work_permit", "residence_visa"]);
        let module = hr(wl_hr::WlHr::new(), "owner");
        let config = module.config();
        assert_eq!(config.cut_off_day, 25);
        assert_eq!(config.cooling_days, 5);
        assert_eq!(config.settlement_alert_days, 10);
        assert_eq!(config.absence_days, 7);
        assert_eq!(config.wps_escalation_days, 1);
    }

    #[test]
    fn the_blocking_kinds_guard_the_deployment_and_flag_the_attendance() {
        for spec in seed::KINDS.iter().filter(|k| k.guards != Guards::Nothing) {
            let kind = kind_seed(spec);
            assert_eq!(kind.blocks.len(), 4, "{}", spec.key);
            assert!(
                kind.blocks
                    .iter()
                    .any(|b| b.guarded == "deployment.extend" && b.mode == BlockMode::Block)
            );
            assert!(
                kind.blocks
                    .iter()
                    .any(|b| b.guarded == "attendance_record.accept" && b.mode == BlockMode::Flag)
            );
            assert_eq!(kind.look_ahead_days, 30);
        }
        assert!(kind_seed(&seed::KINDS[0]).legal_block);
        assert_eq!(
            kind_seed(&seed::KINDS[3]).override_mode,
            OverrideMode::WithApproval
        );
    }

    #[test]
    fn every_checklist_step_is_valid_for_wl_hr() {
        for spec in &ONBOARDING {
            let input = template_input(spec);
            assert_eq!(input.steps.len(), spec.steps.len());
            for step in &input.steps {
                assert!(wl_hr::template::is_valid_owner(&step.owner));
            }
        }
    }
}
