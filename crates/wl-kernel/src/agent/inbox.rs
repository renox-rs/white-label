//! The agent inbox (BL-AGT-09, decision 0006 point 8, `PLT.AGT.03`): an
//! assistant cannot be woken by the app, so it asks. [`inbox`] returns what
//! the user should look at now, in five groups:
//!
//! 1. *Waiting for your approval*: the same items as the dashboard's
//!    "awaiting you" — the kernel's approval requests whose current step the
//!    user may decide ([`crate::approvals::awaiting_items`]) plus what the
//!    app adds there (the host registers its pay accounts waiting for a
//!    second user with [`register_source`]). Listed with a link: the agent
//!    cannot decide them (BL-AGT-08).
//! 2. *Overdue*: the user's overdue items, from the sources modules register
//!    (none in the kernel: it does not know the modules' due dates).
//! 3. *Mentions*: unread notifications of a `mention` kind.
//! 4. *Notifications*: the other unread notifications (the bell).
//! 5. *Failed automations*: rule runs that failed, for a user who may run
//!    them again (`automations.retry`; rule runs have no owner, so the
//!    holders of the retry permission are the people who own them).
//!
//! Every item has a title, why it is there, a due date (Asia/Dubai) when it
//! has one, and a URL. Only the user's own items are ever listed: each
//! source reads for the calling user. `since` keeps the dated items from
//! that instant on; approvals are listed while they wait.
//!
//! [`mark_read`] marks notifications read, which clears them from the bell.

use std::sync::{Arc, PoisonError, RwLock};

use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::Serialize;

use super::AgentCall;
use crate::automation::{RuleRun, RuleRunStatus};
use crate::documents::{dubai_date, dubai_text};
use crate::machines::BoxFuture;

/// The groups, in the order the inbox lists them: key and heading.
pub const GROUPS: &[(&str, &str)] = &[
    (APPROVALS, "Waiting for your approval"),
    (OVERDUE, "Overdue"),
    (MENTIONS, "Mentions"),
    (NOTIFICATIONS, "Notifications"),
    (FAILED, "Failed automations"),
];

/// Approvals the user may decide.
pub const APPROVALS: &str = "approvals";
/// The user's overdue items.
pub const OVERDUE: &str = "overdue";
/// Unread mentions.
pub const MENTIONS: &str = "mentions";
/// Unread notifications.
pub const NOTIFICATIONS: &str = "notifications";
/// Failed rule runs.
pub const FAILED: &str = "failed_automations";

/// The prefix of a notification item's id, which [`mark_read`] takes.
pub const NOTIFICATION_PREFIX: &str = "notification:";

/// One thing that needs the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InboxItem {
    /// A stable id, e.g. `approval:12` or `notification:40`.
    pub id: String,
    /// What it is.
    pub title: String,
    /// Why it needs the user.
    pub why: String,
    /// When it is due (Asia/Dubai date), if it has a due date.
    pub due: Option<String>,
    /// When it happened (Asia/Dubai), if known.
    pub at: Option<String>,
    /// Where to open it.
    pub url: Option<String>,
    #[serde(skip)]
    happened: Option<DateTime>,
}

impl InboxItem {
    /// An item `id` called `title`, there because of `why`.
    pub fn new(id: impl Into<String>, title: impl Into<String>, why: impl Into<String>) -> Self {
        InboxItem {
            id: id.into(),
            title: title.into(),
            why: why.into(),
            due: None,
            at: None,
            url: None,
            happened: None,
        }
    }

    /// Its page.
    pub fn url(mut self, url: Option<String>) -> Self {
        self.url = url;
        self
    }

    /// Its due date.
    pub fn due(mut self, due: Option<renox::chrono::NaiveDate>) -> Self {
        self.due = due.map(|d| d.to_string());
        self
    }

    /// When it happened.
    pub fn at(mut self, at: Option<DateTime>) -> Self {
        self.at = at.map(dubai_text);
        self.happened = at;
        self
    }
}

/// One group of the inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InboxGroup {
    /// The group's key, e.g. `approvals`.
    pub key: String,
    /// Its heading.
    pub label: String,
    /// Its items.
    pub items: Vec<InboxItem>,
}

/// The whole inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Inbox {
    /// The five groups, always in the same order.
    pub groups: Vec<InboxGroup>,
    /// How many items in all.
    pub total: usize,
    /// When it was read (Asia/Dubai).
    pub as_of: String,
}

impl Inbox {
    /// The group `key`.
    pub fn group(&self, key: &str) -> Option<&InboxGroup> {
        self.groups.iter().find(|g| g.key == key)
    }
}

/// A source of items: reads for the calling user only.
pub type InboxSource =
    Arc<dyn Fn(AgentCall) -> BoxFuture<'static, Result<Vec<InboxItem>>> + Send + Sync>;

static SOURCES: RwLock<Vec<(String, String, InboxSource)>> = RwLock::new(Vec::new());

/// Adds a source of items to `group` (one of [`GROUPS`]) under `name`,
/// replacing one of the same group and name.
///
/// # Panics
///
/// For an unknown group: a programming error, found at boot.
pub fn register_source<F>(group: &str, name: &str, source: F)
where
    F: Fn(AgentCall) -> BoxFuture<'static, Result<Vec<InboxItem>>> + Send + Sync + 'static,
{
    assert!(
        GROUPS.iter().any(|(key, _)| *key == group),
        "`{group}` is not an inbox group"
    );
    let mut sources = SOURCES.write().unwrap_or_else(PoisonError::into_inner);
    let source: InboxSource = Arc::new(source);
    match sources
        .iter_mut()
        .find(|(g, n, _)| g == group && n == name)
    {
        Some(existing) => existing.2 = source,
        None => sources.push((group.to_owned(), name.to_owned(), source)),
    }
}

