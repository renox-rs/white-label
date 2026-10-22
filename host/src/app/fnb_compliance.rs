//! The 06 F&B client's values for the shared stock, document, kernel,
//! connection, notice, sales and staff mechanisms (06 §3.3, §4.1, §4.4, §4.6,
//! §5, §7.2; BL-FNB-17).
//!
//! The mechanisms live in the shared modules; the vertical reaches them only
//! through `wl-kernel`. Like the other clients ([`super::logistics_seed`],
//! [`super::clinics_seed`]) the host turns the tables of values below into:
//!
//! - the document kinds registered with wl-documents ([`register_documents`]):
//!   *trade licence* (the operations director renews it), *food permit*,
//!   *health card* and *PIC certificate* (the kitchen manager of the outlet
//!   renews them: the reminders go to the holders in the outlet's scope, the
//!   employee's outlets for a card). All four are compliance-critical, so the
//!   owner is told on expiry (`DOC.RUL.03`), and **none blocks anything**:
//!   an expired health card stops no transition in the MVP (06 §4.6, open
//!   item AU-15 for the validity). The registration kinds *Halal certificate*
//!   (the storekeeper), *Trademark registration* and *Other certification*
//!   (the operations director) are on;
//! - the compliance-critical rules ([`register_critical`]): `FNB.RUL.03`,
//!   `FNB.RUL.04`, `FNB.RUL.05` and `DOC.RUL.03`, owner alert to the
//!   operations director (`PLT.RUL.04`);
//! - the connections with a manual fallback ([`register_integrations`]):
//!   Deliverect, the payment gateway and the SMS gateway (wl-kernel §5.14);
//! - the notification module's caps ([`notify`]), the HR module's manager
//!   ([`hr`]);
//! - the seed ([`install`], run by [`super::fnb_seed::FnbSeed`] and the command
//!   `fnb:install`): the late-entry limits, the onboarding checklists, the SOP
//!   library, the marketing send window, the loyalty programme, the segments,
//!   and the stock locations of the outlets (negative stock and the scope the
//!   hold deciders are checked in). It adds what the database lacks and keeps
//!   what the client changed, so a re-run changes nothing.
//!
//! Nothing here is a rule: the rules run in the modules with these values.

use std::future::Future;
use std::pin::Pin;

use fnb::catalogue::{OUTLET_SUBJECT_TYPE, Outlet};
use renox::auth::permissions::Scope;
use renox::prelude::*;
use wl_documents::registrations::{
    HALAL, LEGAL_ENTITY_SUBJECT, OTHER, TRADEMARK, VerticalSettings, register_vertical,
};
use wl_documents::{KindSeed, Sop, SopReader, register_kind};
use wl_hr::{OnboardingTemplate, StepInput, TemplateInput};
use wl_inventory::location::NegativeStockPolicy;
use wl_inventory::Location;
use wl_kernel::compliance::CriticalRule;
use wl_kernel::documents::{DocumentFuture, ReminderSchedule, register_reminder_schedule};
use wl_kernel::integrations::{Delivery, IntegrationDef, Outbound};
use wl_kernel::late_entries::{LateEntryLimits, Limits, set_limits};
use wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;
use wl_kernel::{Manifest, Subject};
use wl_notify::WlNotify;
use wl_notify::window::Window;
use wl_sales::{LoyaltyProgramme, MatchMode, NewProgramme, Rule, Segment};

use super::roles::{
    AREA_MANAGER, CASHIER, DRIVER, HEAD_CHEF, KITCHEN_MANAGER, LINE_COOK, OPERATIONS_DIRECTOR,
    STOREKEEPER,
};

/// The client app these values are for.
pub const FNB: &str = super::fnb_seed::FNB;

// ---------------------------------------------------------------- documents

/// The trade licence of a legal entity.
pub const TRADE_LICENCE: &str = "trade_licence";
/// The food permit of an outlet.
pub const FOOD_PERMIT: &str = "food_permit";
/// The health card of an employee.
pub const HEALTH_CARD: &str = "health_card";
/// The PIC (person in charge) certificate of an employee.
pub const PIC_CERTIFICATE: &str = "pic_certificate";

/// The days before expiry the permits and the licence remind (`DOC.RUL.01`).
pub const PERMIT_REMINDERS: [i64; 3] = [90, 30, 7];
/// The days before expiry the cards remind.
pub const CARD_REMINDERS: [i64; 3] = [60, 30, 7];

