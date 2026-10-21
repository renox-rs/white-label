//! The 03 Logistics client's values for the shared document, rule,
//! integration, notice, sales, staff, money and access mechanisms (03 §3.3,
//! §4.1, §7.1, §7.2, §8, §9; BL-LOG-25 and BL-LOG-26).
//!
//! The tables are `logistics::defaults`; the modules keep their settings as
//! builder calls on the module and as seeds, as for the other clients
//! ([`super::property_seed`], [`super::accounting_seed`],
//! [`super::construction_seed`]). This file turns the tables into:
//!
//! - the document kinds registered with wl-documents ([`register_documents`]):
//!   driving licence, rider permit, Mulkiya, vehicle insurance and vehicle
//!   permit block `run.dispatch` (the first two also `delivery.assign`) as
//!   legal blocks with the override `never`, which nobody can change until the
//!   lawyer answers `L-19`; the responsible role is the operations manager of
//!   the driver's depot or the fleet manager; the registration kinds are on
//!   with the company admin;
//! - the connection with a manual fallback ([`register_integrations`]): the SMS
//!   gateway; the e-invoicing ASP is registered by wl-invoicing for an entity
//!   with a go-live date;
//! - the invoicing, payments, sales, HR and delivery settings ([`invoicing`],
//!   [`payments`], [`sales`], [`hr`], [`delivery`]) and the e-invoicing scope
//!   ([`AirFreightExclusion`]);
//! - the seed ([`LogisticsSeed`], command `logistics:install`): the late-entry
//!   limits of the e-POD, the lead sources 03 uses, the onboarding checklists,
//!   the SOP library and the budget category owners. It adds what the database
//!   lacks and keeps what the client changed, so a re-run changes nothing.
//!
//! Nothing here is a rule: the rules run in the modules with these values.

use std::sync::Arc;

use logistics::defaults::{self, KindSpec, OnboardingSpec};
use logistics::freight::FreightJob;
use renox::auth::permissions::Scope;
use renox::chrono::{Months, Weekday};
use renox::prelude::*;
use wl_documents::registrations::{OTHER, TRADEMARK, VerticalSettings, register_vertical};
use wl_documents::{KindSeed, Sop, SopReader, register_kind};
use wl_hr::{OnboardingTemplate, StepInput, TemplateInput};
use wl_invoicing::einvoice_adapter::{AspFuture, EInvoiceScope};
use wl_invoicing::{BudgetCategory, InvoicingConfig, WlInvoicing};
use wl_kernel::documents::{
    BlockCondition, BlockMode, DocumentFuture, ReminderSchedule, register_reminder_schedule, today,
};
use wl_kernel::entities::LegalEntity;
use wl_kernel::integrations::{Delivery, IntegrationDef};
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::late_entries::{LateEntryLimits, Limits, set_limits};
use wl_kernel::{COMPANY_ADMIN_ROLE, Manifest, PartyKind, Subject};
use wl_payments::WlPayments;
use wl_sales::{LeadSource, WlSales};
use wl_ticketing::{Category, Priority, SlaPolicy};

use super::roles::{ACCOUNTANT, ADMIN, DISPATCHER, MARKETING_MANAGER, OPERATIONS_MANAGER};

/// The client app these values are for.
pub const LOGISTICS: &str = "logistics";

/// The document kind `spec` as wl-documents' seed (§9 *Document blocks*): a
/// kind that blocks is a legal block (override `never`, locked, compliance-
/// critical) that looks 30 days ahead; the others only remind their
/// responsible role.
pub fn kind_seed(spec: &KindSpec) -> KindSeed {
    let mut seed = KindSeed::new(spec.key, spec.name_en)
        .name_ar(spec.name_ar)
        .responsible_role(spec.role)
        .expires()
        .numbered();
    for subject in spec.subjects {
        seed = seed.subject_type(subject);
    }
    if spec.is_legal_block() {
        seed = seed
            .legal_block()
            .look_ahead(defaults::LOOK_AHEAD_DAYS)
            .reminders(&defaults::BLOCKING_REMINDERS);
        for guarded in spec.blocks {
            seed = match spec.when {
                Some((field, values)) => seed.blocks_when(
                    guarded,
                    BlockMode::Block,
                    BlockCondition::new(field, values.iter().copied()),
                ),
                None => seed.blocks(guarded, BlockMode::Block),
            };
        }
    } else {
        seed = seed.reminders(&defaults::OTHER_REMINDERS);
    }
    seed
}

