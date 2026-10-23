//! The 08 Auto client's values for the shared document, scheduling,
//! notification, alert, sales, staff and integration mechanisms (08 §4.1, §7.1
//! *Negative-flow settings*, *Notification types registered by 08*, §7.2, §8,
//! §9; BL-AUT-15, BL-AUT-18), and the rows of the page `/settings/compliance`
//! that shows them.
//!
//! The mechanisms live in the shared modules; the numbers are
//! `auto::defaults` (and `auto::seed` for what the earlier issues seeded) and
//! this file turns them into the owning modules' own calls:
//!
//! - the connections with their manual fallbacks ([`register_integrations`]):
//!   the SMS gateway (the advisors call the customer, the messages are queued);
//!   the e-invoicing ASP is wl-invoicing's, the insurer portals and the police
//!   reports are manual already and have no connection;
//! - the notification module ([`notify_for`]): at most 2 marketing messages per
//!   contact in 7 days, a broadcast to more than 200 needs a second approver;
//! - the sales module ([`sales`]): back-orders on the counter, phone and sales
//!   rep channels, no source requires a campaign, a cost entry over a
//!   campaign's budget needs a manager, points expire with a reminder 30 days
//!   before, a manual adjustment above 500 points needs a manager, a redemption
//!   is confirmed by a one-time code;
//! - the seed ([`install_for`], run by `auto:install`): the lead sources (the
//!   aggregator turned off), the marketing send window 07:00-21:00, the loyalty
//!   programme, the onboarding checklists, the SOP library, the reactions of
//!   the notification types of the shared modules and the budget category
//!   owners. It adds what the database lacks and keeps what the client changed.
//!
//! Decisions (the rest is in `$S/f8/DEFERRED.md`):
//!
//! - **Segments.** The segments of the spec (service due by date or mileage, no
//!   visit in 12 months, vehicle make, spend over AED 5,000) need rules over the
//!   vehicle register; wl-sales has "at least" rules and the vertical rule kind,
//!   which 08 does not register. None is seeded.
//! - **Loyalty VAT.** The VAT of the discounted supply is `⚠ open: accountant
//!   (A-15)`; the rest of the loyalty settings are built.
//! - **Uncollected vehicles.** The storage fee switch lives in the table
//!   `auto_settings` ([`auto::settings`]); the page shows its stored value and
//!   the command `auto:storage-fee on|off` sets it.

use std::future::Future;
use std::pin::Pin;

use auto::defaults::{self, OnboardingSpec, SOPS};
use auto::permissions::{
    BRANCH_MANAGER_ROLE, INSURANCE_COORDINATOR_ROLE, OWNER_ROLE, WORKSHOP_MANAGER_ROLE,
};
use renox::db::Json;
use renox::prelude::*;
use wl_documents::{Sop, SopReader};
use wl_hr::{OnboardingTemplate, StepInput, TemplateInput};
use wl_kernel::compliance::is_critical;
use wl_kernel::integrations::{self, Delivery, IntegrationDef};
use wl_notify::window::Window;
use wl_notify::{NotificationType, SendWindow, WlNotify};
use wl_sales::{LeadSource, LoyaltyProgramme, NewProgramme, WlSales};

use super::auto_seed::AUTO;
use super::compliance_settings::{Row, kind_text, type_text};

/// The row's value is this client's seed.
const SEED: &str = "08 seed (BL-AUT-18)";
/// The row's value is the module's own default.
const MODULE: &str = "default of the module";

/// The order channels whose lines without stock are back-ordered in 08
/// (`BL-AUT-15`, wl-sales §7 *Back-orders allowed per channel*): the counter,
/// the phone and the sales reps. The portal stays off.
pub const BACK_ORDER_CHANNELS: &[&str] = &["counter", "phone", "sales_rep"];

/// The notification module with the 08 client's values (`NTF.RUL.02`,
/// `NTF.ACL.03`): at most 2 marketing messages per recipient in 7 days, and a
/// broadcast to more than 200 recipients needs a second approver. The send
/// window 07:00-21:00 is stored by [`install_for`].
pub fn notify_for() -> WlNotify {
    WlNotify::new()
        .marketing_frequency_cap(defaults::MARKETING_CAP)
        .broadcast_approval_above(defaults::BROADCAST_APPROVAL_ABOVE)
}

