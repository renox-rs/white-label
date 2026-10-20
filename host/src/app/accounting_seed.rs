//! The 04 Accounting client's values for the shared compliance, document,
//! block, sales and staff mechanisms (04 §3.3, §7.1, §7.2, §8, §9;
//! BL-ACC-23).
//!
//! The mechanisms live in the shared modules (`wl-parties`, `wl-documents`,
//! `wl-sales`, `wl-hr`, `wl-ticketing`) and the vertical reaches them only
//! through `wl-kernel`. The tables are `accounting::seed`; the modules keep
//! their settings as builder calls on the module and as seeds, as for the
//! Property client ([`super::property_seed`]). This file turns the tables
//! into:
//!
//! - the document kinds registered with wl-documents ([`register_documents`]):
//!   the client KYC kinds that block engagement `activate` (with the MLRO's
//!   override) and flag `return.file`, and the registration kinds on for 04
//!   with the partner as responsible role;
//! - the parties settings ([`parties_for`]): screening on, a list stale after
//!   24 hours blocks `activate` and CDD `submit` and `approve`, the MLRO's
//!   manual upload, a confirmed match blocks the company, freeze and report
//!   tasks due 24 hours after the flag;
//! - what a block does to an engagement and its retainer
//!   ([`register_block_effects`]);
//! - the sales, HR and notification settings ([`sales_for`], [`hr_for`]);
//! - the seed ([`AccountingSeed`], command `accounting:install`): the lead
//!   sources 04 uses, the onboarding checklists and the SOP library. It adds
//!   what the database lacks and keeps what the client changed, so a re-run
//!   changes nothing.

use std::sync::Arc;

use accounting::seed::{self, KYC_KINDS, KycKind, OnboardingSpec, SOPS};
use renox::prelude::*;
use wl_documents::registrations::{OTHER, TRADEMARK, VerticalSettings, register_vertical};
use wl_documents::{KindSeed, Sop, SopReader, register_kind};
use wl_hr::{OnboardingTemplate, StepInput, TemplateInput};
use wl_kernel::Manifest;
use wl_kernel::block_effects::{BlockEffect, BlockEffectOverride};
use wl_kernel::documents::{BlockCondition, BlockMode};
use wl_parties::WlParties;
use wl_sales::{LeadSource, WlSales};

use super::roles::{BILLING, COMPLIANCE, ENGAGEMENT_MANAGER, PARTNER};

/// The client app these values are for.
const ACCOUNTING: &str = "accounting";

/// The days before expiry the KYC kinds are reminded (`DOC.RUL.01`; the
/// module's usual identity-document steps).
const KYC_REMINDERS: &[i64] = &[90, 30, 7];

/// The KYC kind `kind` as wl-documents' seed (§9 *Document blocks*): it
/// blocks engagement `activate` for the parties whose role it names, with the
/// MLRO's override (`DOC.ACL.02`), and flags `return.file` without stopping
/// it; its responsible role is the compliance officer (MLRO), it is
/// compliance-critical (`DOC.RUL.03` for the blocking kinds) and looks 30
/// days ahead.
pub fn kyc_seed(kind: &KycKind) -> KindSeed {
    let condition = || {
        BlockCondition::new(
            accounting::documents::PARTY_ROLE_FIELD,
            kind.roles.iter().copied(),
        )
    };
    KindSeed::new(kind.key, kind.name_en)
        .name_ar(kind.name_ar)
        .subject_type(wl_kernel::parties::PARTY_SUBJECT_TYPE)
        .expires()
        .numbered()
        .reminders(KYC_REMINDERS)
        .look_ahead(kind.look_ahead_days)
        .responsible_role(COMPLIANCE)
        .compliance_critical()
        .blocks_when(
            accounting::documents::ACTIVATE_GUARDED,
            BlockMode::Block,
            condition(),
        )
        .blocks_when(
            accounting::documents::RETURN_FILE_GUARDED,
            BlockMode::Flag,
            condition(),
        )
}

