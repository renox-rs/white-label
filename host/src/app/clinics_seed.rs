//! The 05 Clinics client's values for the shared scheduling, HR and document
//! mechanisms (05 §4.5, §7.1, §7.2 *Settings for the shared negative flows*;
//! BL-CLN-12, BL-CLN-01).
//!
//! The mechanisms live in the shared modules (`wl-scheduling`, `wl-hr`,
//! `wl-documents`) and the vertical reaches them only through `wl-kernel`.
//! The tables are `clinics::seed` and `clinics::reasons`; this file turns
//! them into:
//!
//! - the document kinds registered with wl-documents ([`register_documents`]):
//!   the practitioner licence and the branch privileges, which block
//!   `booking.book` and `booking.check_in` with override `never`
//!   (`SCH.ACL.04`), remind at 90, 60, 30 and 7 days and look 30 days ahead;
//!   the clinic manager of the branch renews them;
//! - the scheduling settings ([`scheduling_for`]): the reason codes 05 adds
//!   (`coverage_not_confirmed`, initiated by the business) and the clinic
//!   manager as the manager told of an open conflict (`SCH.RUL.06`);
//! - the HR settings ([`hr`]): the clinic manager is the line manager of a
//!   practitioner, told when a qualification expires (`HRM.RUL.10`);
//! - the seed ([`ClinicsSeed`], command `clinics:install`): the service types
//!   every clinic starts with, each needing the licence and the privileges
//!   (`SCH.RUL.05`), the onboarding checklists, the SOP library, the loyalty
//!   programme and the lapsed-patients segment. It adds what the database
//!   lacks and keeps what the client changed.
//!
//! BL-CLN-15 adds the rest of the values of the shared mechanisms: the
//! facility licence, the patient Emirates ID and the registration kinds
//! ([`register_documents`]), the scheduling numbers ([`scheduling_for`]), the
//! notification caps ([`notify_for`]), the sales and loyalty settings and the
//! advertising permit guard ([`sales`], [`AdvertisingPermitGuard`]) and the
//! connections with their manual fallbacks ([`register_integrations`]).
//!
//! Decisions: the two kinds are not marked compliance-critical in
//! wl-documents (and so not `legal_block`): the owner is told by `CLN.RUL.01`
//! in one message with the number of appointments, not a second time by
//! `DOC.RUL.03` (§7.2), and `DOC.RUL.03` is a compliance-critical rule
//! owned by the owner role all the same. Whether the law makes them legal
//! blocks is `⚠ open: lawyer (L-19)`; the override mode is `never` as the
//! spec says. The kinds block `booking.start` as well, as 05 §4.5 lists it:
//! wl-scheduling has no document provider for `start` (it asks the
//! qualification again at `start` by itself, which is what refuses a start
//! after the expiry), so the entry is the kind's statement of intent and
//! the rule's future-work lists do not read it.

use std::sync::Arc;

use std::future::Future;
use std::pin::Pin;

use clinics::card_requests::CARD_KIND;
use clinics::permissions::{
    BILLING_COORDINATOR_ROLE, CLINIC_MANAGER_ROLE, OWNER_ROLE, RECEPTIONIST_ROLE,
};
use clinics::permits::{self, Problem};
use clinics::policy::POLICY_SUBJECT_TYPE;
use clinics::seed::{
    self, LICENCE_KIND, LOOK_AHEAD_DAYS, ONBOARDING, OnboardingSpec, PRIVILEGES_KIND,
    REMINDER_DAYS, REQUIRED_QUALIFICATIONS, SOPS,
};
use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use wl_documents::registrations::{
    LEGAL_ENTITY_SUBJECT, OTHER, TRADEMARK, VerticalSettings, register_vertical,
};
use wl_documents::{KindSeed, Sensitivity, Sop, SopReader, register_kind};
use wl_hr::{OnboardingTemplate, StepInput, TemplateInput};
use wl_kernel::approvals::{ApprovalPolicy, Step};
use wl_kernel::documents::{BlockMode, today};
use wl_kernel::integrations::{self, Delivery, IntegrationDef};
use wl_kernel::notify::{GiveUpReaction, notification_type, register_type};
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;
use wl_kernel::{COMPANY_ADMIN_ROLE, Manifest};
use wl_notify::SendWindow;
use wl_notify::WlNotify;
use wl_notify::window::Window;
use wl_sales::{
    Campaign, CampaignGuard, LoyaltyProgramme, MatchMode, NewProgramme, Rule, Segment, WlSales,
};
use wl_scheduling::eligibility::{GUARDED_BOOK, GUARDED_CHECK_IN};
use wl_scheduling::{ServiceType, WlScheduling};

