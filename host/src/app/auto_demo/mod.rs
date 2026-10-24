//! The demo company of the Auto client (DEMO-DATA.md, 08: a parts trader
//! with a workshop, *Al Noor Auto Parts & Garage LLC* of
//! `clients/demo-auto/client.toml`, all fictional): `auto:demo` fills a local
//! database so that every screen of F8 has data the moment someone logs in,
//! and `db:seed` runs it in `local`, never in production.
//!
//! - **Routes where there is one, services and fixtures where there is none.**
//!   Vehicles, the labour menu, insurers, job cards (intake, technician,
//!   inspection, estimate, approval, work, parts, QC, delivery), insurance
//!   claims, fleet contracts and export sales go through the app's own router
//!   as the person who does them, so every rule and guard runs; the parties,
//!   the opening stock, the workshop bay and the bookings (wl-scheduling's
//!   service, as the service advisor) are the modules' own services and
//!   records.
//! - **Relative dates.** Every date is `day(n)`, today plus or minus days.
//! - **Idempotent.** The seed is done once its marker exists (an audit entry
//!   of the subject `auto_demo`); logins are kept when they exist.
//!
//! # The workshop's day
//!
//! | Vehicle | What it shows |
//! |---|---|
//! | Land Cruiser of Mona Hassan | periodic service delivered and invoiced, parts from the store |
//! | Patrol of Rashid Al Nuaimi | ready for collection, not collected yet |
//! | Hilux of Gulf Build Contracting | in quality check |
//! | Accord of Priya Raman | in progress: labour started, parts issued |
//! | Pajero of Omar Siddiqui | waiting for a special-order alternator |
//! | Camry of Fatima Al Zaabi | estimate sent, waiting for the customer's answer |
//! | Corolla of Joseph Fernandes | being inspected |
//! | Altima of Ahmed Karim | just checked in |
//! | Sunny of Ravi Kumar | estimate declined by the customer |
//! | Fortuner of Sara Al Mansoori | insurance repair: claim approved by the insurer, work running |
//! | Tiida of Khalid Obaid | insurance repair: estimate with the insurer |
//! | Two vans of Emirates Fleet Rentals | a fleet contract with its service plans |
//!
//! Every login has the password of [`super::seed::PASSWORD`].

use renox::Environment;
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;

use super::demo_data::driver::Driver;
use super::roles::{
    ACCOUNTANT, ADMIN, BRANCH_MANAGER, COUNTER_SALES, FLEET_ACCOUNT_MANAGER, INSURANCE_COORDINATOR,
    OWNER, PROCUREMENT, SERVICE_ADVISOR, STOREKEEPER, TECHNICIAN, WORKSHOP_MANAGER, define,
};
use super::seed::PASSWORD;

mod claims;
mod fleet;
mod screens;
mod setup;
mod trade;
mod workshop;

pub use screens::{Screen, map as screen_map};

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

/// The e-mail of the owner.
pub const OWNER_EMAIL: &str = "owner@alnoorauto.example";
/// The e-mail of the company admin.
pub const ADMIN_EMAIL: &str = "admin@alnoorauto.example";
/// The e-mail of the branch manager.
pub const BRANCH_EMAIL: &str = "branch@alnoorauto.example";
/// The e-mail of the service advisor.
pub const ADVISOR_EMAIL: &str = "advisor@alnoorauto.example";
/// The e-mail of the workshop manager.
pub const WORKSHOP_EMAIL: &str = "workshop@alnoorauto.example";
/// The e-mail of the technician.
pub const TECH_EMAIL: &str = "tech@alnoorauto.example";
/// The e-mail of the second technician.
pub const TECH2_EMAIL: &str = "tech2@alnoorauto.example";
/// The e-mail of the counter sales person.
pub const COUNTER_EMAIL: &str = "counter@alnoorauto.example";
/// The e-mail of the storekeeper.
pub const STORE_EMAIL: &str = "store@alnoorauto.example";
/// The e-mail of the purchaser.
pub const PURCHASER_EMAIL: &str = "purchaser@alnoorauto.example";
/// The e-mail of the accountant.
pub const ACCOUNTANT_EMAIL: &str = "accountant@alnoorauto.example";
/// The e-mail of the insurance coordinator.
pub const INSURANCE_EMAIL: &str = "insurance@alnoorauto.example";
/// The e-mail of the fleet account manager.
pub const FLEET_EMAIL: &str = "fleet@alnoorauto.example";