/// The sales module with the 08 client's values (`BL-AUT-15`, `SAL.CRM.02`,
/// `SAL.ACL.07`, `SAL.LOY.01`, `SAL.RUL.08`): back-orders on the counter,
/// phone and sales rep channels; no lead source requires a campaign; a cost
/// entry over a campaign's budget needs a manager; points expire with a
/// reminder 30 days before; a manual adjustment above 500 points needs a
/// manager; a redemption is confirmed by a one-time code.
pub fn sales(module: WlSales) -> WlSales {
    module
        .back_orders_for(BACK_ORDER_CHANNELS)
        .over_budget_approval(true)
        .campaign_required_for(defaults::CAMPAIGN_REQUIRED_SOURCES)
        .expiry_reminder_days(defaults::LOYALTY_REMINDER_DAYS)
        .adjustment_limit(defaults::LOYALTY_ADJUSTMENT_LIMIT)
        .confirm_redemption_by_code(true)
}

/// The send handler of a connection nothing is queued for: the manual way is
/// the fallback.
fn sent<'a>(
    _state: &'a AppState,
    _call: &'a integrations::Outbound,
) -> Pin<Box<dyn Future<Output = Result<Delivery>> + Send + 'a>> {
    Box::pin(async { Ok(Delivery::Sent) })
}

/// Registers the connection 08 keeps a manual fallback for (08 §7.1
/// *Integrations*, §8, wl-kernel §5.14): the SMS gateway (the advisors call the
/// customer; the messages are queued and sent on recovery). It shows its
/// banner on the job card screens. wl-notify reports the SMS sends; an
/// administrator can take it out of service by hand.
pub fn register_integrations(client_app: &str) {
    if client_app != AUTO {
        return;
    }
    IntegrationDef::of(integrations::SMS, "notify", sent)
        .owner_role(BRANCH_MANAGER_ROLE)
        .screens([defaults::JOB_CARDS_SCREEN])
        .fallback(defaults::SMS_FALLBACK_EN, Some(defaults::SMS_FALLBACK_AR))
        .register();
}

/// What [`install_for`] stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Lead sources turned off.
    pub sources_off: usize,
    /// Onboarding checklists created.
    pub checklists: usize,
    /// SOPs created.
    pub sops: usize,
    /// Marketing send windows stored.
    pub send_windows: usize,
    /// Loyalty programmes created (0 or 1).
    pub loyalty: usize,
    /// Notification types given their 08 reactions.
    pub notification_types: usize,
    /// Budget categories given the accountant as owner role.
    pub budget_categories: usize,
}

impl Installed {
    /// Everything created or changed.
    pub fn total(&self) -> usize {
        self.sources_off
            + self.checklists
            + self.sops
            + self.send_windows
            + self.loyalty
            + self.notification_types
            + self.budget_categories
    }
}

/// Stores what the 08 client's shared modules lack, for the modules `on` says
/// are enabled, and keeps what the client changed.
///
/// # Errors
///
/// The database's errors.
pub async fn install_for(
    state: &AppState,
    on: &(dyn Fn(&str) -> bool + Sync),
) -> Result<Installed> {
    let mut installed = Installed::default();
    if on("sales") {
        installed.sources_off = turn_off_unused_sources(&state.db).await?;
        installed.loyalty = install_loyalty(&state.db).await?;
    }
    if on("hr") {
        installed.checklists = install_checklists(&state.db).await?;
    }
    if on("documents") {
        installed.sops = install_sops(&state.db).await?;
    }
    if on("invoicing") {
        installed.budget_categories =
            super::auto_money::install_budget_categories(&state.db).await?;
    }
    if on("notify") {
        installed.send_windows = install_send_windows(&state.db).await?;
        installed.notification_types = install_notification_types(&state.db).await?;
    }
    Ok(installed)
}