/// The transition the kinds also guard (05 §4.5).
pub const GUARDED_START: &str = "booking.start";

/// The client app these values are for.
pub const CLINICS: &str = "clinics";

/// The kind `key` of a practitioner: an expiring document of the employee that
/// blocks `booking.book`, `booking.check_in` and `booking.start`, with the
/// override mode fixed to `never` (the client cannot loosen it, `SCH.ACL.04`,
/// 05 §4.5).
fn practitioner_kind(key: &str, name_en: &str, name_ar: &str) -> KindSeed {
    KindSeed::new(key, name_en)
        .name_ar(name_ar)
        .subject_type(EMPLOYEE_SUBJECT_TYPE)
        .expires()
        .numbered()
        .reminders(REMINDER_DAYS)
        .look_ahead(LOOK_AHEAD_DAYS)
        .responsible_role(CLINIC_MANAGER_ROLE)
        .override_fixed()
        .blocks(GUARDED_BOOK, BlockMode::Block)
        .blocks(GUARDED_CHECK_IN, BlockMode::Block)
        .blocks(GUARDED_START, BlockMode::Block)
}

/// The practitioner licence as wl-documents' seed.
pub fn licence_kind() -> KindSeed {
    practitioner_kind(LICENCE_KIND, "Practitioner licence", "رخصة مزاولة المهنة")
}

/// The branch privileges as wl-documents' seed.
pub fn privileges_kind() -> KindSeed {
    practitioner_kind(PRIVILEGES_KIND, "Branch privileges", "صلاحيات الفرع")
}

/// The insurance card as wl-documents' seed: an expiring document of the
/// insurance policy that the patient may upload in the portal
/// (`CLN.RUL.10`); the receptionist is the responsible role (05 §7.2
/// *Settings for the shared negative flows*) and the class is personal data,
/// so the image needs the permission of that class (`CLN.ACL.12`). When
/// staff accept it, the app takes the expiry date as the card's last valid
/// day (`clinics::card_requests::on_accepted`).
pub fn insurance_card_kind() -> KindSeed {
    KindSeed::new(CARD_KIND, "Insurance card")
        .name_ar("بطاقة التأمين")
        .subject_type(POLICY_SUBJECT_TYPE)
        .expires()
        .sensitivity(Sensitivity::Personal)
        .portal_upload()
        .responsible_role(RECEPTIONIST_ROLE)
        .notify_party()
}

/// The facility licence as wl-documents' seed: an expiring document of the
/// legal entity, renewed by the owner / medical director, reminded from 180
/// days ahead and compliance-critical (`DOC.RUL.03`: the owner is told when
/// it expires). It blocks nothing.
pub fn facility_licence_kind() -> KindSeed {
    KindSeed::new(seed::FACILITY_LICENCE_KIND, "Facility licence")
        .name_ar("رخصة المنشأة الصحية")
        .subject_type(LEGAL_ENTITY_SUBJECT)
        .expires()
        .numbered()
        .reminders(seed::FACILITY_REMINDER_DAYS)
        .responsible_role(OWNER_ROLE)
        .compliance_critical()
}

/// The patient's Emirates ID as wl-documents' seed: an expiring document of
/// the patient, kept by the receptionist; the patient (or the guardian) is
/// reminded as the party. The image is personal data, so it needs
/// `patients.sensitive.view` (`CLN.ACL.12`).
pub fn emirates_id_kind() -> KindSeed {
    KindSeed::new(seed::EMIRATES_ID_KIND, "Patient Emirates ID")
        .name_ar("الهوية الإماراتية للمريض")
        .subject_type(PARTY_SUBJECT_TYPE)
        .expires()
        .numbered()
        .reminders(seed::EMIRATES_ID_REMINDER_DAYS)
        .sensitivity(Sensitivity::Personal)
        .portal_upload()
        .responsible_role(RECEPTIONIST_ROLE)
        .notify_party()
}

