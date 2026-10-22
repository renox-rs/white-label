//! The demo clinic of the Clinics client (DEMO-DATA.md, 05; *Palm Grove
//! Clinic LLC*, two branches in Dubai, all fictional): `clinics:demo` fills a
//! local database so that every screen of F5 has something to show the moment
//! someone logs in, and `db:seed` runs it in `local`, never in production.
//!
//! It follows the Manpower demo (`super::manpower_demo`):
//!
//! - **Through the real services.** Bookings are made, confirmed, checked in,
//!   started, completed, cancelled and marked no-show by wl-scheduling's own
//!   service; the clinic's appointment and visit follow from its events. What
//!   a person does at a screen (record the eligibility, choose the coverage
//!   outcome, capture charges, bill the visit, submit a claim, issue a payer
//!   bill, record a remittance) is a request to the app's own router as the
//!   demo user who does it ([`Driver`]), so the permissions, the guards and
//!   the state machines are the real ones and a refusal stops the seed with
//!   the page's message. A record in a state the machines only reach after
//!   days of real life is inserted in that state with `insert_in_state`, and
//!   the place says why.
//! - **Relative dates.** Every date is `day(n)`, today plus or minus days
//!   (Asia/Dubai), so the screens look alive whenever the seed runs: a
//!   practitioner licence that expires in 20 days, a package that ends this
//!   week, an authorisation that is still pending. A step of the past runs on
//!   the seed's [`Clock`] set to its day.
//! - **Money in fils.**
//! - **Idempotent.** The seed is done once its marker exists (an audit entry
//!   of the subject `clinics_demo`); a login that exists is kept.
//!
//! # The clinic
//!
//! Two branches (`clinics.branch` 1 *Jumeirah*, 2 *Business Bay*), three
//! practitioners, twelve patients (one blocked, one a child with a guardian),
//! three payers with their contracts, and the stories on the screens:
//!
//! | Screen | What it shows |
//! |---|---|
//! | Appointments | every state of a booking, a conflict, a waiting-list entry, a deposit paid and refunded |
//! | Visits | an insured visit divided by co-pay, a self-pay one, a billed one |
//! | Policies, authorisations, contracts | several payers, one expired policy, one approved and one pending authorisation |
//! | Claims | draft, submitted, paid, denied and written off |
//! | Recalls | a recalled lot traced to patients, with its notice |
//! | Loyalty, counter sales, packages | members with points, a package sold, a sale and its return |
//! | Invoices, payments, refunds | cash and card, a payment link, a refund, a chargeback |
//! | HR | a pending leave, a sick leave without its note, a licence that expires in 20 days |
//!
//! Every login has the password of [`super::seed::PASSWORD`].

mod claims;
mod desk;
mod insurance;
mod money;
mod notes;
mod people;
mod staffing;
mod stock;
mod trade;

use renox::Environment;
use renox::prelude::*;
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{PartyId, Subject};

use super::demo_data::clock::Clock;
use super::demo_data::driver::Driver;
use super::roles::{
    ACCOUNTANT, ADMIN, BILLING_COORDINATOR, CALL_CENTRE_AGENT, CLINIC_MANAGER, MARKETING_MANAGER,
    NURSE, OWNER, PRACTITIONER, RECEPTIONIST, STOREKEEPER,
};
use super::seed::PASSWORD;

/// The command that fills a local database with the demo clinic.
pub const COMMAND: &str = "clinics:demo";

/// The audit subject of the marker of a finished seed.
const MARKER_SUBJECT: &str = "clinics_demo";
const DONE_ACTION: &str = "clinics_demo.seeded";

/// The type of the branch subjects (`clinics.branch`): a booking lies in the
/// scope of its location, and an employee works in the scope of a branch.
pub const BRANCH: &str = "clinics.branch";
/// Jumeirah, the first branch.
pub const JUMEIRAH: i64 = 1;
/// Business Bay, the second branch.
pub const BUSINESS_BAY: i64 = 2;

/// One demo login.
#[derive(Debug, Clone, Copy)]
pub struct Login {
    /// The name shown on the login page.
    pub name: &'static str,
    /// The sign-in e-mail.
    pub email: &'static str,
    /// The role.
    pub role: &'static str,
    /// The branch the person works in (their employee scope); `None` for all of them.
    pub branch: Option<i64>,
    /// What it is for, shown beside the name.
    pub label: &'static str,
}