/// Where a driver's licence reminders go: the depots the driver's employee
/// record is assigned to (`wl-hr` scopes), so the operations managers of that
/// depot are told and those of another depot are not (§9: the responsible
/// role is the operations manager of the driver's depot). An employee with no
/// depot, or a driver with no employee record, is told to every holder of the
/// role.
struct DriverDepots;

impl ReminderSchedule for DriverDepots {
    fn scopes<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<Vec<Scope>>> {
        Box::pin(async move {
            if subject.subject_type != defaults::EMPLOYEE_SUBJECT {
                return Ok(None);
            }
            let Some(employee) = wl_hr::Employee::find(&state.db, subject.subject_id).await? else {
                return Ok(None);
            };
            let depots = employee.scopes(&state.db).await?;
            if depots.is_empty() {
                return Ok(None);
            }
            Ok(Some(
                depots
                    .iter()
                    .map(|depot| Scope::new(&depot.subject_type, depot.subject_id))
                    .collect(),
            ))
        })
    }
}

/// Registers the 03 kinds with wl-documents and turns on the two registration
/// kinds 03 uses (*Trademark registration*, *Other certification*) with the
/// company admin as responsible role and no `blocks`.
pub fn register_documents(client_app: &str) {
    if client_app != LOGISTICS {
        return;
    }
    for spec in &defaults::KINDS {
        register_kind(kind_seed(spec));
    }
    for key in [defaults::DRIVING_LICENCE, defaults::RIDER_PERMIT] {
        register_reminder_schedule(key, DriverDepots);
    }
    register_vertical(
        LOGISTICS,
        VerticalSettings::new()
            .enabled(&[TRADEMARK, OTHER])
            .role(TRADEMARK, COMPANY_ADMIN_ROLE)
            .role(OTHER, COMPANY_ADMIN_ROLE),
    );
}

/// Registers the connections 03 keeps a manual fallback for (§8 *Integration
/// down*): the SMS gateway (tracking SMS queued; the dispatcher calls the
/// recipients on today's runs; the client is told by e-mail,
/// `logistics::sms_fallback`). The e-invoicing ASP is registered by
/// wl-invoicing for an entity with a go-live date. Mirsal and ATLP are not
/// connections: their outage is a freight job hold.
pub fn register_integrations(client_app: &str) {
    if client_app != LOGISTICS {
        return;
    }
    IntegrationDef::of(defaults::SMS_GATEWAY, "notify", |_state, _call| {
        Box::pin(async { Ok(Delivery::Sent) })
    })
    .owner_role(DISPATCHER)
    .screens(["logistics.tracking_sms", "logistics.shipment"])
    .fallback(defaults::SMS_FALLBACK_EN, Some(defaults::SMS_FALLBACK_AR))
    .register();
}