/// Registers the 05 kinds with wl-documents (only for the Clinics client),
/// and turns on the two registration kinds 05 uses (*Trademark registration*
/// and *Other certification*, `DOC.EXP.01`, `DOC.RUL.01`), renewed by the
/// owner / medical director, with no `blocks`. The reminders are the
/// platform's: 180, 90 and 30 days for a trademark and 90, 30 and 7 for the
/// other. Decision: the platform table also gives *Other certification* the
/// supplier and item subjects the spec does not mention; the extra subjects
/// block nothing.
pub fn register_documents(client_app: &str) {
    if client_app != CLINICS {
        return;
    }
    register_kind(licence_kind());
    register_kind(privileges_kind());
    register_kind(insurance_card_kind());
    register_kind(facility_licence_kind());
    register_kind(emirates_id_kind());
    register_vertical(
        CLINICS,
        VerticalSettings::new()
            .enabled(&[TRADEMARK, OTHER])
            .role(TRADEMARK, OWNER_ROLE)
            .role(OTHER, OWNER_ROLE),
    );
}

/// The scheduling module with the Clinics client's values: the 05 reason
/// codes and the clinic manager as the manager of the branch. Any other
/// client keeps the module's defaults.
pub fn scheduling_for(client_app: &str) -> WlScheduling {
    let mut module = WlScheduling::new();
    if client_app == CLINICS {
        // The numbers of 05 §7.2 (they are also the module's defaults; set
        // here so that a change of a default never moves the clinic): D = 30
        // days (`SCH.RUL.05`), H = 2 hours (`SCH.RUL.06`), reminders 24 and 2
        // hours before (`SCH.RUL.01`), no-show after 15 minutes
        // (`SCH.RUL.02`). The clinic manager is the manager told of a
        // conflict; the kernel has no `ApproverResolver`, so the module
        // looks for the holders of that role in the booking's branch.
        module = module
            .look_ahead_days(LOOK_AHEAD_DAYS)
            .escalation_hours(seed::ESCALATION_HOURS)
            .reminder_hours(seed::REMINDER_HOURS)
            .no_show_grace_minutes(seed::NO_SHOW_GRACE_MINUTES)
            .manager_role(CLINIC_MANAGER_ROLE)
            // 05 §7.1: after a walk-out the remainder of the deposit is
            // refunded, for the seeded service types and those added later.
            .walk_out_forfeits(false);
        for (code, initiated_by) in clinics::reasons::SCHEDULING_REASON_CODES {
            module = module.reason_code(code, *initiated_by);
        }
    }
    module
}

/// The HR module with the Clinics client's values: the clinic manager is the
/// line manager of a practitioner and the owner role of the client the second
/// person for waiving and reopening a step.
pub fn hr(module: wl_hr::WlHr, owner_role: &str) -> wl_hr::WlHr {
    module
        .manager_role(CLINIC_MANAGER_ROLE)
        .owner_role(owner_role)
}

/// The notification module with the Clinics client's values (`NTF.RUL.02`,
/// `NTF.ACL.03`): at most 2 marketing messages per patient in 7 days, and a
/// broadcast to more than 200 patients needs a second approver. The send
/// window 07:00-21:00 is stored by [`install`]. Any other client keeps the
/// module's defaults.
pub fn notify_for(client_app: &str) -> WlNotify {
    let module = WlNotify::new();
    if client_app == CLINICS {
        return module
            .marketing_frequency_cap(seed::MARKETING_CAP)
            .broadcast_approval_above(seed::BROADCAST_APPROVAL_ABOVE);
    }
    module
}

