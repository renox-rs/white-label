//! The 07 Manufacturing client's values for the shared modules (07 §2, §3.3,
//! §7.1 *Negative-flow settings*, §9; BL-MFG-11, BL-MFG-12).
//!
//! - **Document kinds** ([`register_documents`]): the product conformity
//!   certificates of a regulated item (the ECAS certificate, the Emirates
//!   Quality Mark licence, the food-contact certificate) attach to the item
//!   and block `dispatch.pick` and `dispatch.dispatch` as legal blocks with
//!   the override `never` (`⚠ open: lawyer L-19`), looking 30 days ahead
//!   (`DOC.RUL.02`); the industrial licence, the Industrial Registry and the
//!   ICV certificate only remind (`DOC.RUL.01`) and block nothing; the exit
//!   evidence of an export (a customs exit certificate, a bill of lading, an
//!   airway bill) attaches to the dispatch and blocks nothing either.
//! - **The unique customer PO number** ([`sales`]): the setting *Unique
//!   customer PO number per customer* is on (BL-MFG-06).
//!
//! - **The shared modules' settings** (BL-MFG-14, 07 §3.3, §7.1 *Negative-flow
//!   settings*): [`invoicing`], [`payments`], [`sales`], [`inventory`] and
//!   [`hr`] are the builder calls of the invoicing, payments, sales, stock and
//!   HR modules for a manufacturer; [`ManufacturingSeed`] (command
//!   `manufacturing:install`) stores what the modules lack: the late-entry
//!   limits of a production report, the lead sources 07 uses, the onboarding
//!   checklists and the SOP library. A re-run changes nothing and keeps what
//!   the client changed.
//!
//! Nothing here is a rule: the rules run in the modules and in the vertical
//! with these values.

use std::sync::Arc;

use manufacturing::conformity::{DISPATCH, ECAS, EQM, FOOD_CONTACT, PICK};
use manufacturing::exports::{AIRWAY_BILL, BILL_OF_LADING, CUSTOMS_EXIT};
use renox::chrono::Weekday;
use renox::prelude::*;
use wl_documents::registrations::{
    HALAL, ISO, OTHER, TRADEMARK, VerticalSettings, register_vertical,
};
use wl_documents::{KindSeed, Sop, SopReader, register_kind};
use wl_hr::{OnboardingTemplate, StepInput, TemplateInput};
use wl_inventory::{ScoreSettings, WlInventory};
use wl_invoicing::WlInvoicing;
use wl_kernel::Manifest;
use wl_kernel::documents::{BlockMode, OverrideMode, register_reminder_schedule};
use wl_kernel::inventory::{DISPATCH_SUBJECT_TYPE, ITEM_SUBJECT_TYPE};
use wl_kernel::late_entries::{LateEntryLimits, Limits, set_limits};
use wl_payments::WlPayments;
use wl_sales::{LeadSource, WlSales};

use super::roles::{
    ACCOUNTANT, ADMIN, LINE_OPERATOR, OWNER, PRODUCTION_MANAGER, PRODUCTION_PLANNER, QC_INSPECTOR,
    SALES_MANAGER, STOREKEEPER, SUPERVISOR,
};

/// The client app these values are for.
pub const MANUFACTURING: &str = "manufacturing";

/// The days before a certificate expires that its holder is reminded and the
/// look-ahead of `DOC.RUL.02` starts.
pub const LOOK_AHEAD_DAYS: i64 = 30;

/// The product conformity kinds: key, English name, Arabic name.
pub const CONFORMITY: [(&str, &str, &str); 3] = [
    (ECAS, "ECAS certificate", "شهادة ECAS"),
    (
        EQM,
        "Emirates Quality Mark licence",
        "رخصة علامة الجودة الإماراتية",
    ),
    (
        FOOD_CONTACT,
        "Food-contact certificate",
        "شهادة ملامسة الأغذية",
    ),
];

/// The key of the ICV certificate.
pub const ICV_CERTIFICATE: &str = "icv_certificate";
/// The days before the ICV certificate expires that it is reminded (07 §13 q7).
pub const ICV_REMINDERS: [i64; 3] = [60, 30, 7];

