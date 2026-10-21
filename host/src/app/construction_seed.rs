//! The Construction client's values for the shared money, procurement,
//! labour and compliance mechanisms (02 Construction §3.3, §4.2 to §4.4, §7;
//! BL-CON-17 and BL-CON-18).
//!
//! The values are `construction::seed`; this file turns them into the
//! owning modules' own builder calls ([`invoicing`], [`payments`],
//! [`inventory`], [`attendance`], [`hr`], [`sales`]), the document kinds
//! ([`register_documents`]) and one idempotent seed ([`ConstructionSeed`],
//! command `construction:install`): the ticket categories (`absence`,
//! `snag`, `defect`), the lead sources, the onboarding checklists, the SOP
//! library and the budget category owners. It adds what the database lacks
//! and keeps what the client changed, so a re-run changes nothing.
//!
//! Nothing here is a rule: the rules run in the modules with these values.
//! What the modules do not expose yet is listed in `construction::seed`.

use std::sync::Arc;

use construction::seed::{self, KindSpec, ONBOARDING, OnboardingSpec, SOPS};
use renox::chrono::Weekday;
use renox::prelude::*;
use wl_documents::registrations::{ISO, OTHER, TRADEMARK, VerticalSettings, register_vertical};
use wl_documents::{KindSeed, Sop, SopReader, register_kind};
use wl_hr::{OnboardingTemplate, StepInput, TemplateInput};
use wl_inventory::{ScoreSettings, WlInventory};
use wl_invoicing::{BudgetCategory, WlInvoicing};
use wl_kernel::Manifest;
use wl_kernel::documents::BlockMode;
use wl_payments::WlPayments;
use wl_sales::{LeadSource, WlSales};
use wl_ticketing::config::InvoiceOn;
use wl_ticketing::{Category, Priority, SlaPolicy};

/// The client app these values are for.
pub const CONSTRUCTION: &str = "construction";

/// `module` with the Construction money settings: the MD approves a
/// write-off (limit AED 0), no portal dispute, the dispute is escalated to the
/// project manager after 14 days, bad-debt relief on, the forecast alert
/// below AED 200,000 for the MD and the accountant, due dates moved by days
/// late, disputed amounts left out, no drawing approved by its requester.
pub fn invoicing(module: WlInvoicing, entities: &[String]) -> WlInvoicing {
    let mut module = module
        .write_off_approvers(&seed::WRITE_OFF_APPROVERS)
        .portal_disputes(seed::PORTAL_DISPUTES)
        .dispute_escalation_days(seed::DISPUTE_ESCALATION_DAYS)
        .dispute_escalation_role(seed::DISPUTE_ESCALATION_ROLE)
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
    for source in seed::AUTO_ISSUE_SOURCES {
        module = module.auto_issue_source(source);
    }
    for code in entities {
        module = module.cash_threshold(code, seed::CASH_THRESHOLD);
    }
    module
}

/// `module` with the Construction cheque and payment settings: two
/// presentations, the chase at 3 and 14 days, the project manager told and
/// deciding, method *set-off* on, and no bounce fee.
pub fn payments(module: WlPayments) -> WlPayments {
    module
        .methods(&seed::PAYMENT_METHODS)
        .cheque_presentations(seed::CHEQUE_PRESENTATIONS)
        .cheque_chase(seed::BOUNCE_REMIND_DAYS, seed::BOUNCE_DECIDE_DAYS)
        .cheque_manager(seed::CHEQUE_MANAGER_ROLE, true)
}

/// `module` with the Construction stock settings: returns above AED 10,000,
/// the return chase at 30 days, the PO limits AED 10,000 and 50,000, count
/// variances above AED 1,000, the rejection reasons with photos, the supplier
/// score and the second person for a preferred supplier.
pub fn inventory(module: WlInventory) -> WlInventory {
    let mut module = module
        .return_approval_fils(seed::RETURN_APPROVAL_FILS)
        .return_chase_days(seed::RETURN_CHASE_DAYS)
        .order_limits_fils(&seed::ORDER_LIMITS_FILS)
        .wastage_approval_fils(seed::WASTAGE_APPROVAL_FILS)
        .supplier_score(ScoreSettings {
            threshold: seed::SCORE_THRESHOLD,
            window_months: seed::SCORE_WINDOW_MONTHS,
            grace_days: seed::SCORE_GRACE_DAYS,
            min_receipts: seed::SCORE_MIN_RECEIPTS,
            ..ScoreSettings::default()
        })
        .preferred_change_needs_approval(seed::PREFERRED_CHANGE_NEEDS_APPROVAL);
    for reason in seed::REJECTION_REASONS {
        module = module.rejection_reason(reason, true);
    }
    module
}

/// The attendance module with the Construction settings: 8 + 2 hours a day
/// and 144 overtime hours in three weeks, work permit and residence visa
/// checked (flag, never refuse), supervisors one day back, HR and the HSE
/// officer told.
pub fn attendance(module: wl_attendance::WlAttendance) -> wl_attendance::WlAttendance {
    module
        .late_entry_days(seed::LATE_ENTRY_DAYS)
        .hr_role(seed::HR)
        .hse_role(seed::HSE_OFFICER)
        .overtime_cap(seed::OVERTIME_PER_DAY, seed::OVERTIME_PER_WINDOW)
        .permit_check(true)
        .permit_kinds(&seed::PERMIT_KINDS)
}

/// The HR module with the Construction settings: the line manager is the
/// project manager, the owner role the client's, absconding after 7 days.
pub fn hr(module: wl_hr::WlHr, owner_role: &str) -> wl_hr::WlHr {
    module
        .manager_role(seed::PROJECT_MANAGER)
        .owner_role(owner_role)
        .absence_days(seed::ABSENCE_DAYS)
}