/// The sales module with the Clinics client's values (`SAL.CRM.02`,
/// `SAL.ACL.07`, `SAL.LOY.01`, `SAL.RUL.08`, `CLN.ACL.13`):
///
/// - a lead or a first sale from *paid ads* or *social* names a campaign;
/// - a cost entry over a campaign's budget needs a manager;
/// - points expire with a reminder 30 days before, a manual adjustment above
///   500 points needs a manager, a redemption is confirmed by a one-time code;
/// - the clinical services of the clinic are not offered to a segment and no
///   segment rule may use them (visit dates, spend and non-clinical items
///   only);
/// - a campaign is activated and sends only with its advertising permit
///   ([`AdvertisingPermitGuard`]).
pub fn sales(module: WlSales) -> WlSales {
    let clinical: Vec<&str> = seed::SERVICE_TYPES.iter().map(|s| s.code).collect();
    module
        .over_budget_approval(true)
        .campaign_required_for(seed::CAMPAIGN_REQUIRED_SOURCES)
        .expiry_reminder_days(seed::LOYALTY_REMINDER_DAYS)
        .adjustment_limit(seed::LOYALTY_ADJUSTMENT_LIMIT)
        .confirm_redemption_by_code(true)
        .clinical_items(&clinical)
        .segment_clinical_rules(false)
        .campaign_guard(AdvertisingPermitGuard)
        // A package whose payment was reversed stays active and the front
        // desk is told to collect before the next session; consumption is
        // not suspended (BL-SAL-16, BL-CLN-16).
        .package_seller_role(RECEPTIONIST_ROLE)
        .suspend_package_consumption_while_unpaid(false)
}

/// The advertising permit guard of wl-sales' campaigns (`CLN.ACL.13`): a
/// campaign that promotes services is activated, and starts a broadcast,
/// only while its permit number and validity are recorded and valid; the day
/// after the permit's last valid day the daily run ends it, which cancels the
/// broadcast still waiting (`NTF.ACL.04`). The permits are the app's
/// ([`clinics::permits`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct AdvertisingPermitGuard;

type GuardFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

impl CampaignGuard for AdvertisingPermitGuard {
    fn check<'a>(
        &'a self,
        state: &'a AppState,
        campaign: &'a Campaign,
    ) -> GuardFuture<'a, Option<String>> {
        Box::pin(async move {
            Ok(permits::problem(&state.db, campaign.id, today())
                .await?
                .map(|problem| problem.code().to_owned()))
        })
    }

    fn activation<'a>(
        &'a self,
        state: &'a AppState,
        campaign: &'a Campaign,
    ) -> GuardFuture<'a, Option<String>> {
        self.check(state, campaign)
    }

    fn describe(&self, code: &str) -> String {
        Problem::from_code(code).map_or_else(|| code.to_owned(), Problem::message)
    }

    fn last_day<'a>(
        &'a self,
        state: &'a AppState,
        campaign: &'a Campaign,
    ) -> GuardFuture<'a, Option<NaiveDate>> {
        Box::pin(async move { permits::last_day(&state.db, campaign.id).await })
    }
}

/// The send handler of a connection nothing is queued for: the manual way is
/// the fallback.
fn sent<'a>(
    _state: &'a AppState,
    _call: &'a integrations::Outbound,
) -> Pin<Box<dyn Future<Output = Result<Delivery>> + Send + 'a>> {
    Box::pin(async { Ok(Delivery::Sent) })
}

