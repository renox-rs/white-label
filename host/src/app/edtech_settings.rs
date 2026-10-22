//! The 10 EdTech client's values for the shared document, scheduling, party,
//! notification, alert, sales, staff and integration mechanisms (10 §4.1,
//! §7.1 *Booking*, §7.2, §8, §9; BL-EDU-17), and the rows of the page
//! `/settings/compliance` that shows them.
//!
//! The mechanisms live in the shared modules; the numbers are
//! `edtech::defaults` (and `edtech::seed` for what the earlier issues seeded)
//! and this file turns them into the owning modules' own calls:
//!
//! - the owner alert of the compliance-critical rules ([`register_critical`]):
//!   `DOC.RUL.03` joins `EDU.RUL.12` and `EDU.RUL.13`, which the app marks;
//!   a run that fails alerts the company owner and the company admin
//!   (`PLT.RUL.04`);
//! - the connections with their manual fallbacks ([`register_integrations`]):
//!   the payment gateway (the desk card terminal or cash), WhatsApp (e-mail),
//!   e-mail, and the attendance entry (paper, entered later);
//! - the notification module ([`notify_for`]): at most 2 marketing messages per
//!   recipient in 7 days, a broadcast to more than 200 needs a second approver;
//! - the sales module ([`sales`]): a cost entry over a campaign's budget needs a
//!   manager, no source requires a campaign, points expire with a reminder 30
//!   days before, a manual adjustment above 500 points needs a manager, a
//!   redemption is confirmed by a one-time code;
//! - the seed ([`install_for`], run by `edtech:install`): the lead sources
//!   (the aggregator turned off), the marketing send window 07:00-21:00, the
//!   loyalty programme, the onboarding checklists, the SOP library and the
//!   reactions of the notification types of the shared modules. It adds what
//!   the database lacks and keeps what the client changed.
//!
//! Decisions (the rest is in `$S/f10/DEFERRED.md`):
//!
//! - **Segments.** The four segments of the spec need rules over the
//!   enrolment data (a package with at most 2 credits, a course completed
//!   without re-enrolment, a trial attended and not enrolled, guardians of two
//!   or more students); wl-sales has "at least" rules and the vertical rule
//!   kind, which 10 does not register. None is seeded.
//! - **Loyalty VAT.** The VAT of the discounted supply is `⚠ open: accountant
//!   (A-15)`; the rest of the loyalty settings are built.
//! - **Not a legal block.** The centre permit and the course approval are not
//!   marked `legal_block` (L-19 is open); their override stays `never` and the
//!   client cannot switch it.

use std::future::Future;
use std::pin::Pin;

use edtech::defaults::{self, OnboardingSpec, SOPS};
use edtech::permissions::{CENTRE_MANAGER_ROLE, COORDINATOR_ROLE, OWNER_ROLE};
use renox::auth::permissions::Scope;
use renox::db::Json;
use renox::prelude::*;
use wl_documents::{Sop, SopReader};
use wl_hr::{OnboardingTemplate, StepInput, TemplateInput};
use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::block_effects::effect_for;
use wl_kernel::compliance::{CriticalRule, is_critical};
use wl_kernel::integrations::{self, Delivery, IntegrationDef};
use wl_notify::window::Window;
use wl_notify::{NotificationType, SendWindow, WlNotify};
use wl_sales::{LeadSource, LoyaltyProgramme, NewProgramme, WlSales};

use super::compliance_settings::{Row, kind_text, type_text};
use super::edtech_seed::EDTECH;
use super::roles::ACCOUNTANT;

/// The row's value is this client's seed.
const SEED: &str = "EdTech seed (BL-EDU-17)";
/// The row's value is the module's own default.
const MODULE: &str = "default of the module";

