//! The demo firm of the Accounting client (DEMO-DATA.md, 04: an accounting
//! and tax firm in Business Bay, *Falcon Ledger Accountants LLC* of
//! `clients/demo-accounting/client.toml`, all fictional):
//! `accounting:demo` fills a local database so that every screen of F4 has
//! data the moment someone logs in, and `db:seed` runs it in `local`, never
//! in production.
//!
//! - **Routes where there is one, services and fixtures where there is none.**
//!   Engagements, tax profiles, CDD files, letters, activations, returns,
//!   AML concerns, KPI packs, suspensions and ends go through the app's own
//!   router as the person of the firm who does them, so every rule and guard
//!   runs; the parties, the KYC documents, the screenings, the portal grants
//!   and the records a machine reaches only after months of real life (a
//!   return filed last quarter, a sanctions match) are the modules' own
//!   services and records.
//! - **Relative dates.** Every date is `day(n)`, today plus or minus days.
//! - **Idempotent.** The seed is done once its marker exists (an audit entry
//!   of the subject `accounting_demo`); logins are kept when they exist.
//!
//! # The firm's clients
//!
//! | Client | What it shows |
//! |---|---|
//! | Marina Bay Trading LLC | active, VAT quarterly: a return waiting for the client's approval in the portal, a filed one, a published KPI pack; the portal owner and finance contact |
//! | Desert Rose Interiors LLC | active: a return in the reviewer's hands, a return filed and corrected by voluntary disclosure |
//! | Oasis Fresh Foods LLC | active: a return being prepared with workpapers missing, a retainer invoice overdue, a WIP write-off waiting for the partner |
//! | Al Waha General Trading FZE | high risk with enhanced due diligence: a sanctions possible match under the MLRO's review, a concern raised by staff |
//! | Palm Tech Solutions LLC | onboarding: the CDD file waiting for the MLRO and the letter sent, not signed |
//! | Sand Dune Contracting LLC | suspended for non-payment: tasks held, the deadline notice prepared |
//! | Coral Reef Restaurants LLC | ended: services stopped |
//! | Pearl Fashion House LLC | declined at onboarding |
//! | Gulf Horizon Logistics LLC | active: a tax profile change waiting for the partner, a KPI pack in draft |
//!
//! Every login has the password of [`super::seed::PASSWORD`].

use renox::Environment;
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;

use super::demo_data::driver::Driver;
use super::roles::{
    ACCOUNTANT, ADMIN, BILLING, COMPLIANCE, ENGAGEMENT_MANAGER, MEMBER, PARTNER, RECEIVABLES,
    TAX_SPECIALIST, define,
};
use super::seed::PASSWORD;

mod clients;
mod engagements;
mod screens;
mod work;

pub use screens::{Screen, map as screen_map};

/// The command that fills a local database with the demo firm.
pub const COMMAND: &str = "accounting:demo";

/// The audit subject of the marker of a finished seed.
const MARKER_SUBJECT: &str = "accounting_demo";
const DONE_ACTION: &str = "accounting_demo.seeded";

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

/// The e-mail of the partner.
pub const PARTNER_EMAIL: &str = "partner@falconledger.example";
/// The e-mail of the company admin.
pub const ADMIN_EMAIL: &str = "admin@falconledger.example";
/// The e-mail of the engagement manager.
pub const MANAGER_EMAIL: &str = "manager@falconledger.example";
/// The e-mail of the accountant.
pub const ACCOUNTANT_EMAIL: &str = "accountant@falconledger.example";
/// The e-mail of the tax specialist.
pub const TAX_EMAIL: &str = "tax@falconledger.example";
/// The e-mail of the MLRO.
pub const MLRO_EMAIL: &str = "mlro@falconledger.example";
/// The e-mail of the billing admin.
pub const BILLING_EMAIL: &str = "billing@falconledger.example";
/// The e-mail of the client owner (portal), Marina Bay Trading.
pub const CLIENT_OWNER_EMAIL: &str = "tariq@marinabay.example";
/// The e-mail of the client owner (portal), Desert Rose Interiors.
pub const ROSE_OWNER_EMAIL: &str = "layla@desertrose.example";
/// The e-mail of the client finance contact (portal), Marina Bay Trading.
pub const CLIENT_FINANCE_EMAIL: &str = "finance@marinabay.example";