/// Registers the connections 05 keeps a manual fallback for (05 §8 *When a
/// connection is down*, wl-kernel §5.14): the SMS and WhatsApp gateways (the
/// front desk calls the patients on the list of each branch), the payment
/// gateway (take the payment on the terminal or in cash), the claims channel
/// (submit through the payer portal and record the reference) and e-mail.
/// Each shows its banner on the screens it affects. wl-notify reports the
/// SMS, WhatsApp and e-mail sends; the gateway and the claims channel are
/// reported by the adapters that are not built (`DEFERRED-b6.md`), and an
/// administrator can take any of them out of service by hand.
pub fn register_integrations(client_app: &str) {
    if client_app != CLINICS {
        return;
    }
    IntegrationDef::of(integrations::SMS, "notify", sent)
        .owner_role(CLINIC_MANAGER_ROLE)
        .screens([clinics::call_list::SCREEN])
        .fallback(seed::SMS_FALLBACK_EN, Some(seed::SMS_FALLBACK_AR))
        .register();
    IntegrationDef::of(integrations::WHATSAPP, "notify", sent)
        .owner_role(CLINIC_MANAGER_ROLE)
        .screens([clinics::call_list::SCREEN])
        .fallback(seed::WHATSAPP_FALLBACK_EN, Some(seed::WHATSAPP_FALLBACK_AR))
        .register();
    IntegrationDef::of(integrations::EMAIL, "notify", sent)
        .owner_role(COMPANY_ADMIN_ROLE)
        .screens([clinics::pages::visits::SCREEN])
        .fallback(seed::EMAIL_FALLBACK_EN, Some(seed::EMAIL_FALLBACK_AR))
        .register();
    IntegrationDef::of(integrations::PAYMENT_GATEWAY, "payments", sent)
        .owner_role(super::roles::ACCOUNTANT)
        .screens([clinics::pages::visits::SCREEN])
        .fallback(seed::GATEWAY_FALLBACK_EN, Some(seed::GATEWAY_FALLBACK_AR))
        .register();
    IntegrationDef::of(seed::CLAIMS_CHANNEL, "clinics", sent)
        .owner_role(BILLING_COORDINATOR_ROLE)
        .screens([
            clinics::pages::claims::CLAIMS_SCREEN,
            clinics::pages::claims::CLAIM_SCREEN,
        ])
        .fallback(seed::CLAIMS_FALLBACK_EN, Some(seed::CLAIMS_FALLBACK_AR))
        .register();
}

/// Gives the appointment reminder the give-up reaction 05 asks for: none
/// (`SCH.RUL.01`, 05 §8: a reminder that gives up needs no action). The
/// other booking messages keep the module's defaults. The registry is the
/// kernel's, process wide, and the type is wl-scheduling's, registered when
/// that module is mounted; this runs after it.
fn quiet_reminders() {
    if let Some(def) = notification_type(wl_scheduling::notices::REMINDER) {
        register_type((*def).clone().give_up(&[GiveUpReaction::None]));
    }
}

/// What [`install`] stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Service types created.
    pub service_types: usize,
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
    /// Budget categories given the accountant as owner role.
    pub budget_categories: usize,
}

impl Installed {
    /// Everything created.
    pub fn total(&self) -> usize {
        self.service_types
            + self.checklists
            + self.sops
            + self.send_windows
            + self.loyalty
            + self.segments
            + self.budget_categories
    }
}

/// Stores what the Clinics client's modules lack, for the modules the
/// manifest enables, and keeps what the client changed: the service types
/// (by code), each checking the patient in, tracking the visit, needing the
/// licence and the privileges and taking no booking deposit; the onboarding
/// checklists; the SOP library; the marketing send windows; the loyalty
/// programme and the lapsed-patients segment.
///
/// # Errors
///
/// The database's errors.
pub async fn install(state: &AppState) -> Result<Installed> {
    let manifest = state.provided::<Arc<Manifest>>();
    install_for(state, &|name: &str| {
        manifest.as_ref().is_some_and(|m| m.is_enabled(name))
    })
    .await
}

/// [`install`] for the modules `on` says are enabled: each step runs only
/// when the module it belongs to is on.
///
/// # Errors
///
/// The database's errors.
pub async fn install_for(state: &AppState, on: &(dyn Fn(&str) -> bool + Sync)) -> Result<Installed> {
    let mut installed = Installed::default();
    if on("scheduling") {
        installed.service_types = install_service_types(state).await?;
    }
    if on("hr") {
        installed.checklists = install_checklists(&state.db).await?;
    }
    if on("documents") {
        installed.sops = install_sops(&state.db).await?;
    }
    if on("notify") {
        installed.send_windows = install_send_windows(&state.db).await?;
    }
    if on("invoicing") {
        installed.budget_categories =
            super::clinics_money::install_budget_categories(&state.db).await?;
        // The payer write-off policy belongs to invoicing, not scheduling.
        keep_write_off_with_the_owner(state).await?;
    }
    if on("sales") {
        installed.loyalty = install_loyalty(&state.db).await?;
        installed.segments = install_segments(state).await?;
    }
    Ok(installed)
}

