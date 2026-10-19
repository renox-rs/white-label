//! The refusal log (wl-portal §6, `PTY.ACL.02`, issue #44): one entry per
//! portal action refused because the party the user acts for is blocked
//! (portal user, party, action, time). Only staff with
//! `parties.block_reason.view` see it; the party and other staff do not. It
//! holds no reason: the portal never learns it.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /portal/block-refusals` | `wl_portal.block_refusals` | `parties.block_reason.view` |

use std::collections::BTreeMap;

use renox::chrono::FixedOffset;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use crate::guard::PortalAction;
use crate::{permissions, texts};

/// One refused portal action.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_portal_refusal_log", index(party_id))]
pub struct RefusalLog {
    /// The primary key.
    pub id: i64,
    /// The portal user.
    pub user_id: i64,
    /// The party the user acts for.
    pub party_id: i64,
    /// The refused action's key.
    pub action: String,
    /// When the row was created: when the action was refused.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RefusalLog {
    /// Logs the refusal of `action` for `user` acting for `party`, inside
    /// `tx`.
    pub async fn record(
        tx: &mut Transaction,
        user: i64,
        party: PartyId,
        action: PortalAction,
    ) -> Result {
        RefusalLog::create(
            &mut *tx,
            RefusalLog {
                user_id: user,
                party_id: party.0,
                action: action.key().to_owned(),
                ..RefusalLog::default()
            },
        )
        .await?;
        Ok(())
    }

    /// The entries, newest first.
    pub async fn latest(db: &Db, limit: u64) -> Result<Vec<RefusalLog>> {
        RefusalLog::query()
            .order_by_desc(RefusalLog::ID)
            .limit(limit)
            .get(db)
            .await
    }
}

/// `at` as `YYYY-MM-DD HH:MM` in Asia/Dubai (UTC+4, no daylight saving).
fn dubai(at: DateTime) -> String {
    let zone = FixedOffset::east_opt(4 * 3600).expect("a valid offset");
    at.with_timezone(&zone).format("%Y-%m-%d %H:%M").to_string()
}

/// The texts a template reads as `tx['refusals.title']`.
type Texts = BTreeMap<&'static str, String>;

/// One row of the log page.
#[derive(Serialize, Debug, Clone)]
pub struct EntryRow {
    /// The primary key.
    pub id: i64,
    /// When it was refused (Asia/Dubai).
    pub at: String,
    /// The portal user's name.
    pub user: String,
    /// The party's name.
    pub party: String,
    /// The action, in words.
    pub action: String,
}

/// What the log page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-portal/refusal_log.html")]
pub struct RefusalLogPage {
    tx: Texts,
    entries: Vec<EntryRow>,
}

/// The routes of the refusal log.
pub fn routes() -> Routes {
    Routes::new()
        .get("/portal/block-refusals", index)
        .name("wl_portal.block_refusals")
        .require_permission(permissions::BLOCK_REASON_VIEW)
}

/// `GET /portal/block-refusals`: the log, newest first.
async fn index(State(state): State<AppState>, lang: Lang) -> Result<View> {
    let mut entries = Vec::new();
    for entry in RefusalLog::latest(&state.db, 500).await? {
        let user = User::find(&state.db, entry.user_id)
            .await?
            .map_or_else(|| format!("#{}", entry.user_id), |u| u.name);
        let party = wl_kernel::parties::directory(&state)
            .display_name(&state.db, PartyId(entry.party_id), &lang.locale)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| format!("#{}", entry.party_id));
        entries.push(EntryRow {
            id: entry.id,
            at: entry.created_at.map(dubai).unwrap_or_default(),
            user,
            party,
            action: texts::t(&lang, &format!("wl_portal.action.{}", entry.action)),
        });
    }
    Ok(View::typed(RefusalLogPage {
        tx: texts::all(&lang),
        entries,
    }))
}
