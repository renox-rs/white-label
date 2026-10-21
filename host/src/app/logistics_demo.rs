//! The demo company of the Logistics client (DEMO-DATA.md, 03: *Najm Express
//! Cargo LLC*): `logistics:demo` fills a local database with the people of
//! the company, a client with its prices and a few booked shipments, and
//! `db:seed` runs it in `local`, never anywhere else.
//!
//! The shipments are booked through the app's own booking service
//! (`logistics::seed`), so the numbering, the labels and the delivery request
//! are the real ones. Seeding twice changes nothing: a person who exists is
//! kept, and a shipment whose client reference is used is not booked again.
//! The people log in with the password of [`super::seed::PASSWORD`].

use logistics::seed::{self, DEMO_CLIENT};
use renox::Environment;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::PartyKind;

use super::roles::{
    self, CUSTOMER_SERVICE, DISPATCHER, DRIVER, HUB_STAFF, OPERATIONS_MANAGER, SHIPPING_CLIENT,
};
use super::seed::PASSWORD;

/// The command that fills a local database with the demo company.
pub const COMMAND: &str = "logistics:demo";

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

/// The demo people, one per role.
pub const LOGINS: &[Login] = &[
    Login {
        name: "Khalid Mansoori",
        email: "operations@najm.example",
        role: OPERATIONS_MANAGER,
        label: "Operations manager",
    },
    Login {
        name: "Dina Farouk",
        email: "dispatcher@najm.example",
        role: DISPATCHER,
        label: "Dispatcher",
    },
    Login {
        name: "Hassan Qadri",
        email: "hub@najm.example",
        role: HUB_STAFF,
        label: "Hub staff",
    },
    Login {
        name: "Imran Siddiqui",
        email: "driver@najm.example",
        role: DRIVER,
        label: "Driver",
    },
    Login {
        name: "Layla Hamdan",
        email: "service@najm.example",
        role: CUSTOMER_SERVICE,
        label: "Customer service",
    },
    Login {
        name: "Noor Haddad",
        email: "shop@souqexpress.example",
        role: SHIPPING_CLIENT,
        label: "Client user (online shop)",
    },
];

/// The module that adds the command and the seeder.
pub struct LogisticsDemo;

impl Module for LogisticsDemo {
    fn name(&self) -> &'static str {
        "logistics_demo"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            COMMAND,
            "Fill the database with the Najm Express demo company (people, a client with prices, shipments); local and testing only",
            |_args, state: AppState| async move { run(state, true).await },
        );
        app.seeder(|state: AppState| async move { run(state, false).await });
    }
}

async fn run(state: AppState, forced: bool) -> Result {
    match state.config.env {
        Environment::Production => {
            return Err(
                renox::anyhow::anyhow!("the demo company is never made in production").into(),
            );
        }
        Environment::Local => {}
        _ if forced => {}
        _ => return Ok(()),
    }
    let db = &state.db;
    roles::define(db).await?;
    let mut booker = None;
    for login in LOGINS {
        let user = match User::find_by_email(db, login.email).await? {
            Some(user) => user,
            None => {
                let mut user = User::register(db, login.name, login.email, PASSWORD).await?;
                user.email_verified_at = Some(renox::db::now());
                user.save(db).await?;
                user.assign_role(db, login.role).await?;
                user
            }
        };
        if login.role == DISPATCHER {
            booker = Some(user.id);
        }
    }
    let existing = wl_parties::Party::query()
        .where_(wl_parties::Party::LEGAL_NAME_EN.eq(DEMO_CLIENT))
        .first(db)
        .await?;
    let client = match existing {
        Some(party) => PartyId(party.id),
        None => {
            let party = wl_parties::create(
                &state,
                wl_parties::NewParty {
                    kind: PartyKind::Company,
                    legal_name_en: DEMO_CLIENT.to_owned(),
                    source: "seed".to_owned(),
                    ..wl_parties::NewParty::default()
                },
            )
            .await?;
            PartyId(party.id)
        }
    };
    seed::demo_rates(db, client).await?;
    if let Some(booker) = booker {
        let booked = seed::demo_shipments(&state, booker, client).await?;
        println!("{booked} demo shipments booked for {DEMO_CLIENT}.");
    }
    Ok(())
}