/// The demo people, one per role of the firm.
pub const LOGINS: &[Login] = &[
    Login {
        name: "Yousef Al Hashimi",
        email: PARTNER_EMAIL,
        role: PARTNER,
        label: "Partner",
    },
    Login {
        name: "Reem Kassab",
        email: ADMIN_EMAIL,
        role: ADMIN,
        label: "Company admin",
    },
    Login {
        name: "Priya Menon",
        email: MANAGER_EMAIL,
        role: ENGAGEMENT_MANAGER,
        label: "Engagement manager",
    },
    Login {
        name: "Joseph Mathew",
        email: ACCOUNTANT_EMAIL,
        role: ACCOUNTANT,
        label: "Accountant",
    },
    Login {
        name: "Hana Saeed",
        email: TAX_EMAIL,
        role: TAX_SPECIALIST,
        label: "Tax specialist",
    },
    Login {
        name: "Faisal Rahman",
        email: MLRO_EMAIL,
        role: COMPLIANCE,
        label: "MLRO (compliance)",
    },
    Login {
        name: "Grace D'Silva",
        email: BILLING_EMAIL,
        role: BILLING,
        label: "Billing admin",
    },
    Login {
        name: "Tariq Al Marri",
        email: CLIENT_OWNER_EMAIL,
        role: MEMBER,
        label: "Client owner (portal), Marina Bay Trading",
    },
    Login {
        name: "Leena Joseph",
        email: CLIENT_FINANCE_EMAIL,
        role: MEMBER,
        label: "Client finance contact (portal), Marina Bay Trading",
    },
    Login {
        name: "Layla Haddad",
        email: ROSE_OWNER_EMAIL,
        role: MEMBER,
        label: "Client owner (portal), Desert Rose Interiors",
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

/// The module that adds the command and the seeder.
pub struct AccountingDemo {
    client: wl_kernel::ClientConfig,
}

impl AccountingDemo {
    /// The module for the client `config`.
    pub fn new(config: wl_kernel::ClientConfig) -> Self {
        Self { client: config }
    }
}

impl Module for AccountingDemo {
    fn name(&self) -> &'static str {
        "accounting_demo"
    }

    fn register(&self, app: &mut Registry) {
        let client = self.client.clone();
        app.command(
            COMMAND,
            "Fill the database with the Falcon Ledger demo firm (clients, engagements, CDD, returns, AML cases, KPI packs, retainers); local and testing only",
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

/// `accounting:demo` and the seeder: boots the app a second time on the same
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
                "the demo firm is made in local or testing only, never in {env:?}"
            )
            .into());
        }
        return Ok(());
    }
    if state.config.database_url.contains(":memory:") {
        if said {
            return Err(renox::anyhow::anyhow!(
                "the demo firm needs a database file, not an in-memory database"
            )
            .into());
        }
        return Ok(());
    }
    let app = crate::app_for(client).map_err(|e| renox::anyhow::anyhow!("{e}"))?;
    let kernel = app.config((*state.config).clone()).boot().await?;
    if !seed(&kernel).await? {
        println!("The demo firm is there already.");
        println!("{}", screens::print(kernel.state()).await?);
        return Ok(());
    }
    println!("The demo firm is made. Log in with any of these (password {PASSWORD}):");
    for login in LOGINS {
        println!("  {:<44} {}", login.email, login.label);
    }
    println!("{}", screens::print(kernel.state()).await?);
    Ok(())
}

/// Seeds the demo firm into the database `kernel` is on; `false` when it is
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

/// One client company of the firm, with the people screened with it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Client {
    pub(crate) company: wl_kernel::PartyId,
    pub(crate) ubo: wl_kernel::PartyId,
    pub(crate) director: wl_kernel::PartyId,
}

/// What the steps share: the state, the driver and the ids they made.
pub(crate) struct Seed {
    pub(crate) state: AppState,
    pub(crate) driver: Driver,
    /// The clients by short key.
    pub(crate) clients: std::collections::BTreeMap<&'static str, Client>,
    /// The engagements by client key.
    pub(crate) engagements: std::collections::BTreeMap<&'static str, i64>,
}

impl Seed {
    fn new(state: AppState, driver: Driver) -> Self {
        Self {
            state,
            driver,
            clients: Default::default(),
            engagements: Default::default(),
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
        self.logins().await?;
        self.clients().await?;
        self.engagements().await?;
        self.work().await?;
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
                    if login.email == BILLING_EMAIL {
                        user.assign_role(self.db(), RECEIVABLES).await?;
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