fn sources_of(group: &str) -> Vec<InboxSource> {
    SOURCES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|(g, _, _)| g == group)
        .map(|(_, _, s)| Arc::clone(s))
        .collect()
}

/// What needs `call`'s user now, from `since` on for dated items.
pub async fn inbox(call: &AgentCall, since: Option<DateTime>) -> Result<Inbox> {
    let mut groups = Vec::with_capacity(GROUPS.len());
    let mut total = 0;
    for (key, label) in GROUPS {
        let mut items = Vec::new();
        for source in sources_of(key) {
            items.extend(source(call.clone()).await?);
        }
        if *key != APPROVALS
            && let Some(since) = since
        {
            items.retain(|item| item.happened.is_none_or(|at| at >= since));
        }
        total += items.len();
        groups.push(InboxGroup {
            key: (*key).to_owned(),
            label: (*label).to_owned(),
            items,
        });
    }
    Ok(Inbox {
        groups,
        total,
        as_of: dubai_text(now()),
    })
}

/// Marks the user's notifications `ids` read (`notification:40` or `40`);
/// ids of other users or other items are ignored. Returns how many changed
/// and the bell's count after.
pub async fn mark_read(call: &AgentCall, ids: &[String]) -> Result<(u64, i64)> {
    let db = &call.state.db;
    let mut changed = 0;
    for id in ids {
        let raw = id.trim();
        let raw = raw.strip_prefix(NOTIFICATION_PREFIX).unwrap_or(raw);
        let Ok(id) = raw.parse::<i64>() else {
            continue;
        };
        if call.user.mark_notification_read(db, id).await? {
            changed += 1;
        }
    }
    let unread = call.user.unread_notification_count(db).await?;
    Ok((changed, unread))
}

/// Whether a notification kind is a mention.
pub fn is_mention(kind: &str) -> bool {
    kind.to_ascii_lowercase().contains("mention")
}

/// The kernel's sources: approvals, mentions, notifications and failed
/// automations.
pub(crate) fn register_kernel_sources() {
    register_source(APPROVALS, "kernel.approvals", |call| {
        Box::pin(async move {
            let lang = call.state.current_lang();
            let items = crate::approvals::awaiting_items(&call.state.db, &call.user, &lang).await?;
            Ok(items
                .into_iter()
                .map(|item| {
                    InboxItem::new(
                        format!("approval:{}", item.request_id),
                        item.title,
                        "Waiting for your decision. Open it to decide: an assistant cannot \
                         approve or reject.",
                    )
                    .url(item.url)
                })
                .collect())
        })
    });
    register_source(MENTIONS, "kernel.mentions", |call| {
        Box::pin(async move { notifications(&call, true).await })
    });
    register_source(NOTIFICATIONS, "kernel.notifications", |call| {
        Box::pin(async move { notifications(&call, false).await })
    });
    register_source(FAILED, "kernel.rule_runs", |call| {
        Box::pin(async move { failed_runs(&call).await })
    });
}

/// The user's unread notifications: the mentions, or all the others.
async fn notifications(call: &AgentCall, mentions: bool) -> Result<Vec<InboxItem>> {
    let unread = call.user.unread_notifications(&call.state.db).await?;
    Ok(unread
        .into_iter()
        .filter(|n| is_mention(&n.kind) == mentions)
        .map(|n| {
            let message = n.message();
            let title = message
                .as_ref()
                .map(|m| m.title.clone())
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| n.kind.clone());
            let why = message
                .as_ref()
                .and_then(|m| m.body.clone())
                .unwrap_or_else(|| {
                    if mentions {
                        "You were mentioned.".to_owned()
                    } else {
                        "Unread notification.".to_owned()
                    }
                });
            InboxItem::new(format!("{NOTIFICATION_PREFIX}{}", n.id), title, why)
                .url(message.and_then(|m| m.url))
                .at(Some(n.created_at))
        })
        .collect())
}

/// The failed rule runs, for a user who may run them again.
async fn failed_runs(call: &AgentCall) -> Result<Vec<InboxItem>> {
    if !call
        .user
        .has_permission(crate::permissions::AUTOMATIONS_RETRY)
    {
        return Ok(Vec::new());
    }
    let runs = RuleRun::query()
        .where_(RuleRun::STATUS.eq(RuleRunStatus::Failed))
        .order_by(RuleRun::ID)
        .get(&call.state.db)
        .await?;
    Ok(runs
        .into_iter()
        .map(|run| {
            let at = run.finished_at.or(run.updated_at);
            InboxItem::new(
                format!("rule_run:{}", run.id),
                format!("{} failed on {}", run.rule, run.subject().words()),
                run.error
                    .clone()
                    .unwrap_or_else(|| "The rule failed after its retries.".to_owned()),
            )
            .url(Some(format!("/rule-runs/{}", run.id)))
            .due(at.map(dubai_date))
            .at(at)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_groups_are_the_five_of_the_spec() {
        let labels: Vec<_> = GROUPS.iter().map(|(_, l)| *l).collect();
        assert_eq!(
            labels,
            [
                "Waiting for your approval",
                "Overdue",
                "Mentions",
                "Notifications",
                "Failed automations"
            ]
        );
    }

    #[test]
    fn a_mention_is_told_by_its_kind() {
        assert!(is_mention("ticket.mention"));
        assert!(is_mention("Mentioned"));
        assert!(!is_mention("wl-notification"));
    }

    #[test]
    #[should_panic(expected = "is not an inbox group")]
    fn an_unknown_group_panics() {
        register_source("later", "x", |_| Box::pin(async { Ok(Vec::new()) }));
    }
}
