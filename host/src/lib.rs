//! Host: the one binary every client runs. It reads the client's
//! `client.toml` (the path in `CLIENT_CONFIG`), applies the kernel (branding,
//! legal entities, the module manifest) and mounts the vertical app and the
//! enabled `wl-*` modules (ARCHITECTURE.md §7).
//!
//! It started from Renox's starter kit (`rnx new --starter`):
//!
//! - login, password reset, email verification and the account
//!   pages (`Auth`), with the notification bell;
//! - roles and permissions (`Permissions`): `admin` manages users and reads
//!   the activity log, `member` is everyone else (`app::roles`);
//! - the activity log (`Audit`): logins and account changes, and what this
//!   app records with `renox::audit::record`;
//! - a dashboard, the users page and the activity page in the kit's sidebar
//!   layout.
//!
//! Make someone an admin with `cargo run -- users:admin you@example.com`, or
//! seed a local database: `cargo run -- db:seed` (admin@example.com,
//! password123).

mod app;

use renox::audit::Audit;
use renox::auth::{Auth, Permissions};
use renox::nav::NavLink;

pub use app::demo_data;
pub use app::negative_flow::{invoicing_for, payments_for};
pub use app::roles;

use wl_invoicing::WlInvoicing;
use wl_kernel::{ClientConfig, ConfigError};
use wl_notify::WlNotify;

/// The variable naming the client's `client.toml`.
pub const CLIENT_CONFIG: &str = "CLIENT_CONFIG";

/// The demo client, used when `CLIENT_CONFIG` is unset outside production.
pub const DEMO_CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-properties/client.toml"
);

/// The application for the client `CLIENT_CONFIG` names (`.env` is read
/// first), or the demo client outside production. `main.rs` runs it.
pub fn try_app() -> Result<renox::App, ConfigError> {
    let _ = renox::Config::load();
    let path = match std::env::var(CLIENT_CONFIG) {
        Ok(path) if !path.is_empty() => path,
        _ if std::env::var("APP_ENV").is_ok_and(|env| env.starts_with("prod")) => {
            return Err(ConfigError(format!(
                "{CLIENT_CONFIG} is not set: name the client's client.toml"
            )));
        }
        _ => DEMO_CLIENT.to_owned(),
    };
    app_for(ClientConfig::load(path)?)
}

/// Commands that wipe or fill the database with demo data. In production
/// they are refused before anything is booted (`main.rs`): `migrate:fresh`
/// drops every table, `db:seed` and `property:demo` make the demo logins
/// (admin@example.com / password123).
pub const DEMO_COMMANDS: &[&str] = &["migrate:fresh", "db:seed", demo_data::COMMAND];

/// The refusal for `command` when `app_env` is a production environment.
///
/// # Errors
///
/// The message to print, when the command is one of [`DEMO_COMMANDS`] and
/// `app_env` starts with `prod`.
pub fn refuse_in_production(command: Option<&str>, app_env: Option<&str>) -> Result<(), String> {
    let production = app_env.is_some_and(|env| env.trim().to_lowercase().starts_with("prod"));
    match command {
        Some(command) if production && DEMO_COMMANDS.contains(&command) => Err(format!(
            "{command} is refused when APP_ENV is production: it would {} \
             Add people with `users:invite`; to start over, restore a backup.",
            if command == "migrate:fresh" {
                "drop every table of the live database."
            } else {
                "put the demo company and its logins (admin@example.com / password123) in the live database."
            }
        )),
        _ => Ok(()),
    }
}

/// [`try_app`], for tests: the demo client unless `CLIENT_CONFIG` says
/// otherwise.
///
/// # Panics
///
/// When the client's file is refused; the message says why.
pub fn app() -> renox::App {
    try_app().unwrap_or_else(|err| panic!("{err}"))
}

/// The application for one client.
pub fn app_for(config: ClientConfig) -> Result<renox::App, ConfigError> {
    app_for_tuned(config, Tuning::default())
}

/// What a client sets on the shared modules besides the host's values, in
/// code: the e-invoicing go-live date of each legal entity and its ASP
/// adapter (`WlInvoicing::einvoicing`, `WlInvoicing::asp_adapter`; BL-PRP-26
/// item 6), or the channel providers of the notifications
/// (`WlNotify::provider`). Each closure runs on the module with the host's
/// values already set, and only when the module is enabled.
#[derive(Default)]
pub struct Tuning {
    invoicing: Option<Box<dyn FnOnce(WlInvoicing) -> WlInvoicing>>,
    notify: Option<Box<dyn FnOnce(WlNotify) -> WlNotify>>,
}

impl Tuning {
    /// Changes the invoicing module.
    pub fn invoicing(mut self, tune: impl FnOnce(WlInvoicing) -> WlInvoicing + 'static) -> Self {
        self.invoicing = Some(Box::new(tune));
        self
    }

    /// Changes the notification module.
    pub fn notify(mut self, tune: impl FnOnce(WlNotify) -> WlNotify + 'static) -> Self {
        self.notify = Some(Box::new(tune));
        self
    }
}

