//! Overdue, late and missed deadline tasks in 04 (§7.1 *Task*, §7.2
//! `ACC.RUL.04`; BL-ACC-13).
//!
//! The Task lifecycle is wl-ticketing's (§4.4, reached through the kernel's
//! [`DeadlineTasks`](wl_kernel::deadlines::DeadlineTasks)); its nightly
//! `mark_overdue` (`WRK.RUL.05`) is the generic form of `ACC.RUL.04`. The 04
//! client runs it as [`OVERDUE_RULE`] at [`OVERDUE_AT`] Asia/Dubai, which the
//! host sets on wl-ticketing (`WlTicketing::overdue_rule_id`, with the
//! manager role [`MANAGER_ROLE`](crate::permissions::MANAGER_ROLE) and the partner role
//! [`PARTNER_ROLE`] told
//! besides the assignee):
//!
//! - the day after the due date, a task not `completed`, `missed` or
//!   `not_required` goes `overdue` (held tasks too; the client is never told);
//!   it is not terminal and not locked: `start` takes it back to work and
//!   `complete` finishes it flagged *late* (shown "completed late");
//! - the assignee, the manager and the partner are told;
//! - only a `tasks.manage` holder closes it as `missed`, with a reason from
//!   [`MISSED_REASONS`] ([`missed_reason`] gives the text stored), and the
//!   next occurrence follows; nothing closes a task as missed by itself;
//! - `ACC.RUL.04` is compliance-critical (registered with the app): a failed or missed
//!   run alerts the partner role and the company admin.
//!
//! Decision: an engagement has no partner of its own yet, so "the
//! engagement's partner; if none, the partner role of the firm" is the
//! holders of the partner role in the engagement's scope, and the manager is
//! the holders of the manager role there (wl-ticketing tells roles, each user
//! once).

use wl_kernel::compliance::CriticalRule;

use crate::permissions::PARTNER_ROLE;
use crate::texts::english;

/// The 04 rule that marks past-due deadline tasks `overdue`.
pub const OVERDUE_RULE: &str = "ACC.RUL.04";
/// When it runs, Asia/Dubai: 00:30 on the day after the due date.
pub const OVERDUE_AT: &str = "00:30";

/// `close_missed`: the client did not provide the documents.
pub const MISSED_CLIENT_NO_DOCUMENTS: &str = "client_no_documents";
/// `close_missed`: the client filed itself.
pub const MISSED_CLIENT_FILED_ITSELF: &str = "client_filed_itself";
/// `close_missed`: the engagement ended.
pub const MISSED_ENGAGEMENT_ENDED: &str = "engagement_ended";
/// `close_missed`: another reason, with a text.
pub const MISSED_OTHER: &str = "other";

/// The reasons for `close_missed` (§7.1 *Task*), in the order the form lists
/// them.
pub const MISSED_REASONS: [&str; 4] = [
    MISSED_CLIENT_NO_DOCUMENTS,
    MISSED_CLIENT_FILED_ITSELF,
    MISSED_ENGAGEMENT_ENDED,
    MISSED_OTHER,
];

/// The reason stored on a task closed as missed with `code` (and `text`
/// for *other*): "Client filed itself", "Other: …". `None` for a code not in
/// [`MISSED_REASONS`] or *other* without a text, which `close_missed` then
/// refuses for want of a reason.
///
/// Decision: wl-ticketing stores the reason as text; the 04 list is the
/// choice the form offers, so the stored text is the reason's English name.
pub fn missed_reason(code: &str, text: Option<&str>) -> Option<String> {
    let code = code.trim();
    if !MISSED_REASONS.contains(&code) {
        return None;
    }
    let name = english(&format!("accounting.tasks.missed.{code}"), &[]);
    if code != MISSED_OTHER {
        return Some(name);
    }
    let text = text.map(str::trim).filter(|t| !t.is_empty())?;
    Some(format!("{name}: {text}"))
}

/// Each reason of [`MISSED_REASONS`] with its English name and whether it
/// needs a text (*other*), as the host gives them to wl-ticketing
/// (`WlTicketing::missed_reason`), so its `close_missed` form offers them and
/// refuses any other reason.
pub fn missed_reason_names() -> Vec<(&'static str, String, bool)> {
    MISSED_REASONS
        .iter()
        .map(|code| {
            (
                *code,
                english(&format!("accounting.tasks.missed.{code}"), &[]),
                *code == MISSED_OTHER,
            )
        })
        .collect()
}

/// Marks `ACC.RUL.04` compliance-critical: its owner role is the partner
/// (with the company admin), and its nightly run at 00:30 is watched for a
/// missed run.
pub(crate) fn register() {
    CriticalRule::new(OVERDUE_RULE)
        .owner_role(PARTNER_ROLE)
        .daily_at(OVERDUE_AT)
        .register();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missed_reason_comes_from_the_list() {
        assert_eq!(
            missed_reason(MISSED_CLIENT_FILED_ITSELF, None).as_deref(),
            Some("Client filed itself")
        );
        assert_eq!(
            missed_reason(" client_no_documents ", None).as_deref(),
            Some("Client did not provide documents")
        );
        assert_eq!(
            missed_reason(MISSED_OTHER, Some(" Return filed by the old agent ")).as_deref(),
            Some("Other: Return filed by the old agent")
        );
        assert_eq!(missed_reason(MISSED_OTHER, Some("  ")), None);
        assert_eq!(missed_reason("forgot", None), None);
    }

    #[test]
    fn the_reason_names_are_what_missed_reason_stores() {
        let names = missed_reason_names();
        assert_eq!(names.len(), MISSED_REASONS.len());
        for (code, name, needs_text) in names {
            let text = needs_text.then_some("x");
            let stored = missed_reason(code, text).expect("a listed reason");
            let expected = if needs_text {
                format!("{name}: x")
            } else {
                name
            };
            assert_eq!(stored, expected);
        }
    }

    #[test]
    fn the_rule_is_critical_with_the_partner_as_owner() {
        register();
        let rule = wl_kernel::compliance::rule(OVERDUE_RULE).expect("registered");
        assert_eq!(rule.owner_role_name(), Some(PARTNER_ROLE));
        assert!(rule.schedule().is_some());
        assert_eq!(crate::permissions::MANAGER_ROLE, "engagement_manager");
    }
}