/// Registers the 04 kinds with wl-documents, and turns on the two
/// registration kinds 04 uses (*Trademark registration*, *Other
/// certification*) with the partner as responsible role and no `blocks`.
pub fn register_documents(client_app: &str) {
    if client_app != ACCOUNTING {
        return;
    }
    for kind in KYC_KINDS {
        register_kind(kind_seed(kind));
    }
    register_vertical(
        ACCOUNTING,
        VerticalSettings::new()
            .enabled(&[TRADEMARK, OTHER])
            .role(TRADEMARK, seed::REGISTRATION_ROLE)
            .role(OTHER, seed::REGISTRATION_ROLE),
    );
}

/// [`kyc_seed`] for registration.
fn kind_seed(kind: &KycKind) -> KindSeed {
    kyc_seed(kind)
}

/// The parties module with the 04 client's settings (§7.2 `PTY.RUL.01`,
/// `PTY.RUL.02`, §4.6, §8): screening on against the UN and UAE lists, a list
/// with no successful update for 24 hours is stale and blocks the guarded
/// onboarding steps until it is current and the party re-screened, the MLRO
/// uploads a list by hand, a confirmed match on a UBO, director or signatory
/// blocks the company, the second reviewer of a match is on (the module's
/// default) and the freeze and report tasks are due 24 hours after the flag
/// (the open item L-30 keeps this a parameter).
pub fn parties_for(client_app: &str) -> WlParties {
    let module = WlParties::new();
    match client_app {
        ACCOUNTING => module
            .screening(true)
            .sanctions_freshness_hours(24)
            .stale_list_at_onboarding(wl_kernel::screening_guard::StaleListPolicy::Block)
            .manual_upload(true)
            .related_company_block(true)
            .freeze_task_due_hours(24)
            .report_task_due_hours(24)
            .exception_approver(COMPLIANCE),
        _ => module,
    }
}

/// Registers the 04 client's block effects (§7.1 *Block effect*): blocking a
/// client suspends its engagement with the neutral reason and pauses the
/// retainer with it; ending it is the MLRO's decision (`compliance`), carried
/// out by the partner through `end`. Money out stops, money in continues
/// (wl-payments, wl-invoicing). A client other than 04 keeps the defaults.
pub fn register_block_effects(client_app: &str) {
    if client_app != ACCOUNTING {
        return;
    }
    for kind in ["engagement", "subscription"] {
        BlockEffectOverride::kind(kind)
            .effect(BlockEffect::Pause)
            .decider(COMPLIANCE)
            .register();
    }
}

/// Registers the connections 04 keeps a manual fallback for (§8 *Integration
/// down*): the sanctions list download (the MLRO screens new clients by hand
/// and records it; onboarding stays blocked until the lists are current) and
/// e-mail (notices stay visible in the portal; statutory notices are
/// delivered by other means, `NTF.RUL.01`). The e-invoicing ASP is
/// registered by wl-invoicing for an entity with a go-live date. EmaraTax and
/// goAML are manual in the MVP and are not connections.
pub fn register_integrations(client_app: &str) {
    use wl_kernel::integrations::{Delivery, IntegrationDef};
    if client_app != ACCOUNTING {
        return;
    }
    IntegrationDef::of(SANCTIONS_CONNECTION, "parties", |_state, _call| {
        Box::pin(async { Ok(Delivery::Sent) })
    })
    .owner_role(COMPLIANCE)
    .screens(["accounting.cdd_file", "parties.screening_lists"])
    .fallback(
        "Screen new clients by hand against the official lists and record the result; onboarding stays blocked until the lists are current.",
        Some("افحص العملاء الجدد يدويًا مقابل القوائم الرسمية وسجّل النتيجة؛ يبقى الانضمام متوقفًا حتى تُحدَّث القوائم."),
    )
    .register();
    IntegrationDef::of(EMAIL_CONNECTION, "notify", |_state, _call| {
        Box::pin(async { Ok(Delivery::Sent) })
    })
    .owner_role(wl_kernel::COMPANY_ADMIN_ROLE)
    .screens(["accounting.deadline_notice", "accounting.portal"])
    .fallback(
        "Notices stay visible in the portal; deliver statutory notices by another means and record the proof.",
        Some("تبقى الإشعارات ظاهرة في البوابة؛ سلّم الإشعارات النظامية بوسيلة أخرى وسجّل الإثبات."),
    )
    .register();
}