/// The invoicing module with the 03 client's settings (§3.3, §4.4, §7.2, §9):
/// a write-off needs the operations manager or the company admin, never the
/// requester, and the client is told (`FIN.ACL.15`); the bad-debt relief is
/// proposed (`FIN.RUL.14`); the client user disputes an invoice in the portal
/// and an open dispute is escalated after 14 days (`FIN.RUL.15`); the
/// cash-flow forecast alerts the company admin and the accountant below
/// AED 100,000 in any of the next 13 weeks, weeks from Monday, checked at
/// 06:00, due dates moved by each client's days late, disputed amounts left
/// out (`FIN.RUL.23`); `FIN.RUL.24` warns the accountant; no drawing is
/// approved by its requester (`FIN.ACL.21`); and an invoice for an
/// international air job is out of the e-invoicing mandate for the first 24
/// months ([`AirFreightExclusion`]). `entities` are the codes of the legal
/// entities.
pub fn invoicing(module: WlInvoicing, entities: &[String]) -> WlInvoicing {
    let mut module = module
        .write_off_approvers(&defaults::WRITE_OFF_APPROVERS)
        .write_off_notice(true)
        .bad_debt_relief(true)
        .portal_disputes(true)
        .dispute_escalation_days(defaults::DISPUTE_ESCALATION_DAYS)
        .dispute_escalation_role(ACCOUNTANT)
        .cash_week_start(Weekday::Mon)
        .cash_forecast_weeks(defaults::CASH_FORECAST_WEEKS)
        .cash_days_late(true)
        .cash_disputed_counted(false)
        .cash_alert_time(defaults::CASH_ALERT_TIME)
        .cash_alert_roles(&[ADMIN, ACCOUNTANT])
        .budget_variance_percent(100)
        .budget_variance_role(ACCOUNTANT)
        .drawings_self_approve(false)
        .drawings_notify_role(ADMIN)
        .einvoice_scope(AirFreightExclusion);
    for code in entities {
        module = module.cash_threshold(code, defaults::CASH_THRESHOLD);
    }
    module
}

/// The payments module with the 03 client's settings: a driver's cash shortage
/// up to AED 50 is written off by the cashier without a second approver, above
/// it an operations manager approves; the manager of a driver's cash is the
/// operations manager; salary deduction is a setting that stays off until the
/// lawyer answers `L-20` (the module's default, set here to say so); a
/// bounced cheque adds no fee (none is set).
pub fn payments(module: WlPayments) -> WlPayments {
    module
        .cash_write_off_limit(defaults::CASH_WRITE_OFF_LIMIT)
        .cash_manager(OPERATIONS_MANAGER)
        .cash_salary_deduction(defaults::SALARY_DEDUCTION)
}

/// The delivery module with the 03 client's settings: a shipment is one
/// consignment, so partial acceptance is off unless the shipment has several
/// parcels (`logistics::partial`); a COD difference above AED 0 needs the
/// operations manager's approval, who picks *client*, *business* or *driver*
/// (never the customer, none preselected), within 10 minutes; a shipment
/// untraced for 7 days is lost; the drivers' cash escalation is
/// `LOG.RUL.13`; a run can be aborted for *driver unreachable*.
pub fn delivery(module: wl_delivery::WlDelivery) -> wl_delivery::WlDelivery {
    module
        .cod_tolerance(defaults::COD_TOLERANCE)
        .cod_approval(true)
        .cod_approval_minutes(defaults::COD_APPROVAL_MINUTES)
        .cod_absorbers(&[
            wl_kernel::delivery::CodAbsorber::Client,
            wl_kernel::delivery::CodAbsorber::Business,
            wl_kernel::delivery::CodAbsorber::Driver,
        ])
        .untraced_days(defaults::UNTRACED_DAYS)
        .replace_cash_escalation(true)
        .hand_over_counts_cash(true)
        .partial_acceptance(logistics::SUBJECT_TYPE, false)
        .abort_reasons(&defaults::EXTRA_ABORT_REASONS)
}

/// The sales module with the 03 client's settings: a cost entry that takes a
/// campaign over its budget needs a manager (`SAL.ACL.07`), and no lead source
/// requires a campaign.
pub fn sales(module: WlSales) -> WlSales {
    module.over_budget_approval(true).campaign_required_for(&[])
}

/// The HR module with the 03 client's settings: the line manager of a driver,
/// hub staff, a dispatcher or customer service is the dispatcher of the depot
/// ([`defaults::LINE_MANAGER_ROLE`]), and the owner role of the client is the
/// second person for waiving and reopening a step.
pub fn hr(module: wl_hr::WlHr, owner_role: &str) -> wl_hr::WlHr {
    module
        .manager_role(defaults::LINE_MANAGER_ROLE)
        .owner_role(owner_role)
}