/// A document kind of 06 (06 §4.6, §7.2): key, English and Arabic name,
/// subject type, responsible role and reminder days.
pub type KindSpec = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static [i64],
);

/// The kinds 06 seeds. None has `blocks`.
pub const KINDS: [KindSpec; 4] = [
    (
        TRADE_LICENCE,
        "Trade licence",
        "الرخصة التجارية",
        LEGAL_ENTITY_SUBJECT,
        OPERATIONS_DIRECTOR,
        &PERMIT_REMINDERS,
    ),
    (
        FOOD_PERMIT,
        "Food permit",
        "تصريح الغذاء",
        OUTLET_SUBJECT_TYPE,
        KITCHEN_MANAGER,
        &PERMIT_REMINDERS,
    ),
    (
        HEALTH_CARD,
        "Health card",
        "البطاقة الصحية",
        EMPLOYEE_SUBJECT_TYPE,
        KITCHEN_MANAGER,
        &CARD_REMINDERS,
    ),
    (
        PIC_CERTIFICATE,
        "PIC certificate",
        "شهادة الشخص المسؤول",
        EMPLOYEE_SUBJECT_TYPE,
        KITCHEN_MANAGER,
        &CARD_REMINDERS,
    ),
];

/// The kind `spec` as wl-documents' seed: an expiring, numbered document that
/// reminds its responsible role and is compliance-critical (the owner is told
/// when it expires, `DOC.RUL.03`); it blocks nothing.
pub fn kind_seed(spec: &KindSpec) -> KindSeed {
    let (key, name_en, name_ar, subject, role, reminders) = *spec;
    KindSeed::new(key, name_en)
        .name_ar(name_ar)
        .subject_type(subject)
        .expires()
        .numbered()
        .reminders(reminders)
        .responsible_role(role)
        .compliance_critical()
}

/// Where the reminders of a kitchen document go: the outlet the permit is
/// for, or the outlets the employee is assigned to (`wl-hr` scopes), so the
/// kitchen manager of that outlet is told and the manager of another is not.
/// An employee with no outlet is told to every holder of the role.
struct OutletOf;

impl ReminderSchedule for OutletOf {
    fn scopes<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<Vec<Scope>>> {
        Box::pin(async move {
            if subject.subject_type == OUTLET_SUBJECT_TYPE {
                return Ok(Some(vec![Scope::new(OUTLET_SUBJECT_TYPE, subject.subject_id)]));
            }
            if subject.subject_type != EMPLOYEE_SUBJECT_TYPE {
                return Ok(None);
            }
            let Some(employee) = wl_hr::Employee::find(&state.db, subject.subject_id).await?
            else {
                return Ok(None);
            };
            let outlets = employee.scopes(&state.db).await?;
            if outlets.is_empty() {
                return Ok(None);
            }
            Ok(Some(
                outlets
                    .iter()
                    .map(|outlet| Scope::new(&outlet.subject_type, outlet.subject_id))
                    .collect(),
            ))
        })
    }
}

/// Registers the 06 kinds with wl-documents (only for the F&B client) and
/// turns on the registration kinds 06 uses: *Halal certificate* (the
/// storekeeper replaces the seeded *quality role, else owner*, 06 has no
/// quality role), *Trademark registration* and *Other certification* (the
/// operations director, the owner), with no `blocks` (`DOC.EXP.01`,
/// `DOC.RUL.01`).
pub fn register_documents(client_app: &str) {
    if client_app != FNB {
        return;
    }
    for spec in &KINDS {
        register_kind(kind_seed(spec));
    }
    for key in [FOOD_PERMIT, HEALTH_CARD, PIC_CERTIFICATE] {
        register_reminder_schedule(key, OutletOf);
    }
    register_vertical(
        FNB,
        VerticalSettings::new()
            .enabled(&[TRADEMARK, HALAL, OTHER])
            .role(TRADEMARK, OPERATIONS_DIRECTOR)
            .role(HALAL, STOREKEEPER)
            .role(OTHER, OPERATIONS_DIRECTOR),
    );
}

// -------------------------------------------------------------------- rules