/// Turns off the seeded lead sources 10 does not use (*aggregator*).
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

/// Creates the loyalty programme where the client has none (`SAL.LOY.01`,
/// `SAL.RUL.07`): 1 point per AED 10 of workshop invoices and counter sales to
/// an individual car owner, 100 points an AED 10 discount line on a later
/// workshop invoice (10 fils a point), valid 24 months. A company has no
/// loyalty account, so B2B trade, fleet and insurer-paid jobs earn nothing.
/// The VAT of the discounted supply is `⚠ open: accountant (A-15)`.
async fn install_loyalty(db: &Db) -> Result<usize> {
    if LoyaltyProgramme::first_active(db).await?.is_some() {
        return Ok(0);
    }
    let new = NewProgramme::new(defaults::LOYALTY_NAME)
        .points_per_aed_units(defaults::LOYALTY_POINTS, defaults::LOYALTY_EARN_PER_AED)
        .redeeming(
            defaults::LOYALTY_POINT_VALUE_FILS,
            defaults::LOYALTY_MIN_REDEEM,
        )
        .validity_months(defaults::LOYALTY_VALIDITY_MONTHS)
        .terms(
            "1",
            "You earn 1 point for every AED 10 you pay the workshop or the parts counter as an individual car owner. 100 points give an AED 10 discount line on a later workshop invoice, confirmed with a one-time code. Points expire 24 months after they are earned. Trade, fleet and insurer-paid jobs earn nothing.",
            "تكسب نقطة واحدة عن كل 10 دراهم تدفعها للورشة أو لمتجر القطع كمالك سيارة فرد. تعادل 100 نقطة خصمًا قدره 10 دراهم على فاتورة ورشة لاحقة، بعد تأكيده برمز لمرة واحدة. تنتهي صلاحية النقاط بعد 24 شهرًا من كسبها. أعمال الشركات والأساطيل وما تدفعه شركات التأمين لا تكسب نقاطًا.",
        );
    wl_sales::loyalty_programme::create(db, new).await?;
    Ok(1)
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
    let (start, end) = defaults::SEND_WINDOW_MINUTES;
    let window = Window {
        start: u32::try_from(start).unwrap_or(420),
        end: u32::try_from(end).unwrap_or(1260),
    };
    let mut stored = 0;
    for channel in [
        NotifyChannel::Sms,
        NotifyChannel::Whatsapp,
        NotifyChannel::Email,
    ] {
        let found = SendWindow::query()
            .where_(SendWindow::CHANNEL.eq(channel))
            .exists(db)
            .await?;
        if found {
            continue;
        }
        if SendWindow::save_checked(db, channel, window).await?.is_ok() {
            stored += 1;
        }
    }
    Ok(stored)
}

/// Gives the notification types of the shared modules their 10 group,
/// give-up reactions and staff role (`defaults::NOTIFICATION_TYPES`). A type
/// the client edited is kept as it is; the write leaves `updated_at` alone, so
/// a second run changes nothing. (The types 10 registers itself carry their
/// own.)
async fn install_notification_types(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for spec in &defaults::NOTIFICATION_TYPES {
        let Some(row) = NotificationType::by_key(db, spec.key).await? else {
            continue;
        };
        let same = row.group_key == spec.group
            && !row.statutory
            && row.give_up.0 == spec.give_up
            && row.staff_role.as_deref() == spec.role;
        if same || !row.is_unedited() {
            continue;
        }
        let give_up = Json(spec.give_up.to_vec());
        let role = spec.role.map(str::to_owned);
        let statutory = false;
        NotificationType::query()
            .where_(NotificationType::ID.eq(row.id))
            .update(
                db,
                &[
                    ("group_key", &spec.group.to_owned()),
                    ("statutory", &statutory),
                    ("give_up", &give_up),
                    ("staff_role", &role),
                ],
            )
            .await?;
        changed += 1;
    }
    Ok(changed)
}

// ---- The page /settings/compliance for the 08 client -----------------------

