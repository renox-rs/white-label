//! The demo agency of the Manpower client (09 Manpower, DEMO-DATA.md: *Desert
//! Star Manpower Services LLC*, a Dubai manpower supply agency, all fictional):
//! `manpower:demo` fills a local database so that every screen of F9 has data
//! the moment someone logs in, and `db:seed` runs it in `local`, never in
//! production.
//!
//! - **Services where there is one, fixtures where there is none.** Parties,
//!   employees, documents, rate cards and cheques are made by the modules' own
//!   services; the invoice is drafted by wl-invoicing and issued through the
//!   app's own router as the accountant (the demo driver of the Property demo).
//!   A record in a state the machines only reach after days of real life (an
//!   active deployment, a submitted timesheet, a draft payroll run with a held
//!   line) is inserted in that state with `insert_in_state`.
//! - **Relative dates.** Every date is `day(n)`, today plus or minus days, so
//!   the screens look alive whenever the seed runs (a document that expires in
//!   20 days, a contract that ends in 45).
//! - **Idempotent.** The seed is done once its marker exists (an audit entry
//!   of the subject `manpower_demo`); logins are kept when they exist.
//!
//! # The agency
//!
//! | Client | Contract | What it shows |
//! |---|---|---|
//! | Gulf Tower Facilities LLC | CC-DEMO-001, 165 days left | rate card v1 accepted, v2 waiting for the client; a partly filled request; a timesheet waiting for the client approver |
//! | Marina Bay Hospitality LLC | CC-DEMO-002, ends in 45 days | the renewal warning; an open request |
//! | Al Quoz Logistics Park LLC | CC-DEMO-003 | an invoice 31 days overdue (the credit hold job holds the contract), a post-dated cheque, a suspended worker |
//!
//! Every login has the password of [`super::seed::PASSWORD`].

use manpower::permissions::{MD_ROLE, client_scope, site_scope};
use manpower::{
    ClientContract, ContractStatus, Deployment, DeploymentKind, DeploymentStatus, ManpowerRequest,
    RequestKind, RequestStatus, Site,
};
use renox::Environment;
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_attendance::record::AttendanceStatus;
use wl_attendance::timesheet::{ApprovalBasis, ApproverKind};
use wl_attendance::{AttendanceRecord, Timesheet, TimesheetStatus, WorkLocation};
use wl_documents::{Document, DocumentKind, DocumentStatus};
use wl_hr::pay_account::{PayAccount, PayAccountStatus};
use wl_hr::payroll::{LineStatus, PayrollLine, PayrollRun, PayrollRunStatus};
use wl_hr::settlement::{FinalSettlement, SettlementStatus};
use wl_hr::visa::{VisaCase, VisaCaseStatus, VisaCaseStep};
use wl_hr::{EmployeeStatus, NewEmployee, SalaryStructure, employees};
use wl_invoicing::rate_card::{RateCard, RateCardLine, RateCardVersion, Unit, VersionStatus};
use wl_invoicing::rate_cards::{self, LineInput};
use wl_kernel::cheques::{RegisterCheque, cheques};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{
    Channel, CompanyDetails, ContactPoint, ContactType, Language, NewParty, PersonDetails,
};
use wl_portal::{AccessGrant, GrantStatus};

use super::demo_data::driver::Driver;
use super::roles::{
    ACCOUNT_MANAGER, ACCOUNTANT, ADMIN, HR, MEMBER, PRO, RECEIVABLES, SUPERVISOR, define,
};
use super::seed::PASSWORD;

/// The command that fills a local database with the demo agency.
pub const COMMAND: &str = "manpower:demo";

/// The audit subject of the marker of a finished seed.
const MARKER_SUBJECT: &str = "manpower_demo";
const DONE_ACTION: &str = "manpower_demo.seeded";

/// One demo login.
#[derive(Debug, Clone, Copy)]
pub struct Login {
    /// The name shown on the login page.
    pub name: &'static str,
    /// The sign-in e-mail.
    pub email: &'static str,
    /// The roles.
    pub roles: &'static [&'static str],
    /// What it is for, shown beside the name.
    pub label: &'static str,
}

/// The demo people, one per role of the agency.
pub const LOGINS: &[Login] = &[
    Login {
        name: "Rana Al Suwaidi",
        email: "admin@desertstar.example",
        roles: &[ADMIN],
        label: "Admin (company admin)",
    },
    Login {
        name: "Omar Haddad",
        email: "md@desertstar.example",
        roles: &[MD_ROLE],
        label: "Managing director",
    },
    Login {
        name: "Mona Farouk",
        email: "hr@desertstar.example",
        roles: &[HR],
        label: "HR and payroll",
    },
    Login {
        name: "Amal Youssef",
        email: "accountmanager@desertstar.example",
        roles: &[ACCOUNT_MANAGER],
        label: "Account manager",
    },
    Login {
        name: "Wendy D'Souza",
        email: "accountant@desertstar.example",
        roles: &[ACCOUNTANT, RECEIVABLES],
        label: "Accountant",
    },
    Login {
        name: "Sami Nasser",
        email: "supervisor@desertstar.example",
        roles: &[SUPERVISOR],
        label: "Site supervisor",
    },
    Login {
        name: "Pedro Alvarez",
        email: "pro@desertstar.example",
        roles: &[PRO],
        label: "PRO (visa officer)",
    },
    Login {
        name: "Carl Whitfield",
        email: "approver@gulftower.example",
        roles: &[MEMBER, CLIENT_APPROVER],
        label: "Client approver (portal), Gulf Tower",
    },
];

/// The role the timesheets of the demo name as their external approver (a
/// timesheet's `approver_role`): the client's approver holds it, so the client
/// approves in the app as in the spec.
const CLIENT_APPROVER: &str = "client_approver";

