//! The notice that cover is not found in time (09 Manpower §4.1 row *No
//! replacement can be found*, §4.6, §7.2 `MPW.RUL.13`; BL-MPW-16).
//!
//! A replacement request (`MPW.DEP.02`) stays `open` until a worker is planned
//! and activated for it. N hours before the cover period starts (the request's
//! start date and shift start time, Asia/Dubai; default
//! [`config::DEFAULT_COVER_NOTICE_HOURS`] = 24) a request that is still `open`
//! or `partially_filled` makes the rule tell the client and the account manager
//! that cover is not yet found, with how many of the headcount are still
//! missing. Decisions:
//!
//! - **Once.** The notice is claimed on the request (`cover_notice_sent_at`) and
//!   never repeated; a failed notice gives the claim back so the next run tries
//!   again. A request `filled` or `cancelled` by then gets none.
//! - **At once for a late request.** A request created less than N hours before
//!   its start is noticed when it is opened ([`crate::cover::open_cover`] calls
//!   [`notify_if_due`]); a request whose start has passed unfilled is noticed
//!   at the next run.
//! - **The run** is every 15 minutes ([`TASK`]) so the notice goes within a
//!   quarter of an hour of the hour (07:00 on 2026-11-20 for a start at 07:00 on
//!   2026-11-21). A shift without a start time starts at 00:00.
//! - **No penalty.** Uncovered days have no deployment and no hours, so nothing is
//!   billed, and no penalty line or credit note is made: the contract's penalty
//!   for missing cover is an open item (C-03).

use renox::chrono::Duration;
use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::BusinessCalendar;

use crate::config::DEFAULT_COVER_NOTICE_HOURS;
use crate::contract::ClientContract;
use crate::notices;
use crate::overlap::parse_time;
use crate::request::{ManpowerRequest, RequestKind, RequestStatus};
use crate::site::Site;
use crate::texts::english;

/// The rule id.
pub const RULE: &str = "MPW.RUL.13";

/// The name of the scheduled task that runs the rule.
pub const TASK: &str = "manpower:cover-notice";

/// The instant the cover period of `request` starts: its start date and shift
/// start time in Asia/Dubai.
pub fn starts_at(request: &ManpowerRequest) -> DateTime {
    let minutes = request
        .shift_start
        .as_deref()
        .and_then(parse_time)
        .unwrap_or(0);
    let local =
        request.start_date.and_hms_opt(0, 0, 0).unwrap_or_default() + Duration::minutes(minutes);
    BusinessCalendar::default().from_local(local)
}

/// Whether the notice for `request` is due at `at`: a replacement request still
/// taking workers, not noticed yet, and no more than N hours before its start.
pub fn is_due(request: &ManpowerRequest, at: DateTime) -> bool {
    request.kind == RequestKind::Replacement
        && matches!(
            request.status,
            RequestStatus::Open | RequestStatus::PartiallyFilled
        )
        && request.cover_notice_sent_at.is_none()
        && at >= starts_at(request) - Duration::hours(DEFAULT_COVER_NOTICE_HOURS)
}

/// Tells the client and the account manager that `request` has no cover yet,
/// once. Returns whether the notice went out now.
///
/// # Errors
///
/// The database's and the notifier's errors; the claim is given back.
async fn notify_one(state: &AppState, id: i64, at: DateTime) -> Result<bool> {
    let Some(request) = ManpowerRequest::find(&state.db, id).await? else {
        return Ok(false);
    };
    if !is_due(&request, at) {
        return Ok(false);
    }
    let claimed = ManpowerRequest::query()
        .where_(ManpowerRequest::ID.eq(id))
        .where_(ManpowerRequest::COVER_NOTICE_SENT_AT.is_null())
        .update(&state.db, &[("cover_notice_sent_at", &Some(now()))])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    match tell(state, &request).await {
        Ok(()) => Ok(true),
        Err(error) => {
            let none: Option<DateTime> = None;
            ManpowerRequest::query()
                .where_(ManpowerRequest::ID.eq(id))
                .update(&state.db, &[("cover_notice_sent_at", &none)])
                .await?;
            Err(error)
        }
    }
}

