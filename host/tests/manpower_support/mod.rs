//! Shared by the host tests of the deployment guards (BL-MPW-08, -10, -11,
//! -12): a manpower agency booted with the real modules, the document kinds of
//! the spec (§9) installed, workers active in HR, a client with an active
//! contract and a site, and deployments as fixtures. Dates are relative to the
//! real day (`day(n)`), so a test reads like the spec's examples.

#![allow(dead_code)]

use host::roles::ACCOUNT_MANAGER;
use manpower::permissions::client_scope;
use manpower::{ClientContract, ContractStatus, Deployment, DeploymentStatus, Site};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_documents::kinds::{KindSeed, install_seeds};
use wl_documents::{Document, DocumentKind, DocumentStatus};
use wl_hr::{EmployeeStatus, NewEmployee, employees};
use wl_kernel::documents::{BlockCondition, BlockMode, OverrideMode, today};
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{ClientConfig, PartyId, PartyKind};
use wl_parties::{ContactPoint, ContactType, NewParty, PersonDetails, create};
use wl_portal::{AccessGrant, GrantStatus};

use crate::support::{company, day, noon_clock, scoped_person, tenant, without_document_kinds};

/// A manpower agency: the vertical on, with the shared modules it reaches.
pub const CLIENT: &str = r#"
[client]
name = "Gulf Manpower Services LLC"
app = "manpower"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "portal"]

[[legal_entities]]
code = "GMS"
name_en = "Gulf Manpower Services LLC"
default = true
"#;

/// A manpower agency with the sales module too: what the seed of BL-MPW-20 and
/// BL-MPW-21 reaches (`boot_seeded`).
pub const SEEDED_CLIENT: &str = r#"
[client]
name = "Gulf Manpower Services LLC"
app = "manpower"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "portal", "sales"]

[[legal_entities]]
code = "GMS"
name_en = "Gulf Manpower Services LLC"
default = true
"#;

/// The three transitions the kinds guard.
const GUARDED: [&str; 3] = [
    "deployment.activate",
    "deployment.extend",
    "deployment.resume",
];

/// A kind of §9 that blocks the deployment transitions.
fn kind(key: &str, name: &str, override_mode: OverrideMode) -> KindSeed {
    let mut seed = KindSeed::new(key, name)
        .subject_type("hr.employee")
        .expires()
        .reminders(&[30])
        .look_ahead(30)
        .responsible_role("hr")
        .override_mode(override_mode);
    if override_mode == OverrideMode::Never {
        seed = seed.legal_block();
    }
    seed
}

/// The kinds of the spec: visa, work permit and the SIRA licence (security
/// roles) never overridden (`⚠ open: lawyer (L-19)`), the others with approval.
/// `emirates_id` takes the mode a test asks for.
pub fn kinds(emirates_id: OverrideMode) -> Vec<KindSeed> {
    let mut seeds = Vec::new();
    for (key, name, mode) in [
        ("residence_visa", "Residence visa", OverrideMode::Never),
        ("work_permit", "Work permit", OverrideMode::Never),
        ("emirates_id", "Emirates ID", emirates_id),
        (
            "health_insurance",
            "Health insurance",
            OverrideMode::WithApproval,
        ),
    ] {
        let mut seed = kind(key, name, mode);
        for guarded in GUARDED {
            seed = seed.blocks(guarded, BlockMode::Block);
        }
        seeds.push(seed);
    }
    // The licences of a site: for the security roles only.
    for (key, name, mode) in [
        ("sira_licence", "SIRA licence", OverrideMode::Never),
        ("health_card", "Health card", OverrideMode::WithApproval),
    ] {
        let mut seed = kind(key, name, mode);
        for guarded in GUARDED {
            seed = seed.blocks_when(
                guarded,
                BlockMode::Block,
                BlockCondition::new("role", ["security_guard"]),
            );
        }
        seeds.push(seed);
    }
    seeds
}

/// The agency with the kinds installed.
pub async fn boot() -> TestApp {
    boot_with(OverrideMode::WithApproval).await
}

