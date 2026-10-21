//! The Manpower client's values in the shared modules (09 Manpower; BL-MPW-14,
//! BL-MPW-17): the ticket categories the vertical opens tickets in
//! (`manpower::seed::CATEGORIES`: the complaint about a worker's conduct and
//! the absence case of HR), stored where the database lacks them by the
//! seeder and by the command `manpower:install`. A re-run changes nothing and
//! a category the client renamed keeps its name.

use std::sync::Arc;

use manpower::seed::CATEGORIES;
use renox::prelude::*;
use wl_kernel::Manifest;
use wl_ticketing::config::InvoiceOn;
use wl_ticketing::{Category, Priority, SlaPolicy};

/// The client app these values are for.
pub const MANPOWER: &str = "manpower";

/// Stores the ticket categories of 09 that the database lacks when ticketing
/// is on. Returns how many were created.
///
/// # Errors
///
/// The database's errors.
pub async fn install(state: &AppState) -> Result<usize> {
    let manifest = state.provided::<Arc<Manifest>>();
    if !manifest
        .as_ref()
        .is_some_and(|manifest| manifest.is_enabled("ticketing"))
    {
        return Ok(0);
    }
    let policy = SlaPolicy::by_key(&state.db, wl_ticketing::seed::STANDARD_POLICY).await?;
    let mut created = 0;
    for spec in &CATEGORIES {
        if Category::by_key(&state.db, spec.key).await?.is_some() {
            continue;
        }
        Category::create(
            &state.db,
            Category {
                key: spec.key.to_owned(),
                name_en: spec.name_en.to_owned(),
                name_ar: spec.name_ar.to_owned(),
                default_priority: Priority::Normal,
                sla_policy_id: policy.as_ref().map(|p| p.id),
                requires_signoff: false,
                active: true,
                invoice_on: InvoiceOn::default(),
                default_assignee_role: spec.assignee_role.map(str::to_owned),
                ..Category::default()
            },
        )
        .await?;
        created += 1;
    }
    Ok(created)
}

/// The module that seeds the Manpower client's values: mounted after the
/// shared modules, so their own seeds (the standard SLA policy) are there
/// first.
pub struct ManpowerSeed;

impl Module for ManpowerSeed {
    fn name(&self) -> &'static str {
        "manpower_seed"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "manpower:install",
            "Store the Manpower ticket categories (conduct, absence), where the database lacks them",
            |_args, state: AppState| async move {
                let created = install(&state).await?;
                println!("{created} Manpower defaults created.");
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { install(&state).await.map(|_| ()) });
    }
}
