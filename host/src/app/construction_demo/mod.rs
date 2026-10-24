//! The demo company of the Construction client (DEMO-DATA.md, 02:
//! *Al Waha Interiors & Fit-out LLC*): `construction:demo` fills a local
//! database so that every F2 screen has data the moment someone logs in, and
//! `db:seed` runs it in `local`, never anywhere else.
//!
//! It follows the Property demo (`super::demo_data`) and shares its driver,
//! clock and day helpers:
//!
//! - **Through the app.** Every step a person takes is a request to the app's
//!   own router as the demo user who does it (the project manager awards and
//!   submits, the QS certifies, the supervisor marks attendance, the
//!   storekeeper posts receipts, receivables issues invoices, the accountant
//!   records payments); parties, documents, employees, items, stores and
//!   purchase orders are made by the modules' own services. The permissions,
//!   the guards and the state machines are the real ones, and a record that
//!   breaks a rule stops the seed with the rule's message. A status is never
//!   written. Where the module has no path to a state, the record is
//!   inserted in it with `insert_in_state`, and the place says why.
//! - **Deterministic.** Names, amounts and dates are written out or computed
//!   from the position in a list.
//! - **Relative dates.** Every date is `day(-n)`, today plus or minus days.
//! - **Idempotent.** The seed is done once its marker exists (an audit entry
//!   of the subject `construction_demo`, which Property's marker does not
//!   share), and seeding again changes nothing.
//!
//! # Screen map
//!
//! The command ends by printing the URLs, with real ids, of every F2 page
//! ([`screens`]). Signed in as the admin demo user
//! (`admin@alwaha.example`, password `password123`):
//!
//! | Screen | URL |
//! |---|---|
//! | IPA (JSON) | `/construction/ipas/{id}` |
//! | Closing screen of a project (JSON) | `/construction/projects/{id}/closing` |
//! | Variation report (JSON) | `/construction/projects/{id}/variation-report` |
//! | Crew attendance of a site and day | `/attendance/crew?location_id=..&date=..&employees=..` |
//! | Employees | `/hr/employees`, `/hr/employees/{id}` |
//! | Supplier prices of an item (JSON) | `/items/{id}/suppliers` |
//! | Reorder suggestion (JSON) | `/reorder-suggestions/{id}` |
//! | Preferred-supplier request (JSON) | `/preferred-requests/{id}` |
//! | Supplier score (JSON) | `/suppliers/{party}/score` |
//! | Invoices | `/invoices`, `/invoices/{id}` |
//! | Settings of the shared modules | `/settings/negative-flow` |
//! | Money | `/cash-forecast`, `/budget-report`, `/cheques`, `/refunds`, `/receivables/ageing` |
//!
//! The list printed by the command has every other screen of the shared
//! modules too (parties, documents, payments, supplier bills, budgets,
//! approvals).

// Runs a block as if it were a day, at 10:00 (the Property demo's macro).
use super::demo_data::at;

/// Runs `body` as if it were `hour:minute` of `day` (Asia/Dubai): the end of
/// a shift, when a supervisor marks the crew with the check-out.
macro_rules! at_time {
    ($seed:ident, $day:expr, $hour:expr, $minute:expr, $body:expr) => {{
        let clock = $seed.clock.clone();
        let when = $day.and_hms_opt($hour, $minute, 0).unwrap_or_default();
        #[allow(unused_braces)]
        let step = Box::pin(async { $body });
        clock.at_time(when, step).await
    }};
}

mod hooks;
mod ipa_plans;
mod people;
mod screens;
mod seed_attendance;
mod seed_books;
mod seed_closing;
mod seed_guarantees;
mod seed_hr;
mod seed_ipas;
mod seed_money;
mod seed_people;
mod seed_projects;
mod seed_stock;
mod seed_variations;
mod stock_plans;

use std::collections::BTreeMap;
use std::sync::Arc;

use renox::App;
use renox::Kernel;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::documents::today;

pub use people::{LOGINS, Login};
pub use screens::{Screen, map as screen_map};

