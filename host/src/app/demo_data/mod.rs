//! The demo company (DEMO-DATA.md, Property: *Palm Grove Properties LLC*):
//! `property:demo` fills a local database so that every screen has data the
//! moment someone logs in, and `db:seed` runs it in `local`, never anywhere
//! else.
//!
//! How it is made (DEMO-DATA.md 1.1):
//!
//! - **Through the modules.** Buildings, units, parties and documents are made
//!   by the modules' own services; everything a user does (submit and
//!   activate a lease, receive and clear a cheque, issue an invoice, assign a
//!   ticket, approve a statement) is a request to the app's own router as the
//!   demo user who does it ([`driver`]), so the permissions, the guards and
//!   the state machines are the real ones, and a record that breaks a rule
//!   stops the seed with the rule's message. A status is never written. Where
//!   the module has no path to a state, the record is inserted in it with
//!   `insert_in_state`, and the place says why.
//! - **Deterministic.** Names, amounts and dates are written out or computed
//!   from the position in a list: the same seed gives the same data.
//! - **Relative dates.** Every date is `today()` plus or minus days, so
//!   "expires in 30 days" is true on the day of the demo.
//! - **Idempotent.** The seed is done once the demo admin exists, and seeding
//!   again changes nothing.

mod driver;
mod hooks;
mod leases;
mod owners;
mod people;
mod portfolio;
mod seed_documents;
mod seed_leases;
mod seed_money;
mod seed_owners;
mod seed_people;
mod seed_portfolio;
mod specimen;

use std::collections::BTreeMap;

use renox::App;
use renox::Kernel;
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;

pub use people::{LOGINS, Login};

use driver::Driver;

use super::roles;
use super::seed::PASSWORD;

/// The command that seeds the demo company.
pub const COMMAND: &str = "property:demo";

/// What a seed made.
#[derive(Debug, Default, Clone)]
pub struct Report {
    /// Records made, by what they are.
    pub made: BTreeMap<&'static str, usize>,
    /// Whether the demo was there already (nothing was made).
    pub already: bool,
}

impl Report {
    fn add(&mut self, what: &'static str, count: usize) {
        *self.made.entry(what).or_default() += count;
    }
}

/// `days` from today (negative: before).
pub fn day(days: i64) -> NaiveDate {
    if days >= 0 {
        today() + Days::new(days as u64)
    } else {
        today() - Days::new(days.unsigned_abs())
    }
}

/// The demo is there once its last login exists (the last thing made).
pub async fn is_seeded(db: &Db) -> Result<bool> {
    Ok(
        User::find_by_email(db, people::LOGINS[people::LOGINS.len() - 1].email)
            .await?
            .is_some(),
    )
}

/// The state of a seed in progress.
pub(crate) struct Seed<'a> {
    pub kernel: &'a Kernel,
    pub state: AppState,
    pub driver: Driver,
    pub report: Report,
    pub parties: BTreeMap<String, wl_parties::Party>,
    pub buildings: BTreeMap<String, property::Building>,
    pub units: BTreeMap<(String, String), property::Unit>,
    pub papered: std::collections::BTreeSet<String>,
    pub leases: BTreeMap<&'static str, i64>,
    pub held_back: std::collections::BTreeSet<i64>,
}

impl Seed<'_> {
    pub(crate) fn db(&self) -> &Db {
        &self.state.db
    }
}

/// Seeds the demo company into the database `kernel` is on. Does nothing when
/// it is there.
///
/// # Errors
///
/// The database's errors, and the message of the rule a demo record broke.
pub async fn seed(kernel: &Kernel) -> Result<Report> {
    let state = kernel.state().clone();
    if is_seeded(&state.db).await? {
        return Ok(Report {
            already: true,
            ..Report::default()
        });
    }
    let mut seed = Seed {
        kernel,
        state: state.clone(),
        driver: Driver::new(kernel.router()),
        report: Report::default(),
        parties: BTreeMap::new(),
        buildings: BTreeMap::new(),
        units: BTreeMap::new(),
        papered: Default::default(),
        leases: BTreeMap::new(),
        held_back: Default::default(),
    };
    seed.run().await?;
    Ok(seed.report)
}

impl Seed<'_> {
    async fn run(&mut self) -> Result {
        roles::define(self.db()).await?;
        self.parties().await?;
        self.charge_types().await?;
        self.portfolio().await?;
        self.logins(false).await?;
        self.leases().await?;
        self.owners().await?;
        // The last login is the sign the seed is done.
        self.logins(true).await?;
        Ok(())
    }
}

/// The module: the command, and the seeder `db:seed` runs in `local`.
pub struct DemoSeed {
    client: wl_kernel::ClientConfig,
}

impl DemoSeed {
    /// The module for the client `config`.
    pub fn new(config: wl_kernel::ClientConfig) -> Self {
        Self { client: config }
    }
}

impl Module for DemoSeed {
    fn name(&self) -> &'static str {
        "demo_seed"
    }

    fn routes(&self) -> Routes {
        hooks::Hooks.routes()
    }

    fn register(&self, app: &mut Registry) {
        let client = self.client.clone();
        app.command(
            COMMAND,
            "Fill the database with the Palm Grove demo company (buildings, leases, cheques, invoices, tickets, people); local and testing only",
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

/// `property:demo` and the seeder: boots the app a second time on the same
/// database to get its router, then seeds. The command works in `local` and
/// `testing`; the seeder of `db:seed` only in `local`, so a test that seeds
/// the modules does not get the whole company.
async fn run_command(client: wl_kernel::ClientConfig, state: AppState, said: bool) -> Result {
    use renox::Environment;
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
    let app: App = crate::app_for(client).map_err(|e| renox::anyhow::anyhow!("{e}"))?;
    let kernel = app.config((*state.config).clone()).boot().await?;
    let report = seed(&kernel).await?;
    if report.already {
        println!("The demo company is there already.");
    } else {
        println!("Demo company made:");
        for (what, count) in &report.made {
            println!("  {count:>5} {what}");
        }
    }
    Ok(())
}