/// [`app_for`] with the shared modules changed by `tuning`.
pub fn app_for_tuned(config: ClientConfig, tuning: Tuning) -> Result<renox::App, ConfigError> {
    let mut tuning = tuning;
    let client_app = config.client.app.clone();
    let vertical = vertical(&client_app)?;
    let manifest = config.manifest()?;
    let entities: Vec<String> = config
        .legal_entities
        .iter()
        .map(|e| e.code.clone())
        .collect();
    let owner_role = config.owner_role().to_owned();
    // What a block does to a lease and to a service contract (BL-PRP-26).
    app::negative_flow::register_block_effects(&client_app);
    app::negative_flow::register_approvals(&client_app);
    // The kernel goes first: its seeder creates the legal entities the other
    // modules' demo data needs, and seeders run in the order modules mount.
    let name = config.client.name.clone();
    let demo_client = config.clone();
    let mut app = wl_kernel::apply(base(), config)?;
    app = with_brand(app, &name);
    for module in manifest.enabled() {
        app = enable(
            app,
            module,
            &client_app,
            &entities,
            &owner_role,
            &mut tuning,
        )?;
    }
    // The Property client's categories, checklists and SOPs go in after the
    // modules' own seeds (BL-PRP-27).
    if client_app == "property" {
        app = app.module(app::property_seed::PropertySeed);
        // The demo company, after the installs: `property:demo`, and `db:seed`
        // in local (never in production).
        app = app.module(app::demo_data::DemoSeed::new(demo_client));
    }
    // The page that shows the shared modules' settings of this client.
    Ok(app
        .module(app::negative_flow::NegativeFlow::new(&client_app, entities))
        .module(app::compliance_settings::ComplianceSettings::new(
            &client_app,
            &owner_role,
        ))
        .module(vertical))
}

/// The client's name is the app's name: `{{ brand }}` in the layouts (the
/// sidebar, the public pages, the sign-in pages, the error pages), and the
/// configuration's `name` for what Renox itself titles with it (the sign-in
/// pages' `<title>`, the mails' sender name, the social cards). `APP_NAME` is
/// not read for it: the name belongs to the client's `client.toml`.
fn with_brand(app: renox::App, name: &str) -> renox::App {
    let shared = name.to_owned();
    let app = app.share("brand", move |_| {
        let name = shared.clone();
        async move { Ok(name) }
    });
    // The tests' own configuration replaces this one, which is why the
    // layouts read `brand` and not `app.name`.
    match renox::Config::load() {
        Ok(mut config) => {
            config.name = name.to_owned();
            app.config(config)
        }
        Err(_) => app,
    }
}

/// The vertical app named in `[client] app`.
fn vertical(name: &str) -> Result<property::Property, ConfigError> {
    match name {
        "property" => Ok(property::Property),
        other => Err(ConfigError(format!(
            "[client] app `{other}` is not built yet; F1 has `property`"
        ))),
    }
}

/// Mounts one enabled `wl-*` module; `client_app` is `[client] app`.
fn enable(
    app: renox::App,
    module: &str,
    client_app: &str,
    entities: &[String],
    owner_role: &str,
    tuning: &mut Tuning,
) -> Result<renox::App, ConfigError> {
    // Each F1 module gets its arm here as it is built (decision 0004).
    match module {
        "parties" => Ok(app.module(wl_parties::WlParties)),
        "documents" => {
            app::property_seed::register_documents(client_app);
            Ok(app.module(wl_documents::WlDocuments))
        }
        "notify" => {
            let module = WlNotify::new();
            Ok(app.module(match tuning.notify.take() {
                Some(tune) => tune(module),
                None => module,
            }))
        }
        "invoicing" => {
            let module = app::negative_flow::invoicing_for(client_app, entities);
            Ok(app.module(match tuning.invoicing.take() {
                Some(tune) => tune(module),
                None => module,
            }))
        }
        "ticketing" => Ok(app.module(app::property_seed::ticketing_for(client_app))),
        "payments" => Ok(app.module(app::negative_flow::payments_for(client_app))),
        "sales" => Ok(app.module(app::property_seed::sales_for(client_app))),
        "hr" => Ok(app.module(app::property_seed::hr_for(client_app, owner_role))),
        "portal" => Ok(app.module(wl_portal::WlPortal)),
        other => Err(ConfigError(format!(
            "[modules] `{other}` is not built yet; remove it from client.toml for now"
        ))),
    }
}

/// What every client runs, whatever their modules: auth, roles, the audit
/// log and the signed-in shell.
fn base() -> renox::App {
    renox::App::new()
        .embed(renox::embedded!())
        .migrations(renox::migrations!())
        .module(
            Auth::new()
                .verify_email() // a link by mail before the dashboard opens
                .account() // /account: name, email, password, devices
                .notifications() // the bell and /notifications
                .magic_link() // "Email me a login link" on the login page: no password needed
                .redirect_to("/dashboard")
                // No public sign-up: an admin adds the people (`/users`,
                // `users:invite`), the first one is made with the command.
                .without_registration(),
        )
        .module(Permissions)
        .module(Audit)
        .module(app::home::Home)
        .module(app::dashboard::Dashboard)
        .module(app::users::Users)
        .module(app::activity::Activity)
        .command(
            "users:invite",
            "Add a user and mail them the link to choose a password",
            app::roles::invite,
        )
        .command(
            "users:admin",
            "Give the user with this email the admin role",
            app::roles::make_admin,
        )
        // The links of the Auth module's pages, in the sidebar menu the other
        // modules add to (`<rx-sidebar menu="app"/>`, layouts/app.html).
        .nav(
            "app",
            NavLink::route("notifications.index", "nav.notifications")
                .icon("bell")
                .order(1)
                .when(|_| true),
        )
        .nav(
            "app",
            NavLink::route("account.show", "nav.account")
                .icon("circle-user")
                .group("nav.app")
                .order(21)
                .when(|_| true),
        )
        .seeder(app::seed::run)
        // The seeded people on the login page, in local only (src/app/demo.rs).
        .share("demo_logins", app::demo::for_view)
}