/// The demo people, one per role (and one per branch where the role is
/// held in a branch).
pub const LOGINS: &[Login] = &[
    Login {
        name: "Layla Al Mansoori",
        email: "admin@palmgrove.example",
        role: ADMIN,
        branch: None,
        label: "Clinic administrator (company admin)",
    },
    Login {
        name: "Dr Rashid Al Falasi",
        email: "owner@palmgrove.example",
        role: OWNER,
        branch: None,
        label: "Owner and medical director",
    },
    Login {
        name: "Mariam Haddad",
        email: "manager.jumeirah@palmgrove.example",
        role: CLINIC_MANAGER,
        branch: Some(JUMEIRAH),
        label: "Clinic manager, Jumeirah",
    },
    Login {
        name: "Tariq Nasser",
        email: "manager.bay@palmgrove.example",
        role: CLINIC_MANAGER,
        branch: Some(BUSINESS_BAY),
        label: "Clinic manager, Business Bay",
    },
    Login {
        name: "Sara Ibrahim",
        email: "reception.jumeirah@palmgrove.example",
        role: RECEPTIONIST,
        branch: Some(JUMEIRAH),
        label: "Receptionist, Jumeirah",
    },
    Login {
        name: "Noura Khan",
        email: "reception.bay@palmgrove.example",
        role: RECEPTIONIST,
        branch: Some(BUSINESS_BAY),
        label: "Receptionist, Business Bay",
    },
    Login {
        name: "Dr Nadia Rahman",
        email: "nadia@palmgrove.example",
        role: PRACTITIONER,
        branch: Some(JUMEIRAH),
        label: "Practitioner (family medicine), Jumeirah",
    },
    Login {
        name: "Dr Samir Joshi",
        email: "samir@palmgrove.example",
        role: PRACTITIONER,
        branch: Some(BUSINESS_BAY),
        label: "Practitioner (dentist), Business Bay",
    },
    Login {
        name: "Dr Yara Kassab",
        email: "yara@palmgrove.example",
        role: PRACTITIONER,
        branch: Some(JUMEIRAH),
        label: "Practitioner (physiotherapy), licence expiring",
    },
    Login {
        name: "Grace Mensah",
        email: "nurse@palmgrove.example",
        role: NURSE,
        branch: Some(JUMEIRAH),
        label: "Nurse, Jumeirah",
    },
    Login {
        name: "Khaled Ali",
        email: "store@palmgrove.example",
        role: STOREKEEPER,
        branch: Some(JUMEIRAH),
        label: "Storekeeper, Jumeirah",
    },
    Login {
        name: "Omar Siddiqui",
        email: "billing@palmgrove.example",
        role: BILLING_COORDINATOR,
        branch: None,
        label: "Billing coordinator (insurance)",
    },
    Login {
        name: "Hind Al Shamsi",
        email: "callcentre@palmgrove.example",
        role: CALL_CENTRE_AGENT,
        branch: None,
        label: "Call-centre agent",
    },
    Login {
        name: "Faisal Gupta",
        email: "accountant@palmgrove.example",
        role: ACCOUNTANT,
        branch: None,
        label: "Accountant",
    },
    Login {
        name: "Dina Karam",
        email: "marketing@palmgrove.example",
        role: MARKETING_MANAGER,
        branch: None,
        label: "Marketing manager",
    },
];

/// The subject of a branch.
fn branch(id: i64) -> Subject {
    Subject::new(BRANCH, id)
}

/// The module that adds the command and the seeder.
pub struct ClinicsDemo {
    client: wl_kernel::ClientConfig,
}

impl ClinicsDemo {
    /// The module for the client `config`.
    pub fn new(config: wl_kernel::ClientConfig) -> Self {
        Self { client: config }
    }
}

impl Module for ClinicsDemo {
    fn name(&self) -> &'static str {
        "clinics_demo"
    }

    fn register(&self, app: &mut Registry) {
        let client = self.client.clone();
        app.command(
            COMMAND,
            "Fill the database with the Palm Grove demo clinic (staff, patients, bookings, visits, insurance, claims, stock, sales, payments); local and testing only",
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

/// `clinics:demo` and the seeder: boots the app a second time on the same
/// database to get its router (the demo users do their work through it), then
/// seeds. The command works in `local` and `testing`; the seeder of
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
                "the demo clinic is made in local or testing only, never in {env:?}"
            )
            .into());
        }
        return Ok(());
    }
    if state.config.database_url.contains(":memory:") {
        if said {
            return Err(renox::anyhow::anyhow!(
                "the demo clinic needs a database file, not an in-memory database"
            )
            .into());
        }
        return Ok(());
    }
    let app = crate::app_for(client).map_err(|e| renox::anyhow::anyhow!("{e}"))?;
    let kernel = app.config((*state.config).clone()).boot().await?;
    if !seed(&kernel).await? {
        println!("The demo clinic is there already.");
        return Ok(());
    }
    println!("The demo clinic is made. Log in with any of these (password {PASSWORD}):");
    for login in LOGINS {
        println!("  {:<42} {}", login.email, login.label);
    }
    Ok(())
}

