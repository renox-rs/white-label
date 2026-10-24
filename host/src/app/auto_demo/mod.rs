//! The demo company of the Auto client (DEMO-DATA.md, 08; *Al Noor Auto Parts
//! & Garage LLC*, all fictional): `auto:demo` fills a local database so that
//! every screen of the app has something to show the moment someone logs in,
//! and `db:seed` runs it in `local`, never in production.
//!
//! It follows the EdTech and Clinics demos (`super::edtech_demo`):
//!
//! - **Through the real services.** What a person does at a screen (check a
//!   vehicle in, inspect, draft the estimate, send it, approve it on the link,
//!   issue and fit a part, finish the work, pass the QC, deliver, cancel,
//!   decline, void, register a claim, ask the insurer, raise a supplier
//!   warranty claim, open a recall) is a call of the app's own service as the
//!   demo user who does it (the services take the signed-in user; the one
//!   route of [`hooks`] hands the seed that user), so the permissions, the
//!   guards and the state machines are the real ones and a refusal stops the
//!   seed with its message. Pages that exist (purchase orders, goods receipts,
//!   sales orders, counter sales, loyalty enrolment, recalls, cash handovers)
//!   are requests to the app's own router ([`Driver`]). Money goes through
//!   wl-invoicing (the invoice raised on delivery, the claim's payer bill, the
//!   fleet's consolidated invoice) and wl-payments (the payment recorder).
//!   A record in a state the machines only reach after days of real life is
//!   inserted in that state with `insert_in_state`, and the place says why:
//!   the draft purchase orders (no screen writes one yet).
//! - **Relative dates, whatever the weekday and the hour.** Every date is
//!   `day(n)`, today plus or minus days (Asia/Dubai); a step of the past runs
//!   on the seed's [`Clock`] set to 10:00 of its day, so the numbers, the
//!   invoices and the audit trail carry the right days, and no step runs
//!   today: a step set to a moment in the future would run at the real time,
//!   which can fall in the quiet hours of the notifier (21:00-08:00) or before
//!   the opening of a send window. The workshop's bay is open all week, so a
//!   booking does not depend on the weekday either.
//! - **Money in fils**, VAT 5 % on top of the prices.
//! - **Deterministic**: names, amounts and days are written out below.
//! - **Idempotent.** The seed is done once its marker exists (an audit entry
//!   of the subject `auto_demo`); a login that exists is kept.
//!
//! # The company
//!
//! One auto parts trader on Naif Road, Deira, with a workshop and a second
//! parts counter in Sharjah, a service advisor, a workshop manager, two
//! technicians, a storekeeper, a purchaser, an insurance coordinator, a fleet
//! account manager, an accountant, the branch manager and the owner. The
//! stories on the screens:
//!
//! | Screen | What it shows |
//! |---|---|
//! | Vehicles | private cars with their plates, VIN and odometer, and the vans of the fleets |
//! | Job cards | checked in (from a booking), awaiting approval (the customer's link), in progress, awaiting parts, quality check, ready, delivered, cancelled (the work done billed), declined, voided |
//! | Not collected | a vehicle left after a cancel and one after a decline: reminders at 3 and 7 days, the storage fee, the branch manager's task at 30 days |
//! | Insurers, claims | two insurers; claims registered, surveyed, estimate sent, approved, supplementary pending, rejected, withdrawn, total loss, invoiced, settled and voided |
//! | Fleet contracts | active (a service booked, a job card with the fleet's PO, the monthly invoice), suspended, ended |
//! | Supplier claims | raised, raised and accepted by the supplier, recovered, one refused with a write-off waiting for the branch manager |
//! | Recalls | brake pads from one supplier traced to a car owner, a fleet, a workshop and the walk-in sales |
//! | Back-orders | B2B sales orders with part reserved and part back-ordered, one served by a receipt, the reorder suggestion |
//! | Export sales | an invoice without exit evidence told at 60 days, one flagged at 90, one with a bill of lading |
//! | Incidents, police reports | a vehicle incident decided by the branch manager, accident job cards with the report waiting and accepted |
//! | Loyalty, marketing | members who earn points on a paid invoice, a fleet that earns none, the marketing consent that is not implied |
//! | Cash | cheques collected in the field, one missing at the count |
//!
//! Every login has the password of [`super::seed::PASSWORD`].

mod cash;
mod claims;
mod fleet;
mod hooks;
mod jobs;
mod money;
mod people;
mod shop;
mod stories;
mod supply;
mod trade;

use std::collections::BTreeMap;
use std::sync::Arc;

use renox::Environment;
use renox::prelude::*;
use wl_kernel::PartyId;

use super::demo_data::clock::Clock;
use super::demo_data::driver::Driver;
use super::roles::{
    ACCOUNTANT, ADMIN, BRANCH_MANAGER, COUNTER_SALES, FLEET_ACCOUNT_MANAGER, INSURANCE_COORDINATOR,
    OWNER, PROCUREMENT, SERVICE_ADVISOR, STOREKEEPER, TECHNICIAN, WORKSHOP_MANAGER,
};
use super::seed::PASSWORD;