/// Stores the service types the database lacks.
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
        service.check_in_required = true;
        service.track_in_progress = true;
        service.required_qualifications = REQUIRED_QUALIFICATIONS.to_owned();
        // 05 §13 question 8: no booking deposit until the clinic enables one
        // for the service type; a walk-out after check-in refunds the
        // remainder, only a late cancellation or a no-show forfeits.
        service.deposit_amount = seed::BOOKING_DEPOSIT_FILS;
        service.deposit_forfeit_hours = seed::DEPOSIT_FORFEIT_HOURS;
        service.save(&state.db).await?;
        created += 1;
    }
    Ok(created)
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

/// Stores the marketing send window 07:00-21:00 of each channel that has
/// none (`NTF.RUL.02`); a window the client changed is kept.
async fn install_send_windows(db: &Db) -> Result<usize> {
    use wl_kernel::notify::NotifyChannel;
    let (start, end) = seed::SEND_WINDOW_MINUTES;
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

/// Creates the loyalty programme where the client has none (`SAL.LOY.01`,
/// `SAL.RUL.07`): 1 point per AED 10, 100 points an AED 10 discount, 12
/// months of validity.
async fn install_loyalty(db: &Db) -> Result<usize> {
    if LoyaltyProgramme::first_active(db).await?.is_some() {
        return Ok(0);
    }
    let new = NewProgramme::new(seed::LOYALTY_NAME)
        .points_per_aed_units(seed::LOYALTY_POINTS, seed::LOYALTY_EARN_PER_AED)
        .redeeming(seed::LOYALTY_POINT_VALUE_FILS, seed::LOYALTY_MIN_REDEEM)
        .validity_months(seed::LOYALTY_VALIDITY_MONTHS)
        .terms(
            "1",
            "You earn 1 point for every AED 10 you pay for self-pay services and products. 100 points give an AED 10 discount on a later self-pay invoice, confirmed with a one-time code. Points expire 12 months after they are earned. Payments by an insurer and package sessions earn nothing.",
            "تكسب نقطة واحدة عن كل 10 دراهم تدفعها مقابل الخدمات والمنتجات الخاصة. تعادل 100 نقطة خصمًا قدره 10 دراهم على فاتورة لاحقة، بعد تأكيده برمز لمرة واحدة. تنتهي صلاحية النقاط بعد 12 شهرًا من كسبها. لا تكسب مدفوعات شركة التأمين وجلسات الباقات نقاطًا.",
        );
    wl_sales::loyalty_programme::create(db, new).await?;
    Ok(1)
}

/// Stores the lapsed-patients segment (`SAL.LOY.01`): no visit in 6 months,
/// by visit dates only. The segments that need booking or birthday data
/// are listed in `DEFERRED-b6.md`.
async fn install_segments(state: &AppState) -> Result<usize> {
    let exists = Segment::query()
        .where_(Segment::NAME.eq(seed::LAPSED_SEGMENT_NAME))
        .exists(&state.db)
        .await?;
    if exists {
        return Ok(0);
    }
    let config = wl_sales::config::config_of(state);
    let segment = Segment::of(
        seed::LAPSED_SEGMENT_NAME,
        MatchMode::All,
        &[Rule::NoSaleForDays {
            days: seed::LAPSED_SEGMENT_DAYS,
        }],
    )
    .store(&state.db, &config)
    .await?;
    match segment {
        Ok(_) => Ok(1),
        Err(refused) => Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{refused:?}"),
        )),
    }
}

/// The approval action of a payer bill's write-off above the limit.
const WRITE_OFF_ACTION: &str = "invoicing.payer_write_off";

/// Brings a stored write-off policy that is still wl-invoicing's default (a
/// database made before the module was built with the owner's approver roles:
/// `install` keeps the rows that exist) to the owner and the company
/// administrators. A policy an admin changed is left as it is. Returns
/// whether a row changed.
///
/// # Errors
///
/// The database's errors.
pub async fn keep_write_off_with_the_owner(state: &AppState) -> Result<bool> {
    let owner = Step::one_of([OWNER_ROLE, COMPANY_ADMIN_ROLE]);
    let default = Step::one_of(["admin"]).or_permission("payer_bills.write_off");
    let mut changed = false;
    for mut policy in ApprovalPolicy::query()
        .where_(ApprovalPolicy::ACTION.eq(WRITE_OFF_ACTION))
        .get(&state.db)
        .await?
    {
        if policy.steps.0 == [default.clone()] {
            policy.steps = Json(vec![owner.clone()]);
            policy.save(&state.db).await?;
            changed = true;
        }
    }
    Ok(changed)
}

