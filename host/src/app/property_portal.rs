//! The 01 Property tenant portal (01 §3.1, §3.3; wl-portal §4, §7;
//! BL-POR-02, BL-POR-03):
//!
//! - the audience `tenant` and its portal roles, seeded ([`TENANT`]):
//!   `primary_contact` (lease, finance, documents, requests, users) and
//!   `finance_contact` (sees and pays invoices only); the company admin
//!   changes their permissions in the portal settings;
//! - `POR.ACL.01`: a terminated lease (`LeaseTerminated`, which also follows
//!   a `force_terminate`, and `LeaseForceTerminated` itself) runs `end` on
//!   the tenant's grants, but only when the tenant has no lease left in
//!   `active`, `renewal_due`, `expired` (holding over) or `notice_given`;
//!   wl-portal's daily timer closes the grant 90 days later.
//!
//! The sections themselves (lease, invoices, cheques, deposit, documents,
//! requests) come with their modules' issues (BL-PRP-30, BL-INV-31,
//! BL-PAY-27, BL-TKT-14); [`SECTIONS`] is the order and permission each one
//! registers with.

use property::{Lease, LeaseForceTerminated, LeaseStatus, LeaseTerminated};
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_portal::{AudienceDef, PortalAccess, RoleDef};

/// The audience key of the tenants.
pub const TENANT_AUDIENCE: &str = "tenant";

/// The tenant audience of 01 (§3.1), closing 90 days after `end`
/// (`POR.ACL.01`).
pub const TENANT: AudienceDef = AudienceDef {
    key: TENANT_AUDIENCE,
    label: "Tenants",
    close_after_days: 90,
    roles: &[
        RoleDef {
            key: "primary_contact",
            label: "Primary contact",
            permissions: &[
                "portal.leases.view",
                "portal.invoices.view",
                "portal.invoices.pay",
                "portal.documents.view",
                "portal.requests.create",
                "portal.users.manage",
            ],
        },
        RoleDef {
            key: "finance_contact",
            label: "Finance contact",
            permissions: &["portal.invoices.view", "portal.invoices.pay"],
        },
    ],
};

/// The tenant sections: key, menu order and the portal permission each
/// needs, for the modules that register them.
pub const SECTIONS: &[(&str, i32, &str)] = &[
    ("lease", 10, "portal.leases.view"),
    ("invoices", 20, "portal.invoices.view"),
    ("cheques", 30, "portal.leases.view"),
    ("deposit", 40, "portal.leases.view"),
    ("documents", 50, "portal.documents.view"),
    ("requests", 60, "portal.requests.create"),
];

/// Seeds the tenant audience where the database lacks it; a role the
/// company admin changed keeps its permissions.
///
/// # Errors
///
/// The database's errors.
pub async fn install(state: &AppState) -> Result<usize> {
    wl_portal::audiences::seed(&state.db, &TENANT).await
}

/// A lease of `tenant` ended: `end` on the tenant's grants unless another
/// lease of theirs still runs. Returns how many grants turned read-only.
///
/// # Errors
///
/// The database's errors.
pub async fn lease_ended(state: &AppState, tenant: PartyId, reason: &str) -> Result<usize> {
    let running = Lease::query()
        .where_(Lease::TENANT_PARTY_ID.eq(tenant.0))
        .where_(Lease::STATUS.is_in(LeaseStatus::RUNNING))
        .first(&state.db)
        .await?;
    if running.is_some() {
        return Ok(0);
    }
    let ended =
        PortalAccess::contract_ended(state, tenant, TENANT_AUDIENCE, today(), reason).await?;
    Ok(ended.len())
}

/// The module: the seed, its command and the contract-end listeners. The
/// host mounts it for a Property client with the portal on.
pub struct PropertyPortal;

impl Module for PropertyPortal {
    fn name(&self) -> &'static str {
        "property_portal"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "property:portal-install",
            "Store the tenant portal audience and its roles, where the database lacks them",
            |_args, state: AppState| async move {
                let created = install(&state).await?;
                println!("{created} tenant portal rows created.");
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { install(&state).await.map(|_| ()) });
        app.listen(|event: LeaseTerminated, state: AppState| async move {
            lease_ended(&state, event.tenant, "lease terminated")
                .await
                .map(|_| ())
        });
        app.listen(|event: LeaseForceTerminated, state: AppState| async move {
            lease_ended(&state, event.tenant, "lease force-terminated")
                .await
                .map(|_| ())
        });
    }
}