fn row(
    number: &str,
    group: &str,
    setting: &str,
    value: String,
    source: &str,
    reference: &str,
) -> Row {
    Row {
        number: number.to_owned(),
        group: group.to_owned(),
        setting: setting.to_owned(),
        value,
        source: source.to_owned(),
        reference: reference.to_owned(),
        open: String::new(),
    }
}

fn open(mut row: Row, item: &str) -> Row {
    row.open = item.to_owned();
    row
}

/// `list` as `a/b/c`.
fn days(list: &[i64]) -> String {
    list.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join("/")
}

/// What the client's configuration says, read back from the modules' own
/// registries and builders. One row per line of the issue's settings table.
pub fn rows(owner_role: &str) -> Vec<Row> {
    let sales = sales(WlSales::new());
    let sales = sales.config();
    let scheduling = super::auto_seed::scheduling_for();
    let scheduling = scheduling.config();
    let critical: Vec<String> = defaults::CRITICAL_RULES
        .iter()
        .map(|rule| {
            let state = if is_critical(rule) {
                "marked"
            } else {
                "not registered"
            };
            format!("{rule} ({state})")
        })
        .collect();
    let kinds = [
        auto::seed::POLICE_REPORT_KIND,
        auto::seed::CUSTOMER_LICENCE_KIND,
        auto::seed::FLEET_CONTRACT_KIND,
    ];
    let kind_rows: Vec<String> = kinds
        .iter()
        .map(|key| format!("{key}: {}", kind_text(key)))
        .collect();
    let responsible = [
        (auto::seed::CUSTOMER_LICENCE_KIND, "accountant"),
        (auto::seed::FLEET_CONTRACT_KIND, OWNER_ROLE),
        (auto::seed::POLICE_REPORT_KIND, INSURANCE_COORDINATOR_ROLE),
        (auto::seed::DRIVING_LICENCE_KIND, INSURANCE_COORDINATOR_ROLE),
        (auto::seed::MULKIYA_KIND, INSURANCE_COORDINATOR_ROLE),
        (auto::seed::EMIRATES_ID_KIND, INSURANCE_COORDINATOR_ROLE),
    ]
    .iter()
    .map(|(key, role)| format!("{key}: {role}"))
    .collect::<Vec<_>>()
    .join("; ");
    let types: Vec<String> = [
        auto::insurer_mail::WITHDRAWAL_TYPE,
        auto::fleet_end::TERMINATION_TYPE,
        auto::notices::VEHICLE_READY,
        auto::notices::CLAIM_APPROVED,
        auto::declines::COLLECT_TYPE,
        auto::notices::FLEET_BOOKING_REQUEST,
        auto::fleet_end::NOTICE_TYPE,
        auto::parts_delays::DELAY_TYPE,
        auto::insurer_mail::SURVEY_TYPE,
        auto::supplier_rules::SUPPLIER_CLAIM_TYPE,
        auto::notices::INSURER_STATEMENT,
        auto::notices::SERVICE_REMINDER,
        auto::recalls::RECALL_NOTICE,
        "invoice.reminder_before_due",
        "invoice.reminder_after_due",
        "payments.payment_returned",
        wl_scheduling::notices::REMINDER,
    ]
    .iter()
    .map(|key| type_text(key))
    .collect();
    let channels: Vec<String> = defaults::CAMPAIGN_CHANNELS
        .iter()
        .map(|(name, kind)| format!("{name} (recorded as {kind})"))
        .collect();
    let onboarding: Vec<String> = defaults::ONBOARDING
        .iter()
        .map(|o| format!("{} ({} steps)", o.name, o.steps.len()))
        .collect();
    let sops: Vec<String> = SOPS
        .iter()
        .map(|s| format!("{} (owner {})", s.title, s.owner_role))
        .collect();
    vec![
        row(
            "1",
            "Documents",
            "Blocks and overrides (police accident report, subject job card or claim)",
            format!(
                "{}. The override stays never (a legal block, AUT.ACL.20): the client cannot switch it",
                kind_rows[0]
            ),
            SEED,
            "§7.1 settings Document blocks; §9",
        ),
        row(
            "2",
            "Documents",
            "Customer trade licences and fleet contracts (DOC.RUL.01)",
            format!(
                "reminders only, no blocks, look-ahead {} days: {} | {}",
                auto::seed::LOOK_AHEAD_DAYS,
                kind_rows[1],
                kind_rows[2]
            ),
            SEED,
            "§7.1 settings",
        ),
        row(
            "3",
            "Documents",
            "Responsible role per document kind (wl-documents §5.2)",
            responsible,
            SEED,
            "§7.1 settings",
        ),
        row(
            "4",
            "Scheduling",
            "Workshop bookings (SCH.RUL.05, SCH.RUL.06)",
            format!(
                "eligibility look-ahead D = {} days; an open conflict goes to the {} {} hours before the start; reminders {} hours before; booking deposit none (SCH.ACL.05 applies when a client takes deposits); a cancellation after check-in is staff only (SCH.ACL.06); check-in opens the job card",
                scheduling.look_ahead_days,
                scheduling.manager_role.as_deref().unwrap_or("nobody"),
                scheduling.escalation_hours,
                days(&scheduling.reminder_hours),
            ),
            SEED,
            "§7.1 Booking",
        ),
        row(
            "5",
            "Integrations",
            "Manual fallbacks (PLT.RUL.05)",
            format!(
                "{}: {} | asp (FIN.RUL.16): invoices are queued and sent on recovery (from 2027) | insurer portals and police reports: manual already, no connection",
                integrations::SMS,
                defaults::SMS_FALLBACK_EN
            ),
            SEED,
            "§7.1 settings Integrations; §8",
        ),
        row(
            "6",
            "Late entry",
            "Late entry (PLT.ACL.10)",
            format!(
                "test drives (entered through auto.test_drive_time), job card times and deliveries keep the business time; more than {} hours late waits for the {} or the {}",
                defaults::LATE_ENTRY_APPROVAL_HOURS, WORKSHOP_MANAGER_ROLE, BRANCH_MANAGER_ROLE
            ),
            "limit: the kernel's own (24 hours); approvers: 08 seed",
            "§7.1 settings Late entry",
        ),
        row(
            "7",
            "Alerts",
            "Compliance-critical rules and owner alerts (PLT.RUL.04)",
            format!(
                "{}; a failed run, or a day with no run an hour after 08:00, alerts the Owner / GM ({owner_role}) and the company admin and opens an owner task",
                critical.join("; ")
            ),
            SEED,
            "§7.1 settings; §7.2",
        ),
        row(
            "8",
            "Notifications",
            "Notification types registered by 08 and their groups",
            format!(
                "{} | staff-only alerts name no group and take none",
                types.join(" | ")
            ),
            SEED,
            "§7.1 Notification types registered by 08",
        ),
        row(
            "9",
            "Notifications",
            "Marketing caps and send window (NTF.RUL.02, NTF.ACL.03)",
            format!(
                "campaigns only to opted-in contacts (NTF.ACL.01), with opt-out, sent 07:00-21:00, at most {} marketing messages per contact in 7 days; a broadcast to more than {} recipients needs a second approver; AUT.RUL.01 service reminders unchanged",
                defaults::MARKETING_CAP,
                defaults::BROADCAST_APPROVAL_ABOVE
            ),
            SEED,
            "§7.1 settings",
        ),
        row(
            "10",
            "Sales",
            "Lead sources used (SAL.CRM.02)",
            format!(
                "{}; fleet contract enquiries are leads (SAL.CRM.01); walk-in customers get the source on their party; a source requires a campaign: {}",
                defaults::LEAD_SOURCES.join(", "),
                if sales.campaign_required_sources.is_empty() {
                    "none".to_owned()
                } else {
                    sales.campaign_required_sources.join(", ")
                }
            ),
            SEED,
            "§5; §7.1 settings",
        ),
        row(
            "11",
            "Sales",
            "Campaign channels (SAL.CRM.02, SAL.ACL.07)",
            format!(
                "{}; converted value: the first sales order, job card or fleet contract invoice; a cost entry over the budget needs a manager: {}",
                channels.join(", "),
                if sales.over_budget_approval { "on" } else { "off" }
            ),
            "channels: the module's fixed list; approval: 08 seed",
            "§5; §7.1 settings",
        ),
        row(
            "12",
            "Sales",
            "Back-orders allowed (BL-AUT-15)",
            format!(
                "on for {}; off for the portal and any other channel",
                sales.back_order_channels.join(", ")
            ),
            SEED,
            "§7.1 Sales order",
        ),
        open(
            row(
                "13",
                "Loyalty",
                "Earn and redeem (SAL.LOY.01, SAL.RUL.07)",
                format!(
                    "individual car owners only: {} point per AED {} on workshop invoices and counter sales (none on trade, fleet or insurer-paid jobs); {} points are an AED {} discount line on a later workshop invoice, confirmed by a one-time code: {}",
                    defaults::LOYALTY_POINTS,
                    defaults::LOYALTY_EARN_PER_AED,
                    defaults::LOYALTY_MIN_REDEEM,
                    defaults::LOYALTY_MIN_REDEEM * defaults::LOYALTY_POINT_VALUE_FILS / 100,
                    if sales.confirm_redemption_by_code { "on" } else { "off" }
                ),
                SEED,
                "§5; §7.1 settings",
            ),
            "A-15 (VAT of the discounted supply)",
        ),
        row(
            "14",
            "Loyalty",
            "Expiry, adjustments and segments (SAL.RUL.08, SAL.ACL.07)",
            format!(
                "points expire {} months after they are earned, reminder {} days before; a manual adjustment above {} points needs a manager; segments: not seeded (see the deferred list)",
                defaults::LOYALTY_VALIDITY_MONTHS,
                sales.expiry_reminder_days,
                sales.adjustment_limit
            ),
            SEED,
            "§5; §7.1 settings",
        ),
        row(
            "15",
            "Staff",
            "Onboarding checklists (HRM.ONB.01, HRM.RUL.16, HRM.RUL.17, HRM.ACL.10)",
            format!(
                "{}; started when the staff record is created in onboarding (none for a record created active); tasks to the branch manager (the workshop manager for workshop roles), the line manager and the employee; completion blocks nothing; HRM.RUL.17 reminds on the first run after the due date, then every 2 days, and tells the branch manager and the line manager after 5 days",
                onboarding.join("; ")
            ),
            "checklists: 08 seed; reminder days: default of the module",
            "§5; §7.1 settings",
        ),
        row(
            "16",
            "Staff",
            "SOP library (HRM.ONB.01, DOC.RUL.04, HRM.ACL.10)",
            format!(
                "{}; DOC.RUL.04 asks each reader to acknowledge within 7 days, reminds every 3 days and tells the reader's manager and the SOP owner role after 7 days overdue; a new version asks again",
                sops.join("; ")
            ),
            "SOPs and readers: 08 seed; days: default of the module",
            "§5; §7.1 settings",
        ),
        row(
            "17",
            "Registrations",
            "Registration and certification kinds (DOC.EXP.01, DOC.RUL.01)",
            format!(
                "Trademark registration ({} days) and Other certification ({} days), subject legal entity, responsible role {OWNER_ROLE}, no blocks: {} | {}",
                "180/90/30",
                "90/30/7",
                kind_text("trademark_registration"),
                kind_text("other_certification")
            ),
            MODULE,
            "§5; §7.1 settings",
        ),
        row(
            "18",
            "Recall",
            "Recall trace start and openers (BL-AUT-16)",
            "the trace starts at the first receipt of the part in the received date range the client gives per recall (parts are not tracked by lot); recalls.manage: the Owner / GM and the branch managers"
                .to_owned(),
            SEED,
            "§7.1 settings Recall",
        ),
        open(
            row(
                "19",
                "Parts supply",
                "Legal parts supply period (AUT.RUL.16)",
                format!(
                    "the shortest of the sources requested: stock and other branch {}, local supplier {}, import {} days until the onboarding sets them",
                    auto::supply::default_days(auto::supply::STOCK),
                    auto::supply::default_days(auto::supply::LOCAL_SUPPLIER),
                    auto::supply::default_days(auto::supply::IMPORT)
                ),
                "code defaults; changed per client with supply::set_period",
                "§7.2 AUT.RUL.16",
            ),
            "L-35 (the lawyer sets the periods)",
        ),
        open(
            row(
                "20",
                "Uncollected vehicles",
                "Reminders and the manager task (AUT.RUL.18)",
                format!(
                    "the customer is texted {} days after the vehicle is ready, declined or cancelled; the branch manager gets a task at {} days (legal next steps open)",
                    days(&auto::storage::REMINDER_DAYS),
                    auto::storage::MANAGER_DAYS
                ),
                "code constants of the app",
                "§7.2 AUT.RUL.18",
            ),
            "L-36 (legal next steps for an abandoned vehicle)",
        ),
    ]
}