/// The rules 06 marks compliance-critical (§7.2): the opening checklist, the
/// temperature readings, the temperature hold and the expiry of the licence,
/// permits and cards. A run that fails after its last attempt, or a scheduled
/// run that does not happen, alerts the owner (`PLT.RUL.04`).
pub const CRITICAL_RULES: [&str; 4] = ["FNB.RUL.03", "FNB.RUL.04", "FNB.RUL.05", "DOC.RUL.03"];

/// Marks the rules of 06 compliance-critical for an F&B client, the operations
/// director being the owner of the alert.
pub fn register_critical(client_app: &str) {
    if client_app != FNB {
        return;
    }
    for rule in CRITICAL_RULES {
        CriticalRule::new(rule)
            .owner_role(OPERATIONS_DIRECTOR)
            .register();
    }
}

// ------------------------------------------------------------- integrations

/// The key of the Deliverect connection (aggregator orders into the kitchen).
pub const DELIVERECT: &str = "fnb.deliverect";

/// The fallback of Deliverect (06 §4.1): the aggregator tablets and quick
/// entry; on recovery an import that matches a number already entered is
/// refused as a duplicate and listed for the kitchen manager.
pub const DELIVERECT_FALLBACK_EN: &str = "Take the orders on the aggregators' tablets and enter them by quick entry. When Deliverect is back, an import whose number was entered already is refused as a duplicate and listed for the kitchen manager.";
/// See [`DELIVERECT_FALLBACK_EN`].
pub const DELIVERECT_FALLBACK_AR: &str = "استقبل الطلبات على أجهزة المنصات وأدخلها بالإدخال السريع. عند عودة Deliverect يُرفض الاستيراد الذي سبق إدخال رقمه كطلب مكرر ويُعرض على مدير المطبخ.";

/// The fallback of the payment gateway (06 §4.1): online payment on the
/// brand's site is paused; cash on delivery and the counter card terminal stay.
pub const GATEWAY_FALLBACK_EN: &str = "Online payment on the ordering site is paused. Cash on delivery and the card terminal at the counter stay available.";
/// See [`GATEWAY_FALLBACK_EN`].
pub const GATEWAY_FALLBACK_AR: &str = "الدفع الإلكتروني في موقع الطلب متوقف. يبقى الدفع نقدًا عند التسليم وجهاز البطاقات عند الكاونتر متاحين.";

/// The fallback of the SMS gateway (06 §4.1): the one-time code login is
/// unavailable and orders are taken by phone.
pub const SMS_FALLBACK_EN: &str = "Login by one-time code is unavailable: the login screen shows the outlet's phone. Take the orders by phone.";
/// See [`SMS_FALLBACK_EN`].
pub const SMS_FALLBACK_AR: &str = "تسجيل الدخول برمز لمرة واحدة غير متاح: تعرض شاشة الدخول هاتف الفرع. استقبل الطلبات بالهاتف.";

/// The send handler of a connection nothing is queued for: the manual way is
/// the fallback.
fn sent<'a>(
    _state: &'a AppState,
    _call: &'a Outbound,
) -> Pin<Box<dyn Future<Output = Result<Delivery>> + Send + 'a>> {
    Box::pin(async { Ok(Delivery::Sent) })
}

/// Registers the connections 06 keeps a manual fallback for (06 §4.1, §7.2,
/// wl-kernel §5.14): Deliverect (the kitchen manager), the payment gateway
/// (the accountant) and the SMS gateway (the kitchen manager). Each shows its
/// banner on the screens it affects.
pub fn register_integrations(client_app: &str) {
    if client_app != FNB {
        return;
    }
    IntegrationDef::of(DELIVERECT, "fnb", sent)
        .owner_role(KITCHEN_MANAGER)
        .screens(["fnb.orders", "fnb.paper_orders"])
        .fallback(DELIVERECT_FALLBACK_EN, Some(DELIVERECT_FALLBACK_AR))
        .register();
    IntegrationDef::of(wl_kernel::integrations::PAYMENT_GATEWAY, "payments", sent)
        .owner_role(super::roles::ACCOUNTANT)
        .screens(["fnb.checkout"])
        .fallback(GATEWAY_FALLBACK_EN, Some(GATEWAY_FALLBACK_AR))
        .register();
    IntegrationDef::of(wl_kernel::integrations::SMS, "notify", sent)
        .owner_role(KITCHEN_MANAGER)
        .screens(["fnb.checkout", "fnb.login"])
        .fallback(SMS_FALLBACK_EN, Some(SMS_FALLBACK_AR))
        .register();
}

