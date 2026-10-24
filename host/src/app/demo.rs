//! The seeded people on the login page (`resources/views/renox/auth/login_options.html`),
//! so trying the app locally is one tap: shared with every view as
//! `demo_logins` (`src/lib.rs`).
//!
//! It is a list only on `/login`, with `APP_ENV=local`, and once `db:seed`
//! made the people (`super::seed::PEOPLE`, and the staff of the demo companies,
//! `super::demo_data::LOGINS` and the other verticals' `LOGINS` (the EdTech centre's included), one per role); anywhere else it is `none`, so
//! a server in production never shows it, whatever its database holds.

use renox::Environment;
use renox::prelude::*;
use serde::Serialize;

use super::seed::{PASSWORD, PEOPLE};

/// One person to log in as.
#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub name: &'static str,
    pub email: &'static str,
    /// What the person is, in words for the page.
    pub label: &'static str,
}

/// What the login page shows: the people and their password.
#[derive(Debug, Clone, Serialize)]
pub struct Demo {
    pub password: &'static str,
    pub accounts: Vec<Account>,
}

/// The `demo_logins` view value.
pub async fn for_view(ctx: renox::view::ViewContext) -> Result<Option<Demo>> {
    if ctx.path != "/login" || ctx.state.config.env != Environment::Local {
        return Ok(None);
    }
    let mut accounts = Vec::new();
    for login in super::demo_data::LOGINS {
        if User::find_by_email(&ctx.state.db, login.email)
            .await?
            .is_some()
        {
            accounts.push(Account {
                name: login.name,
                email: login.email,
                label: login.label,
            });
        }
    }
    for login in super::construction_demo::LOGINS {
        if User::find_by_email(&ctx.state.db, login.email)
            .await?
            .is_some()
        {
            accounts.push(Account {
                name: login.name,
                email: login.email,
                label: login.label,
            });
        }
    }
    for login in super::logistics_demo::LOGINS {
        if User::find_by_email(&ctx.state.db, login.email)
            .await?
            .is_some()
        {
            accounts.push(Account {
                name: login.name,
                email: login.email,
                label: login.label,
            });
        }
    }
    for login in super::fnb_demo::LOGINS {
        if User::find_by_email(&ctx.state.db, login.email)
            .await?
            .is_some()
        {
            accounts.push(Account {
                name: login.name,
                email: login.email,
                label: login.label,
            });
        }
    }
    for login in super::manufacturing_demo::LOGINS {
        if User::find_by_email(&ctx.state.db, login.email)
            .await?
            .is_some()
        {
            accounts.push(Account {
                name: login.name,
                email: login.email,
                label: login.label,
            });
        }
    }
    for login in super::manpower_demo::LOGINS {
        if User::find_by_email(&ctx.state.db, login.email)
            .await?
            .is_some()
        {
            accounts.push(Account {
                name: login.name,
                email: login.email,
                label: login.label,
            });
        }
    }
    for login in super::clinics_demo::LOGINS {
        if User::find_by_email(&ctx.state.db, login.email)
            .await?
            .is_some()
        {
            accounts.push(Account {
                name: login.name,
                email: login.email,
                label: login.label,
            });
        }
    }
    for login in super::edtech_demo::LOGINS {
        if User::find_by_email(&ctx.state.db, login.email)
            .await?
            .is_some()
        {
            accounts.push(Account {
                name: login.name,
                email: login.email,
                label: login.label,
            });
        }
    }
    for login in super::auto_demo::LOGINS {
        if User::find_by_email(&ctx.state.db, login.email)
            .await?
            .is_some()
        {
            accounts.push(Account {
                name: login.name,
                email: login.email,
                label: login.label,
            });
        }
    }
    for &(name, email, role) in PEOPLE {
        if User::find_by_email(&ctx.state.db, email).await?.is_some() {
            accounts.push(Account {
                name,
                email,
                label: if role == super::roles::ADMIN {
                    "Starter admin"
                } else {
                    "Member"
                },
            });
        }
    }
    Ok((!accounts.is_empty()).then_some(Demo {
        password: PASSWORD,
        accounts,
    }))
}