/// Marks `DOC.RUL.03` compliance-critical for the EdTech client, owned by the
/// company owner (`PLT.RUL.04`). `EDU.RUL.12` and `EDU.RUL.13` are marked by
/// the app itself.
pub fn register_critical(client_app: &str) {
    if client_app != EDTECH {
        return;
    }
    CriticalRule::new("DOC.RUL.03")
        .owner_role(defaults::ALERT_ROLE)
        .register();
}

/// Gives the company owner of an EdTech deployment what deciding an override
/// needs besides the role: reading the approvals and the documents; and the
/// centre manager what approving a cash shortage needs. The
/// approval policy itself is the app's ([`edtech::defaults::override_policy`]).
/// The roles table is shared by every vertical, so this is a grant of this
/// client only.
pub fn grant_roles(client_app: &str) {
    if client_app != EDTECH {
        return;
    }
    super::roles::grant_extra(
        super::roles::OWNER,
        &[
            wl_kernel::permissions::APPROVALS_VIEW,
            wl_documents::permissions::VIEW,
        ],
    );
    // The centre manager approves a desk cash shortage and the write-offs
    // (`FIN.ACL.06`, `FIN.ACL.15`): `cash.write_off` is what the approval of
    // a shortage asks for, and the approvals page shows what waits.
    super::roles::grant_extra(
        super::roles::CENTRE_MANAGER,
        &[
            wl_kernel::permissions::APPROVALS_VIEW,
            wl_payments::permissions::CASH_VIEW,
            wl_payments::permissions::CASH_WRITE_OFF,
        ],
    );
}

/// The notification module with the EdTech client's values (`NTF.RUL.02`,
/// `NTF.ACL.03`): at most 2 marketing messages per recipient in 7 days, and a
/// broadcast to more than 200 recipients needs a second approver. The send
/// window 07:00-21:00 is stored by [`install_for`].
pub fn notify_for() -> WlNotify {
    WlNotify::new()
        .marketing_frequency_cap(defaults::MARKETING_CAP)
        .broadcast_approval_above(defaults::BROADCAST_APPROVAL_ABOVE)
}

/// The sales module with the EdTech client's values (`SAL.CRM.02`,
/// `SAL.ACL.07`, `SAL.LOY.01`, `SAL.RUL.08`): no lead source requires a
/// campaign; a cost entry over a campaign's budget needs a manager; points
/// expire with a reminder 30 days before; a manual adjustment above 500 points
/// needs a manager; a redemption is confirmed by a one-time code.
pub fn sales(module: WlSales) -> WlSales {
    module
        .over_budget_approval(true)
        .campaign_required_for(defaults::CAMPAIGN_REQUIRED_SOURCES)
        .expiry_reminder_days(defaults::LOYALTY_REMINDER_DAYS)
        .adjustment_limit(defaults::LOYALTY_ADJUSTMENT_LIMIT)
        .confirm_redemption_by_code(true)
        // A package whose invoice was reopened stays active and the counsellor
        // is told to collect; consumption is not suspended (BL-EDU-18).
        .package_seller_role(edtech::money_defaults::PACKAGE_COLLECT_ROLE)
        .suspend_package_consumption_while_unpaid(
            edtech::money_defaults::SUSPEND_PACKAGE_WHILE_UNPAID,
        )
}

/// The send handler of a connection nothing is queued for: the manual way is
/// the fallback.
fn sent<'a>(
    _state: &'a AppState,
    _call: &'a integrations::Outbound,
) -> Pin<Box<dyn Future<Output = Result<Delivery>> + Send + 'a>> {
    Box::pin(async { Ok(Delivery::Sent) })
}