/// The command that fills a local database with the demo company.
pub const COMMAND: &str = "auto:demo";

/// The audit subject of the marker of a finished seed.
const MARKER_SUBJECT: &str = "auto_demo";
const DONE_ACTION: &str = "auto_demo.seeded";

/// One demo login.
#[derive(Debug, Clone, Copy)]
pub struct Login {
    /// The name shown on the login page.
    pub name: &'static str,
    /// The sign-in e-mail.
    pub email: &'static str,
    /// The role.
    pub role: &'static str,
    /// What it is for, shown beside the name.
    pub label: &'static str,
}

/// The e-mails the steps below act as.
pub(crate) const ADMIN_LOGIN: &str = "admin@alnoorauto.example";
pub(crate) const OWNER_LOGIN: &str = "owner@alnoorauto.example";
pub(crate) const BASMA: &str = "basma@alnoorauto.example";
pub(crate) const SAMI: &str = "sami@alnoorauto.example";
pub(crate) const WALEED: &str = "waleed@alnoorauto.example";
pub(crate) const TAREQ: &str = "tareq@alnoorauto.example";
pub(crate) const NABIL: &str = "nabil@alnoorauto.example";
pub(crate) const HUSSEIN: &str = "hussein@alnoorauto.example";
pub(crate) const KARIM: &str = "karim@alnoorauto.example";
pub(crate) const PIA: &str = "pia@alnoorauto.example";
pub(crate) const CARLA: &str = "carla@alnoorauto.example";
pub(crate) const FATIMA: &str = "fatima@alnoorauto.example";
pub(crate) const AISHA: &str = "aisha@alnoorauto.example";

/// The demo people, one per role (two technicians).
pub const LOGINS: &[Login] = &[
    Login {
        name: "Maryam Al Falasi",
        email: ADMIN_LOGIN,
        role: ADMIN,
        label: "Company administrator",
    },
    Login {
        name: "Rashid Al Noor",
        email: OWNER_LOGIN,
        role: OWNER,
        label: "Owner / GM",
    },
    Login {
        name: "Basma Haddad",
        email: BASMA,
        role: BRANCH_MANAGER,
        label: "Branch manager (Deira)",
    },
    Login {
        name: "Sami Khoury",
        email: SAMI,
        role: SERVICE_ADVISOR,
        label: "Service advisor",
    },
    Login {
        name: "Waleed Mansour",
        email: WALEED,
        role: WORKSHOP_MANAGER,
        label: "Workshop manager",
    },
    Login {
        name: "Tareq Aziz",
        email: TAREQ,
        role: TECHNICIAN,
        label: "Technician (mechanical)",
    },
    Login {
        name: "Nabil Rahman",
        email: NABIL,
        role: TECHNICIAN,
        label: "Technician (body and electrical)",
    },
    Login {
        name: "Hussein Karim",
        email: HUSSEIN,
        role: COUNTER_SALES,
        label: "Counter sales and field cheques",
    },
    Login {
        name: "Karim Sadiq",
        email: KARIM,
        role: STOREKEEPER,
        label: "Storekeeper",
    },
    Login {
        name: "Pia Fernandes",
        email: PIA,
        role: PROCUREMENT,
        label: "Purchaser",
    },
    Login {
        name: "Carla D'Souza",
        email: CARLA,
        role: INSURANCE_COORDINATOR,
        label: "Insurance coordinator",
    },
    Login {
        name: "Fatima Yusuf",
        email: FATIMA,
        role: FLEET_ACCOUNT_MANAGER,
        label: "Fleet account manager",
    },
    Login {
        name: "Aisha Bakr",
        email: AISHA,
        role: ACCOUNTANT,
        label: "Accountant",
    },
];

/// The module that adds the command and the seeder.
pub struct AutoDemo {
    client: wl_kernel::ClientConfig,
}

impl AutoDemo {
    /// The module for the client `config`.
    pub fn new(config: wl_kernel::ClientConfig) -> Self {
        Self { client: config }
    }
}

impl Module for AutoDemo {
    fn name(&self) -> &'static str {
        "auto_demo"
    }

    fn register(&self, app: &mut Registry) {
        let client = self.client.clone();
        app.command(
            COMMAND,
            "Fill the database with the Al Noor demo company (staff, customers, vehicles, job cards in every state, insurance claims, fleet contracts, supplier claims, a recall, back-orders, export sales); local and testing only",
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

    fn routes(&self) -> Routes {
        hooks::routes()
    }
}

/// `auto:demo` and the seeder: boots the app a second time on the same
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
                "the demo company is made in local or testing only, never in {env:?}"
            )
            .into());
        }
        return Ok(());
    }
    if state.config.database_url.contains(":memory:") {
        if said {
            return Err(renox::anyhow::anyhow!(
                "the demo company needs a database file, not an in-memory database"
            )
            .into());
        }
        return Ok(());
    }
    let app = crate::app_for(client).map_err(|e| renox::anyhow::anyhow!("{e}"))?;
    let kernel = app.config((*state.config).clone()).boot().await?;
    if !seed(&kernel).await? {
        println!("The demo company is there already.");
        return Ok(());
    }
    println!("The demo company is made. Log in with any of these (password {PASSWORD}):");
    for login in LOGINS {
        println!("  {:<42} {}", login.email, login.label);
    }
    Ok(())
}

