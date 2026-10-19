//! The manual screening check (wl-parties §4, issue #42 part 3): a reviewer
//! checked a party against the official file of a PDF-only sanctions list by
//! hand and records the result, `NoMatch` or a `Candidate` with its list
//! entry. It is a [`Screening`] with trigger and method `manual`, the
//! reviewer, and the list and its date in `lists_used`, recorded through
//! [`record_run`] so `complete_clear` / `flag`, the compliance task and the
//! alert are the ones of the automatic path.
//!
//! The list date is the version of the list file the reviewer read. The
//! stale-list flag of the Screening is false: the file of that date was
//! checked by hand. A candidate scores 1.0, since a person decided it.

use std::fmt;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::PartyStatus;

use crate::list_upload::{today_dubai, version_date};
use crate::party::Party;
use crate::permissions;
use crate::sanctions_lists::{ListFormat, SanctionsList};
use crate::screening::{
    ListUsed, NewCandidate, Screening, ScreeningRun, ScreeningSettings, record_run,
};

/// What the reviewer found in the list file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManualResult {
    /// The party is not on the list.
    NoMatch,
    /// A name on the list looks like the party.
    Candidate {
        /// The name as written on the list.
        name: String,
        /// The entry of the list (its reference); required.
        list_entry: String,
    },
}

/// Why a manual check is refused. Nothing is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManualRefusal {
    /// Screening is not turned on.
    ScreeningOff,
    /// The party is archived.
    Archived,
    /// The list is not PDF-only.
    NotPdfList,
    /// The list date is missing.
    ListDateMissing,
    /// The list date is not an ISO date (`YYYY-MM-DD`).
    ListDateInvalid,
    /// The list date is after today (Asia/Dubai).
    ListDateInFuture,
    /// A candidate has no name.
    CandidateNameMissing,
    /// A candidate has no list entry.
    ListEntryMissing,
}

impl fmt::Display for ManualRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ManualRefusal::ScreeningOff => "screening is not turned on",
            ManualRefusal::Archived => "the party is archived",
            ManualRefusal::NotPdfList => {
                "this list is not checked by hand; use the normal screening"
            }
            ManualRefusal::ListDateMissing => "the list date is required",
            ManualRefusal::ListDateInvalid => "the list date must be a date written YYYY-MM-DD",
            ManualRefusal::ListDateInFuture => "the list date is in the future",
            ManualRefusal::CandidateNameMissing => "the name of the candidate is required",
            ManualRefusal::ListEntryMissing => "the list entry of the candidate is required",
        })
    }
}

/// Why `record_manual_check` did not record.
#[derive(Debug)]
pub enum ManualCheckError {
    /// The user lacks `screening.review`.
    Forbidden,
    /// The party or the list does not exist.
    NotFound,
    /// The check was refused.
    Refused(ManualRefusal),
    /// The run was refused by the Screening machine, or the database failed.
    Run(TransitionError),
}