/// `n` days from today (negative: before).
fn day(n: i64) -> NaiveDate {
    if n >= 0 {
        today() + Days::new(n as u64)
    } else {
        today() - Days::new(n.unsigned_abs())
    }
}

/// Inserts `row` in `status` (a fixture: the machine would not take it there).
macro_rules! fixture {
    ($db:expr, $row:expr, $status:expr) => {{
        let mut row = $row;
        let mut tx = $db.begin().await?;
        row.insert_in_state(&mut tx, $status, "demo seed").await?;
        tx.commit().await?;
        row
    }};
}

/// A worker of the agency.
struct Worker {
    key: &'static str,
    name: &'static str,
    nationality: &'static str,
    role: &'static str,
    /// The basic monthly wage in fils.
    basic: i64,
    hired: i64,
    status: EmployeeStatus,
}

const W_WASIM: usize = 0;
const W_RAVI: usize = 1;
const W_JOSEPH: usize = 2;
const W_BILAL: usize = 3;
const W_MARIA: usize = 4;
const W_EKO: usize = 5;
const W_SURESH: usize = 6;
const W_TESFAYE: usize = 7;
const W_HAMZA: usize = 8;
const W_KAMAL: usize = 9;
const W_FAISAL: usize = 10;
const W_ANIL: usize = 11;
const W_DAWIT: usize = 12;
const W_NOEL: usize = 13;

const WORKERS: &[Worker] = &[
    Worker {
        key: "wasim",
        name: "Wasim Khan",
        nationality: "PK",
        role: "security_guard",
        basic: 180_000,
        hired: -400,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "ravi",
        name: "Ravi Kumar",
        nationality: "IN",
        role: "security_guard",
        basic: 185_000,
        hired: -380,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "joseph",
        name: "Joseph Mensah",
        nationality: "GH",
        role: "security_guard",
        basic: 175_000,
        hired: -300,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "bilal",
        name: "Bilal Ahmed",
        nationality: "PK",
        role: "cleaner",
        basic: 140_000,
        hired: -350,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "maria",
        name: "Maria Santos",
        nationality: "PH",
        role: "cleaner",
        basic: 150_000,
        hired: -330,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "eko",
        name: "Eko Prasetyo",
        nationality: "ID",
        role: "cleaner",
        basic: 140_000,
        hired: -120,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "suresh",
        name: "Suresh Nair",
        nationality: "IN",
        role: "driver",
        basic: 200_000,
        hired: -250,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "tesfaye",
        name: "Tesfaye Alemu",
        nationality: "ET",
        role: "labourer",
        basic: 130_000,
        hired: -220,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "hamza",
        name: "Hamza Yousuf",
        nationality: "EG",
        role: "security_guard",
        basic: 190_000,
        hired: -500,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "kamal",
        name: "Kamal Hossain",
        nationality: "BD",
        role: "labourer",
        basic: 125_000,
        hired: -90,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "faisal",
        name: "Faisal Rahman",
        nationality: "BD",
        role: "security_guard",
        basic: 178_000,
        hired: -200,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "anil",
        name: "Anil Thapa",
        nationality: "NP",
        role: "security_guard",
        basic: 176_000,
        hired: -150,
        status: EmployeeStatus::Active,
    },
    Worker {
        key: "dawit",
        name: "Dawit Bekele",
        nationality: "ET",
        role: "labourer",
        basic: 128_000,
        hired: -600,
        status: EmployeeStatus::Notice,
    },
    Worker {
        key: "noel",
        name: "Noel Reyes",
        nationality: "PH",
        role: "cleaner",
        basic: 145_000,
        hired: -5,
        status: EmployeeStatus::Onboarding,
    },
];

/// The module that adds the command and the seeder.
pub struct ManpowerDemo {
    client: wl_kernel::ClientConfig,
}

impl ManpowerDemo {
    /// The module for the client `config`.
    pub fn new(config: wl_kernel::ClientConfig) -> Self {
        Self { client: config }
    }
}

impl Module for ManpowerDemo {
    fn name(&self) -> &'static str {
        "manpower_demo"
    }

    fn register(&self, app: &mut Registry) {
        let client = self.client.clone();
        app.command(
            COMMAND,
            "Fill the database with the Desert Star manpower agency (clients, contracts, workers, deployments, timesheets, payroll); local and testing only",
            move |_args, state: AppState| {
                let client = client.clone();
                async move { run_command(client, state, true).await }
            },
        );
        let client = self.client.clone();
        app.seeder(move |state: AppState| {
            let client = client.clone();
            async move { run_command(client, state, false).await }
        });
    }
}

/// `manpower:demo` and the seeder: boots the app a second time on the same
/// database to get its router (the accountant issues the invoice through it),
/// then seeds. The command works in `local` and `testing`; the seeder of
/// `db:seed` only in `local`.
async fn run_command(client: wl_kernel::ClientConfig, state: AppState, said: bool) -> Result {
    let env = state.config.env;
    let allowed = if said {
        matches!(env, Environment::Local | Environment::Testing)
    } else {
        env == Environment::Local
    };
    if !allowed {
        if said {
            return Err(renox::anyhow::anyhow!(
                "the demo agency is made in local or testing only, never in {env:?}"
            )
            .into());
        }
        return Ok(());
    }
    if state.config.database_url.contains(":memory:") {
        if said {
            return Err(renox::anyhow::anyhow!(
                "the demo agency needs a database file, not an in-memory database"
            )
            .into());
        }
        return Ok(());
    }
    let app = crate::app_for(client).map_err(|e| renox::anyhow::anyhow!("{e}"))?;
    let kernel = app.config((*state.config).clone()).boot().await?;
    if !seed(&kernel).await? {
        println!("The demo agency is there already.");
        return Ok(());
    }
    println!("The demo agency is made. Log in with any of these (password {PASSWORD}):");
    for login in LOGINS {
        println!("  {:<40} {}", login.email, login.label);
    }
    Ok(())
}

