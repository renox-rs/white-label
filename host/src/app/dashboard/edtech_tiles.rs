//! The figures of the EdTech client: the sessions today, the enrolments waiting
//! for payment, the batches open, the certificates to issue and the sessions
//! flagged, then the shared modules' figures (cheques, invoices, tickets).

use renox::prelude::*;
use wl_kernel::documents::today;

use super::{Figures, Tile, cheque_tiles, invoice_tile, number, ticket_tiles};

/// The figures `user` may see in an EdTech client.
pub(super) async fn figures(state: &AppState, user: &AuthUser) -> Result<Figures> {
    let mut out = Figures {
        lead: "dashboard.lead_edtech",
        ..Figures::default()
    };
    let own = ::edtech::dashboard::figures(state, user, today()).await?;
    if let Some(n) = own.sessions_today {
        out.tiles.push(Tile::new(
            "dashboard.tiles.sessions_today",
            number(n),
            "/edtech/sessions",
            "calendar-days",
        ));
    }
    if let Some(n) = own.enrolments_waiting {
        out.tiles.push(
            Tile::new(
                "dashboard.tiles.enrolments_waiting",
                number(n),
                "/edtech/enrolments",
                "clipboard-list",
            )
            .toned("sand"),
        );
    }
    if let Some(n) = own.batches_open {
        out.tiles.push(Tile::new(
            "dashboard.tiles.batches_open",
            number(n),
            "/edtech/batches",
            "users",
        ));
    }
    if let Some(n) = own.certificates_draft {
        out.tiles.push(Tile::new(
            "dashboard.tiles.certificates_draft",
            number(n),
            "/edtech/certificates",
            "award",
        ));
    }
    if let Some(n) = own.sessions_flagged {
        out.tiles.push(
            Tile::new(
                "dashboard.tiles.sessions_flagged",
                number(n),
                "/edtech/sessions",
                "shield",
            )
            .toned("clay"),
        );
    }
    cheque_tiles(state, user, true, &mut out.tiles).await?;
    invoice_tile(state, user, &mut out.tiles).await?;
    ticket_tiles(state, user, true, &mut out).await?;
    Ok(out)
}