/// The demo people, one per role of the company.
pub const LOGINS: &[Login] = &[
    Login {
        name: "Hamdan Al Mazrouei",
        email: OWNER_EMAIL,
        role: OWNER,
        label: "Owner / general manager",
    },
    Login {
        name: "Nadia Rizvi",
        email: ADMIN_EMAIL,
        role: ADMIN,
        label: "Company admin",
    },
    Login {
        name: "Tariq Mahmood",
        email: BRANCH_EMAIL,
        role: BRANCH_MANAGER,
        label: "Branch manager, Al Quoz workshop",
    },
    Login {
        name: "Sami Haddad",
        email: ADVISOR_EMAIL,
        role: SERVICE_ADVISOR,
        label: "Service advisor",
    },
    Login {
        name: "Waleed Barakat",
        email: WORKSHOP_EMAIL,
        role: WORKSHOP_MANAGER,
        label: "Workshop manager",
    },
    Login {
        name: "Tareq Anwar",
        email: TECH_EMAIL,
        role: TECHNICIAN,
        label: "Technician",
    },
    Login {
        name: "Rommel Santos",
        email: TECH2_EMAIL,
        role: TECHNICIAN,
        label: "Technician",
    },
    Login {
        name: "Vikram Shetty",
        email: COUNTER_EMAIL,
        role: COUNTER_SALES,
        label: "Counter sales, Deira",
    },
    Login {
        name: "Imran Baig",
        email: STORE_EMAIL,
        role: STOREKEEPER,
        label: "Storekeeper",
    },
    Login {
        name: "Elena Petrova",
        email: PURCHASER_EMAIL,
        role: PROCUREMENT,
        label: "Purchaser",
    },
    Login {
        name: "Wendy D'Costa",
        email: ACCOUNTANT_EMAIL,
        role: ACCOUNTANT,
        label: "Accountant",
    },
    Login {
        name: "Layla Nasser",
        email: INSURANCE_EMAIL,
        role: INSURANCE_COORDINATOR,
        label: "Insurance coordinator",
    },
    Login {
        name: "Daniel Okafor",
        email: FLEET_EMAIL,
        role: FLEET_ACCOUNT_MANAGER,
        label: "Fleet account manager",
    },
];

/// The workshop in Al Quoz: the wl-scheduling location of the bookings, the
/// scope of the workshop's store.
pub(crate) fn workshop() -> wl_kernel::Subject {
    wl_kernel::Subject::new("scheduling.location", 1)
}

/// `n` days from today (negative: before).
fn day(n: i64) -> NaiveDate {
    if n >= 0 {
        today() + Days::new(n as u64)
    } else {
        today() - Days::new(n.unsigned_abs())
    }
}

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
            "Fill the database with the Al Noor Auto demo company (vehicles, job cards, insurance claims, fleet contracts, stock, sales); local and testing only",
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

/// `auto:demo` and the seeder: boots the app a second time on the same
/// database to get its router (the people of the demo do their work through
/// it), then seeds. The command works in `local` and `testing`; the seeder of
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
        println!("{}", screens::print(kernel.state()).await?);
        return Ok(());
    }
    println!("The demo company is made. Log in with any of these (password {PASSWORD}):");
    for login in LOGINS {
        println!("  {:<44} {}", login.email, login.label);
    }
    println!("{}", screens::print(kernel.state()).await?);
    Ok(())
}

/// Seeds the demo company into the database `kernel` is on; `false` when it is
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

/// What the steps share: the state, the driver and the ids they made.
pub(crate) struct Seed {
    pub(crate) state: AppState,
    pub(crate) driver: Driver,
    /// The parties by short key.
    pub(crate) parties: std::collections::BTreeMap<&'static str, wl_kernel::PartyId>,
    /// The vehicles by short key.
    pub(crate) vehicles: std::collections::BTreeMap<&'static str, i64>,
    /// The job cards by short key.
    pub(crate) jobs: std::collections::BTreeMap<&'static str, i64>,
    /// The workshop bay.
    pub(crate) bay: i64,
}

impl Seed {
    fn new(state: AppState, driver: Driver) -> Self {
        Self {
            state,
            driver,
            parties: Default::default(),
            vehicles: Default::default(),
            jobs: Default::default(),
            bay: 0,
        }
    }

    pub(crate) fn db(&self) -> &Db {
        &self.state.db
    }

    pub(crate) async fn user(&self, email: &str) -> Result<User> {
        Ok(User::find_by_email(self.db(), email)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("the demo login {email}"))?)
    }

    async fn run(&mut self) -> Result {
        define(self.db()).await?;
        // The workshop's service types and ticket categories (`auto:install`,
        // also run by `db:seed`): kept when they are there.
        super::auto_seed::install(&self.state).await?;
        self.logins().await?;
        self.setup().await?;
        self.workshop().await?;
        self.claims().await?;
        self.fleet().await?;
        self.trade().await?;
        renox::audit::record(
            self.db(),
            renox::audit::Entry::new(DONE_ACTION).subject(MARKER_SUBJECT, 1),
        )
        .await?;
        Ok(())
    }

    /// The people with a login, one per role.
    async fn logins(&mut self) -> Result {
        for login in LOGINS {
            let user = match User::find_by_email(self.db(), login.email).await? {
                Some(user) => user,
                None => {
                    let mut user =
                        User::register(self.db(), login.name, login.email, PASSWORD).await?;
                    user.email_verified_at = Some(renox::db::now());
                    user.save(self.db()).await?;
                    user.assign_role(self.db(), login.role).await?;
                    user
                }
            };
            let db = self.state.db.clone();
            self.driver.sign_in(&db, login.email, &user).await?;
        }
        Ok(())
    }
}