/// Seeds the demo clinic into the database `kernel` is on; `false` when it is
/// there already and nothing was made. (The app must have been installed and
/// seeded: the legal entity of `client.toml` and the service types exist.)
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
    let clock = Clock::new().await;
    Box::pin(Seed::new(state, driver, clock).run()).await?;
    Ok(true)
}

/// The demo is there once its last step recorded that it is done.
pub async fn is_seeded(db: &Db) -> Result<bool> {
    Ok(!renox::audit::for_subject(db, MARKER_SUBJECT, 1, 1)
        .await?
        .is_empty())
}

/// The state of a seed in progress: what the later steps need of the earlier.
pub(crate) struct Seed {
    pub(crate) state: AppState,
    pub(crate) driver: Driver,
    pub(crate) clock: Clock,
    /// The legal entity of the clinic.
    pub(crate) entity: i64,
    /// The demo users by e-mail.
    pub(crate) users: Vec<(&'static str, User)>,
    /// The patients, in the order of [`people::PATIENTS`].
    pub(crate) patients: Vec<PartyId>,
    /// The three payers.
    pub(crate) payers: Vec<PartyId>,
    /// The practitioners' employees, in the order of [`people::DOCTORS`].
    pub(crate) doctors: Vec<EmployeeId>,
    /// Every employee by the e-mail of the login behind it.
    pub(crate) employees: Vec<(&'static str, EmployeeId)>,
    /// The resources and service types.
    pub(crate) kit: desk::Kit,
    /// The policy of a patient (an index of [`people::PATIENTS`]).
    pub(crate) policies: Vec<(usize, i64)>,
    /// The contract of a payer (an index of [`people::PAYERS`]).
    pub(crate) contracts: Vec<i64>,
    /// The stores of the branches and the items, in the order of their creation.
    pub(crate) stores: Vec<i64>,
    pub(crate) items: Vec<i64>,
    /// The recall of the lot.
    pub(crate) recall_id: i64,
    /// The appointments made: patient, booking and how it ended.
    pub(crate) bookings: Vec<(usize, i64, desk::End)>,
}

impl Seed {
    fn new(state: AppState, driver: Driver, clock: Clock) -> Self {
        Self {
            state,
            driver,
            clock,
            entity: 0,
            users: Vec::new(),
            patients: Vec::new(),
            payers: Vec::new(),
            doctors: Vec::new(),
            employees: Vec::new(),
            kit: desk::Kit::default(),
            policies: Vec::new(),
            contracts: Vec::new(),
            bookings: Vec::new(),
            stores: Vec::new(),
            items: Vec::new(),
            recall_id: 0,
        }
    }

    pub(crate) fn db(&self) -> &Db {
        &self.state.db
    }

    /// The demo user with this e-mail.
    pub(crate) fn user(&self, email: &str) -> &User {
        self.users
            .iter()
            .find(|(known, _)| *known == email)
            .map(|(_, user)| user)
            .unwrap_or_else(|| panic!("the demo login {email}"))
    }

    async fn run(&mut self) -> Result {
        // The seed's dates are relative to the real today, also inside a step
        // that runs on the clock of a day in the past.
        *super::demo_data::REAL_TODAY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(wl_kernel::documents::today());
        let done = self.story().await;
        *super::demo_data::REAL_TODAY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        done
    }

    async fn story(&mut self) -> Result {
        super::roles::define(self.db()).await?;
        self.entity = wl_kernel::LegalEntity::default_entity(self.db()).await?.id;
        self.logins().await?;
        self.patients().await?;
        self.staff().await?;
        self.desk_setup().await?;
        self.insurance().await?;
        self.stock().await?;
        self.desk().await?;
        self.staffing().await?;
        self.authorisations().await?;
        self.claims().await?;
        self.recall().await?;
        self.trade().await?;
        self.money().await?;
        self.notes().await?;
        renox::audit::record(
            self.db(),
            renox::audit::Entry::new(DONE_ACTION).subject(MARKER_SUBJECT, 1),
        )
        .await?;
        Ok(())
    }
}