/// The licences of the company: key, English name, Arabic name. They remind
/// and block nothing.
pub const COMPANY_KINDS: [(&str, &str, &str); 3] = [
    (
        "industrial_licence",
        "Industrial licence",
        "الرخصة الصناعية",
    ),
    (
        "industrial_registry",
        "Industrial Registry",
        "السجل الصناعي",
    ),
    (
        ICV_CERTIFICATE,
        "ICV certificate",
        "شهادة القيمة الوطنية المضافة",
    ),
];

/// The exit evidence of an export: key, English name, Arabic name.
pub const EXIT_EVIDENCE: [(&str, &str, &str); 3] = [
    (
        CUSTOMS_EXIT,
        "Customs exit certificate",
        "شهادة الخروج الجمركية",
    ),
    (BILL_OF_LADING, "Bill of lading", "بوليصة الشحن"),
    (AIRWAY_BILL, "Airway bill", "بوليصة الشحن الجوي"),
];

/// The conformity kind `key` as wl-documents' seed: a legal block of both
/// steps of a dispatch, attached to the item.
pub fn conformity_seed(key: &str, name_en: &str, name_ar: &str) -> KindSeed {
    KindSeed::new(key, name_en)
        .name_ar(name_ar)
        .subject_type(ITEM_SUBJECT_TYPE)
        .expires()
        .numbered()
        .reminders(&[60, 30, 7])
        .look_ahead(LOOK_AHEAD_DAYS)
        .responsible_role(QC_INSPECTOR)
        // Sales, the planner and the production manager get the look-ahead of
        // the orders promised after the expiry (`DOC.RUL.02`); the QC
        // inspectors of the plants that hold the item are responsible.
        .also_notify(&[
            manufacturing::permissions::SALES_ROLE,
            PRODUCTION_PLANNER,
            PRODUCTION_MANAGER,
        ])
        .override_mode(OverrideMode::Never)
        .legal_block()
        .blocks(PICK, BlockMode::Block)
        .blocks(DISPATCH, BlockMode::Block)
}

/// Registers the 07 kinds with wl-documents and turns on the two
/// registration kinds with the company admin as responsible role.
pub fn register_documents(client_app: &str) {
    if client_app != MANUFACTURING {
        return;
    }
    for (key, name_en, name_ar) in CONFORMITY {
        register_kind(conformity_seed(key, name_en, name_ar));
    }
    for (key, name_en, name_ar) in COMPANY_KINDS {
        register_kind(
            KindSeed::new(key, name_en)
                .name_ar(name_ar)
                .expires()
                .numbered()
                .reminders(if key == ICV_CERTIFICATE {
                    &ICV_REMINDERS
                } else {
                    &[90, 30, 7]
                })
                .responsible_role(OWNER),
        );
    }
    for (key, name_en, name_ar) in EXIT_EVIDENCE {
        register_kind(
            KindSeed::new(key, name_en)
                .name_ar(name_ar)
                .subject_type(DISPATCH_SUBJECT_TYPE)
                .numbered()
                .responsible_role(ACCOUNTANT),
        );
    }
    // The QC inspectors of the plants that hold the item or work on it are
    // told that a certificate of it expires (`DOC.RUL.01`, BL-MFG-14).
    for (key, ..) in CONFORMITY {
        register_reminder_schedule(key, manufacturing::responsible::ItemPlants);
    }
    // Halal, ISO and other certifications: the QC inspector; the trademark:
    // the general manager / owner (the platform's defaults for 07).
    register_vertical(
        MANUFACTURING,
        VerticalSettings::new()
            .enabled(&[TRADEMARK, HALAL, ISO, OTHER])
            .role(OTHER, QC_INSPECTOR),
    );
}

/// Marks `MFG.RUL.08` and `DOC.RUL.03` compliance-critical for a manufacturing client: a run
/// that fails after its last attempt, or a daily one that does not run, alerts
/// the owner role and the company admin (BL-KRN-08).
pub fn register_critical(client_app: &str) {
    if client_app != MANUFACTURING {
        return;
    }
    wl_kernel::compliance::CriticalRule::new(manufacturing::exports::RULE)
        .daily_at(manufacturing::exports::RUN_AT)
        .register();
    // The expiry of a conformity certificate (`DOC.RUL.03`, BL-MFG-14): the
    // owner is told when the renewal task of a legal block fails to run.
    wl_kernel::compliance::CriticalRule::new("DOC.RUL.03").register();
}