use super::demo_data::clock::Clock;
use super::demo_data::driver::Driver;
use super::demo_data::{REAL_TODAY, day};
use super::roles;

/// The command that seeds the demo company.
pub const COMMAND: &str = "construction:demo";

/// The audit entry that marks a finished seed.
const DONE_ACTION: &str = "construction_demo.seeded";

/// The audit subject of the marker; Property's is `demo`.
const MARKER_SUBJECT: &str = "construction_demo";

/// What a seed made.
#[derive(Debug, Default, Clone)]
pub struct Report {
    /// Records made, by what they are.
    pub made: BTreeMap<&'static str, usize>,
    /// Whether the demo was there already (nothing was made).
    pub already: bool,
    /// The time the seed had taken when each part was done.
    pub laps: Vec<(&'static str, std::time::Duration)>,
}

impl Report {
    fn add(&mut self, what: &'static str, count: usize) {
        *self.made.entry(what).or_default() += count;
    }
}

/// `date` as the forms take it.
pub(crate) fn text(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// The earlier of `date` and the real today: a step cannot happen in the
/// future.
pub(crate) fn upto(date: NaiveDate) -> NaiveDate {
    date.min(super::demo_data::real_today())
}

/// `fils` as the dirhams a form takes: `6991.66`.
pub(crate) fn aed(fils: i64) -> String {
    format!("{}.{:02}", fils / 100, fils % 100)
}

/// The demo is there once its last step recorded that it is done.
pub async fn is_seeded(db: &Db) -> Result<bool> {
    Ok(!renox::audit::for_subject(db, MARKER_SUBJECT, 1, 1)
        .await?
        .is_empty())
}

/// The state of a seed in progress.
pub(crate) struct Seed {
    pub state: AppState,
    pub driver: Driver,
    pub report: Report,
    pub clock: Arc<Clock>,
    /// The parties by English name.
    pub parties: BTreeMap<String, wl_parties::Party>,
    /// The employees by key.
    pub employees: BTreeMap<&'static str, i64>,
    /// The work locations by project code.
    pub sites: BTreeMap<String, i64>,
    /// The items by code.
    pub items: BTreeMap<&'static str, i64>,
}

impl Seed {
    pub(crate) fn db(&self) -> &Db {
        &self.state.db
    }

    /// The party named `en`.
    pub(crate) fn party(&self, en: &str) -> &wl_parties::Party {
        self.parties
            .get(en)
            .unwrap_or_else(|| panic!("the demo party {en}"))
    }

    /// The user id of the demo login `email`.
    pub(crate) async fn user_id(&self, email: &str) -> Result<i64> {
        Ok(User::find_by_email(self.db(), email)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("the demo login {email}"))?
            .id)
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
        state: state.clone(),
        driver: Driver::new(kernel.router()),
        report: Report::default(),
        clock: Arc::new(Clock::new().await),
        parties: BTreeMap::new(),
        employees: BTreeMap::new(),
        sites: BTreeMap::new(),
        items: BTreeMap::new(),
    };
    Box::pin(seed.run()).await?;
    // The demo ends with a full search index (BL-AGT-11).
    wl_kernel::search::rebuild(&kernel.state().db).await?;
    Ok(seed.report)
}

impl Seed {
    /// The story of the projects, in the order it happened: each project is
    /// awarded and started on its day, and the cut-offs make the IPAs.
    async fn story(&mut self) -> Result {
        // The office fit-out that was completed.
        self.award("OFC-06").await?;
        self.boq("OFC-06").await?;
        self.start("OFC-06").await?;
        for c in ipa_plans::COMPLETED {
            self.month(c.cutoff, &[*c]).await?;
        }
        self.complete("OFC-06", -130).await?;
        // The showroom (terminated later), the tower and the fashion store.
        for code in ["TRM-07", "TWR-01", "MALL-02"] {
            self.award(code).await?;
            self.boq(code).await?;
            self.start(code).await?;
        }
        self.month(-118, ipa_plans::JUNE).await?;
        for code in ["CLN-03"] {
            self.award(code).await?;
            self.boq(code).await?;
            self.start(code).await?;
        }
        self.month(-88, ipa_plans::JULY).await?;
        for code in ["RST-04"] {
            self.award(code).await?;
            self.boq(code).await?;
            self.start(code).await?;
        }
        self.month(-57, ipa_plans::AUGUST).await?;
        // The client terminated the showroom's subcontract; its third IPA
        // became the final account, which is submitted and certified.
        self.terminate("TRM-07", -45, -40).await?;
        // The retention of the terminated project fell due; the first part is
        // invoiced once the termination took effect.
        self.invoice_retention("TRM-07", construction::retention::RetentionPart::First, -35)
            .await?;
        self.cycle(&ipa_plans::FINAL_ACCOUNT).await?;
        self.month(-26, ipa_plans::SEPTEMBER).await?;
        // This month's drafts, and a project just awarded.
        self.cutoff(0).await?;
        for (code, number, bp) in [
            ("TWR-01", 5, 6_100),
            ("MALL-02", 5, 7_200),
            ("CLN-03", 4, 4_300),
            ("RST-04", 3, 4_600),
        ] {
            let draft = self.ipa_of(code, number).await?;
            let today = crate::app::demo_data::real_today();
            at!(self, today, { self.progress(&draft, bp).await })?;
        }
        self.award("VIL-05").await?;
        self.boq("VIL-05").await?;
        Ok(())
    }

