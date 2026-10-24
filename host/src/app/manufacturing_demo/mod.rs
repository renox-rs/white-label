//! The demo company of the Manufacturing client (DEMO-DATA.md, 07: *Emirates
//! Pack Industries LLC*, a corrugated packaging maker in the Sharjah
//! Industrial Area, all fictional): `manufacturing:demo` fills a local
//! database so that every screen of F7 has data the moment someone logs in,
//! and `db:seed` runs it in `local`, never in production.
//!
//! - **Routes where there is one, services and fixtures where there is none.**
//!   People, parties, items, bills of materials, work orders, reports,
//!   issues, NCRs, flags, the recall, the credit accounts, the sales orders
//!   and the dispatches go through the app's own router as the person of the
//!   demo who does it, so every rule and guard runs; the opening stock, the
//!   lots, the documents, the invoices and the events of a delivery that came
//!   back are the modules' own services and records inserted in the state a
//!   machine reaches only after days of real life.
//! - **Relative dates.** Every date is `day(n)`, today plus or minus days.
//! - **Idempotent.** The seed is done once its marker exists (an audit entry
//!   of the subject `manufacturing_demo`); logins are kept when they exist.
//!
//! # The company
//!
//! One plant with two lines (corrugator and converting, flexo printing and sleeves), a raw
//! material store, a finished goods store and a quarantine store, and these
//! customers:
//!
//! | Customer | What it shows |
//! |---|---|
//! | Al Noor Trading LLC | credit account, a work order for its order in progress, a dispatch that left, a delivery that came back in part (returned goods, inspection) and a portal buyer |
//! | Gulf Fresh Foods LLC | food boxes (regulated: food-contact certificate), an order whose work order waits |
//! | Emirates Print House LLC | printed sleeves made for it alone (customer specific), an order cancelled after the work started |
//! | Sahara Retail LLC | an invoice 31 days overdue, so its next order is on credit hold |
//! | Atlas Gulf Export FZE | zero-rated exports: one dispatch with its exit certificate, one without |
//!
//! Every login has the password of [`super::seed::PASSWORD`].

use renox::Environment;
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;

use super::demo_data::driver::Driver;
use super::roles::{
    ACCOUNTANT, ADMIN, LINE_OPERATOR, MEMBER, OWNER, PRODUCTION_MANAGER, PRODUCTION_PLANNER,
    QC_INSPECTOR, RECEIVABLES, SALES, SALES_MANAGER, STOREKEEPER, SUPERVISOR, define,
};
use super::seed::PASSWORD;

mod floor;
mod orders;
mod quality;
mod screens;
mod shipping;
mod work;

pub use screens::{Screen, map as screen_map};

/// The command that fills a local database with the demo company.
pub const COMMAND: &str = "manufacturing:demo";

/// The audit subject of the marker of a finished seed.
const MARKER_SUBJECT: &str = "manufacturing_demo";
const DONE_ACTION: &str = "manufacturing_demo.seeded";

/// The role of the customer's buyer in the portal.
pub const BUYER: &str = "customer_buyer";

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

/// The e-mail of the general manager / owner.
pub const OWNER_EMAIL: &str = "owner@emiratespack.example";
/// The e-mail of the company admin.
pub const ADMIN_EMAIL: &str = "admin@emiratespack.example";
/// The e-mail of the production manager.
pub const MANAGER_EMAIL: &str = "production@emiratespack.example";
/// The e-mail of the production planner.
pub const PLANNER_EMAIL: &str = "planner@emiratespack.example";
/// The e-mail of the line supervisor.
pub const SUPERVISOR_EMAIL: &str = "supervisor@emiratespack.example";
/// The e-mail of the line operator.
pub const OPERATOR_EMAIL: &str = "operator@emiratespack.example";
/// The e-mail of the QC inspector.
pub const QC_EMAIL: &str = "qc@emiratespack.example";
/// The e-mail of the storekeeper.
pub const STORE_EMAIL: &str = "store@emiratespack.example";
/// The e-mail of the sales person.
pub const SALES_EMAIL: &str = "sales@emiratespack.example";
/// The e-mail of the sales manager.
pub const SALES_MANAGER_EMAIL: &str = "salesmanager@emiratespack.example";
/// The e-mail of the accountant.
pub const ACCOUNTANT_EMAIL: &str = "accountant@emiratespack.example";
/// The e-mail of the customer buyer (portal).
pub const BUYER_EMAIL: &str = "buyer@alnoor.example";