/// The key of the sanctions list download connection.
pub const SANCTIONS_CONNECTION: &str = wl_kernel::integrations::SANCTIONS_DOWNLOAD;
/// The key of the e-mail connection.
pub const EMAIL_CONNECTION: &str = wl_kernel::integrations::EMAIL;

/// The sales module with the 04 client's settings: a cost entry that takes a
/// campaign over its budget needs a manager (`SAL.ACL.07`), and no lead
/// source requires a campaign.
pub fn sales_for(client_app: &str) -> WlSales {
    let module = WlSales::new();
    match client_app {
        ACCOUNTING => module.over_budget_approval(true).campaign_required_for(&[]),
        _ => module,
    }
}

/// The HR module with the 04 client's settings: the line manager of a member
/// of staff is the engagement manager, and the owner role of the client is
/// the second person for waiving and reopening a step.
pub fn hr_for(client_app: &str, owner_role: &str) -> wl_hr::WlHr {
    let module = wl_hr::WlHr::new();
    match client_app {
        ACCOUNTING => module
            .manager_role(ENGAGEMENT_MANAGER)
            .owner_role(owner_role),
        _ => module,
    }
}

/// The projected cash balance below which `FIN.RUL.23` alerts, in fils
/// (AED 25,000).
pub const CASH_THRESHOLD: i64 = 2_500_000;

/// The invoicing module with the 04 client's settings (§3.3, §4.5, §7.1,
/// §7.2, §9): a write-off needs a partner other than the requester and the
/// client is told (`FIN.ACL.15`); the client approver disputes an invoice in
/// the portal and the engagement manager gets the ticket; the cash-flow
/// forecast alerts the partner and billing below AED 25,000 in any of the
/// next 13 weeks, weeks from Monday, checked at 06:00, due dates moved by
/// each client's days late, disputed amounts left out (`FIN.RUL.23`);
/// `FIN.RUL.24` warns billing; the retainer's cycle invoices are issued by
/// themselves. `entities` are the codes of the legal entities.
pub fn invoicing(
    module: wl_invoicing::WlInvoicing,
    entities: &[String],
) -> wl_invoicing::WlInvoicing {
    let mut module = module
        .auto_issue_source("invoicing.subscription")
        .write_off_approvers(&[PARTNER])
        .write_off_notice(true)
        .bad_debt_relief(true)
        .portal_disputes(true)
        .dispute_ticket_role(ENGAGEMENT_MANAGER)
        .dispute_escalation_role(ENGAGEMENT_MANAGER)
        .cash_week_start(renox::chrono::Weekday::Mon)
        .cash_forecast_weeks(13)
        .cash_days_late(true)
        .cash_disputed_counted(false)
        .cash_alert_time("06:00")
        .cash_alert_roles(&[PARTNER, BILLING])
        .budget_variance_percent(100)
        .budget_variance_role(BILLING)
        .drawings_self_approve(false)
        .drawings_notify_role(PARTNER);
    for code in entities {
        module = module.cash_threshold(code, CASH_THRESHOLD);
    }
    module
}

/// The payments module with the 04 client's settings: a bounced cheque is
/// chased by the module's own path, adds no fee (none is set) and is decided
/// by billing with the engagement manager told.
pub fn payments(module: wl_payments::WlPayments) -> wl_payments::WlPayments {
    module.cheque_manager(BILLING, true)
}

/// What [`install`] stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Lead sources turned off.
    pub sources_off: usize,
    /// Onboarding checklists created.
    pub checklists: usize,
    /// SOPs created.
    pub sops: usize,
    /// Notification types given billing as the collector role.
    pub notice_roles: usize,
    /// Budget categories given billing as owner role.
    pub budget_categories: usize,
}

impl Installed {
    /// Everything created or changed.
    pub fn total(&self) -> usize {
        self.sources_off + self.checklists + self.sops + self.notice_roles + self.budget_categories
    }
}