/// The messages of the notice.
async fn tell(state: &AppState, request: &ManpowerRequest) -> Result {
    let contract = ClientContract::find_or_404(&state.db, request.client_contract_id).await?;
    let site = Site::find(&state.db, request.site_id)
        .await?
        .map(|site| site.name)
        .unwrap_or_default();
    let missing = request.missing();
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 5] = [
        ("role", &request.role),
        ("site", &site),
        ("from", &request.start_date),
        ("missing", &missing),
        ("headcount", &request.headcount),
    ];
    let title = english("manpower.cover.missing_title", &params);
    let body = english("manpower.cover.missing_body", &params);
    let managers =
        notices::account_managers(state, contract.client(), contract.account_manager_user_id)
            .await?;
    notices::tell(
        state,
        request.subject(),
        &contract.reference,
        &managers,
        &title,
        &body,
    )
    .await?;
    notices::tell_client(
        state,
        request.subject(),
        &contract.reference,
        request.client(),
        &title,
        &body,
    )
    .await
}

/// The notice for request `id` if it is due at `at`: what a request opened
/// late gets at once. Returns whether it went out.
///
/// # Errors
///
/// As [`notify_one`].
pub async fn notify_if_due(state: &AppState, id: i64, at: DateTime) -> Result<bool> {
    notify_one(state, id, at).await
}

/// `MPW.RUL.13` at `at`: the notice of every replacement request that is due.
/// Returns the requests noticed now; a second run notices none of them again.
///
/// # Errors
///
/// The first error of the notices; the others are still tried.
pub async fn notify_due(state: &AppState, at: DateTime) -> Result<Vec<i64>> {
    let candidates = ManpowerRequest::query()
        .where_(ManpowerRequest::KIND.eq(RequestKind::Replacement))
        .where_(
            ManpowerRequest::STATUS.is_in([RequestStatus::Open, RequestStatus::PartiallyFilled]),
        )
        .where_(ManpowerRequest::COVER_NOTICE_SENT_AT.is_null())
        .order_by(ManpowerRequest::ID)
        .get(&state.db)
        .await?;
    let mut sent = Vec::new();
    let mut failure = None;
    for request in candidates {
        if !is_due(&request, at) {
            continue;
        }
        match notify_one(state, request.id, at).await {
            Ok(true) => sent.push(request.id),
            Ok(false) => {}
            Err(error) => {
                eprintln!("{RULE}: request {}: {error:?}", request.id);
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(sent), Err)
}

/// The scheduled run.
///
/// # Errors
///
/// As [`notify_due`].
pub async fn run(state: &AppState) -> Result {
    notify_due(state, now()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;

    fn request(shift: Option<&str>) -> ManpowerRequest {
        ManpowerRequest {
            kind: RequestKind::Replacement,
            status: RequestStatus::Open,
            start_date: NaiveDate::from_ymd_opt(2026, 11, 21).unwrap(),
            shift_start: shift.map(str::to_owned),
            ..ManpowerRequest::default()
        }
    }

    fn dubai(day: u32, hour: u32, minute: u32) -> DateTime {
        BusinessCalendar::default().from_local(
            NaiveDate::from_ymd_opt(2026, 11, day)
                .unwrap()
                .and_hms_opt(hour, minute, 0)
                .unwrap(),
        )
    }

    #[test]
    fn the_cover_starts_at_the_shift_start_in_dubai() {
        assert_eq!(starts_at(&request(Some("07:00"))), dubai(21, 7, 0));
        assert_eq!(starts_at(&request(None)), dubai(21, 0, 0));
    }

    #[test]
    fn the_notice_is_due_24_hours_before_the_start_and_not_before() {
        let open = request(Some("07:00"));
        assert!(!is_due(&open, dubai(20, 6, 59)));
        assert!(is_due(&open, dubai(20, 7, 0)));
        let mut sent = open.clone();
        sent.cover_notice_sent_at = Some(dubai(20, 7, 0));
        assert!(!is_due(&sent, dubai(20, 8, 0)));
        let mut filled = open;
        filled.status = RequestStatus::Filled;
        assert!(!is_due(&filled, dubai(20, 8, 0)));
    }
}