/// Which documents are in the e-invoicing mandate for 03 (§9 *E-invoicing*): a
/// company recipient is, as by default, except an invoice for international
/// goods transport with an air waybill (a freight job by air marked
/// international) issued within the first 24 months from the go-live date of
/// the entity; sea and road freight are in scope from the start.
#[derive(Debug, Clone, Copy, Default)]
pub struct AirFreightExclusion;

impl EInvoiceScope for AirFreightExclusion {
    fn in_scope<'a>(
        &'a self,
        state: &'a AppState,
        entity: &'a LegalEntity,
        document: &'a Subject,
        recipient: Option<PartyKind>,
    ) -> AspFuture<'a, bool> {
        Box::pin(async move {
            if recipient != Some(PartyKind::Company) {
                return Ok(false);
            }
            if document.subject_type != INVOICE_SUBJECT_TYPE {
                return Ok(true);
            }
            let Some(invoice) = wl_invoicing::Invoice::find(&state.db, document.subject_id).await?
            else {
                return Ok(true);
            };
            if invoice.source_type != logistics::freight::SUBJECT_TYPE {
                return Ok(true);
            }
            let Some(job) = FreightJob::find(&state.db, invoice.source_id).await? else {
                return Ok(true);
            };
            if job.mode != "air" || !job.international {
                return Ok(true);
            }
            let go_live = state
                .provided::<InvoicingConfig>()
                .and_then(|config| config.einvoice_of(&entity.code).map(|e| e.go_live));
            let Some(go_live) = go_live else {
                return Ok(true);
            };
            let on = invoice.issue_date.unwrap_or_else(today);
            let window_ends = go_live + Months::new(defaults::EINVOICE_EXCLUSION_MONTHS);
            Ok(on >= window_ends)
        })
    }
}

/// What [`install`] stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Late-entry limit rows created.
    pub late_entry_limits: usize,
    /// Lead sources turned off.
    pub sources_off: usize,
    /// Onboarding checklists created.
    pub checklists: usize,
    /// SOPs created.
    pub sops: usize,
    /// Budget categories given the accountant as owner role.
    pub budget_categories: usize,
    /// Ticket categories created (the claim and the billing dispute of 03).
    pub ticket_categories: usize,
}

impl Installed {
    /// Everything created or changed.
    pub fn total(&self) -> usize {
        self.late_entry_limits
            + self.sources_off
            + self.checklists
            + self.sops
            + self.budget_categories
            + self.ticket_categories
    }
}

/// Stores what the 03 client's modules lack, for the modules the manifest
/// enables.
///
/// # Errors
///
/// The database's errors.
pub async fn install(state: &AppState) -> Result<Installed> {
    let manifest = state.provided::<Arc<Manifest>>();
    let on = |name: &str| manifest.as_ref().is_some_and(|m| m.is_enabled(name));
    let mut installed = Installed {
        late_entry_limits: install_late_entry_limits(&state.db).await?,
        ..Installed::default()
    };
    if on("sales") {
        installed.sources_off = turn_off_unused_sources(&state.db).await?;
    }
    if on("hr") {
        installed.checklists = install_checklists(&state.db).await?;
    }
    if on("documents") {
        installed.sops = install_sops(&state.db).await?;
    }
    if on("ticketing") {
        installed.ticket_categories = install_ticket_categories(&state.db).await?;
    }
    if on("invoicing") {
        installed.budget_categories = install_budget_categories(&state.db).await?;
    }
    Ok(installed)
}