/// Seeds the demo agency into the database `kernel` is on; `false` when it is
/// there already and nothing was made. (The app must have been installed:
/// the legal entity of `client.toml` exists.)
///
/// # Errors
///
/// The database's errors, and the message of the rule a demo record broke.
pub async fn seed(kernel: &renox::Kernel) -> Result<bool> {
    let state = kernel.state().clone();
    if is_seeded(&state.db).await? {
        return Ok(false);
    }
    let driver = Driver::new(kernel.router());
    Box::pin(Seed::new(state, driver).run()).await?;
    Ok(true)
}

/// The demo is there once its last step recorded that it is done.
pub async fn is_seeded(db: &Db) -> Result<bool> {
    Ok(!renox::audit::for_subject(db, MARKER_SUBJECT, 1, 1)
        .await?
        .is_empty())
}

/// The state of a seed in progress.
struct Seed {
    state: AppState,
    driver: Driver,
    entity: i64,
    accountant: i64,
    hr: i64,
    md: i64,
    account_manager: i64,
    employees: Vec<EmployeeId>,
    parties: Vec<PartyId>,
}

// The plans of the story are rows of a table, written as tuples.
#[allow(clippy::type_complexity)]
impl Seed {
    fn new(state: AppState, driver: Driver) -> Self {
        Self {
            state,
            driver,
            entity: 0,
            accountant: 0,
            hr: 0,
            md: 0,
            account_manager: 0,
            employees: Vec::new(),
            parties: Vec::new(),
        }
    }

    fn db(&self) -> &Db {
        &self.state.db
    }

    async fn run(&mut self) -> Result {
        define(self.db()).await?;
        renox::auth::permissions::define_role(self.db(), CLIENT_APPROVER, &[]).await?;
        self.entity = wl_kernel::LegalEntity::default_entity(self.db()).await?.id;
        self.logins().await?;
        let clients = self.clients().await?;
        let contracts = self.contracts(&clients).await?;
        self.workers().await?;
        self.documents().await?;
        self.pay_accounts().await?;
        let sites = self.sites(&clients).await?;
        let requests = self.requests(&contracts, &sites).await?;
        let deployments = self.deployments(&contracts, &sites, &requests).await?;
        self.attendance(&deployments, &sites).await?;
        self.timesheets(&deployments).await?;
        self.payroll().await?;
        self.exits().await?;
        self.money(&clients, &contracts).await?;
        renox::audit::record(
            self.db(),
            renox::audit::Entry::new(DONE_ACTION).subject(MARKER_SUBJECT, 1),
        )
        .await?;
        Ok(())
    }

    /// The people with a login, one per role, and the client approver's
    /// portal grant is made with the clients.
    async fn logins(&mut self) -> Result {
        for login in LOGINS {
            let user = match User::find_by_email(self.db(), login.email).await? {
                Some(user) => user,
                None => {
                    let mut user =
                        User::register(self.db(), login.name, login.email, PASSWORD).await?;
                    user.email_verified_at = Some(renox::db::now());
                    user.save(self.db()).await?;
                    // The account manager and the supervisor hold their role
                    // for a client or a site only: scoped when the clients exist.
                    if !login.roles.contains(&ACCOUNT_MANAGER) && !login.roles.contains(&SUPERVISOR)
                    {
                        for role in login.roles {
                            user.assign_role(self.db(), role).await?;
                        }
                    }
                    user
                }
            };
            match login.email {
                "accountant@desertstar.example" => self.accountant = user.id,
                "hr@desertstar.example" => self.hr = user.id,
                "md@desertstar.example" => self.md = user.id,
                "accountmanager@desertstar.example" => self.account_manager = user.id,
                _ => {}
            }
            let db = self.state.db.clone();
            self.driver.sign_in(&db, login.email, &user).await?;
        }
        Ok(())
    }