// ------------------------------------------------------ notices, staff, sales

/// At most this many marketing messages go to one customer in 7 days
/// (`NTF.RUL.02`).
pub const MARKETING_CAP: i64 = 2;
/// A broadcast to more than this many recipients needs a second person
/// (`NTF.ACL.03`).
pub const BROADCAST_APPROVAL_ABOVE: i64 = 200;
/// The send window of a marketing message, in minutes after midnight (07:00 to
/// 21:00, `NTF.RUL.02`).
pub const SEND_WINDOW_MINUTES: (u32, u32) = (7 * 60, 21 * 60);

/// The notification module with the 06 client's values: at most 2 marketing
/// messages per customer in 7 days and a second person for a broadcast to more
/// than 200 customers; the send window is stored by [`install`].
pub fn notify(module: WlNotify) -> WlNotify {
    module
        .marketing_frequency_cap(MARKETING_CAP)
        .broadcast_approval_above(BROADCAST_APPROVAL_ABOVE)
}

/// The HR module with the 06 client's values: the kitchen manager is the line
/// manager of the kitchen, cashier, rider and storekeeper staff. 06 has no HR
/// role: the company admin is the second person for waiving or reopening a
/// step, the same reason as in [`super::logistics_seed::hr`].
pub fn hr(module: wl_hr::WlHr) -> wl_hr::WlHr {
    module
        .manager_role(KITCHEN_MANAGER)
        .owner_role(super::roles::ADMIN)
}

// ------------------------------------------------------------- late entries

/// M of `PLT.ACL.10`: minutes of delay above which a late entry is flagged
/// with a reason.
pub const LATE_ENTRY_FLAG_MINUTES: i64 = 30;
/// N of `PLT.ACL.10`: hours of delay (or an entry into a submitted daily
/// closing) above which the area manager approves.
pub const LATE_ENTRY_APPROVAL_HOURS: i64 = 24;

/// The entities that can be entered late after an outage, by business time
/// (06 §7.2): the paper orders (one type per outlet besides the general one),
/// wastage, temperature readings, counter sales and the delivery proof.
pub const LATE_ENTRY_TYPES: [&str; 7] = [
    "fnb.paper_order",
    "fnb.paper_order_marina",
    "fnb.paper_order_sharjah",
    "fnb.wastage",
    "fnb.temperature_reading",
    "fnb.counter_sale",
    "delivery.pod",
];

// -------------------------------------------------------------------- staff

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

const WELCOME: StepSpec = ("Welcome and introductions", "manager", 1, "task");
const KITCHEN_MANAGER_STEP: &str = "role:kitchen_manager";
const HEAD_CHEF_STEP: &str = "role:head_chef";
const EMPLOYEE_DOCUMENTS: StepSpec = (
    "Upload your health card and personal documents",
    "employee",
    3,
    "task",
);
const SOP_STEP: StepSpec = ("SOPs acknowledged", "employee", 7, "sops");