    async fn run(&mut self) -> Result {
        let started = std::time::Instant::now();
        let lap = |what: &'static str, seed: &mut Self| {
            seed.report.laps.push((what, started.elapsed()));
        };
        roles::define(self.db()).await?;
        // The company is set up some 15 months ago.
        at!(self, day(-470), {
            Box::pin(self.parties()).await?;
            Box::pin(self.logins()).await?;
            self.registrations().await
        })?;
        lap("people", self);
        Box::pin(self.staff()).await?;
        lap("staff", self);
        Box::pin(self.story()).await?;
        lap("projects and IPAs", self);
        Box::pin(self.variations()).await?;
        lap("variations", self);
        Box::pin(self.guarantees()).await?;
        self.pending_documents().await?;
        lap("guarantees", self);
        Box::pin(self.stock()).await?;
        Box::pin(self.stock_rules()).await?;
        lap("stock", self);
        Box::pin(self.money()).await?;
        lap("money", self);
        self.permits().await?;
        self.assign_sites().await?;
        self.sites().await?;
        Box::pin(self.attendance()).await?;
        lap("attendance", self);
        self.daily_rules().await?;
        lap("daily rules", self);
        // The sign that the seed is done.
        renox::audit::record(
            self.db(),
            renox::audit::Entry::new(DONE_ACTION).subject(MARKER_SUBJECT, 1),
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
        "construction_demo"
    }

    fn routes(&self) -> Routes {
        hooks::Hooks.routes()
    }

    fn register(&self, app: &mut Registry) {
        let client = self.client.clone();
        app.command(
            COMMAND,
            "Fill the database with the Al Waha demo company (projects, IPAs, variations, attendance, stock, invoices, people); local and testing only",
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

/// `construction:demo` and the seeder: boots the app a second time on the
/// same database to get its router, then seeds. The command works in `local`
/// and `testing`; the seeder of `db:seed` only in `local`, so a test that
/// seeds the modules does not get the whole company.
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
    // The second app needs the same database; an in-memory one (a test's) is
    // private to its app, so there the seeder has nothing to fill.
    if state.config.database_url.contains(":memory:") {
        if said {
            return Err(renox::anyhow::anyhow!(
                "the demo company needs a database file, not an in-memory database"
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
        println!(
            "Demo company made in {:?}:",
            report.laps.last().map(|l| l.1).unwrap_or_default()
        );
        let mut before = std::time::Duration::default();
        for (what, at) in &report.laps {
            println!("  {:>6.1}s {what}", (*at - before).as_secs_f32());
            before = *at;
        }
        for (what, count) in &report.made {
            println!("  {count:>5} {what}");
        }
    }
    println!("{}", screens::print(kernel.state()).await?);
    Ok(())
}