/// `module` with the 07 sales settings (§3.3, §4.1, §7.1): the customer PO
/// number is unique per customer with its call-off number (BL-MFG-06); the
/// invoice is drafted for the accepted quantities on delivery (`MFG.RUL.04`);
/// a discount above 5% needs a manager (`SAL.ACL.02`); an order on account is
/// held when an invoice is more than 30 days overdue, or the exposure is above
/// the limit, and credit is checked again at each dispatch; a bounced cheque
/// holds every open order on account of the customer (`SAL.RUL.05`); a started
/// line is cancelled as *finish to stock*, *scrap* or *return to stock* with
/// no fee, the fee line standard rated until open item A-04 is answered; a
/// cost entry over a campaign's budget needs a manager (`SAL.ACL.07`) and no
/// lead source requires a campaign.
pub fn sales(module: WlSales) -> WlSales {
    module
        .unique_customer_po(true)
        .invoice_on_delivery(true)
        .manual_discount_limit_percent(DISCOUNT_LIMIT_PERCENT)
        .credit_check(true)
        .credit_grace_days(OVERDUE_DAYS)
        .hold_on_overdue(true)
        .hold_on_limit(true)
        .credit_recheck_at_dispatch(true)
        .bounced_cheque_hold_group(true)
        .cancel_outcomes(&CANCEL_OUTCOMES)
        .cancellation_fee_percent(CANCELLATION_FEE_PERCENT)
        .cancellation_fee_vat(CANCELLATION_FEE_VAT)
        .order_manager_role(SALES_MANAGER)
        .over_budget_approval(true)
        .campaign_required_for(&[])
}

// ---------------------------------------------------------------- the values

/// `SAL.ACL.02`: the manual discount above which a manager decides, percent.
pub const DISCOUNT_LIMIT_PERCENT: i64 = 5;
/// `SAL.ACL.03`: days past terms before an overdue invoice holds an order.
pub const OVERDUE_DAYS: i64 = 30;
/// The outcomes offered when a started line is cancelled (§7.1 *Sales order*).
pub const CANCEL_OUTCOMES: [&str; 3] = ["finish_to_stock", "scrap", "return_to_stock"];
/// `SAL.ACL.05`: the default cancellation fee, percent (none).
pub const CANCELLATION_FEE_PERCENT: i64 = 0;
/// The VAT category of a cancellation fee line until open item A-04 is
/// answered: standard rated (5%).
pub const CANCELLATION_FEE_VAT: &str = "standard";
/// `FIN.RUL.23`: the projected balance below which the owner is told, in fils
/// (AED 200,000).
pub const CASH_THRESHOLD_FILS: i64 = 20_000_000;
/// `FIN.RUL.23`: when the daily check runs.
pub const CASH_ALERT_TIME: &str = "06:00";
/// `FIN.RUL.23`: the weeks the forecast looks at.
pub const CASH_FORECAST_WEEKS: u32 = 13;
/// `FIN.RUL.15`: days an invoice dispute stays open before it is escalated.
pub const DISPUTE_ESCALATION_DAYS: u32 = 14;
/// `FIN.RUL.18`: the days after a bounce when the customer is reminded and the
/// accountant decides.
pub const BOUNCE_CHASE_DAYS: (i64, i64) = (3, 14);
/// The most presentations of one cheque.
pub const CHEQUE_PRESENTATIONS: i64 = 2;
/// `STK.ACL.02`: a wastage above this value needs approval, in fils (AED 2,000).
pub const WASTAGE_APPROVAL_FILS: i64 = 200_000;
/// `STK.ACL.06`: a return to supplier above this value needs approval, in
/// fils (AED 5,000).
pub const RETURN_APPROVAL_FILS: i64 = 500_000;
/// `STK.RUL.07`: hours an open stock hold waits before its deciders are
/// reminded.
pub const HOLD_REMINDER_HOURS: i64 = 24;
/// `STK.RUL.08`: days a return to a supplier may stay uncredited.
pub const RETURN_CHASE_DAYS: i64 = 30;
/// `STK.SUP.01`: the supplier score is over this many months.
pub const SCORE_WINDOW_MONTHS: u32 = 3;
/// `STK.SUP.01`: receipts needed before a supplier is scored.
pub const SCORE_MIN_RECEIPTS: i64 = 3;
/// `STK.RUL.09`: a score under this opens a review task.
pub const SCORE_THRESHOLD: i64 = 70;
/// The notification type a recall's notices to customers go out under.
pub const RECALL_NOTICE: &str = "mfg.recall_notice";
/// `PLT.ACL.10`: minutes of delay above which a late entry is flagged.
pub const LATE_ENTRY_FLAG_MINUTES: i64 = 30;
/// `PLT.ACL.10`: hours of delay above which the production manager approves.
pub const LATE_ENTRY_APPROVAL_HOURS: i64 = 24;
/// The lead sources 07 uses (`SAL.CRM.02`); the others are turned off.
pub const LEAD_SOURCES: [&str; 6] = [
    "referral", "website", "event", "partner", "paid_ads", "other",
];
/// The campaign channels of 07 as (07's name, the `wl-sales` marketing channel
/// it is recorded as). The module's list is fixed: LinkedIn is recorded as
/// *other online*, the trade exhibitions as *event*.
pub const CAMPAIGN_CHANNELS: [(&str, &str); 4] = [
    ("Trade exhibitions", "event"),
    ("LinkedIn", "other_online"),
    ("Search ads", "google"),
    ("Email", "email"),
];

