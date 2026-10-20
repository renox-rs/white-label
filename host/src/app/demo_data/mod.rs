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

/// Runs `body` (a block that may use `?` and `.await`) as if it were `day`.
/// The day is worked out first, from the real today.
macro_rules! at {
    ($seed:ident, $day:expr, $body:expr) => {{
        let clock = $seed.clock.clone();
        let when = $day;
        clock.at(when, async { $body }).await
    }};
}

mod clock;
mod driver;
mod hooks;
mod leases;
mod owners;
mod people;
mod portfolio;
mod sales;
mod seed_documents;
mod finance;
mod seed_finance;
mod seed_hr;
mod seed_leases;
mod seed_money;
mod seed_owners;
mod seed_people;
mod seed_portfolio;
mod seed_sales;
mod seed_service;
mod service;
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

/// The real today, which the seed's dates are relative to. Inside a step
/// that runs at a day in the past, `today()` is that day, so the seed keeps
/// its own.
static REAL_TODAY: std::sync::Mutex<Option<NaiveDate>> = std::sync::Mutex::new(None);

/// The real today.
pub fn real_today() -> NaiveDate {
    REAL_TODAY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .unwrap_or_else(today)
}

/// `days` from the real today (negative: before).
pub fn day(days: i64) -> NaiveDate {
    let base = real_today();
    if days >= 0 {
        base + Days::new(days as u64)
    } else {
        base - Days::new(days.unsigned_abs())
    }
}

/// The audit entry that marks a finished seed.
const DONE_ACTION: &str = "demo.seeded";

/// The demo is there once its last step recorded that it is done.
pub async fn is_seeded(db: &Db) -> Result<bool> {
    Ok(!renox::audit::for_subject(db, "demo", 1, 1)
        .await?
        .is_empty())
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
    pub clock: std::sync::Arc<clock::Clock>,
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
    *REAL_TODAY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(today());
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
        clock: std::sync::Arc::new(clock::Clock::new().await),
    };
    seed.run().await?;
    Ok(seed.report)
}

impl Seed<'_> {
    async fn run(&mut self) -> Result {
        roles::define(self.db()).await?;
        // The company is set up some 17 months ago.
        at!(self, day(-520), {
            self.charge_types().await?;
            self.parties().await?;
            self.portfolio().await?;
            self.logins().await
        })?;
        self.leases().await?;
        self.owners().await?;
        self.service().await?;
        self.sales().await?;
        self.staff().await?;
        self.finance().await?;
        // The sign that the seed is done.
        renox::audit::record(
            self.db(),
            renox::audit::Entry::new(DONE_ACTION).subject("demo", 1),
        )
        .await?;
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
