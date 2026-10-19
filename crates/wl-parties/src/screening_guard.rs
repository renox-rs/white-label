//! The module's [`ScreeningGuard`] (wl-parties §7, issue #41): warns about, or
//! refuses, onboarding a party while a sanctions list is stale.
//! [`crate::WlParties`] provides it with
//! [`wl_kernel::screening_guard::provide_screening_guard`] only when screening
//! is on; otherwise other modules get the kernel's no-op.
//!
//! The lists to name are the stale lists the party's latest [`Screening`]
//! used (when it carries the stale flag) plus every list that is stale now,
//! each once, with its last successful update date in Asia/Dubai (`None`:
//! never updated). The check reads only `Screening` and `SanctionsList`; it
//! runs no screening, calls no provider and writes nothing, so a warn pass
//! leaves no row. The notice is derived: it shows while the latest screening
//! is flagged stale and goes away once a re-run (the new screening is not
//! flagged) replaces it.
//!
//! - `Warn` never refuses: stale lists give [`ScreeningOutcome::Warn`].
//! - `Block` refuses with a [`ScreeningRefusal`] naming the lists, until no
//!   list is stale and the latest screening is not flagged. Whether the
//!   re-run ended `clear` or `possible_match` is not decided here. A party
//!   never screened is allowed when no list is stale.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::screening_guard::{
    ScreeningFuture, ScreeningGuard, ScreeningOutcome, ScreeningRefusal, StaleList, StaleListPolicy,
};

use crate::sanctions_lists::{ListStatus, SanctionsList};
use crate::screening::Screening;

/// Reads `wl_parties_screenings` and `wl_parties_sanctions_lists` for the
/// kernel contract.
#[derive(Debug, Clone)]
pub struct PartiesScreeningGuard {
    policy: StaleListPolicy,
    freshness_hours: i64,
}

impl PartiesScreeningGuard {
    /// A guard with `policy` and the freshness limit of the lists.
    pub fn new(policy: StaleListPolicy, freshness_hours: i64) -> Self {
        PartiesScreeningGuard {
            policy,
            freshness_hours,
        }
    }

    /// What the guard does on a stale list.
    pub fn policy(&self) -> StaleListPolicy {
        self.policy
    }

    /// The stale lists of `party` at `now`: empty when there is none.
    async fn stale_lists(
        &self,
        tx: &mut Transaction,
        party: PartyId,
        now: DateTime,
    ) -> Result<Vec<StaleList>> {
        let mut lists = self.flagged_lists(tx, party).await?;
        for list in SanctionsList::all(&mut *tx).await? {
            let stale =
                list.status == ListStatus::Stale || list.is_stale_at(now, self.freshness_hours);
            if stale && !lists.iter().any(|known| known.name == list.name) {
                lists.push(StaleList::new(
                    list.name,
                    list.last_updated_at.map(dubai_date),
                ));
            }
        }
        Ok(lists)
    }

    /// The stale lists the latest screening used, when it is flagged.
    async fn flagged_lists(&self, tx: &mut Transaction, party: PartyId) -> Result<Vec<StaleList>> {
        let latest = Screening::query()
            .where_(Screening::PARTY_ID.eq(party.0))
            .order_by_desc(Screening::ID)
            .limit(1)
            .get(&mut *tx)
            .await?
            .into_iter()
            .next();
        Ok(latest.map(|s| flagged_in(&s)).unwrap_or_default())
    }
}

/// The stale lists `screening` used when it carries the stale flag.
fn flagged_in(screening: &Screening) -> Vec<StaleList> {
    if !screening.lists_stale {
        return Vec::new();
    }
    let mut lists: Vec<StaleList> = Vec::new();
    for used in screening.lists_used.0.iter().filter(|used| used.stale) {
        if !lists.iter().any(|known| known.name == used.name) {
            lists.push(StaleList::new(
                used.name.clone(),
                used.last_updated_at.map(dubai_date),
            ));
        }
    }
    lists
}

