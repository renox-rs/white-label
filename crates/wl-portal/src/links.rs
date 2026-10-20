//! Signed links into the portal (wl-portal §5.4, issue #44): a link a module
//! sends by SMS, e-mail or QR code that works without a login until it
//! expires.
//!
//! [`SignedLinks::issue`] signs `GET /portal/link/{party}/{action}`. Opening
//! it checks the signature (403 when it is altered or expired), then asks the
//! portal guard ([`PortalGuard::require_link`]): the link for paying an
//! invoice keeps working while the party is blocked, a link whose action
//! starts new business or moves money out shows the same neutral text as the
//! pages do and creates nothing. When the action is allowed the page says so
//! and `SignedLinkUsed` is emitted; the owning module's own page does the
//! work after the guard.

use std::collections::BTreeMap;
use std::time::Duration;

use renox::prelude::*;
use renox::signed::ValidSignature;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use crate::guard::{PortalAction, PortalGuard};
use crate::texts;

/// A signed link was opened and its action is allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedLinkUsed {
    /// The party the link is for.
    pub party: PartyId,
    /// The action key, e.g. `pay_invoice`.
    pub action: String,
}

impl Event for SignedLinkUsed {}

/// Issues signed links.
pub struct SignedLinks;

impl SignedLinks {
    /// A signed absolute link for `action` of `party`, valid for `ttl`.
    ///
    /// # Errors
    ///
    /// When the route is not registered.
    pub fn issue(
        state: &AppState,
        party: PartyId,
        action: PortalAction,
        ttl: Duration,
    ) -> Result<String> {
        state.signed_url("wl_portal.link", &[&party.0, &action.key()], ttl)
    }
}

/// What the link page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-portal/link.html")]
pub struct LinkPage {
    tx: BTreeMap<&'static str, String>,
}

/// The routes of the signed links (public: the signature is the credential).
pub fn routes() -> Routes {
    Routes::new()
        .get("/portal/link/{party}/{action}", open)
        .name("wl_portal.link")
}

/// `GET /portal/link/{party}/{action}`.
async fn open(
    _signature: ValidSignature,
    State(state): State<AppState>,
    lang: Lang,
    Path((party, action)): Path<(i64, String)>,
) -> Result<View> {
    let action = PortalAction::from_key(&action).ok_or(Error::NotFound)?;
    PortalGuard::require_link(&state, &lang, PartyId(party), action).await?;
    state
        .emit(SignedLinkUsed {
            party: PartyId(party),
            action: action.key().to_owned(),
        })
        .await?;
    Ok(View::typed(LinkPage {
        tx: texts::all(&lang),
    }))
}
