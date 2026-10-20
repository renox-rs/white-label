//! The screening checks of engagement `activate` and CDD `submit` and
//! `approve` (`PTY.RUL.02`, BL-ACC-01, BL-ACC-08), read through the kernel's
//! [`ScreeningGuard`](wl_kernel::screening_guard::ScreeningGuard) in the
//! caller's transaction, for every screened party: the client company and
//! its UBOs, directors and signatories.
//!
//! - **Stale:** a list is stale now, or the latest screening of a party
//!   carries the stale list flag. Decision: 04 refuses whatever wl-parties'
//!   onboarding policy is (`warn` or `block`), because `PTY.RUL.02` says that
//!   while stale, `activate`, `submit` and `approve` are refused. The line
//!   names each list and its last update; everyone may see it (it names no
//!   party or candidate).
//! - **Open possible match:** a party has a `possible_match` no reviewer
//!   decided. Holders of `parties.block_reason.view` (the MLRO and deputy)
//!   see which party; everyone else sees only "Screening pending".
//! - **Not screened:** a party has no screening result yet (`submit`).
//!
//! No override exists for any of these.

use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::parties::directory;
use wl_kernel::screening_guard::{ScreeningOutcome, ScreeningRefusal, StaleList, screening_guard};

use crate::cdd::ScreenedRole;
use crate::permissions;
use crate::texts::english;

/// What the screening of a set of parties says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreeningCheck {
    /// The stale lists, each once.
    pub stale: Vec<StaleList>,
    /// The parties with an open possible match.
    pub open_matches: Vec<(PartyId, ScreenedRole)>,
    /// The parties with no screening result.
    pub unscreened: Vec<(PartyId, ScreenedRole)>,
}

impl ScreeningCheck {
    /// Whether nothing is stale and no match is open.
    pub fn settled(&self) -> bool {
        self.stale.is_empty() && self.open_matches.is_empty()
    }

    /// The line naming the stale lists and their last update, if any.
    pub fn stale_line(&self) -> Option<String> {
        if self.stale.is_empty() {
            return None;
        }
        // The kernel's text: "Screening list is out of date: <list> of <date>".
        let refusal = ScreeningRefusal::new(PartyId(0), self.stale.clone());
        Some(english(
            "accounting.screening.stale",
            &[("refusal", &refusal.message_en())],
        ))
    }

    /// The lines about open matches for `viewer`: one per party for a holder
    /// of `parties.block_reason.view`, else one "Screening pending".
    pub async fn match_lines(&self, state: &AppState, viewer: &AuthUser) -> Vec<String> {
        if self.open_matches.is_empty() {
            return Vec::new();
        }
        if !viewer.has_permission(permissions::BLOCK_REASON_VIEW) {
            return vec![english("accounting.screening.pending", &[])];
        }
        let mut lines = Vec::new();
        for (party, role) in &self.open_matches {
            let name = party_label(state, *party).await;
            let role = english(&format!("accounting.cdd.role.{}", role.as_str()), &[]);
            lines.push(english(
                "accounting.screening.possible_match",
                &[("party", &name), ("role", &role)],
            ));
        }
        lines
    }

    /// The lines about parties not screened yet.
    pub async fn unscreened_lines(&self, state: &AppState) -> Vec<String> {
        let mut lines = Vec::new();
        for (party, role) in &self.unscreened {
            let name = party_label(state, *party).await;
            let role = english(&format!("accounting.cdd.role.{}", role.as_str()), &[]);
            lines.push(english(
                "accounting.screening.not_screened",
                &[("party", &name), ("role", &role)],
            ));
        }
        lines
    }
}

/// The name of `party`, else `#id`.
pub(crate) async fn party_label(state: &AppState, party: PartyId) -> String {
    directory(state)
        .display_name(&state.db, party, "en")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", party.0))
}

/// Adds `lists` to `into`, each name once.
fn merge(into: &mut Vec<StaleList>, lists: Vec<StaleList>) {
    for list in lists {
        if !into.iter().any(|known| known.name == list.name) {
            into.push(list);
        }
    }
}

/// Checks `parties` in `tx`: stale lists, open matches, parties not
/// screened. Nothing is written.
///
/// # Errors
///
/// The database's errors.
pub async fn check_in(
    tx: &mut Transaction,
    state: &AppState,
    parties: &[(PartyId, ScreenedRole)],
) -> Result<ScreeningCheck> {
    let guard = screening_guard(state);
    let mut check = ScreeningCheck::default();
    for (party, role) in parties {
        match guard.check_in(&mut *tx, *party).await? {
            ScreeningOutcome::Allowed => {}
            ScreeningOutcome::Warn(lists) => merge(&mut check.stale, lists),
            ScreeningOutcome::Blocked(refusal) => merge(&mut check.stale, refusal.lists),
        }
        let standing = guard.standing_in(&mut *tx, *party).await?;
        if standing.open_match {
            check.open_matches.push((*party, *role));
        }
        if !standing.screened {
            check.unscreened.push((*party, *role));
        }
    }
    Ok(check)
}
