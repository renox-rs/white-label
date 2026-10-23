//! The flag of a session whose tutor will not be eligible or available (10 §7.2
//! `EDU.RUL.12`, BL-EDU-10): the session shows "tutor not available or not
//! eligible" until it is resolved. The flag is kept on the session row
//! (`flag_reason`, `flagged_at`, `flag_escalated_at`, `flag_alert_id`).
//!
//! A flag is **resolved** when the session's tutor changes (the batch's or the
//! session's own `change_tutor`), the session is rescheduled to a date the tutor
//! can teach, or it is cancelled: the machine's effects call [`clear`]. When the
//! document is renewed or the block lifted, the rule's recompute clears it
//! ([`crate::tutor_alerts`]).

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;

use crate::session::Session;

/// Takes the flag off the sessions `ids` (resolved).
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn clear(tx: &mut Transaction, ids: &[i64]) -> Result {
    if ids.is_empty() {
        return Ok(());
    }
    let no_reason: Option<String> = None;
    let no_time: Option<DateTime> = None;
    let no_alert: Option<i64> = None;
    Session::query()
        .where_(Session::ID.is_in(ids.to_vec()))
        .where_(Session::FLAG_REASON.is_not_null())
        .update(
            &mut *tx,
            &[
                ("flag_reason", &no_reason),
                ("flagged_at", &no_time),
                ("flag_escalated_at", &no_time),
                ("flag_alert_id", &no_alert),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}