    async fn user(&self, email: &str) -> Result<User> {
        Ok(User::find_by_email(self.db(), email)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("the demo login {email}"))?)
    }

    /// A company party with its billing e-mail and phone.
    async fn company(&self, name: &str, name_ar: &str, slug: &str) -> Result<PartyId> {
        let contact = |contact_type, value: String| ContactPoint {
            contact_type,
            value,
            is_primary: true,
            ..Default::default()
        };
        Ok(wl_parties::create(
            &self.state,
            NewParty {
                kind: PartyKind::Company,
                legal_name_en: name.to_owned(),
                legal_name_ar: Some(name_ar.to_owned()),
                company: Some(CompanyDetails::default()),
                default_language: Language::En,
                preferred_channel: Channel::Email,
                contacts: vec![
                    contact(ContactType::Email, format!("accounts@{slug}.example")),
                    contact(ContactType::Mobile, "+97144000000".to_owned()),
                ],
                source: "seed".to_owned(),
                ..Default::default()
            },
        )
        .await?
        .party_id())
    }

    /// A person party (a client's contact or a worker).
    async fn person(
        &self,
        name: &str,
        nationality: &str,
        email: &str,
        mobile: &str,
    ) -> Result<PartyId> {
        let contact = |contact_type, value: String| ContactPoint {
            contact_type,
            value,
            is_primary: true,
            ..Default::default()
        };
        Ok(wl_parties::create(
            &self.state,
            NewParty {
                kind: PartyKind::Person,
                legal_name_en: name.to_owned(),
                person: Some(PersonDetails {
                    nationality: Some(nationality.to_owned()),
                    ..Default::default()
                }),
                default_language: Language::En,
                preferred_channel: Channel::Whatsapp,
                contacts: vec![
                    contact(ContactType::Email, email.to_owned()),
                    contact(ContactType::Mobile, mobile.to_owned()),
                ],
                source: "seed".to_owned(),
                ..Default::default()
            },
        )
        .await?
        .party_id())
    }

    /// The three client companies, their contacts, and the client approver's
    /// portal grant.
    async fn clients(&mut self) -> Result<Vec<PartyId>> {
        let tower = self
            .company(
                "Gulf Tower Facilities LLC",
                "شركة جلف تاور لإدارة المرافق",
                "gulftower",
            )
            .await?;
        let marina = self
            .company(
                "Marina Bay Hospitality LLC",
                "شركة مارينا باي للضيافة",
                "marinabay",
            )
            .await?;
        let quoz = self
            .company(
                "Al Quoz Logistics Park LLC",
                "شركة القوز لمجمعات الخدمات اللوجستية",
                "alquozlogistics",
            )
            .await?;
        // The contacts: the client approver, a finance contact for the
        // escalation of a timesheet, and the managers of the other two.
        let carl = self
            .person(
                "Carl Whitfield",
                "GB",
                "approver@gulftower.example",
                "+971500000101",
            )
            .await?;
        let fiona = self
            .person(
                "Fiona Mathew",
                "IN",
                "finance@gulftower.example",
                "+971500000102",
            )
            .await?;
        let layla = self
            .person(
                "Layla Rashed",
                "AE",
                "ops@marinabay.example",
                "+971500000103",
            )
            .await?;
        let tariq = self
            .person(
                "Tariq Aziz",
                "PK",
                "manager@alquozlogistics.example",
                "+971500000104",
            )
            .await?;
        let user = self.user("approver@gulftower.example").await?;
        let grant = AccessGrant {
            user_id: Some(user.id),
            person_party_id: carl.0,
            party_id: tower.0,
            audience: "tenant".into(),
            roles: "approver".into(),
            source: "invitation".into(),
            valid_from: day(-60),
            status: GrantStatus::Active,
            ..AccessGrant::default()
        };
        fixture!(self.db(), grant, GrantStatus::Active);
        self.parties = vec![tower, marina, quoz, carl, fiona, layla, tariq];
        Ok(vec![tower, marina, quoz])
    }

    /// The three contracts with their rate cards, and the scoped roles of the
    /// account manager.
    async fn contracts(&mut self, clients: &[PartyId]) -> Result<Vec<ClientContract>> {
        let manager = self.user("accountmanager@desertstar.example").await?;
        // (reference, start, end, payment terms, deemed days, escalation
        // contact, rates for the roles, the next version)
        let plan: [(&str, i64, i64, Option<i64>, Option<i64>); 3] = [
            ("CC-DEMO-001", -200, 165, Some(10), Some(self.parties[4].0)),
            ("CC-DEMO-002", -320, 45, None, None),
            ("CC-DEMO-003", -120, 245, Some(14), Some(self.parties[6].0)),
        ];
        let mut contracts = Vec::new();
        for (index, (reference, start, end, deemed, contact)) in plan.into_iter().enumerate() {
            let client = clients[index];
            let contract = ClientContract {
                reference: reference.into(),
                client_party_id: client.0,
                start_date: day(start),
                end_date: day(end),
                payment_terms_days: 30,
                notice_period_days: 30,
                cover_required: index == 0,
                deemed_approval_days: deemed,
                escalation_contact_party_id: contact,
                account_manager_user_id: Some(manager.id),
                signed_on: Some(day(start - 10)),
                signed_document: Some(format!("{reference}-signed.pdf")),
                activated_on: Some(day(start)),
                status: ContractStatus::Active,
                ..ClientContract::default()
            };
            let contract = fixture!(self.db(), contract, ContractStatus::Active);
            manager
                .assign_role_in(self.db(), ACCOUNT_MANAGER, &client_scope(client.0))
                .await?;
            self.rate_card(client, &contract, start, index == 0).await?;
            contracts.push(contract);
        }
        Ok(contracts)
    }

    /// The accepted rate card version of a contract (and a pending next one).
    async fn rate_card(
        &self,
        client: PartyId,
        contract: &ClientContract,
        start: i64,
        next: bool,
    ) -> Result {
        let card = rate_cards::create_card(self.db(), None, client, &contract.subject()).await?;
        let lines = |factor: i64| {
            vec![
                LineInput::new("security_guard", "day", Unit::Hour, 2_800 * factor / 100)
                    .overtime(125),
                LineInput::new("security_guard", "night", Unit::Hour, 3_200 * factor / 100)
                    .overtime(125),
                LineInput::new("cleaner", "day", Unit::Hour, 1_900 * factor / 100).overtime(125),
                LineInput::new("driver", "day", Unit::Hour, 3_400 * factor / 100).overtime(125),
                LineInput::new("labourer", "day", Unit::Hour, 1_700 * factor / 100).overtime(125),
            ]
        };
        let accepted = RateCardVersion {
            rate_card_id: card.id,
            version: 1,
            effective_from: day(start),
            submitted_on: day(start - 12),
            status: VersionStatus::Accepted,
            accepted_on: Some(day(start - 5)),
            accepted_document: Some("rate-card-v1-signed.pdf".into()),
            ..RateCardVersion::default()
        };
        let accepted = fixture!(self.db(), accepted, VersionStatus::Accepted);
        for line in lines(100) {
            self.line(accepted.id, line).await?;
        }
        if next {
            let pending = rate_cards::new_version(self.db(), None, card.id, day(30), false).await?;
            for line in lines(105) {
                rate_cards::add_line(self.db(), pending.id, line).await?;
            }
        }
        let _: Option<RateCard> = None;
        Ok(())
    }

    async fn line(&self, version_id: i64, input: LineInput) -> Result {
        RateCardLine::create(
            self.db(),
            RateCardLine {
                version_id,
                role: input.role,
                shift_type: input.shift_type,
                unit: input.unit,
                rate: input.rate,
                ot_multiplier: input.ot_multiplier,
                holiday_multiplier: input.holiday_multiplier,
                min_hours_milli: input.min_hours_milli,
                ..RateCardLine::default()
            },
        )
        .await?;
        Ok(())
    }

    /// The workers: a party, an employee and a salary structure each.
    async fn workers(&mut self) -> Result {
        for (index, worker) in WORKERS.iter().enumerate() {
            let mobile = format!("+97150000{:04}", 2000 + index);
            let email = format!("{}@workers.desertstar.example", worker.key);
            let party = self
                .person(worker.name, worker.nationality, &email, &mobile)
                .await?;
            let status = match worker.status {
                EmployeeStatus::Onboarding => EmployeeStatus::Onboarding,
                _ => EmployeeStatus::Active,
            };
            let mut new = NewEmployee::new(party, worker.role, day(worker.hired)).status(status);
            new.employee_number = format!("DSM-{:04}", index + 1);
            new.legal_entity_id = Some(self.entity);
            let title = worker.role.replace('_', " ");
            new.job_title = title[..1].to_uppercase() + &title[1..];
            let employee = employees::create(&self.state, new).await?;
            if worker.status == EmployeeStatus::Notice {
                // Serving notice: no path from `active` without an exit
                // request, which the exit page makes; the fixture is
                // written directly.
                wl_hr::Employee::query()
                    .where_(wl_hr::Employee::ID.eq(employee.id))
                    .update(self.db(), &[("status", &"notice")])
                    .await?;
            }
            SalaryStructure::create(
                self.db(),
                SalaryStructure {
                    employee_id: employee.id,
                    effective_from: day(worker.hired),
                    basic: worker.basic,
                    housing: worker.basic / 4,
                    transport: 30_000,
                    hourly_rate: Some(worker.basic / 240),
                    ..SalaryStructure::default()
                },
            )
            .await?;
            self.employees.push(EmployeeId(employee.id));
        }
        Ok(())
    }

    /// The documents of the workers: valid for a year, but the permit of Ravi
    /// expires in 20 days, the visa of Faisal expired 10 days ago and the
    /// permit of Tesfaye 3 days ago.
    async fn documents(&self) -> Result {
        for (index, worker) in WORKERS.iter().enumerate() {
            if worker.status == EmployeeStatus::Onboarding {
                continue;
            }
            let who = self.employees[index];
            let mut kinds = vec![
                "residence_visa",
                "work_permit",
                "emirates_id",
                "health_insurance",
                "site_pass",
                // The health card guards every deployment (`MPW.ACL.07`).
                "health_card",
            ];
            if worker.role.starts_with("security") {
                kinds.push("sira_licence");
            }
            for kind in kinds {
                let until = match (index, kind) {
                    (W_RAVI, "work_permit") => 20,
                    (W_FAISAL, "residence_visa") => -10,
                    (W_TESFAYE, "work_permit") => -3,
                    _ => 300 - (index as i64) * 7,
                };
                self.document(who, kind, day(until), index).await?;
            }
        }
        Ok(())
    }

    async fn document(&self, who: EmployeeId, kind: &str, until: NaiveDate, n: usize) -> Result {
        let Some(row) = DocumentKind::by_key(self.db(), kind).await? else {
            return Ok(());
        };
        Document::create(
            self.db(),
            Document {
                subject_type: "hr.employee".into(),
                subject_id: who.0,
                kind_id: row.id,
                title: row.name_en.clone(),
                number: Some(format!(
                    "{}-{:06}",
                    kind.to_uppercase().replace('_', ""),
                    700_000 + n * 17
                )),
                issue_date: until.checked_sub_days(Days::new(730)),
                expiry_date: Some(until),
                issuing_authority: Some("GDRFA Dubai".into()),
                current_version: 1,
                status: DocumentStatus::Accepted,
                channel: "back_office".into(),
                ..Document::default()
            },
        )
        .await?;
        Ok(())
    }

    /// Pay accounts: active for the workers on payroll, one waiting for a
    /// second user (Anil), one cooling (Ravi).
    async fn pay_accounts(&self) -> Result {
        for (index, worker) in WORKERS.iter().enumerate() {
            // Noel has not started: no account yet. Dawit, who is leaving, is
            // still paid through WPS until his last day.
            if index == W_NOEL {
                continue;
            }
            let (status, requested_by, approved_by) = match index {
                W_ANIL => (PayAccountStatus::PendingApproval, Some(self.hr), None),
                W_RAVI => (PayAccountStatus::Cooling, Some(self.hr), Some(self.md)),
                _ => (PayAccountStatus::Active, Some(self.hr), Some(self.md)),
            };
            let number = format!(
                "AE07033{:016}",
                1_234_567_890_123_456_u64 + index as u64 * 7919
            );
            let account = PayAccount {
                employee_id: self.employees[index].0,
                channel: "bank_account".into(),
                routing_code: Some("033100101".into()),
                last4: number[number.len() - 4..].to_owned(),
                account_number: number,
                holder_name: worker.name.into(),
                requested_by,
                approved_by,
                approved_on: approved_by.map(|_| day(-30)),
                cooling_days: 5,
                // Ravi's account is cooling: it may be used from in 3 days.
                cooling_ends_on: (status == PayAccountStatus::Cooling).then(|| day(3)),
                status,
                ..PayAccount::default()
            };
            fixture!(self.db(), account, status);
        }
        Ok(())
    }

    /// The sites of the clients; the supervisor looks after the tower's.
    async fn sites(&self, clients: &[PartyId]) -> Result<Vec<Site>> {
        let spec = [
            (
                0,
                "Tower A, lobby and security desk",
                "Sheikh Zayed Road, Dubai",
            ),
            (
                0,
                "Tower B, podium and car park",
                "Sheikh Zayed Road, Dubai",
            ),
            (1, "Marina Bay Resort", "Dubai Marina, Dubai"),
            (2, "Warehouse complex 7", "Al Quoz Industrial 3, Dubai"),
        ];
        let supervisor = self.user("supervisor@desertstar.example").await?;
        let mut sites = Vec::new();
        for (client, name, address) in spec {
            let site = Site::create(
                self.db(),
                Site {
                    client_party_id: clients[client].0,
                    name: name.into(),
                    address: Some(address.into()),
                    active: true,
                    ..Site::default()
                },
            )
            .await?;
            if client == 0 {
                supervisor
                    .assign_role_in(self.db(), SUPERVISOR, &site_scope(site.id))
                    .await?;
            }
            sites.push(site);
        }
        Ok(sites)
    }

    /// Requests: a draft, an open, a partly filled and a filled one.
    async fn requests(
        &self,
        contracts: &[ClientContract],
        sites: &[Site],
    ) -> Result<Vec<ManpowerRequest>> {
        // (contract, site, role, shift, headcount, filled, start, end, status)
        let spec: [(usize, usize, &str, &str, i64, i64, i64, i64, RequestStatus); 5] = [
            (
                0,
                0,
                "security_guard",
                "night",
                2,
                0,
                14,
                165,
                RequestStatus::Draft,
            ),
            (1, 2, "cleaner", "day", 2, 0, 7, 45, RequestStatus::Open),
            (
                0,
                0,
                "security_guard",
                "day",
                3,
                2,
                -90,
                165,
                RequestStatus::PartiallyFilled,
            ),
            (
                1,
                2,
                "cleaner",
                "day",
                2,
                2,
                -100,
                45,
                RequestStatus::Filled,
            ),
            (2, 3, "labourer", "day", 3, 2, -60, 245, RequestStatus::Open),
        ];
        let mut requests = Vec::new();
        for (c, s, role, shift, headcount, filled, start, end, status) in spec {
            let request = ManpowerRequest {
                client_contract_id: contracts[c].id,
                client_party_id: contracts[c].client_party_id,
                site_id: sites[s].id,
                role: role.into(),
                headcount,
                filled_count: filled,
                shift_type: shift.into(),
                shift_start: Some(if shift == "night" { "19:00" } else { "07:00" }.into()),
                shift_end: Some(if shift == "night" { "07:00" } else { "15:00" }.into()),
                start_date: day(start),
                end_date: Some(day(end)),
                kind: RequestKind::default(),
                status,
                submitted_by: (status != RequestStatus::Draft).then_some(self.account_manager),
                ..ManpowerRequest::default()
            };
            requests.push(fixture!(self.db(), request, status));
        }
        Ok(requests)
    }

    /// Deployments: planned, active, suspended and ended.
    async fn deployments(
        &self,
        contracts: &[ClientContract],
        sites: &[Site],
        requests: &[ManpowerRequest],
    ) -> Result<Vec<Deployment>> {
        // (worker, contract, site, request, role, shift, start, end, status)
        let spec: [(
            usize,
            usize,
            usize,
            Option<usize>,
            &str,
            &str,
            i64,
            i64,
            DeploymentStatus,
        ); 10] = [
            (
                W_WASIM,
                0,
                0,
                Some(2),
                "security_guard",
                "day",
                -90,
                165,
                DeploymentStatus::Active,
            ),
            (
                W_RAVI,
                0,
                0,
                Some(2),
                "security_guard",
                "day",
                -60,
                165,
                DeploymentStatus::Active,
            ),
            (
                W_JOSEPH,
                0,
                1,
                None,
                "security_guard",
                "day",
                -40,
                165,
                DeploymentStatus::Active,
            ),
            (
                W_BILAL,
                1,
                2,
                Some(3),
                "cleaner",
                "day",
                -100,
                45,
                DeploymentStatus::Active,
            ),
            (
                W_MARIA,
                1,
                2,
                Some(3),
                "cleaner",
                "day",
                -100,
                45,
                DeploymentStatus::Active,
            ),
            (
                W_EKO,
                1,
                2,
                Some(1),
                "cleaner",
                "day",
                7,
                45,
                DeploymentStatus::Planned,
            ),
            (
                W_SURESH,
                2,
                3,
                None,
                "driver",
                "day",
                -30,
                245,
                DeploymentStatus::Active,
            ),
            (
                W_KAMAL,
                2,
                3,
                Some(4),
                "labourer",
                "day",
                -25,
                245,
                DeploymentStatus::Active,
            ),
            (
                W_TESFAYE,
                2,
                3,
                Some(4),
                "labourer",
                "day",
                -60,
                245,
                DeploymentStatus::Suspended,
            ),
            (
                W_HAMZA,
                0,
                0,
                None,
                "security_guard",
                "day",
                -200,
                -20,
                DeploymentStatus::Ended,
            ),
        ];
        let mut deployments = Vec::new();
        for (w, c, s, r, role, shift, start, end, status) in spec {
            let mut deployment = Deployment {
                employee_id: self.employees[w].0,
                client_contract_id: contracts[c].id,
                client_party_id: contracts[c].client_party_id,
                site_id: sites[s].id,
                request_id: r.map(|r| requests[r].id),
                role: role.into(),
                shift_type: shift.into(),
                shift_start: Some("07:00".into()),
                shift_end: Some("15:00".into()),
                start_date: day(start),
                end_date: Some(day(end)),
                kind: DeploymentKind::default(),
                status,
                created_by: Some(self.account_manager),
                ..Deployment::default()
            };
            match status {
                DeploymentStatus::Active | DeploymentStatus::Suspended => {
                    deployment.activated_on = Some(day(start));
                }
                DeploymentStatus::Ended => {
                    deployment.activated_on = Some(day(start));
                    deployment.ended_on = Some(day(end));
                    deployment.end_reason = Some("client_rejected".into());
                    deployment.end_note = Some("The client asked for another guard".into());
                }
                _ => {}
            }
            if status == DeploymentStatus::Suspended {
                deployment.suspended_on = Some(day(-3));
            }
            let deployment = fixture!(self.db(), deployment, status);
            if status == DeploymentStatus::Suspended {
                manpower::suspension::DeploymentSuspension::create(
                    self.db(),
                    manpower::suspension::DeploymentSuspension {
                        deployment_id: deployment.id,
                        reason: "work_permit".into(),
                        expired_on: Some(day(-3)),
                        added_on: day(-3),
                        ..Default::default()
                    },
                )
                .await?;
            }
            deployments.push(deployment);
        }
        Ok(deployments)
    }

    /// A work location per site, and the attendance of the workers on site:
    /// the supervisor marked them present on each of the last seven days
    /// (07:00 to 15:00), but Kamal, who has not been seen for nine days, which
    /// is what the absence rule (`MPW.RUL.09`) opens a case for.
    async fn attendance(&self, deployments: &[Deployment], sites: &[Site]) -> Result {
        let mut locations = Vec::new();
        for site in sites {
            locations.push(
                WorkLocation::create(
                    self.db(),
                    WorkLocation {
                        name: site.name.clone(),
                        scope_type: Some(manpower::permissions::SITE_SCOPE_TYPE.to_owned()),
                        scope_id: Some(site.id),
                        ..WorkLocation::default()
                    },
                )
                .await?,
            );
        }
        let at = |n: i64, hour: u32| day(n).and_hms_opt(hour, 0, 0).unwrap_or_default().and_utc();
        for deployment in deployments {
            if deployment.status != DeploymentStatus::Active {
                continue;
            }
            let Some(index) = sites.iter().position(|s| s.id == deployment.site_id) else {
                continue;
            };
            let kamal = deployment.employee_id == self.employees[W_KAMAL].0;
            let days: Vec<i64> = if kamal {
                (-20..=-9).collect()
            } else {
                (-6..=0).collect()
            };
            for n in days {
                let record = AttendanceRecord {
                    employee_id: deployment.employee_id,
                    location_id: locations[index].id,
                    date: day(n),
                    // 07:00 to 15:00 in Dubai, UTC+4.
                    check_in: at(n, 3),
                    check_out: Some(at(n, 11)),
                    recorded_by: Some(self.account_manager),
                    recorded_at: at(n, 11),
                    status: AttendanceStatus::Accepted,
                    ..AttendanceRecord::default()
                };
                fixture!(self.db(), record, AttendanceStatus::Accepted);
            }
        }
        // The absence rule runs at 02:00 in real life; the demo has it run
        // once, so the case of the worker who has not turned up is open.
        manpower::absence::run_on(&self.state, day(0)).await?;
        Ok(())
    }

    /// Timesheets: open, submitted to an external approver (the client's) and
    /// approved.
    async fn timesheets(&self, deployments: &[Deployment]) -> Result {
        let context = |d: &Deployment| Subject::new(manpower::deployment::SUBJECT_TYPE, d.id);
        let sheet = |d: &Deployment, from: i64, to: i64| {
            let mut sheet = Timesheet::regular(EmployeeId(d.employee_id), day(from), day(to))
                .in_context(&context(d));
            sheet.approver_kind = ApproverKind::External;
            sheet.approver_role = Some("client_approver".into());
            sheet
        };
        // Wasim: this week, still open.
        let open = sheet(&deployments[0], -6, 0);
        let open = fixture!(self.db(), open, TimesheetStatus::Open);
        self.hours(&open, &deployments[0], -6..=0, None).await?;
        // Ravi and Bilal: submitted two and four days ago, waiting for the
        // client approver.
        for (index, ago) in [(1, 2), (3, 4)] {
            let mut waiting = sheet(&deployments[index], -6, 0);
            waiting.status = TimesheetStatus::Submitted;
            waiting.submitted_at = Some(renox::db::now() - renox::chrono::Duration::days(ago));
            let waiting = fixture!(self.db(), waiting, TimesheetStatus::Submitted);
            // Ravi worked two hours of overtime on the third day.
            let overtime = (index == 1).then_some(-4);
            self.hours(&waiting, &deployments[index], -6..=0, overtime)
                .await?;
        }
        // Joseph: last week, approved by the client.
        let mut approved = sheet(&deployments[2], -13, -7);
        approved.status = TimesheetStatus::Approved;
        approved.approval_basis = Some(ApprovalBasis::Approved);
        approved.submitted_at = Some(renox::db::now() - renox::chrono::Duration::days(6));
        let approved = fixture!(self.db(), approved, TimesheetStatus::Approved);
        self.hours(&approved, &deployments[2], -13..=-7, None)
            .await?;
        Ok(())
    }

    /// The hours of `sheet`: eight regular hours on each day of `days`
    /// against the deployment, and two hours of overtime on `overtime`.
    async fn hours(
        &self,
        sheet: &Timesheet,
        deployment: &Deployment,
        days: std::ops::RangeInclusive<i64>,
        overtime: Option<i64>,
    ) -> Result {
        let cost = Subject::new(manpower::deployment::SUBJECT_TYPE, deployment.id);
        for n in days {
            let extra = overtime == Some(n);
            wl_attendance::TimesheetEntry::create(
                self.db(),
                wl_attendance::TimesheetEntry {
                    timesheet_id: sheet.id,
                    employee_id: sheet.owner_employee_id,
                    date: day(n),
                    cost_object_type: Some(cost.subject_type.clone()),
                    cost_object_id: Some(cost.subject_id),
                    normal_minutes: 480,
                    overtime_minutes: if extra { 120 } else { 0 },
                    billable: true,
                    ..wl_attendance::TimesheetEntry::default()
                },
            )
            .await?;
        }
        Ok(())
    }

    /// The payroll run of this month, in draft, with a line for each worker on
    /// payroll and one held.
    async fn payroll(&self) -> Result {
        let period = today().format("%Y-%m").to_string();
        let run = PayrollRun {
            legal_entity_id: Some(self.entity),
            period,
            run_type: "regular".into(),
            status: PayrollRunStatus::Draft,
            prepared_by: Some(self.hr),
            ..PayrollRun::default()
        };
        let run = fixture!(self.db(), run, PayrollRunStatus::Draft);
        for (index, worker) in WORKERS.iter().enumerate() {
            if matches!(index, W_NOEL | W_DAWIT | W_HAMZA | W_FAISAL | W_ANIL) {
                continue;
            }
            let fixed = worker.basic + worker.basic / 4 + 30_000;
            let held = index == W_TESFAYE;
            PayrollLine::create(
                self.db(),
                PayrollLine {
                    run_id: run.id,
                    employee_id: self.employees[index].0,
                    fixed_wage: fixed,
                    net_amount: if held { 0 } else { fixed },
                    status: if held {
                        LineStatus::Held
                    } else {
                        LineStatus::Included
                    },
                    held_reason: held
                        .then(|| "Work permit expired, held until it is renewed".to_owned()),
                    ..PayrollLine::default()
                },
            )
            .await?;
        }
        Ok(())
    }

    /// Dawit's final settlement (a draft) and Noel's visa case.
    async fn exits(&self) -> Result {
        let dawit = self.employees[W_DAWIT].0;
        let settlement = FinalSettlement {
            employee_id: dawit,
            reason: "resignation".into(),
            payee: "employee".into(),
            last_working_day: Some(day(10)),
            unpaid_salary: 128_000,
            leave_encashment: 42_000,
            gratuity: 215_000,
            net: 385_000,
            statement_version: 1,
            prepared_by: Some(self.hr),
            status: SettlementStatus::Draft,
            ..FinalSettlement::default()
        };
        fixture!(self.db(), settlement, SettlementStatus::Draft);
        // The entry permit was done three days ago: the case is in progress.
        let case = VisaCase {
            employee_id: self.employees[W_NOEL].0,
            case_type: wl_hr::visa::TYPE_NEW.to_owned(),
            status: VisaCaseStatus::InProgress,
            ..VisaCase::default()
        };
        let case = fixture!(self.db(), case, VisaCaseStatus::InProgress);
        VisaCaseStep::create(
            self.db(),
            VisaCaseStep {
                case_id: case.id,
                step_type: "entry_permit".to_owned(),
                done_on: Some(day(-3)),
                ..VisaCaseStep::default()
            },
        )
        .await?;
        Ok(())
    }

    /// The overdue invoice of Al Quoz (issued by the accountant through the
    /// router) and its post-dated cheque.
    async fn money(&self, clients: &[PartyId], contracts: &[ClientContract]) -> Result {
        let quoz = clients[2];
        let contract = &contracts[2];
        let draft = InvoiceDraft::new(quoz, contract.subject())
            .line(
                DraftLine::new("Labour supply, September", 4_200_000, VatCategory::Exempt)
                    .origin(contract.subject()),
            )
            .due_date(day(-31));
        let id = invoicer(&self.state)
            .draft(self.db(), draft)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("invoicing is off"))?;
        self.driver
            .post(
                "accountant@desertstar.example",
                &format!("/invoices/{}/issue", id.0),
                &[],
            )
            .await?;
        wl_invoicing::overdue::run(&self.state).await?;
        // A timesheet an invoice used: locked, with the invoice it was billed
        // on, and a worker's hours that a correction would have to reverse.
        let billed_on = Deployment::query()
            .where_(Deployment::CLIENT_CONTRACT_ID.eq(contract.id))
            .order_by(Deployment::ID)
            .first(self.db())
            .await?;
        if let Some(deployment) = billed_on {
            let context = Subject::new(manpower::deployment::SUBJECT_TYPE, deployment.id);
            let mut billed =
                Timesheet::regular(EmployeeId(deployment.employee_id), day(-40), day(-34))
                    .in_context(&context);
            billed.approver_kind = ApproverKind::External;
            billed.approver_role = Some("client_approver".into());
            billed.status = TimesheetStatus::Locked;
            billed.approval_basis = Some(ApprovalBasis::Approved);
            billed.submitted_at = Some(renox::db::now() - renox::chrono::Duration::days(33));
            billed.approved_at = Some(renox::db::now() - renox::chrono::Duration::days(31));
            billed.billed_by = Some(format!("invoicing.invoice:{}", id.0));
            let billed = fixture!(self.db(), billed, TimesheetStatus::Locked);
            self.hours(&billed, &deployment, -40..=-34, None).await?;
        }
        cheques(&self.state)
            .register(
                &self.state,
                RegisterCheque::new(
                    quoz,
                    self.entity,
                    "Emirates NBD",
                    "CHQ-DEMO-1042",
                    day(5),
                    1_500_000,
                ),
                Some(self.accountant),
            )
            .await?;
        Ok(())
    }
}