/// The invoicing module with the 07 settings (§5, §7.1): a write-off, whatever
/// its amount, waits for the general manager / owner or the company admin
/// (`FIN.ACL.15`, limit AED 0); the bad-debt relief is proposed after 6 months
/// (`FIN.RUL.14`); the customer buyer disputes an invoice in the portal and an
/// open dispute goes to the accountant after 14 days (`FIN.RUL.15`); the
/// cash-flow forecast looks 13 weeks from Monday and tells the general
/// manager / owner and the accountant below AED 200,000 at 06:00, with the due
/// dates moved by each customer's days late and the disputed amounts left out
/// (`FIN.RUL.23`); a month's actual reaching its budget line warns the
/// category's owner (the accountant) and the accountant (`FIN.RUL.24`); an
/// owner drawing is never approved by the user who records it
/// (`FIN.ACL.21`). `entities` are the codes of the legal entities.
pub fn invoicing(module: WlInvoicing, entities: &[String]) -> WlInvoicing {
    let mut module = module
        .write_off_approvers(&[OWNER, ADMIN])
        .write_off_notice(true)
        .bad_debt_relief(true)
        .portal_disputes(true)
        .dispute_escalation_days(DISPUTE_ESCALATION_DAYS)
        .dispute_escalation_role(ACCOUNTANT)
        .cash_week_start(Weekday::Mon)
        .cash_forecast_weeks(CASH_FORECAST_WEEKS)
        .cash_days_late(true)
        .cash_disputed_counted(false)
        .cash_alert_time(CASH_ALERT_TIME)
        .cash_alert_roles(&[OWNER, ACCOUNTANT])
        .budget_variance_percent(100)
        .budget_variance_role(ACCOUNTANT)
        .budget_category_owner(ACCOUNTANT)
        .drawings_self_approve(false)
        .drawings_notify_role(OWNER);
    for code in entities {
        module = module.cash_threshold(code, CASH_THRESHOLD_FILS);
    }
    module
}

/// The payments module with the 07 settings: a cheque is presented twice at
/// most, a bounced cheque is chased after 3 and 14 days (`FIN.RUL.18`), and no
/// bounce fee is invoiced (none is set).
pub fn payments(module: WlPayments) -> WlPayments {
    module
        .cheque_presentations(CHEQUE_PRESENTATIONS)
        .cheque_chase(BOUNCE_CHASE_DAYS.0, BOUNCE_CHASE_DAYS.1)
}

/// The stock module with the 07 settings (§4.2, §4.4, §7.1): a wastage above
/// AED 2,000 and a return to a supplier above AED 5,000 need approval
/// (`STK.ACL.02`, `STK.ACL.06`); an open stock hold is reminded after 24 hours
/// (`STK.RUL.07`); only a recall hold needs a second person to release
/// (`STK.ACL.05`); a return not credited is chased after 30 days
/// (`STK.RUL.08`); the supplier score is over 3 months, at least 3 receipts,
/// a review task below 70 (`STK.RUL.09`) and a change of the preferred
/// supplier needs a second person (`STK.ACL.07`); the customers of a recall are
/// told under [`RECALL_NOTICE`], urgent, no group.
pub fn inventory(module: WlInventory) -> WlInventory {
    module
        .wastage_approval_fils(WASTAGE_APPROVAL_FILS)
        .return_approval_fils(RETURN_APPROVAL_FILS)
        .hold_reminder_hours(HOLD_REMINDER_HOURS)
        .hold_second_person_reasons(&["recall"])
        .return_chase_days(RETURN_CHASE_DAYS)
        .supplier_score(ScoreSettings {
            window_months: SCORE_WINDOW_MONTHS,
            min_receipts: SCORE_MIN_RECEIPTS,
            threshold: SCORE_THRESHOLD,
            ..ScoreSettings::default()
        })
        .preferred_change_needs_approval(true)
        .recall_notice_key(RECALL_NOTICE)
}