/// The checklists 06 seeds (`HRM.RUL.16`): started when the employee record
/// is created in `onboarding`; tasks go to the kitchen manager of the outlet,
/// the head chef and the employee; completion blocks nothing (health cards
/// stay tracked by `DOC.EXP.01`).
pub const ONBOARDING: [OnboardingSpec; 5] = [
    OnboardingSpec {
        name: "Line cook onboarding",
        role: LINE_COOK,
        steps: &[
            WELCOME,
            ("Kitchen walk-through and food safety", HEAD_CHEF_STEP, 2, "task"),
            ("Station training", HEAD_CHEF_STEP, 5, "task"),
            ("Access to the kitchen display", KITCHEN_MANAGER_STEP, 2, "task"),
            EMPLOYEE_DOCUMENTS,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Cashier and order taker onboarding",
        role: CASHIER,
        steps: &[
            WELCOME,
            ("Till and order entry training", KITCHEN_MANAGER_STEP, 3, "task"),
            ("Cash handling and refunds walk-through", KITCHEN_MANAGER_STEP, 3, "task"),
            EMPLOYEE_DOCUMENTS,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Rider onboarding",
        role: DRIVER,
        steps: &[
            WELCOME,
            ("Rider app and proof of delivery training", KITCHEN_MANAGER_STEP, 2, "task"),
            ("Cash handover and food handling", KITCHEN_MANAGER_STEP, 3, "task"),
            EMPLOYEE_DOCUMENTS,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Storekeeper and purchaser onboarding",
        role: STOREKEEPER,
        steps: &[
            WELCOME,
            ("Receiving, lots and temperature checks", HEAD_CHEF_STEP, 3, "task"),
            ("Stock counts and purchase orders", KITCHEN_MANAGER_STEP, 3, "task"),
            EMPLOYEE_DOCUMENTS,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Outlet and kitchen manager onboarding",
        role: KITCHEN_MANAGER,
        steps: &[
            ("Welcome and introductions", "role:area_manager", 1, "task"),
            ("Daily closing, cash and delivery walk-through", "role:area_manager", 3, "task"),
            ("Food safety and recall walk-through", HEAD_CHEF_STEP, 3, "task"),
            EMPLOYEE_DOCUMENTS,
            SOP_STEP,
        ],
    },
];

/// An SOP of the library with its owner role and required readers.
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
    /// The roles that must read and acknowledge it, per outlet.
    pub readers: &'static [&'static str],
}

/// The SOP library 06 seeds: the head chef owns the recipes and the food
/// safety, the area manager the service, the cash and the delivery.
/// `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3
/// days and tells the reader's manager and the SOP's owner role after 7 days
/// overdue (the module's defaults).
pub const SOPS: [SopSpec; 5] = [
    SopSpec {
        code: "SOP-FNB-RECIPES",
        title: "Recipes and portions",
        category: "kitchen",
        owner_role: HEAD_CHEF,
        readers: &[LINE_COOK, HEAD_CHEF],
    },
    SopSpec {
        code: "SOP-FNB-FOOD-SAFETY",
        title: "Food safety, temperatures and recalls",
        category: "food_safety",
        owner_role: HEAD_CHEF,
        readers: &[LINE_COOK, HEAD_CHEF, KITCHEN_MANAGER, STOREKEEPER],
    },
    SopSpec {
        code: "SOP-FNB-SERVICE",
        title: "Service and customer complaints",
        category: "service",
        owner_role: AREA_MANAGER,
        readers: &[CASHIER, KITCHEN_MANAGER],
    },
    SopSpec {
        code: "SOP-FNB-CASH",
        title: "Cash, tills and refunds",
        category: "finance",
        owner_role: AREA_MANAGER,
        readers: &[CASHIER, DRIVER, KITCHEN_MANAGER],
    },
    SopSpec {
        code: "SOP-FNB-DELIVERY",
        title: "Delivery and failed deliveries",
        category: "delivery",
        owner_role: AREA_MANAGER,
        readers: &[DRIVER, KITCHEN_MANAGER],
    },
];

// -------------------------------------------------------------------- sales

/// The loyalty programme 06 seeds (`SAL.LOY.01`, `SAL.RUL.07`): 1 point per
/// AED 1 of a direct order (an aggregator order has no member: none earned),
/// 100 points an AED 5 discount line of at most 50% of the order, confirmed by
/// a one-time code to the member, points valid 12 months.
pub const LOYALTY_NAME: &str = "Zaatar Rewards";
/// The value of a point, in fils (100 points = AED 5).
pub const LOYALTY_POINT_VALUE_FILS: i64 = 5;
/// The fewest points that can be redeemed.
pub const LOYALTY_MIN_REDEEM: i64 = 100;
/// The share of an order a redemption may take, in basis points (50%).
pub const LOYALTY_MAX_SHARE_BPS: i64 = 5_000;
/// The months points are valid after they are earned (`SAL.RUL.08`).
pub const LOYALTY_VALIDITY_MONTHS: i64 = 12;
/// The days before an earn lot expires that its member is reminded.
pub const LOYALTY_REMINDER_DAYS: i64 = 30;
/// A manual adjustment above this many points needs a manager (`SAL.ACL.07`).
pub const LOYALTY_ADJUSTMENT_LIMIT: i64 = 500;
/// The regulars: at least 3 orders in 30 days.
pub const REGULARS: (&str, i64, i64) = ("Regulars", 3, 30);
/// The lapsed customers: no order in 45 days.
pub const LAPSED: (&str, i64) = ("Lapsed customers", 45);

// ------------------------------------------------------------------- install

/// What [`install`] stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Late-entry limit rows created.
    pub late_entry_limits: usize,
    /// Onboarding checklists created.
    pub checklists: usize,
    /// SOPs created.
    pub sops: usize,
    /// Marketing send windows stored.
    pub send_windows: usize,
    /// Loyalty programmes created (0 or 1).
    pub loyalty: usize,
    /// Segments created.
    pub segments: usize,
    /// Stock locations of outlets given their scope or negative policy.
    pub locations: usize,
}

impl Installed {
    /// Everything created or changed.
    pub fn total(&self) -> usize {
        self.late_entry_limits
            + self.checklists
            + self.sops
            + self.send_windows
            + self.loyalty
            + self.segments
            + self.locations
    }
}

/// Stores what the 06 client's modules lack, for the modules the manifest
/// enables.
///
/// # Errors
///
/// The database's errors.
pub async fn install(state: &AppState) -> Result<Installed> {
    let manifest = state.provided::<std::sync::Arc<Manifest>>();
    let on = |name: &str| manifest.as_ref().is_some_and(|m| m.is_enabled(name));
    let mut installed = Installed {
        late_entry_limits: install_late_entry_limits(&state.db).await?,
        ..Installed::default()
    };
    if on("hr") {
        installed.checklists = install_checklists(&state.db).await?;
    }
    if on("documents") {
        installed.sops = install_sops(&state.db).await?;
    }
    if on("notify") {
        installed.send_windows = install_send_windows(&state.db).await?;
    }
    if on("sales") {
        installed.loyalty = install_loyalty(&state.db).await?;
        installed.segments = install_segments(state).await?;
    }
    if on("inventory") {
        installed.locations = outlet_stock(&state.db).await?;
    }
    Ok(installed)
}

/// M and N of the late entries of 06 (`PLT.ACL.10`): flagged with a reason
/// after 30 minutes, approved by the area manager after 24 hours (or into a
/// submitted closing). A limit the client changed is kept.
async fn install_late_entry_limits(db: &Db) -> Result<usize> {
    let mut created = 0;
    for subject_type in LATE_ENTRY_TYPES {
        let found = LateEntryLimits::query()
            .where_(LateEntryLimits::SUBJECT_TYPE.eq(subject_type))
            .exists(db)
            .await?;
        if found {
            continue;
        }
        set_limits(
            db,
            subject_type,
            Limits {
                flag_after_minutes: LATE_ENTRY_FLAG_MINUTES,
                approval_after_hours: LATE_ENTRY_APPROVAL_HOURS,
            },
        )
        .await?;
        created += 1;
    }
    Ok(created)
}

/// The step of an onboarding checklist as wl-hr's input.
fn step_input(step: &StepSpec) -> StepInput {
    let (title, owner, due_days, kind) = *step;
    if kind == "sops" {
        StepInput::sops(title, owner, due_days)
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
/// their owner role and required readers; the owner attaches a version, and
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

/// Stores the marketing send window 07:00-21:00 of each channel that has none
/// (`NTF.RUL.02`); a window the client changed is kept.
async fn install_send_windows(db: &Db) -> Result<usize> {
    use wl_kernel::notify::NotifyChannel;
    let (start, end) = SEND_WINDOW_MINUTES;
    let window = Window { start, end };
    let mut stored = 0;
    for channel in [
        NotifyChannel::Sms,
        NotifyChannel::Whatsapp,
        NotifyChannel::Email,
    ] {
        let found = wl_notify::SendWindow::query()
            .where_(wl_notify::SendWindow::CHANNEL.eq(channel))
            .exists(db)
            .await?;
        if found {
            continue;
        }
        if wl_notify::SendWindow::save_checked(db, channel, window)
            .await?
            .is_ok()
        {
            stored += 1;
        }
    }
    Ok(stored)
}

/// Creates the loyalty programme where the client has none (`SAL.LOY.01`,
/// `SAL.RUL.07`).
async fn install_loyalty(db: &Db) -> Result<usize> {
    if LoyaltyProgramme::first_active(db).await?.is_some() {
        return Ok(0);
    }
    let mut new = NewProgramme::new(LOYALTY_NAME)
        .points_per_aed(1)
        .redeeming(LOYALTY_POINT_VALUE_FILS, LOYALTY_MIN_REDEEM)
        .validity_months(LOYALTY_VALIDITY_MONTHS)
        .terms(
            "1",
            "You earn 1 point for every AED 1 you pay on our own ordering site and at the counter. Orders through delivery apps earn nothing. 100 points give an AED 5 discount on a later order, up to half of the order, confirmed with a one-time code. Points expire 12 months after they are earned.",
            "تكسب نقطة واحدة عن كل درهم تدفعه على موقعنا للطلب وعند الكاونتر. لا تكسب الطلبات عبر تطبيقات التوصيل نقاطًا. تعادل 100 نقطة خصمًا قدره 5 دراهم على طلب لاحق، بحد أقصى نصف قيمة الطلب، بعد تأكيده برمز لمرة واحدة. تنتهي صلاحية النقاط بعد 12 شهرًا من كسبها.",
        );
    new.max_share_bps = LOYALTY_MAX_SHARE_BPS;
    wl_sales::loyalty_programme::create(db, new).await?;
    Ok(1)
}

/// Stores the segments 06 uses that the rules of the sales module can express:
/// the regulars (3 or more orders in 30 days) and the lapsed customers (no
/// order in 45 days). *First order only* and *corporate catering customers*
/// need a rule the module does not offer (`DEFERRED-fnb-settings.md`).
async fn install_segments(state: &AppState) -> Result<usize> {
    let config = wl_sales::config::config_of(state);
    let mut created = 0;
    let wanted = [
        (
            REGULARS.0,
            Rule::SalesAtLeast {
                count: REGULARS.1,
                days: REGULARS.2,
            },
        ),
        (LAPSED.0, Rule::NoSaleForDays { days: LAPSED.1 }),
    ];
    for (name, rule) in wanted {
        let exists = Segment::query()
            .where_(Segment::NAME.eq(name))
            .exists(&state.db)
            .await?;
        if exists {
            continue;
        }
        let stored = Segment::of(name, MatchMode::All, &[rule])
            .store(&state.db, &config)
            .await?;
        match stored {
            Ok(_) => created += 1,
            Err(refused) => {
                return Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("{refused:?}"),
                ));
            }
        }
    }
    Ok(created)
}

/// The outlet that is the central kitchen: its stock is lot-tracked and counted,
/// so it blocks a movement below zero.
pub const CENTRAL_OUTLET: &str = "central";

/// Gives the stock location of each outlet its scope (the outlet, so that the
/// head chef and the kitchen manager of that outlet decide its hold lines,
/// `STK.ACL.05`) and its negative stock policy (`STK.RUL.06`): an outlet's
/// kitchen allows automatic consumption below zero for items that are not
/// lot-tracked, the central kitchen blocks. A location the client edited since
/// it was created is left as it is. Returns how many locations it changed.
///
/// # Errors
///
/// The database's errors.
pub async fn outlet_stock(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for outlet in Outlet::query().get(db).await? {
        let Some(location_id) = outlet.location_id else {
            continue;
        };
        let Some(mut location) = Location::find(db, location_id).await? else {
            continue;
        };
        let untouched = location.updated_at == location.created_at;
        if !untouched && location.scope_type.is_some() {
            continue;
        }
        let policy = if outlet.code == CENTRAL_OUTLET {
            NegativeStockPolicy::Block
        } else {
            NegativeStockPolicy::AllowConsumption
        };
        let scope_missing = location.scope_type.is_none();
        let policy_wrong = untouched && location.negative_stock_policy != policy;
        if !scope_missing && !policy_wrong {
            continue;
        }
        if scope_missing {
            location = location.in_scope(&outlet.subject());
        }
        if policy_wrong {
            location.negative_stock_policy = policy;
        }
        location.save(db).await?;
        changed += 1;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_kind_blocks_anything() {
        for spec in &KINDS {
            let seed = kind_seed(spec);
            assert!(seed.blocks.is_empty(), "{}", spec.0);
            assert!(!seed.legal_block, "{}", spec.0);
            assert!(seed.compliance_critical, "{}", spec.0);
            assert_eq!(seed.responsible_role.as_deref(), Some(spec.4));
        }
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
            assert_eq!(spec.steps.last().map(|s| s.3), Some("sops"));
        }
    }

    #[test]
    fn the_outlet_subject_type_is_the_outlets() {
        assert_eq!(OUTLET_SUBJECT_TYPE, "fnb.outlet");
    }
}