/// The demo people, one per role of the company.
pub const LOGINS: &[Login] = &[
    Login {
        name: "Khalid Al Mansoori",
        email: OWNER_EMAIL,
        role: OWNER,
        label: "General manager / owner",
    },
    Login {
        name: "Rana Haddad",
        email: ADMIN_EMAIL,
        role: ADMIN,
        label: "Company admin",
    },
    Login {
        name: "Mona Farouk",
        email: MANAGER_EMAIL,
        role: PRODUCTION_MANAGER,
        label: "Production manager",
    },
    Login {
        name: "Amal Youssef",
        email: PLANNER_EMAIL,
        role: PRODUCTION_PLANNER,
        label: "Production planner",
    },
    Login {
        name: "Sami Nasser",
        email: SUPERVISOR_EMAIL,
        role: SUPERVISOR,
        label: "Line supervisor, line 1",
    },
    Login {
        name: "Omar Khan",
        email: OPERATOR_EMAIL,
        role: LINE_OPERATOR,
        label: "Line operator, line 1",
    },
    Login {
        name: "Qasim Ali",
        email: QC_EMAIL,
        role: QC_INSPECTOR,
        label: "QC inspector",
    },
    Login {
        name: "Karim Saleh",
        email: STORE_EMAIL,
        role: STOREKEEPER,
        label: "Storekeeper",
    },
    Login {
        name: "Sara Mahmoud",
        email: SALES_EMAIL,
        role: SALES,
        label: "Sales",
    },
    Login {
        name: "Sam Dawood",
        email: SALES_MANAGER_EMAIL,
        role: SALES_MANAGER,
        label: "Sales manager",
    },
    Login {
        name: "Wendy D'Souza",
        email: ACCOUNTANT_EMAIL,
        role: ACCOUNTANT,
        label: "Accountant",
    },
    Login {
        name: "Nadia Al Noor",
        email: BUYER_EMAIL,
        role: MEMBER,
        label: "Customer buyer (portal), Al Noor Trading",
    },
];

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
pub(crate) use fixture;

/// The module that adds the command and the seeder.
pub struct ManufacturingDemo {
    client: wl_kernel::ClientConfig,
}

impl ManufacturingDemo {
    /// The module for the client `config`.
    pub fn new(config: wl_kernel::ClientConfig) -> Self {
        Self { client: config }
    }
}

impl Module for ManufacturingDemo {
    fn name(&self) -> &'static str {
        "manufacturing_demo"
    }

    fn register(&self, app: &mut Registry) {
        let client = self.client.clone();
        app.command(
            COMMAND,
            "Fill the database with the Emirates Pack demo company (plant, stock, BOMs, orders, work orders, NCRs, recall, dispatches, conformity); local and testing only",
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

/// `manufacturing:demo` and the seeder: boots the app a second time on the
/// same database to get its router (the people of the demo do their work
/// through it), then seeds. The command works in `local` and `testing`; the
/// seeder of `db:seed` only in `local`.
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

/// Seeds the demo company into the database `kernel` is on; `false` when it
/// is there already and nothing was made. (The app must have been installed:
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

/// What the steps share: the state, the driver and the ids they made.
pub(crate) struct Seed {
    pub(crate) state: AppState,
    pub(crate) driver: Driver,
    pub(crate) entity: i64,
    pub(crate) plant: i64,
    /// Line 1 and line 2.
    pub(crate) lines: Vec<i64>,
    /// The locations: raw store, finished goods, quarantine.
    pub(crate) rm: i64,
    pub(crate) fg: i64,
    pub(crate) quarantine: i64,
    /// Items by code.
    pub(crate) items: std::collections::BTreeMap<&'static str, i64>,
    /// Lots by number.
    pub(crate) lots: std::collections::BTreeMap<&'static str, i64>,
    /// BOMs by finished item code.
    pub(crate) boms: std::collections::BTreeMap<&'static str, i64>,
    /// Customers by short key.
    pub(crate) customers: std::collections::BTreeMap<&'static str, wl_kernel::PartyId>,
    /// Sales orders by key: (order id, line id).
    pub(crate) orders: std::collections::BTreeMap<&'static str, (i64, i64)>,
    /// Work orders by key.
    pub(crate) work_orders: std::collections::BTreeMap<&'static str, i64>,
}

impl Seed {
    fn new(state: AppState, driver: Driver) -> Self {
        Self {
            state,
            driver,
            entity: 0,
            plant: 0,
            lines: Vec::new(),
            rm: 0,
            fg: 0,
            quarantine: 0,
            items: Default::default(),
            lots: Default::default(),
            boms: Default::default(),
            customers: Default::default(),
            orders: Default::default(),
            work_orders: Default::default(),
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
        self.entity = wl_kernel::LegalEntity::default_entity(self.db()).await?.id;
        self.logins().await?;
        self.floor().await?;
        self.assign_scoped_roles().await?;
        self.stock().await?;
        self.boms().await?;
        self.conformity().await?;
        self.customers().await?;
        self.money().await?;
        self.orders().await?;
        self.work().await?;
        self.shipping().await?;
        self.quality().await?;
        renox::audit::record(
            self.db(),
            renox::audit::Entry::new(DONE_ACTION).subject(MARKER_SUBJECT, 1),
        )
        .await?;
        Ok(())
    }

    /// The people with a login, one per role. The roles of the plant and of
    /// the line are given once they exist ([`Self::assign_scoped_roles`]).
    async fn logins(&mut self) -> Result {
        for login in LOGINS {
            let user = match User::find_by_email(self.db(), login.email).await? {
                Some(user) => user,
                None => {
                    let mut user =
                        User::register(self.db(), login.name, login.email, PASSWORD).await?;
                    user.email_verified_at = Some(renox::db::now());
                    user.save(self.db()).await?;
                    if !SCOPED.contains(&login.role) {
                        user.assign_role(self.db(), login.role).await?;
                        if login.email == ACCOUNTANT_EMAIL {
                            user.assign_role(self.db(), RECEIVABLES).await?;
                        }
                    }
                    user
                }
            };
            let db = self.state.db.clone();
            self.driver.sign_in(&db, login.email, &user).await?;
        }
        Ok(())
    }
}

/// The roles held for the plant or a line only.
const SCOPED: &[&str] = &[
    PRODUCTION_MANAGER,
    PRODUCTION_PLANNER,
    SUPERVISOR,
    LINE_OPERATOR,
    QC_INSPECTOR,
];