/// The sales module with the Construction settings: a cost entry over a
/// campaign's budget needs a manager, no source requires a campaign.
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
        .expires()
        .numbered()
        .reminders(spec.reminders);
    for subject in spec.subjects {
        kind = kind.subject_type(subject);
    }
    if spec.look_ahead > 0 {
        kind = kind.look_ahead(spec.look_ahead);
    }
    if let Some(guarded) = spec.flags {
        kind = kind.blocks(guarded, BlockMode::Flag);
    }
    kind
}

/// Registers the Construction kinds with wl-documents and turns on the three
/// registration kinds 02 uses with their responsible roles.
pub fn register_documents() {
    for spec in &seed::KINDS {
        register_kind(kind_seed(spec));
    }
    let mut settings = VerticalSettings::new().enabled(&[ISO, TRADEMARK, OTHER]);
    for (key, role) in seed::REGISTRATION_KINDS {
        settings = settings.role(key, role);
    }
    register_vertical(CONSTRUCTION, settings);
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
    /// Budget categories given an owner role.
    pub budget_categories: usize,
}

impl Installed {
    /// Everything created or changed.
    pub fn total(&self) -> usize {
        self.categories + self.sources_off + self.checklists + self.sops + self.budget_categories
    }
}

/// Stores what the Construction client's modules lack, for the modules the
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

/// The ticket categories of 02: `absence` (the absence case of HR), `snag`
/// and `defect`, none of them chargeable.
async fn install_categories(db: &Db) -> Result<usize> {
    let policy = SlaPolicy::by_key(db, wl_ticketing::seed::STANDARD_POLICY).await?;
    let mut created = 0;
    for spec in &seed::CATEGORIES {
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
                active: true,
                chargeable: spec.chargeable,
                invoice_on: InvoiceOn::default(),
                ..Category::default()
            },
        )
        .await?;
        created += 1;
    }
    Ok(created)
}

/// The accountant owns every budget category that has no owner role yet.
async fn install_budget_categories(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for mut category in BudgetCategory::listed(db, true).await? {
        if category.owner_role.is_none() && !category.is_drawings() {
            category.owner_role = Some(seed::BUDGET_CATEGORY_OWNER.to_owned());
            category.save(db).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Turns off the seeded lead sources 02 does not use.
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

/// The checklist `spec` as wl-hr's input.
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

/// The module that seeds the Construction client's values: the seeder runs
/// after the shared modules' own (it is mounted after them), and the command
/// `construction:install` runs the same install.
pub struct ConstructionSeed;

impl Module for ConstructionSeed {
    fn name(&self) -> &'static str {
        "construction_seed"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "construction:install",
            "Store the Construction ticket categories, onboarding checklists and SOP library, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = install(&state).await?;
                println!("{} Construction defaults created.", installed.total());
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
        let entities = vec!["GBC".to_owned()];
        let module = invoicing(WlInvoicing::new(), &entities);
        let config = module.config();
        assert_eq!(config.write_off_approvers, ["admin"]);
        assert!(!config.portal_disputes);
        assert_eq!(config.dispute_escalation_days, 14);
        assert_eq!(config.dispute_escalation_role, "project_manager");
        assert!(config.bad_debt_relief);
        assert_eq!(config.cash_threshold_of("GBC"), 20_000_000);
        assert_eq!(config.cash_alert_roles, ["admin", "accountant"]);
        assert!(config.cash_days_late);
        assert!(!config.cash_disputed_counted);
        assert!(!config.drawings_self_approve);
        let module = payments(WlPayments::new());
        let config = module.config();
        assert!(config.method_enabled("set_off"));
        assert_eq!(config.cheque_max_presentations, 2);
        assert_eq!(
            (config.cheque_reminder_days, config.cheque_decision_days),
            (3, 14)
        );
        assert!(config.bounce_fee.is_none(), "no bounce fee is seeded");
    }

    #[test]
    fn the_stock_builders_carry_the_spec_values() {
        let module = inventory(WlInventory::new());
        let config = module.config();
        assert_eq!(config.return_approval_fils, 1_000_000);
        assert_eq!(config.return_chase_days, 30);
        assert_eq!(config.order_limits_fils, [1_000_000, 5_000_000]);
        assert_eq!(config.wastage_approval_fils, 100_000);
        assert_eq!(config.score.threshold, 70);
        assert!(config.preferred_change_needs_approval);
        for reason in seed::REJECTION_REASONS {
            assert!(config.reason(reason).is_some_and(|r| r.evidence_required));
        }
    }

    #[test]
    fn the_labour_builders_carry_the_spec_values() {
        let module = attendance(wl_attendance::WlAttendance::new());
        let config = module.config();
        assert_eq!(config.late_entry_days, 1);
        assert_eq!(config.overtime_day_minutes, 120);
        assert_eq!(config.overtime_window_minutes, 144 * 60);
        assert_eq!(config.overtime_window_days, 21);
        assert_eq!(config.hse_role, "hse");
        assert_eq!(config.hr_role, "hr");
        assert!(config.permit_check);
        assert_eq!(config.permit_kinds, ["work_permit", "residence_visa"]);
    }

    #[test]
    fn the_permit_kinds_flag_and_the_others_only_remind() {
        for spec in &seed::KINDS {
            let kind = kind_seed(spec);
            assert!(kind.blocks.iter().all(|b| b.mode == BlockMode::Flag));
            assert_eq!(kind.blocks.len(), usize::from(spec.flags.is_some()));
        }
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