/// The module that seeds the Clinics client's values: the seeder runs after
/// the shared modules' own (it is mounted after them), and the command
/// `clinics:install` runs the same install.
pub struct ClinicsSeed;

impl Module for ClinicsSeed {
    fn name(&self) -> &'static str {
        "clinics_seed"
    }

    fn register(&self, app: &mut Registry) {
        quiet_reminders();
        app.command(
            "clinics:install",
            "Store the Clinics service types, checklists, SOPs, windows, loyalty programme and segment, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = install(&state).await?;
                println!("{} Clinics defaults created.", installed.total());
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

    #[test]
    fn the_practitioner_kinds_block_booking_check_in_and_start_and_are_never_overridden() {
        for kind in [licence_kind(), privileges_kind()] {
            let guarded: Vec<&str> = kind.blocks.iter().map(|b| b.guarded.as_str()).collect();
            assert_eq!(
                guarded,
                ["booking.book", "booking.check_in", "booking.start"]
            );
            assert_eq!(kind.override_mode, OverrideMode::Never);
            assert!(kind.override_fixed && !kind.legal_block);
            assert_eq!(kind.responsible_role.as_deref(), Some("clinic_manager"));
            assert!(kind.has_expiry && kind.has_number);
            assert_eq!(kind.look_ahead_days, 30);
            assert_eq!(kind.reminder_offsets, [90, 60, 30, 7]);
            assert_eq!(kind.subject_types, ["hr.employee"]);
        }
        assert_eq!(licence_kind().key, "practitioner_licence");
        assert_eq!(privileges_kind().key, "branch_privileges");
    }

    #[test]
    fn the_facility_licence_is_the_owners_and_the_emirates_id_notifies_the_patient() {
        let facility = facility_licence_kind();
        assert_eq!(facility.responsible_role.as_deref(), Some("owner"));
        assert!(facility.compliance_critical && facility.blocks.is_empty());
        assert_eq!(facility.reminder_offsets, [180, 90, 30, 7]);
        assert_eq!(facility.subject_types, [LEGAL_ENTITY_SUBJECT]);

        for kind in [emirates_id_kind(), insurance_card_kind()] {
            assert_eq!(kind.responsible_role.as_deref(), Some("receptionist"));
            assert!(kind.notify_party && kind.portal_upload);
            assert_eq!(kind.sensitivity, Sensitivity::Personal);
        }
    }

    #[test]
    fn the_sales_module_has_the_05_values() {
        let module = sales(WlSales::new());
        let config = module.config();
        assert!(config.over_budget_approval);
        assert_eq!(config.campaign_required_sources, ["paid_ads", "social"]);
        assert_eq!(config.expiry_reminder_days, 30);
        assert_eq!(config.adjustment_limit, 500);
        assert!(config.confirm_redemption_by_code);
        assert!(!config.segment_clinical_rules);
        assert!(config.is_blocked_clinical("consultation"));
        assert!(!config.is_blocked_clinical("shampoo"));
    }

    #[test]
    fn the_scheduling_and_notify_modules_have_the_05_numbers() {
        let scheduling = scheduling_for(CLINICS);
        let config = scheduling.config();
        assert_eq!(config.look_ahead_days, 30);
        assert_eq!(config.escalation_hours, 2);
        assert_eq!(config.reminder_hours, [24, 2]);
        assert_eq!(config.no_show_grace_minutes, 15);
        assert_eq!(config.manager_role.as_deref(), Some("clinic_manager"));
        assert!(
            scheduling_for("property").config().manager_role.is_none(),
            "another client keeps the module's defaults"
        );
    }

    #[test]
    fn every_onboarding_step_is_valid_for_wl_hr() {
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
            // Tasks to the clinic manager, the line manager and the employee.
            for owner in ["role:clinic_manager", "manager", "employee"] {
                assert!(spec.steps.iter().any(|s| s.1 == owner), "{}", spec.name);
            }
            assert_eq!(spec.steps.last().map(|s| s.3), Some("sops"));
        }
    }
}