/// The ticket categories the app opens tickets in (BL-TKT-11: a category only
/// the vertical uses comes from the vertical's starter data): the claim of a
/// loss, disposal or hold (`claim`) and the dispute of a re-weigh (`billing`).
/// `Ticket::open` refuses an unknown category, so without these rows every
/// claim ticket of 03 would fail. A row the client changed is kept.
async fn install_ticket_categories(db: &Db) -> Result<usize> {
    let policy = SlaPolicy::by_key(db, wl_ticketing::seed::STANDARD_POLICY).await?;
    let mut created = 0;
    for (key, name_en, name_ar) in [
        (logistics::hold::CLAIM_CATEGORY, "Claim", "مطالبة"),
        (logistics::reweigh::DISPUTE_CATEGORY, "Billing", "الفوترة"),
    ] {
        if Category::by_key(db, key).await?.is_some() {
            continue;
        }
        Category::create(
            db,
            Category {
                key: key.to_owned(),
                name_en: name_en.to_owned(),
                name_ar: name_ar.to_owned(),
                default_priority: Priority::Normal,
                sla_policy_id: policy.as_ref().map(|p| p.id),
                requires_signoff: false,
                active: true,
                ..Category::default()
            },
        )
        .await?;
        created += 1;
    }
    Ok(created)
}

/// M and N of a driver's e-POD entered after the stop (`PLT.ACL.10`): flagged
/// with a reason after 30 minutes, approved by an operations manager after 24
/// hours. A limit the client changed is kept.
async fn install_late_entry_limits(db: &Db) -> Result<usize> {
    let found = LateEntryLimits::query()
        .where_(LateEntryLimits::SUBJECT_TYPE.eq(defaults::EPOD_LATE_ENTRY))
        .count(db)
        .await?;
    if found > 0 {
        return Ok(0);
    }
    set_limits(
        db,
        defaults::EPOD_LATE_ENTRY,
        Limits {
            flag_after_minutes: defaults::LATE_ENTRY_FLAG_MINUTES,
            approval_after_hours: defaults::LATE_ENTRY_APPROVAL_HOURS,
        },
    )
    .await?;
    Ok(1)
}

/// The accountant owns every budget category that has no owner role yet, and
/// the *Marketing* category too: its default owner, the marketing manager,
/// is a role 03 does not have (the categories of 03 are the accountant's,
/// §5 *Budget categories and approval*).
async fn install_budget_categories(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for mut category in BudgetCategory::listed(db, true).await? {
        let ownerless = category
            .owner_role
            .as_deref()
            .is_none_or(|role| role == MARKETING_MANAGER);
        if ownerless && !category.is_drawings() {
            category.owner_role = Some(ACCOUNTANT.to_owned());
            category.save(db).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Turns off the lead sources 03 does not use (the module's *walk-in* and
/// *aggregator*).
async fn turn_off_unused_sources(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for mut source in LeadSource::ordered(db).await? {
        let used = defaults::LEAD_SOURCES.contains(&source.code.as_str());
        if source.seeded && !used && source.active {
            source.active = false;
            source.save(db).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// The step of an onboarding checklist as wl-hr's input.
fn step_input(step: &defaults::StepSpec) -> StepInput {
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
/// nothing.
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
    for spec in &defaults::ONBOARDING {
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
/// their owner role and required readers.
async fn install_sops(db: &Db) -> Result<usize> {
    let mut created = 0;
    for spec in &defaults::SOPS {
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

/// The module that seeds the 03 client's values: the seeder runs after the
/// shared modules' own (it is mounted after them), and the command
/// `logistics:install` runs the same install.
pub struct LogisticsSeed;

impl Module for LogisticsSeed {
    fn name(&self) -> &'static str {
        "logistics_seed"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "logistics:install",
            "Store the 03 late-entry limits, lead sources, onboarding checklists and SOP library, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = install(&state).await?;
                println!("{} logistics defaults created.", installed.total());
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
    fn a_blocking_kind_is_a_legal_block_that_looks_30_days_ahead() {
        for spec in defaults::KINDS.iter().filter(|spec| spec.is_legal_block()) {
            let seed = kind_seed(spec);
            assert!(seed.legal_block && seed.compliance_critical, "{}", spec.key);
            assert_eq!(seed.look_ahead_days, 30);
            assert_eq!(seed.responsible_role.as_deref(), Some(spec.role));
            assert_eq!(seed.blocks.len(), spec.blocks.len());
            assert!(seed.blocks.iter().all(|b| b.mode == BlockMode::Block));
        }
    }

    #[test]
    fn every_checklist_step_is_valid_for_wl_hr() {
        for spec in &defaults::ONBOARDING {
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