/// Seeds the demo company into the database `kernel` is on; `false` when it
/// is there already and nothing was made. (The app must have been installed
/// and seeded: the legal entity of `client.toml` and the service types
/// exist.)
///
/// # Errors
///
/// The database's errors, and the message of the rule a demo record broke.
pub async fn seed(kernel: &renox::Kernel) -> Result<bool> {
    let state = kernel.state().clone();
    if is_seeded(&state.db).await? {
        return Ok(false);
    }
    // The seeds of the client (the workshop's service types, the loyalty
    // programme, ...) come first: `db:seed` runs them, then this.
    let seeded = wl_sales::LoyaltyProgramme::first_active(&state.db)
        .await?
        .is_some();
    if !seeded {
        return Err(renox::anyhow::anyhow!(
            "the seeds of the client are not there: run `db:seed` (it runs them and then this command in local), or `wl:install` and the seeds first"
        )
        .into());
    }
    let driver = Driver::new(kernel.router());
    let clock = Arc::new(Clock::new().await);
    Box::pin(Seed::new(state, driver, clock).run()).await?;
    // The demo ends with a full search index (BL-AGT-11).
    wl_kernel::search::rebuild(&kernel.state().db).await?;
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
    pub(crate) clock: Arc<Clock>,
    /// The legal entity of the company.
    pub(crate) entity: i64,
    /// The demo users by e-mail.
    pub(crate) users: Vec<(&'static str, User)>,
    /// The demo users as the services take them, by e-mail.
    pub(crate) staff: BTreeMap<&'static str, AuthUser>,
    /// The parties by their key in [`people`].
    pub(crate) parties: BTreeMap<&'static str, PartyId>,
    /// The vehicles by plate key (`dubai-a-12345`).
    pub(crate) vehicles: BTreeMap<&'static str, auto::Vehicle>,
    /// The job cards by the key of the story.
    pub(crate) cards: BTreeMap<&'static str, i64>,
    /// The claims by the key of the story.
    pub(crate) claims: BTreeMap<&'static str, i64>,
    /// The insurers by key.
    pub(crate) insurers: BTreeMap<&'static str, auto::Insurer>,
    /// The stores: the Deira parts counter and the workshop's.
    pub(crate) stores: BTreeMap<&'static str, wl_inventory::Location>,
    /// The items by code.
    pub(crate) items: BTreeMap<&'static str, wl_inventory::Item>,
}

impl Seed {
    fn new(state: AppState, driver: Driver, clock: Arc<Clock>) -> Self {
        Self {
            state,
            driver,
            clock,
            entity: 0,
            users: Vec::new(),
            staff: BTreeMap::new(),
            parties: BTreeMap::new(),
            vehicles: BTreeMap::new(),
            cards: BTreeMap::new(),
            claims: BTreeMap::new(),
            insurers: BTreeMap::new(),
            stores: BTreeMap::new(),
            items: BTreeMap::new(),
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

    /// The demo user with this e-mail, as the services take them.
    pub(crate) fn who(&self, email: &str) -> &AuthUser {
        self.staff
            .get(email)
            .unwrap_or_else(|| panic!("the demo login {email}"))
    }

    /// The party of the story `key`.
    pub(crate) fn party(&self, key: &str) -> PartyId {
        *self
            .parties
            .get(key)
            .unwrap_or_else(|| panic!("the demo party {key}"))
    }

    /// The vehicle with this key.
    pub(crate) fn vehicle(&self, key: &str) -> &auto::Vehicle {
        self.vehicles
            .get(key)
            .unwrap_or_else(|| panic!("the demo vehicle {key}"))
    }

    async fn run(&mut self) -> Result {
        // The seed's dates are relative to today, also inside a step that
        // runs on the clock of a day in the past.
        *super::demo_data::REAL_TODAY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(wl_kernel::documents::today());
        // Not cleared afterwards: seeds that run at the same time in one
        // process (the tests) share it, and the first to end would send the
        // others back to the clock of their step. The next seed sets it again.
        self.story().await
    }

    async fn story(&mut self) -> Result {
        super::roles::define(self.db()).await?;
        self.entity = wl_kernel::LegalEntity::default_entity(self.db()).await?.id;
        self.logins().await?;
        self.people().await?;
        self.shop().await?;
        self.insurers().await?;
        self.labour_menu().await?;
        self.workshop().await?;
        self.claims().await?;
        self.fleets().await?;
        self.supplier_claims().await?;
        self.trade().await?;
        self.stories().await?;
        self.money().await?;
        self.cash().await?;
        self.reminders().await?;
        renox::audit::record(
            self.db(),
            renox::audit::Entry::new(DONE_ACTION).subject(MARKER_SUBJECT, 1),
        )
        .await?;
        Ok(())
    }
}
