//! The re-run of screenings made on a stale list (wl-parties §3, `PTY.RUL.02`,
//! issue #40).
//!
//! After [`SanctionsListUpdated`] the listener finds every screening that
//! was flagged `lists_stale`, has no `superseded_by` and used the updated
//! list while it was stale. For each one whose party is neither archived nor
//! merged it runs a new screening with trigger `list_update` and
//! `rerun_of` set, then sets `superseded_by` on the old one, so a second
//! update re-runs nothing twice. If another list used is still stale the new
//! screening is flagged again and is re-run after that list updates.
//!
//! One party failing is counted and does not stop the rest.

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::PartyStatus;

use crate::events::SanctionsListUpdated;
use crate::party::Party;
use crate::screening::{self, Screening, ScreeningSettings, ScreeningTrigger};

/// What a re-run pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Screenings re-run.
    pub rerun: usize,
    /// Screenings left alone because their party is archived or merged.
    pub skipped: usize,
    /// Screenings whose re-run failed; they stay open for the next update.
    pub failed: usize,
}

/// Whether `screening` is waiting for `list` to update.
fn waits_for(screening: &Screening, list: i64) -> bool {
    screening.lists_stale
        && screening.superseded_by.is_none()
        && screening
            .lists_used
            .0
            .iter()
            .any(|used| used.list_id == list && used.stale)
}

/// Re-runs the screenings that waited for list `list`.
pub async fn rerun_for_list(state: &AppState, list: i64) -> Result<Report> {
    let mut report = Report::default();
    if !ScreeningSettings::of(state).on {
        return Ok(report);
    }
    let waiting: Vec<Screening> = Screening::query()
        .where_(Screening::LISTS_STALE.eq(true))
        .where_(Screening::SUPERSEDED_BY.is_null())
        .order_by(Screening::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|screening| waits_for(screening, list))
        .collect();
    for old in waiting {
        match rerun_one(state, &old).await {
            Ok(Some(_)) => report.rerun += 1,
            Ok(None) => report.skipped += 1,
            Err(error) => {
                report.failed += 1;
                eprintln!(
                    "wl-parties: re-running screening {} failed: {error}",
                    old.id
                );
            }
        }
    }
    Ok(report)
}

/// Re-runs one screening; `None` when its party is archived or merged.
async fn rerun_one(
    state: &AppState,
    old: &Screening,
) -> std::result::Result<Option<Screening>, TransitionError> {
    let Some(party) = Party::find(&state.db, old.party_id).await? else {
        return Ok(None);
    };
    if matches!(party.status, PartyStatus::Archived | PartyStatus::Merged) {
        return Ok(None);
    }
    let new =
        screening::run_rerun(state, &party, ScreeningTrigger::ListUpdate, Some(old.id)).await?;
    let mut old = old.clone();
    old.superseded_by = Some(new.id);
    old.save(&state.db).await?;
    Ok(Some(new))
}

/// Registers the listener of [`SanctionsListUpdated`].
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: SanctionsListUpdated, state: AppState| async move {
        rerun_for_list(&state, event.list).await.map(|_| ())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screening::ListUsed;
    use renox::db::Json;

    fn waiting(list: i64, stale: bool) -> Screening {
        Screening {
            lists_stale: true,
            lists_used: Json(vec![ListUsed {
                list_id: list,
                name: "UN".into(),
                version: None,
                last_updated_at: None,
                stale,
            }]),
            ..Screening::default()
        }
    }

    #[test]
    fn a_screening_waits_for_a_list_it_used_while_stale() {
        assert!(waits_for(&waiting(1, true), 1));
        assert!(!waits_for(&waiting(1, true), 2));
        assert!(!waits_for(&waiting(1, false), 1));
    }

    #[test]
    fn a_superseded_or_unflagged_screening_waits_for_nothing() {
        let done = Screening {
            superseded_by: Some(9),
            ..waiting(1, true)
        };
        assert!(!waits_for(&done, 1));
        let unflagged = Screening {
            lists_stale: false,
            ..waiting(1, true)
        };
        assert!(!waits_for(&unflagged, 1));
    }
}