impl fmt::Display for ManualCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManualCheckError::Forbidden => f.write_str("not allowed"),
            ManualCheckError::NotFound => f.write_str("no such party or list"),
            ManualCheckError::Refused(why) => why.fmt(f),
            ManualCheckError::Run(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ManualCheckError {}

impl From<renox::Error> for ManualCheckError {
    fn from(e: renox::Error) -> Self {
        ManualCheckError::Run(TransitionError::from(e))
    }
}

/// Checks the list date (ISO, not after `today`) and the result.
pub(crate) fn validate(
    list_date: &str,
    result: &ManualResult,
    today: NaiveDate,
) -> Result<NaiveDate, ManualRefusal> {
    let text = list_date.trim();
    if text.is_empty() {
        return Err(ManualRefusal::ListDateMissing);
    }
    let date = version_date(text)
        .filter(|_| text.len() == 10)
        .ok_or(ManualRefusal::ListDateInvalid)?;
    if date > today {
        return Err(ManualRefusal::ListDateInFuture);
    }
    if let ManualResult::Candidate { name, list_entry } = result {
        if name.trim().is_empty() {
            return Err(ManualRefusal::CandidateNameMissing);
        }
        if list_entry.trim().is_empty() {
            return Err(ManualRefusal::ListEntryMissing);
        }
    }
    Ok(date)
}

/// Records the manual check of `party` against `list_id` as of `list_date`
/// (`YYYY-MM-DD`) by `actor`, who needs `screening.review`. A `NoMatch`
/// ends `clear`; a `Candidate` ends `possible_match` with the compliance
/// task and alert of [`record_run`] (the party is not blocked).
///
/// # Errors
///
/// See [`ManualCheckError`]. Nothing is stored on an error.
pub async fn record_manual_check(
    state: &AppState,
    actor: &AuthUser,
    party: &Party,
    list_id: i64,
    list_date: &str,
    result: ManualResult,
) -> Result<Screening, ManualCheckError> {
    if !actor.has_permission(permissions::SCREENING_REVIEW) {
        return Err(ManualCheckError::Forbidden);
    }
    if !ScreeningSettings::of(state).on {
        return Err(ManualCheckError::Refused(ManualRefusal::ScreeningOff));
    }
    let party = Party::find(&state.db, party.id)
        .await?
        .ok_or(ManualCheckError::NotFound)?;
    if party.status == PartyStatus::Archived {
        return Err(ManualCheckError::Refused(ManualRefusal::Archived));
    }
    let list = SanctionsList::find(&state.db, list_id)
        .await?
        .ok_or(ManualCheckError::NotFound)?;
    if list.format != ListFormat::Pdf {
        return Err(ManualCheckError::Refused(ManualRefusal::NotPdfList));
    }
    let now = renox::db::now();
    let date = validate(list_date, &result, today_dubai(now)).map_err(ManualCheckError::Refused)?;
    let candidates = match result {
        ManualResult::NoMatch => Vec::new(),
        ManualResult::Candidate { name, list_entry } => vec![NewCandidate {
            name,
            list_name: list.name.clone(),
            score: 1.0,
            list_entry: Some(list_entry),
        }],
    };
    let mut used = ListUsed::of(&list, now, i64::MAX / 4);
    used.version = Some(date.format("%Y-%m-%d").to_string());
    used.stale = false;
    let run = ScreeningRun {
        run_at: Some(now),
        lists_used: vec![used],
        ..ScreeningRun::manual(candidates)
    };
    record_run(state, &party, run, Some(actor))
        .await
        .map_err(ManualCheckError::Run)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    fn candidate(name: &str, entry: &str) -> ManualResult {
        ManualResult::Candidate {
            name: name.into(),
            list_entry: entry.into(),
        }
    }

    #[test]
    fn the_list_date_is_required_iso_and_not_in_the_future() {
        let none = ManualResult::NoMatch;
        assert_eq!(validate("2026-11-03", &none, day(5)), Ok(day(3)));
        assert_eq!(validate("2026-11-05", &none, day(5)), Ok(day(5)));
        assert_eq!(
            validate("  ", &none, day(5)),
            Err(ManualRefusal::ListDateMissing)
        );
        for bad in ["03/11/2026", "2026-11", "2026-13-01", "2026-11-03x"] {
            assert_eq!(
                validate(bad, &none, day(5)),
                Err(ManualRefusal::ListDateInvalid),
                "{bad}"
            );
        }
        assert_eq!(
            validate("2026-11-06", &none, day(5)),
            Err(ManualRefusal::ListDateInFuture)
        );
    }

    #[test]
    fn a_candidate_needs_a_name_and_a_list_entry() {
        let ok = |r| validate("2026-11-03", &r, day(5));
        assert_eq!(ok(candidate("Omar K.", "UAE-12")), Ok(day(3)));
        assert_eq!(
            ok(candidate(" ", "UAE-12")),
            Err(ManualRefusal::CandidateNameMissing)
        );
        assert_eq!(
            ok(candidate("Omar K.", " ")),
            Err(ManualRefusal::ListEntryMissing)
        );
    }
}
