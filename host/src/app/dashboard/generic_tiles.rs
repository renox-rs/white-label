//! The figures of a client whose vertical app has no figures of its own (the
//! accounting firm's, for now): what the shared modules give, each left out
//! when the person lacks its page's permission or the module is off. Nothing
//! of Property is read, so the page opens on a database without its tables.

use renox::prelude::*;

use super::{Figures, cheque_tiles, invoice_tile, ticket_tiles};

/// The shared modules' figures `user` may see.
pub(super) async fn figures(state: &AppState, user: &AuthUser) -> Result<Figures> {
    let mut out = Figures {
        lead: "dashboard.lead_generic",
        ..Figures::default()
    };
    cheque_tiles(state, user, true, &mut out.tiles).await?;
    invoice_tile(state, user, &mut out.tiles).await?;
    ticket_tiles(state, user, true, &mut out).await?;
    Ok(out)
}