/// The HR module with the 07 settings: the line manager of the plant's staff
/// is the line supervisor, and the general manager / owner is the second
/// person for waiving or reopening an onboarding step.
pub fn hr(module: wl_hr::WlHr) -> wl_hr::WlHr {
    module.manager_role(SUPERVISOR).owner_role(OWNER)
}

// ----------------------------------------------------------------- the seed

/// One step of an onboarding checklist: title, owner (`hr`, `manager`,
/// `employee` or `role:<key>`), due days after the start and kind (`task` or
/// `sops`).
pub type StepSpec = (&'static str, &'static str, i64, &'static str);

/// An onboarding checklist for one job role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardingSpec {
    /// The template's name.
    pub name: &'static str,
    /// The job role it applies to.
    pub role: &'static str,
    /// The steps.
    pub steps: &'static [StepSpec],
}

const DOCUMENTS_STEP: StepSpec = ("Documents and access", "role:production_manager", 2, "task");
const WELCOME_STEP: StepSpec = ("Welcome and introductions", "manager", 1, "task");
const EMPLOYEE_STEP: StepSpec = ("Upload your personal documents", "employee", 3, "task");
const SOP_STEP: StepSpec = ("SOPs acknowledged", "employee", 7, "sops");

/// The checklists 07 seeds (`HRM.RUL.16`): started when the employee record
/// is created in `onboarding`; the production manager, the line supervisor
/// and the employee get tasks; completion blocks nothing (the role is active
/// before the checklist is complete).
pub const ONBOARDING: [OnboardingSpec; 5] = [
    OnboardingSpec {
        name: "Operator onboarding",
        role: LINE_OPERATOR,
        steps: &[
            DOCUMENTS_STEP,
            WELCOME_STEP,
            ("Safety and line walk-through", "manager", 2, "task"),
            ("Job card and tablet training", "manager", 3, "task"),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Line supervisor onboarding",
        role: SUPERVISOR,
        steps: &[
            DOCUMENTS_STEP,
            WELCOME_STEP,
            (
                "Work orders and reporting walk-through",
                "manager",
                3,
                "task",
            ),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "QC inspector onboarding",
        role: QC_INSPECTOR,
        steps: &[
            DOCUMENTS_STEP,
            WELCOME_STEP,
            ("Inspection and NCR walk-through", "manager", 3, "task"),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Storekeeper onboarding",
        role: STOREKEEPER,
        steps: &[
            DOCUMENTS_STEP,
            WELCOME_STEP,
            ("Receiving and issuing walk-through", "manager", 3, "task"),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Maintenance technician onboarding",
        role: "maintenance_technician",
        steps: &[
            DOCUMENTS_STEP,
            WELCOME_STEP,
            ("Machine and lock-out walk-through", "manager", 3, "task"),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
];

/// A SOP of the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SopSpec {
    /// The code.
    pub code: &'static str,
    /// The English title.
    pub title: &'static str,
    /// The category.
    pub category: &'static str,
    /// The role whose holders accept and review it.
    pub owner_role: &'static str,
    /// The roles that must read and acknowledge it.
    pub readers: &'static [&'static str],
}

/// The SOP library (`HRM.ONB.01`, `DOC.RUL.04`): work instructions are the
/// production manager's, inspection and quality SOPs the QC inspector's.
pub const SOPS: [SopSpec; 5] = [
    SopSpec {
        code: "SOP-MFG-WORK-INSTRUCTIONS",
        title: "Work instructions of the line",
        category: "production",
        owner_role: PRODUCTION_MANAGER,
        readers: &[LINE_OPERATOR, SUPERVISOR],
    },
    SopSpec {
        code: "SOP-MFG-MATERIALS",
        title: "Material issue and storage",
        category: "production",
        owner_role: PRODUCTION_MANAGER,
        readers: &[STOREKEEPER, SUPERVISOR],
    },
    SopSpec {
        code: "SOP-MFG-INSPECTION",
        title: "Incoming and in-process inspection",
        category: "quality",
        owner_role: QC_INSPECTOR,
        readers: &[QC_INSPECTOR, SUPERVISOR],
    },
    SopSpec {
        code: "SOP-MFG-NCR",
        title: "Non-conformance and quarantine",
        category: "quality",
        owner_role: QC_INSPECTOR,
        readers: &[QC_INSPECTOR, LINE_OPERATOR, SUPERVISOR, STOREKEEPER],
    },
    SopSpec {
        code: "SOP-MFG-RECALL",
        title: "Product recall",
        category: "quality",
        owner_role: QC_INSPECTOR,
        readers: &[QC_INSPECTOR, SUPERVISOR, STOREKEEPER],
    },
];

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
}

impl Installed {
    /// Everything created or changed.
    pub fn total(&self) -> usize {
        self.late_entry_limits + self.sources_off + self.checklists + self.sops
    }
}

/// Stores what the 07 client's modules lack, for the modules the manifest
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
    Ok(installed)
}

/// Only the production report has these limits. Deferred (F7, BL-MFG-14): the
/// same limits for goods receipts and for the inspections of returned goods
/// (07 §7.1 *Late entry*), which keep their business time but have no limit
/// installed; and no kind is registered for calibration certificates
/// (`DOC.EXP.01`), there being no equipment record to attach them to.
///
/// M and N of a production report entered after the shift (`PLT.ACL.10`):
/// flagged with a reason after 30 minutes, approved by the production manager
/// after 24 hours. A limit the client changed is kept.
async fn install_late_entry_limits(db: &Db) -> Result<usize> {
    let found = LateEntryLimits::query()
        .where_(LateEntryLimits::SUBJECT_TYPE.eq(manufacturing::late_report::SUBJECT_TYPE))
        .count(db)
        .await?;
    if found > 0 {
        return Ok(0);
    }
    set_limits(
        db,
        manufacturing::late_report::SUBJECT_TYPE,
        Limits {
            flag_after_minutes: LATE_ENTRY_FLAG_MINUTES,
            approval_after_hours: LATE_ENTRY_APPROVAL_HOURS,
        },
    )
    .await?;
    Ok(1)
}

/// Turns off the seeded lead sources 07 does not use.
async fn turn_off_unused_sources(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for mut source in LeadSource::ordered(db).await? {
        let used = LEAD_SOURCES.contains(&source.code.as_str());
        if source.seeded && !used && source.active {
            source.active = false;
            source.save(db).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

fn step_input(step: &StepSpec) -> StepInput {
    let (title, owner, due_days, kind) = *step;
    if kind == "sops" {
        StepInput::sops(title, owner, due_days)
    } else {
        StepInput::task(title, owner, due_days)
    }
}

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

/// The module that seeds the 07 client's values; the seeder runs after the
/// shared modules' own, and the command `manufacturing:install` runs the same
/// install.
pub struct ManufacturingSeed;

impl Module for ManufacturingSeed {
    fn name(&self) -> &'static str {
        "manufacturing_seed"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "manufacturing:install",
            "Store the 07 late-entry limits, lead sources, onboarding checklists and SOP library, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = install(&state).await?;
                println!("{} manufacturing defaults created.", installed.total());
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { install(&state).await.map(|_| ()) });
    }
}

/// What the roles that 07 shares with other verticals hold for 07's tasks: the
/// storekeeper and the line supervisor are given tasks (a hold for material,
/// the re-pick of a recalled dispatch) and open them as tickets.
pub fn grant_roles(client_app: &str) {
    if client_app != MANUFACTURING {
        return;
    }
    use wl_ticketing::permissions as tickets;
    for role in [STOREKEEPER, SUPERVISOR, LINE_OPERATOR] {
        super::roles::grant_extra(role, &[tickets::VIEW, tickets::WORK]);
    }
    // `MFG.RUL.08`: the export exit evidence is the accountant's document
    // (the kind's responsible role); the export evidence page links to the
    // dispatch's documents, so the accountant may open and fill them.
    super::roles::grant_extra(
        ACCOUNTANT,
        &[
            wl_documents::permissions::VIEW,
            wl_documents::permissions::UPLOAD,
        ],
    );
}