/// Registers the connections 10 keeps a manual fallback for (10 §8, wl-kernel
/// §5.14): the payment gateway (the desk card terminal or cash with a receipt;
/// payment links are sent again on recovery), WhatsApp (e-mail instead),
/// e-mail (a printed copy) and the attendance entry (paper, entered later by
/// the tutor or the coordinator). Each shows its banner on the screens it
/// affects. wl-notify reports the WhatsApp and e-mail sends; the gateway and
/// the attendance entry are taken out of service by an administrator. The ASP
/// connection of each legal entity is wl-invoicing's (`FIN.RUL.16`).
pub fn register_integrations(client_app: &str) {
    if client_app != EDTECH {
        return;
    }
    IntegrationDef::of(integrations::PAYMENT_GATEWAY, "payments", sent)
        .owner_role(ACCOUNTANT)
        .screens([defaults::DESK_SCREEN, defaults::COLLECT_SCREEN])
        .fallback(
            defaults::GATEWAY_FALLBACK_EN,
            Some(defaults::GATEWAY_FALLBACK_AR),
        )
        .register();
    IntegrationDef::of(integrations::WHATSAPP, "notify", sent)
        .owner_role(CENTRE_MANAGER_ROLE)
        .screens([defaults::DESK_SCREEN])
        .fallback(
            defaults::WHATSAPP_FALLBACK_EN,
            Some(defaults::WHATSAPP_FALLBACK_AR),
        )
        .register();
    IntegrationDef::of(integrations::EMAIL, "notify", sent)
        .owner_role(COMPANY_ADMIN_ROLE)
        .screens([defaults::DESK_SCREEN])
        .fallback(
            defaults::EMAIL_FALLBACK_EN,
            Some(defaults::EMAIL_FALLBACK_AR),
        )
        .register();
    IntegrationDef::of(defaults::ATTENDANCE_CONNECTION, "attendance", sent)
        .owner_role(COORDINATOR_ROLE)
        .screens([defaults::SESSION_SCREEN])
        .fallback(
            defaults::ATTENDANCE_FALLBACK_EN,
            Some(defaults::ATTENDANCE_FALLBACK_AR),
        )
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
    /// Notification types given their 10 reactions.
    pub notification_types: usize,
    /// Budget categories added or given the accountant as owner role.
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

/// Stores what the EdTech client's shared modules lack, for the modules `on`
/// says are enabled, and keeps what the client changed.
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
            super::edtech_money::install_budget_categories(&state.db).await?;
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
/// `SAL.RUL.07`): 1 point per AED 10 paid, 100 points an AED 10 discount (10
/// fils a point), at most 10% of the fee, valid 12 months. A company has no
/// loyalty account, so corporate-paid fees earn nothing. The VAT of the
/// discounted supply is `⚠ open: accountant (A-15)`.
async fn install_loyalty(db: &Db) -> Result<usize> {
    if LoyaltyProgramme::first_active(db).await?.is_some() {
        return Ok(0);
    }
    let mut new = NewProgramme::new(defaults::LOYALTY_NAME)
        .points_per_aed_units(defaults::LOYALTY_POINTS, defaults::LOYALTY_EARN_PER_AED)
        .redeeming(
            defaults::LOYALTY_POINT_VALUE_FILS,
            defaults::LOYALTY_MIN_REDEEM,
        )
        .validity_months(defaults::LOYALTY_VALIDITY_MONTHS)
        .terms(
            "1",
            "You earn 1 point for every AED 10 of fees you pay as a student or guardian. 100 points give an AED 10 discount on a later enrolment or package, up to 10% of its fee, confirmed with a one-time code. Points expire 12 months after they are earned. Fees paid by a company earn nothing.",
            "تكسب نقطة واحدة عن كل 10 دراهم من الرسوم التي تدفعها كطالب أو ولي أمر. تعادل 100 نقطة خصمًا قدره 10 دراهم على تسجيل أو باقة لاحقة، بحد أقصى 10% من رسومها، بعد تأكيده برمز لمرة واحدة. تنتهي صلاحية النقاط بعد 12 شهرًا من كسبها. الرسوم التي تدفعها شركة لا تكسب نقاطًا.",
        );
    new.max_share_bps = defaults::LOYALTY_MAX_SHARE_BPS;
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

// ---- The page /settings/compliance for the EdTech client -------------------

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

/// The block effect of `kind` in words.
fn effect_text(kind: &str) -> String {
    let rule = effect_for(kind, &Scope::global());
    format!(
        "{}; decided by {}",
        rule.effect,
        rule.decider_role.as_deref().unwrap_or("nobody")
    )
}

/// What the client's configuration says, read back from the modules' own
/// registries and builders. One row per line of the issue's settings table.
pub fn rows(owner_role: &str) -> Vec<Row> {
    let sales = sales(WlSales::new());
    let sales = sales.config();
    let scheduling = super::edtech_seed::scheduling_for(EDTECH);
    let scheduling = scheduling.config();
    let hr = super::edtech_seed::hr(wl_hr::WlHr::new(), owner_role);
    let hr = hr.config();
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
        edtech::seed::CENTRE_PERMIT_KIND,
        edtech::seed::COURSE_APPROVAL_KIND,
        edtech::seed::POLICE_CLEARANCE_KIND,
        edtech::seed::TUTOR_PERMIT_KIND,
    ];
    let kind_rows: Vec<String> = kinds
        .iter()
        .map(|key| format!("{key}: {}", kind_text(key)))
        .collect();
    let responsible = [
        (edtech::seed::CENTRE_PERMIT_KIND, CENTRE_MANAGER_ROLE),
        (edtech::seed::COURSE_APPROVAL_KIND, COORDINATOR_ROLE),
        (edtech::seed::POLICE_CLEARANCE_KIND, COORDINATOR_ROLE),
        (edtech::seed::TUTOR_PERMIT_KIND, COORDINATOR_ROLE),
    ]
    .iter()
    .map(|(key, role)| format!("{key}: {role}"))
    .collect::<Vec<_>>()
    .join("; ");
    let types: Vec<String> = [
        edtech::notices::FAMILY_NOTICE,
        edtech::notices::TERMS_REQUEST,
        edtech::notices::NOTICE,
        "invoice.reminder_before_due",
        "invoice.reminder_after_due",
        "invoicing.subscription_past_due",
        "invoicing.subscription_retry",
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
    let connections = format!(
        "{}: {} | {}: {} | {}: {} | {}: {}",
        integrations::PAYMENT_GATEWAY,
        defaults::GATEWAY_FALLBACK_EN,
        integrations::WHATSAPP,
        defaults::WHATSAPP_FALLBACK_EN,
        defaults::ATTENDANCE_CONNECTION,
        defaults::ATTENDANCE_FALLBACK_EN,
        "asp (FIN.RUL.16)",
        "corporate (B2B) invoices are still issued and sent as PDF; the module's default scope",
    );
    vec![
        row(
            "1",
            "Documents",
            "Blocks and overrides (centre permit, course approval, police clearance, tutor permit or freelance visa)",
            format!(
                "{}. Only the police clearance may be switched to with_approval (the owner approves, one use, 72 hours); the others are fixed to never",
                kind_rows.join(" | ")
            ),
            SEED,
            "§9; §7.2",
        ),
        row(
            "2",
            "Documents",
            "Look-ahead windows (DOC.RUL.02, EDU.RUL.13)",
            format!(
                "tutor kinds {} days; centre permit and course approval {} days",
                edtech::seed::TUTOR_LOOK_AHEAD_DAYS,
                edtech::seed::CENTRE_LOOK_AHEAD_DAYS
            ),
            SEED,
            "§4.2, §7.2",
        ),
        row(
            "3",
            "Scheduling",
            "One-to-one booking (SCH.RUL.05, SCH.RUL.06)",
            format!(
                "required of the tutor: {} (the private tutor permit or freelance visa too, for a freelance tutor); eligibility look-ahead D = {} days; an open conflict goes to the {} {} hours before the start; reminders {} hours before; booking deposit none by default; check-in not used",
                edtech::seed::ONE_TO_ONE_QUALIFICATIONS,
                scheduling.look_ahead_days,
                scheduling.manager_role.as_deref().unwrap_or("nobody"),
                scheduling.escalation_hours,
                days(&scheduling.reminder_hours),
            ),
            SEED,
            "§7.1 Booking",
        ),
        row(
            "4",
            "Scheduling",
            "Tutor alerts and the lapse of a seat hold (EDU.RUL.12, EDU.RUL.11)",
            format!(
                "EDU.RUL.12: D = {}; H = {}. EDU.RUL.11: N = {} hours",
                edtech::tutor_alerts::WINDOW_DAYS,
                edtech::tutor_alerts::ESCALATION_HOURS,
                edtech::enrolments::HOLD_HOURS
            ),
            SEED,
            "§7.2",
        ),
        row(
            "5",
            "Parties",
            "Block effect (PTY.ACL.03)",
            format!(
                "enrolment: {} (through freeze without the freeze-day limit, the seat kept); membership: {}",
                effect_text(edtech::blocked::KIND),
                effect_text("subscription")
            ),
            SEED,
            "§4.1; wl-parties §5.6",
        ),
        row(
            "6",
            "Notifications",
            "Give-up reaction of guardian messages (NTF.RUL.01)",
            "alternative_recipient (the other guardian), then staff_task to the counsellor; an adult student without a guardian goes straight to the task; an invoice e-mail marks the contact invalid, tries the other guardian or payer contact, then the accountant gets the task".to_owned(),
            SEED,
            "§4.1, §7.2",
        ),
        row(
            "7",
            "Notifications",
            "Notification types and their groups",
            format!("{} | no type is statutory; staff-only types name no group and take none", types.join(" | ")),
            SEED,
            "§7.2 Notification types",
        ),
        row(
            "8",
            "Documents",
            "Responsible role per document kind",
            format!("{responsible}; for a tutor's document, in the campuses of the affected batches and bookings"),
            SEED,
            "§9; wl-documents §5.2",
        ),
        row(
            "9",
            "Alerts",
            "Compliance-critical rules and owner alerts (PLT.RUL.04)",
            format!(
                "{}; a failed run alerts the company owner ({owner_role}) and the company admin",
                critical.join("; ")
            ),
            SEED,
            "§7.2",
        ),
        row(
            "10",
            "Integrations",
            "Manual fallbacks (PLT.RUL.05)",
            connections,
            SEED,
            "§8",
        ),
        row(
            "11",
            "Sales",
            "Lead sources used (SAL.CRM.02)",
            format!(
                "{}; recorded on the lead and on the student or guardian party; a source requires a campaign: {}",
                defaults::LEAD_SOURCES.join(", "),
                if sales.campaign_required_sources.is_empty() {
                    "none".to_owned()
                } else {
                    sales.campaign_required_sources.join(", ")
                }
            ),
            SEED,
            "§5; §7.2 settings",
        ),
        row(
            "12",
            "Sales",
            "Campaign channels (SAL.CRM.02, SAL.ACL.07)",
            format!(
                "{}; converted value: the first enrolment invoice; a cost entry over the budget needs a manager: {}",
                channels.join(", "),
                if sales.over_budget_approval { "on" } else { "off" }
            ),
            "channels: the module's fixed list; approval: EdTech seed",
            "§5; §7.2 settings",
        ),
        open(
            row(
                "13",
                "Loyalty",
                "Earn and redeem (SAL.LOY.01, SAL.RUL.07)",
                format!(
                    "{} point per AED {} of fees paid by the student or guardian (none on corporate-paid fees); {} points are an AED {} discount line on a later enrolment or package, at most {}% of its fee, confirmed by a one-time code: {}",
                    defaults::LOYALTY_POINTS,
                    defaults::LOYALTY_EARN_PER_AED,
                    defaults::LOYALTY_MIN_REDEEM,
                    defaults::LOYALTY_MIN_REDEEM * defaults::LOYALTY_POINT_VALUE_FILS / 100,
                    defaults::LOYALTY_MAX_SHARE_BPS / 100,
                    if sales.confirm_redemption_by_code { "on" } else { "off" }
                ),
                SEED,
                "§5; §7.2 settings",
            ),
            "A-15 (VAT of the discounted supply)",
        ),
        row(
            "14",
            "Loyalty",
            "Expiry, adjustments, marketing caps and segments (SAL.RUL.08, SAL.ACL.07, NTF.RUL.02, NTF.ACL.03)",
            format!(
                "points expire {} months after they are earned, reminder {} days before; a manual adjustment above {} points needs a manager; marketing only with consent (the guardian's for a minor), sent 07:00-21:00, at most {} marketing messages per recipient in 7 days; a broadcast to more than {} recipients needs a second approver; segments: not seeded (see the deferred list)",
                defaults::LOYALTY_VALIDITY_MONTHS,
                sales.expiry_reminder_days,
                sales.adjustment_limit,
                defaults::MARKETING_CAP,
                defaults::BROADCAST_APPROVAL_ABOVE
            ),
            SEED,
            "§5; §7.2 settings",
        ),
        row(
            "15",
            "Staff",
            "Onboarding checklists (HRM.ONB.01, HRM.RUL.16, HRM.RUL.17, HRM.ACL.10)",
            format!(
                "{}; started when the employee record is created in onboarding; the line manager is the {}; tasks to the centre manager, the academic coordinator and the employee; completion blocks nothing; HRM.RUL.17 reminds on the first run after the due date, then every 2 days, and tells the centre manager and the line manager after 5 days",
                onboarding.join("; "),
                hr.manager_role
            ),
            "checklists: EdTech seed; reminder days: default of the module",
            "§5; §7.2 settings",
        ),
        row(
            "16",
            "Staff",
            "SOP library (HRM.ONB.01, DOC.RUL.04, HRM.ACL.10)",
            format!(
                "{}; DOC.RUL.04 asks each reader to acknowledge within 7 days, reminds every 3 days and tells the reader's manager and the SOP owner role after 7 days overdue; a new version asks again",
                sops.join("; ")
            ),
            "SOPs and readers: EdTech seed; days: default of the module",
            "§5; §7.2 settings",
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
            "§5; §7.2 settings",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notify_and_sales_modules_have_the_10_values() {
        // wl-notify keeps its numbers private; the builder takes the same two.
        let _notify = notify_for();
        assert_eq!(defaults::MARKETING_CAP, 2);
        assert_eq!(defaults::BROADCAST_APPROVAL_ABOVE, 200);
        let module = sales(WlSales::new());
        let config = module.config();
        assert!(config.over_budget_approval);
        assert!(config.campaign_required_sources.is_empty());
        assert_eq!(config.expiry_reminder_days, 30);
        assert_eq!(config.adjustment_limit, 500);
        assert!(config.confirm_redemption_by_code);
        assert!(!config.suspend_package_consumption_while_unpaid);
        assert_eq!(config.package_seller_role, "counsellor");
    }

    #[test]
    fn every_onboarding_step_is_valid_for_wl_hr() {
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
    fn the_page_lists_every_line_of_the_issue_table_once() {
        let rows = rows("owner");
        let numbers: Vec<String> = rows.iter().map(|r| r.number.clone()).collect();
        let expected: Vec<String> = (1..=17).map(|n| n.to_string()).collect();
        assert_eq!(numbers, expected);
        assert!(
            rows.iter()
                .all(|r| !r.value.is_empty() && !r.source.is_empty() && !r.reference.is_empty())
        );
    }

    #[test]
    fn only_the_loyalty_row_waits_on_the_accountant() {
        let open: Vec<String> = rows("owner")
            .into_iter()
            .filter(|r| !r.open.is_empty())
            .map(|r| r.number)
            .collect();
        assert_eq!(open, ["13"]);
    }
}