/// The rows that come from the database, read when the page is shown: the
/// storage fee switch (`AUT.RUL.18`, off until the client turns it on).
///
/// # Errors
///
/// The database's errors.
pub async fn stored_rows(db: &Db) -> Result<Vec<Row>> {
    let enabled = auto::settings::storage_fee_enabled(db).await?;
    Ok(vec![row(
        "20a",
        "Uncollected vehicles",
        "Storage fee (AUT.RUL.18, switch storage_fee_enabled)",
        format!(
            "{}; when on, the daily rate agreed at check-in is charged from the first reminder to the day before release, never after a total loss; set with the command auto:storage-fee on|off",
            if enabled { "on" } else { "off" }
        ),
        "stored in auto_settings",
        "§7.2 AUT.RUL.18",
    )])
}

/// Sets the storage fee switch (`auto:storage-fee on|off`).
///
/// # Errors
///
/// The database's errors.
pub async fn set_storage_fee(db: &Db, on: bool) -> Result {
    auto::settings::set(
        db,
        auto::settings::STORAGE_FEE_ENABLED,
        if on { "on" } else { "off" },
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aut_15_the_sales_module_has_back_orders_on_for_three_channels() {
        let module = sales(WlSales::new());
        let config = module.config();
        for channel in ["counter", "phone", "sales_rep"] {
            assert!(config.back_orders_allowed(channel), "{channel}");
        }
        assert!(!config.back_orders_allowed("portal"));
        assert!(!WlSales::new().config().back_orders_allowed("counter"));
    }

    #[test]
    fn aut_18_the_notify_and_sales_modules_have_the_08_values() {
        // wl-notify keeps its numbers private: what the module does with the
        // cap of 2 a week, the limit of 200 recipients, the window and the
        // opt-in is run in `host/tests/auto_marketing_real.rs`. Here only the
        // sales values, which the module shows.
        let module = sales(WlSales::new());
        let config = module.config();
        assert!(config.over_budget_approval);
        assert!(config.campaign_required_sources.is_empty());
        assert_eq!(config.expiry_reminder_days, 30);
        assert_eq!(config.adjustment_limit, 500);
        assert!(config.confirm_redemption_by_code);
    }

    #[test]
    fn aut_18_every_onboarding_step_is_valid_for_wl_hr() {
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

    #[test]
    fn aut_18_the_page_lists_every_line_once() {
        let rows = rows("owner");
        let numbers: Vec<String> = rows.iter().map(|r| r.number.clone()).collect();
        let expected: Vec<String> = (1..=20).map(|n| n.to_string()).collect();
        assert_eq!(numbers, expected);
        assert!(
            rows.iter()
                .all(|r| !r.value.is_empty() && !r.source.is_empty() && !r.reference.is_empty())
        );
    }

    #[test]
    fn aut_18_only_the_loyalty_vat_and_the_two_legal_periods_wait_on_an_open_item() {
        let open: Vec<String> = rows("owner")
            .into_iter()
            .filter(|r| !r.open.is_empty())
            .map(|r| r.number)
            .collect();
        assert_eq!(open, ["13", "19", "20"]);
    }
}