/// The Asia/Dubai calendar date of `at`.
fn dubai_date(at: DateTime) -> NaiveDate {
    at.with_timezone(&wl_kernel::BusinessCalendar::offset())
        .date_naive()
}

/// The outcome for `lists` under `policy`.
fn outcome(policy: StaleListPolicy, party: PartyId, lists: Vec<StaleList>) -> ScreeningOutcome {
    if lists.is_empty() {
        return ScreeningOutcome::Allowed;
    }
    match policy {
        StaleListPolicy::Warn => ScreeningOutcome::Warn(lists),
        StaleListPolicy::Block => ScreeningOutcome::Blocked(ScreeningRefusal::new(party, lists)),
    }
}

impl ScreeningGuard for PartiesScreeningGuard {
    fn check_in<'a>(
        &'a self,
        tx: &'a mut Transaction,
        party: PartyId,
    ) -> ScreeningFuture<'a, ScreeningOutcome> {
        Box::pin(async move {
            let lists = self.stale_lists(tx, party, renox::db::now()).await?;
            Ok(outcome(self.policy, party, lists))
        })
    }

    fn latest_notice_in<'a>(
        &'a self,
        tx: &'a mut Transaction,
        party: PartyId,
    ) -> ScreeningFuture<'a, Vec<StaleList>> {
        Box::pin(async move { self.flagged_lists(tx, party).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screening::ListUsed;

    fn at(day: u32, hour: u32) -> DateTime {
        NaiveDate::from_ymd_opt(2026, 11, day)
            .unwrap()
            .and_hms_opt(hour, 0, 0)
            .unwrap()
            .and_utc()
    }

    fn used(name: &str, stale: bool, updated: Option<DateTime>) -> ListUsed {
        ListUsed {
            list_id: 1,
            name: name.to_owned(),
            version: None,
            last_updated_at: updated,
            stale,
        }
    }

    fn screening(flag: bool, lists: Vec<ListUsed>) -> Screening {
        Screening {
            lists_stale: flag,
            lists_used: renox::db::Json(lists),
            ..Screening::default()
        }
    }

    #[test]
    fn the_date_is_the_dubai_day() {
        // 21:00 UTC is already the next day in Dubai.
        assert_eq!(dubai_date(at(2, 21)).to_string(), "2026-11-03");
        assert_eq!(dubai_date(at(2, 10)).to_string(), "2026-11-02");
    }

    #[test]
    fn only_a_flagged_screening_names_its_stale_lists_once() {
        let lists = vec![
            used("UN Consolidated List", true, Some(at(2, 10))),
            used("UAE Local Terrorist List", false, Some(at(2, 10))),
            used("UN Consolidated List", true, Some(at(2, 10))),
        ];
        let flagged = flagged_in(&screening(true, lists.clone()));
        assert_eq!(
            flagged,
            vec![StaleList::new(
                "UN Consolidated List",
                NaiveDate::from_ymd_opt(2026, 11, 2)
            )]
        );
        assert!(flagged_in(&screening(false, lists)).is_empty());
    }

    #[test]
    fn a_list_never_updated_has_no_date() {
        let flagged = flagged_in(&screening(true, vec![used("UN", true, None)]));
        assert_eq!(flagged, vec![StaleList::new("UN", None)]);
    }

    #[test]
    fn no_stale_list_is_allowed_in_both_modes() {
        for policy in [StaleListPolicy::Warn, StaleListPolicy::Block] {
            assert!(outcome(policy, PartyId(1), Vec::new()).is_allowed());
        }
    }

    #[test]
    fn warn_never_refuses_and_block_names_the_lists() {
        let lists = vec![StaleList::new("UN", NaiveDate::from_ymd_opt(2026, 11, 2))];
        let warn = outcome(StaleListPolicy::Warn, PartyId(1), lists.clone());
        assert_eq!(warn, ScreeningOutcome::Warn(lists.clone()));
        let block = outcome(StaleListPolicy::Block, PartyId(1), lists.clone());
        assert_eq!(
            block,
            ScreeningOutcome::Blocked(ScreeningRefusal::new(PartyId(1), lists))
        );
    }
}