/// The agency with the Emirates ID kind in `emirates_id` override mode.
pub async fn boot_with(emirates_id: OverrideMode) -> TestApp {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    // The fixtures of the spec replace the kinds the host seeded.
    without_document_kinds(&app).await;
    let seeds = kinds(emirates_id);
    install_seeds(app.db(), seeds.iter()).await.unwrap();
    app
}

/// The agency as the host seeds it for a manpower client: the kinds, ticket
/// categories, checklists, SOPs and notification reactions of BL-MPW-20 and
/// BL-MPW-21, with no fixture of a test.
pub async fn boot_seeded() -> TestApp {
    boot_seeded_with(host::Tuning::default()).await
}

/// [`boot_seeded`] with the shared modules changed by `tuning` (a failing
/// channel provider).
pub async fn boot_seeded_with(tuning: host::Tuning) -> TestApp {
    let config = ClientConfig::parse(SEEDED_CLIENT).unwrap();
    let app = TestApp::new(
        host::app_for_tuned(config, tuning)
            .unwrap()
            .provide(noon_clock()),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// [`boot_seeded`] with the test routes of `support::Probe` (the upload of an
/// SOP version needs an `AuthUser`).
pub async fn boot_seeded_probe() -> TestApp {
    let config = ClientConfig::parse(SEEDED_CLIENT).unwrap();
    let app = TestApp::new(
        host::app_for(config)
            .unwrap()
            .provide(noon_clock())
            .module(crate::support::Probe),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// The notifier of a test that can be switched off. While it is down the
/// notices of the vertical (`manpower.*`) fail and everything else, the notices
/// of wl-hr and the owner alert of a compliance-critical failure among them, is
/// recorded; while it is up everything is recorded. Nothing is delivered either
/// way.
#[derive(Clone, Default)]
pub struct SwitchNotifier {
    down: std::sync::Arc<std::sync::atomic::AtomicBool>,
    sent: std::sync::Arc<std::sync::Mutex<Vec<wl_kernel::notify::NotifyRequest>>>,
}

impl SwitchNotifier {
    /// Switches the notifier off or on.
    pub fn set_down(&self, down: bool) {
        self.down.store(down, std::sync::atomic::Ordering::SeqCst);
    }

    /// What it was asked to send and accepted, in order.
    pub fn sent(&self) -> Vec<wl_kernel::notify::NotifyRequest> {
        self.sent.lock().unwrap().clone()
    }
}

impl wl_kernel::notify::Notifier for SwitchNotifier {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: wl_kernel::notify::NotifyRequest,
    ) -> wl_kernel::notify::NotifyFuture<'a, Vec<wl_kernel::notify::MessageId>> {
        Box::pin(async move {
            let down = self.down.load(std::sync::atomic::Ordering::SeqCst);
            if down && request.type_key.starts_with("manpower.") {
                return Err(Error::Internal(renox::anyhow::anyhow!(
                    "the notifier is down"
                )));
            }
            self.sent.lock().unwrap().push(request);
            Ok(Vec::new())
        })
    }
}

/// Provides a [`SwitchNotifier`] after the modules of the host.
struct SwitchModule(SwitchNotifier);

impl Module for SwitchModule {
    fn name(&self) -> &'static str {
        "switch_notifier"
    }

    fn register(&self, app: &mut Registry) {
        wl_kernel::notify::provide_notifier(app, self.0.clone());
    }
}

/// [`boot_seeded`] with the notifier of the host replaced by a
/// [`SwitchNotifier`], so a rule run of the agency can be made to fail.
pub async fn boot_seeded_switch() -> (TestApp, SwitchNotifier) {
    let switch = SwitchNotifier::default();
    let config = ClientConfig::parse(SEEDED_CLIENT).unwrap();
    let app = TestApp::new(
        host::app_for(config)
            .unwrap()
            .provide(noon_clock())
            .module(SwitchModule(switch.clone())),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    (app, switch)
}

/// An employee `active` in HR.
pub async fn worker(app: &TestApp, name: &str) -> EmployeeId {
    let party = tenant(app, name).await;
    let employee = employees::create(
        app.state(),
        NewEmployee::new(party, "security_guard", today()).status(EmployeeStatus::Active),
    )
    .await
    .unwrap();
    EmployeeId(employee.id)
}

/// An employee `active` in HR whose party has the mobile number `number`.
pub async fn worker_with_mobile(app: &TestApp, name: &str, number: &str) -> EmployeeId {
    let party = create(
        app.state(),
        NewParty {
            kind: PartyKind::Person,
            legal_name_en: name.into(),
            person: Some(PersonDetails::default()),
            contacts: vec![ContactPoint {
                contact_type: ContactType::Mobile,
                value: number.into(),
                is_primary: true,
                ..Default::default()
            }],
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id();
    let employee = employees::create(
        app.state(),
        NewEmployee::new(party, "security_guard", today()).status(EmployeeStatus::Active),
    )
    .await
    .unwrap();
    EmployeeId(employee.id)
}

/// A salary structure for `who` from 1 Jan 2026: a basic wage of `wage` fils.
pub async fn salary(app: &TestApp, who: EmployeeId, wage: i64) {
    wl_hr::SalaryStructure::create(
        app.db(),
        wl_hr::SalaryStructure {
            employee_id: who.0,
            effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            basic: wage,
            hourly_rate: Some(wage / 240),
            ..wl_hr::SalaryStructure::default()
        },
    )
    .await
    .unwrap();
}

/// A regular payroll run of `period` in `draft`, for the preparer to calculate.
pub async fn draft_run(app: &TestApp, period: &str) -> wl_hr::PayrollRun {
    let mut run = wl_hr::PayrollRun {
        period: period.to_owned(),
        run_type: "regular".into(),
        status: wl_hr::PayrollRunStatus::Draft,
        ..wl_hr::PayrollRun::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    run.insert_in_state(&mut tx, wl_hr::PayrollRunStatus::Draft, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    run
}

/// `by` runs the transition `name` of payroll run `run` (the page route).
pub async fn run_transition(
    app: &TestApp,
    by: &User,
    run: &wl_hr::PayrollRun,
    name: &str,
) -> TestResponse {
    post(
        app,
        by,
        &format!("/hr/payroll/{}/transitions/{name}", run.id),
        &[("reason", "")],
    )
    .await
}

/// An SMS provider that delivers everything and records the numbers.
#[derive(Clone, Default)]
pub struct DeliveringSms {
    sent: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl DeliveringSms {
    /// The numbers it was asked to send to, in order.
    pub fn numbers(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }
}

impl wl_notify::ChannelProvider for DeliveringSms {
    fn name(&self) -> &'static str {
        "delivering-sms"
    }

    fn channel(&self) -> wl_kernel::notify::NotifyChannel {
        wl_kernel::notify::NotifyChannel::Sms
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        outgoing: &'a wl_notify::Outgoing,
    ) -> wl_notify::channels::ProviderFuture<'a, std::result::Result<String, wl_notify::SendError>>
    {
        self.sent.lock().unwrap().push(outgoing.address.clone());
        Box::pin(async { Ok("delivered".to_owned()) })
    }
}

/// An accepted document of `kind` for `who`, valid through `until`.
pub async fn document(app: &TestApp, who: EmployeeId, kind: &str, until: NaiveDate) -> Document {
    let kind = DocumentKind::by_key(app.db(), kind).await.unwrap().unwrap();
    Document::create(
        app.db(),
        Document {
            subject_type: "hr.employee".into(),
            subject_id: who.0,
            kind_id: kind.id,
            title: kind.key.clone(),
            expiry_date: Some(until),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// Every kind of the spec valid for `who` for a year.
pub async fn all_valid(app: &TestApp, who: EmployeeId) {
    all_valid_except(app, who, &[]).await;
}

/// Every kind of the spec valid for `who` for a year, but `except` (a test
/// gives those its own document).
pub async fn all_valid_except(app: &TestApp, who: EmployeeId, except: &[&str]) {
    for kind in [
        "residence_visa",
        "work_permit",
        "emirates_id",
        "health_insurance",
        "sira_licence",
        "health_card",
    ] {
        if !except.contains(&kind) {
            document(app, who, kind, day(365)).await;
        }
    }
}

/// The client company, its account manager, an active contract running from
/// 200 days ago to 400 days ahead, and a site.
pub struct Account {
    pub client: PartyId,
    pub amal: User,
    pub contract: ClientContract,
    pub site: Site,
}

pub async fn account(app: &TestApp) -> Account {
    account_named(app, "Gulf Tower LLC", "CC-0001", "Amal").await
}

/// Another client company `name` with its own contract `reference` and
/// account manager `manager`, as [`account`].
pub async fn account_named(app: &TestApp, name: &str, reference: &str, manager: &str) -> Account {
    let client = company(app, name).await;
    let amal = scoped_person(app, manager, ACCOUNT_MANAGER, &client_scope(client.0)).await;
    let mut contract = ClientContract {
        reference: reference.into(),
        client_party_id: client.0,
        start_date: day(-200),
        end_date: day(400),
        payment_terms_days: 30,
        notice_period_days: 30,
        account_manager_user_id: Some(amal.id),
        cover_required: true,
        status: ContractStatus::Active,
        ..ClientContract::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    contract
        .insert_in_state(&mut tx, ContractStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let site = site(app, client, "Tower A").await;
    Account {
        client,
        amal,
        contract,
        site,
    }
}

/// A site of `client`.
pub async fn site(app: &TestApp, client: PartyId, name: &str) -> Site {
    Site::create(
        app.db(),
        Site {
            client_party_id: client.0,
            name: name.into(),
            active: true,
            ..Site::default()
        },
    )
    .await
    .unwrap()
}

/// A deployment of `who` as `role` at `site`, from `start` to `end`, with the
/// shift `from` to `to`, in `status` (a fixture).
#[allow(clippy::too_many_arguments)]
pub async fn deployment(
    app: &TestApp,
    account: &Account,
    site: &Site,
    who: EmployeeId,
    role: &str,
    status: DeploymentStatus,
    start: NaiveDate,
    end: Option<NaiveDate>,
) -> Deployment {
    let mut deployment = Deployment {
        employee_id: who.0,
        client_contract_id: account.contract.id,
        client_party_id: account.client.0,
        site_id: site.id,
        role: role.into(),
        shift_type: "day".into(),
        shift_start: Some("07:00".into()),
        shift_end: Some("15:00".into()),
        start_date: start,
        end_date: end,
        status,
        ..Deployment::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    deployment
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    deployment
}

/// A POST by `by`.
pub async fn post(app: &TestApp, by: &User, path: &str, form: &[(&str, &str)]) -> TestResponse {
    app.acting_as(by);
    app.htmx().post(path, form).await
}

/// `activate` of `deployment` by `by`.
pub async fn activate(app: &TestApp, by: &User, deployment: &Deployment) -> TestResponse {
    post(
        app,
        by,
        &format!("/manpower/deployments/{}/activate", deployment.id),
        &[],
    )
    .await
}

/// `extend` of `deployment` to `end` by `by`.
pub async fn extend(
    app: &TestApp,
    by: &User,
    deployment: &Deployment,
    end: NaiveDate,
) -> TestResponse {
    let end = end.to_string();
    post(
        app,
        by,
        &format!("/manpower/deployments/{}/extend", deployment.id),
        &[("end_date", end.as_str())],
    )
    .await
}

/// The deployment `id` as it is now.
pub async fn reload(app: &TestApp, id: i64) -> Deployment {
    Deployment::find_or_404(app.db(), id).await.unwrap()
}

/// `user` acts for `client` in the portal with the portal role `roles`
/// (`owner` or `approver` make them one of the client's approvers).
pub async fn portal_grant(app: &TestApp, user: &User, client: PartyId, roles: &str) {
    let person = tenant(app, &format!("{} (portal person)", user.name)).await;
    let mut grant = AccessGrant {
        user_id: Some(user.id),
        person_party_id: person.0,
        party_id: client.0,
        audience: "tenant".into(),
        roles: roles.into(),
        source: "invitation".into(),
        valid_from: day(-30),
        status: GrantStatus::Active,
        ..AccessGrant::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    grant
        .insert_in_state(&mut tx, GrantStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}
