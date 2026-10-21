//! The figures of the Property client: units, leases, cheques, invoices,
//! tickets and the compliance flags of the buildings.

use renox::prelude::*;
use wl_kernel::documents::today;

use super::{Figures, Tile, cheque_tiles, invoice_tile, number, ticket_tiles};

/// The figures `user` may see in a Property client.
pub(super) async fn figures(state: &AppState, user: &AuthUser) -> Result<Figures> {
    let mut out = Figures {
        lead: "dashboard.lead",
        ..Figures::default()
    };
    // The vertical's own: units, leases, compliance (per building).
    let property = ::property::dashboard::figures(state, user, today()).await?;
    if let Some(n) = property.units_available {
        out.tiles.push(Tile::new(
            "dashboard.tiles.units_available",
            number(n),
            "/property",
            "building-2",
        ));
    }
    if let Some(n) = property.units_leased {
        out.tiles.push(
            Tile::new(
                "dashboard.tiles.units_leased",
                number(n),
                "/property",
                "house",
            )
            .toned("clay"),
        );
    }
    if let Some(n) = property.leases_expiring {
        out.tiles.push(
            Tile::new(
                "dashboard.tiles.leases_expiring",
                number(n),
                "/property/leases",
                "calendar-days",
            )
            .toned("sand"),
        );
    }
    out.expiring = property.expiring;
    cheque_tiles(state, user, true, &mut out.tiles).await?;
    invoice_tile(state, user, &mut out.tiles).await?;
    ticket_tiles(state, user, true, &mut out).await?;
    if let Some(n) = property.buildings_flagged {
        out.tiles.push(
            Tile::new(
                "dashboard.tiles.compliance_flags",
                number(n),
                "/property/compliance",
                "shield",
            )
            .toned("clay"),
        );
    }
    Ok(out)
}
