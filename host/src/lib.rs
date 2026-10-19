//! Host: the one binary every client runs. It reads the client's
//! `client.toml` (the path in `CLIENT_CONFIG`), applies the kernel (branding,
//! legal entities, the module manifest) and mounts the vertical app and the
//! enabled `wl-*` modules (ARCHITECTURE.md §7).
//!
//! It started from Renox's starter kit (`rnx new --starter`):
//!
//! - sign-up, login, password reset, email verification and the account
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

pub use app::roles;

use wl_kernel::{ClientConfig, ConfigError};

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
    let client_app = config.client.app.clone();
    let vertical = vertical(&client_app)?;
    let manifest = config.manifest()?;
    // The kernel goes first: its seeder creates the legal entities the other
    // modules' demo data needs, and seeders run in the order modules mount.
    let mut app = wl_kernel::apply(base(), config)?;
    for module in manifest.enabled() {
        app = enable(app, module, &client_app)?;
    }
    Ok(app.module(vertical))
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

/// The invoicing module with the settings of the client's app: `property`
/// also holds a supplier bill for the same contract, period and amount
/// (`PRP.ACL.22`).
fn invoicing_for(client_app: &str) -> wl_invoicing::WlInvoicing {
    use wl_invoicing::DuplicateKey::{ContractPeriodAmount, SupplierInvoiceNumber};
    let module = wl_invoicing::WlInvoicing::new();
    match client_app {
        "property" => module.duplicate_keys(&[SupplierInvoiceNumber, ContractPeriodAmount]),
        _ => module,
    }
}

/// Mounts one enabled `wl-*` module; `client_app` is `[client] app`.
fn enable(app: renox::App, module: &str, client_app: &str) -> Result<renox::App, ConfigError> {
    // Each F1 module gets its arm here as it is built (decision 0004).
    match module {
        "parties" => Ok(app.module(wl_parties::WlParties)),
        "documents" => Ok(app.module(wl_documents::WlDocuments)),
        "notify" => Ok(app.module(wl_notify::WlNotify::new())),
        "invoicing" => Ok(app.module(invoicing_for(client_app))),
        "ticketing" => Ok(app.module(wl_ticketing::WlTicketing::new())),
        "payments" => Ok(app.module(wl_payments::WlPayments::new())),
        "sales" => Ok(app.module(wl_sales::WlSales::new())),
        "hr" => Ok(app.module(wl_hr::WlHr::new())),
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
                // Everyone who signs up is a member.
                .on_registered(|user, _form, state| async move {
                    roles::define(&state.db).await?;
                    user.assign_role(&state.db, roles::MEMBER).await
                }),
        )
        .module(Permissions)
        .module(Audit)
        .module(app::home::Home)
        .module(app::dashboard::Dashboard)
        .module(app::users::Users)
        .module(app::activity::Activity)
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

#[cfg(test)]
mod tests {
    use super::*;
    use wl_invoicing::DuplicateKey::{ContractPeriodAmount, SupplierInvoiceNumber};

    #[test]
    fn the_property_app_adds_the_contract_period_amount_key() {
        assert_eq!(
            invoicing_for("property").config().duplicate_keys,
            [SupplierInvoiceNumber, ContractPeriodAmount]
        );
        assert_eq!(
            invoicing_for("other").config().duplicate_keys,
            [SupplierInvoiceNumber]
        );
    }
}