/// Stores what the 04 client's modules lack, for the modules the manifest
/// enables.
///
/// # Errors
///
/// The database's errors.
pub async fn install(state: &AppState) -> Result<Installed> {
    let manifest = state.provided::<Arc<Manifest>>();
    let on = |name: &str| manifest.as_ref().is_some_and(|m| m.is_enabled(name));
    let mut installed = Installed::default();
    if on("sales") {
        installed.sources_off = turn_off_unused_sources(&state.db).await?;
    }
    if on("hr") {
        installed.checklists = install_checklists(&state.db).await?;
    }
    if on("documents") {
        installed.sops = install_sops(&state.db).await?;
    }
    if on("notify") {
        installed.notice_roles = install_collector_role(&state.db).await?;
    }
    if on("invoicing") {
        installed.budget_categories = install_budget_categories(&state.db).await?;
    }
    Ok(installed)
}

/// The notification types of the *invoices* and *statements* groups (invoices,
/// statements, retainer and write-off notices) tell the billing role, the
/// collector of 04, when a message gives up (§9 *Notices*): the groups' own
/// `receivables` role is replaced unless the client edited the type.
async fn install_collector_role(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for mut row in wl_notify::NotificationType::query().get(db).await? {
        let in_group = ["invoices", "statements"].contains(&row.group_key.as_str());
        let default_role = row
            .staff_role
            .as_deref()
            .is_none_or(|role| role == "receivables");
        if in_group && default_role && row.is_unedited() {
            let at = row.updated_at;
            row.staff_role = Some(BILLING.to_owned());
            // Keep the "never edited" mark: write the column only.
            renox::db::sql(format!(
                "UPDATE {} SET staff_role = ? WHERE id = ?",
                wl_notify::NotificationType::TABLE
            ))
            .bind(row.staff_role.clone())
            .bind(row.id)
            .execute(db)
            .await?;
            let _ = at;
            changed += 1;
        }
    }
    Ok(changed)
}

/// The budget categories are owned by billing (`FIN.BUD.01`, `FIN.RUL.24`):
/// every category with no owner role gets it; the mapping to the client's
/// export accounts is the client's.
async fn install_budget_categories(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for mut category in wl_invoicing::BudgetCategory::listed(db, true).await? {
        if category.owner_role.is_none() && !category.is_drawings() {
            category.owner_role = Some(BILLING.to_owned());
            category.save(db).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Turns off the lead sources 04 does not use (the module's *walk-in* and
/// *aggregator*).
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
/// nothing (the role is active before the checklist is complete).
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
    for spec in &seed::ONBOARDING {
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

/// The module that seeds the 04 client's values: the seeder runs after the
/// shared modules' own (it is mounted after them), and the command
/// `accounting:install` runs the same install.
pub struct AccountingSeed;

impl Module for AccountingSeed {
    fn name(&self) -> &'static str {
        "accounting_seed"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "accounting:install",
            "Store the 04 lead sources, onboarding checklists and SOP library, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = install(&state).await?;
                println!("{} accounting defaults created.", installed.total());
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
    fn every_kyc_kind_blocks_activate_and_flags_the_filing() {
        for kind in KYC_KINDS {
            let seed = kyc_seed(kind);
            assert_eq!(seed.key, kind.key);
            assert_eq!(seed.responsible_role.as_deref(), Some(COMPLIANCE));
            assert!(seed.compliance_critical);
            assert_eq!(seed.look_ahead_days, 30);
            let blocks: Vec<(&str, BlockMode)> = seed
                .blocks
                .iter()
                .map(|b| (b.guarded.as_str(), b.mode))
                .collect();
            assert_eq!(
                blocks,
                [
                    ("engagement.activate", BlockMode::Block),
                    ("return.file", BlockMode::Flag)
                ]
            );
        }
    }

    #[test]
    fn the_04_client_turns_screening_on_with_the_mlro_as_approver() {
        let module = parties_for("accounting");
        assert_eq!(module.freshness_hours(), 24);
        assert_eq!(module.approver(), COMPLIANCE);
        assert!(module.manual_upload_on());
        assert_eq!(
            module.stale_list_policy(),
            wl_kernel::screening_guard::StaleListPolicy::Block
        );
        // Another client keeps the module's defaults.
        let other = parties_for("property");
        assert_eq!(
            other.stale_list_policy(),
            wl_kernel::screening_guard::StaleListPolicy::Warn
        );
    }

    #[test]
    fn every_checklist_step_is_valid_for_wl_hr() {
        for spec in &seed::ONBOARDING {
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
