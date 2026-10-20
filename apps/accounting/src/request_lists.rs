//! The document request lists of an engagement's periods (04 §4.2, §7.2
//! `ACC.RUL.02`, `POR.RUL.01`): the minimal base the suspension and the
//! resume need (BL-ACC-03, BL-ACC-05).
//!
//! - **`ACC.RUL.02`** ([`run_period_end`]): when a month ends, each `active`
//!   engagement gets the request list of that period, due
//!   [`LEAD_DAYS`] days later, and the client is told. A `suspended`
//!   engagement gets none: the period is recorded as *skipped*, so `resume`
//!   creates it ([`create_skipped`]).
//! - **`POR.RUL.01`** ([`remind`]): 3 and 7 days after the due date of an
//!   open list ([`REMIND_AFTER_DAYS`]) the client is reminded, unless the
//!   engagement is suspended: an open list keeps its state, but the client
//!   is not reminded while suspended.
//!
//! Decision: the document request lifecycle (the items asked, uploads,
//! review, the portal's request pages) is not built yet; a list here is
//! one row per engagement and period, open until it is marked received
//! ([`mark_received`]). The periods are months for every engagement, and
//! the template's lead time is [`LEAD_DAYS`]. The client is reached like
//! the deadline notice: its client approvers, else the company
//! ([`crate::deadline_notices::client_recipients`]).

use std::sync::{Arc, Mutex, PoisonError};

use renox::chrono::{Datelike, Duration, Months, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::compliance::{occurrence_key, schedule_subject};
use wl_kernel::documents::today;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest, notifier};
use wl_kernel::{PartyId, Subject};

use crate::deadline_notices::client_recipients;
use crate::engagement::{Engagement, EngagementStatus};
use crate::permissions::MANAGER_ROLE;
use crate::texts::english;

/// The rule that creates the request lists.
pub const RULE: &str = "ACC.RUL.02";
/// The rule that reminds the client of an open list.
pub const REMINDER_RULE: &str = "POR.RUL.01";
/// Days after the period end (or the day it is created late) a list is
/// due: the template's lead time.
pub const LEAD_DAYS: i64 = 10;
/// Days after the due date the client is reminded (N = 3, 7).
pub const REMIND_AFTER_DAYS: [i64; 2] = [3, 7];
/// When the daily run creates the lists of a period that ended and sends
/// the reminders, Asia/Dubai.
pub const RUN_AT: &str = "01:00";
/// The scheduled task and command of the daily run.
pub const TASK: &str = "accounting:request_lists";

/// The client is asked for the documents of one or more periods.
pub const REQUEST_LIST_NOTICE: &str = "accounting.request_list";
/// The client is reminded of an open list.
pub const REQUEST_REMINDER_NOTICE: &str = "accounting.request_reminder";
/// The subject type of a request list.
pub const REQUEST_LIST_SUBJECT_TYPE: &str = "accounting.request_list";

/// The module that sends the notices.
const MODULE: &str = "accounting";

/// The request list of an engagement's period, or the period skipped.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "accounting_request_lists", index(engagement_id))]
pub struct RequestList {
    /// The primary key.
    pub id: i64,
    /// The engagement.
    #[model(references = "accounting_engagements")]
    pub engagement_id: i64,
    /// The client company.
    pub client_party_id: i64,
    /// The last day of the period.
    pub period_end: NaiveDate,
    /// The day the period ended while the engagement was suspended: no
    /// list was created then.
    pub skipped_on: Option<NaiveDate>,
    /// The day the list was created and the client asked; `None` while the
    /// period is skipped.
    pub requested_on: Option<NaiveDate>,
    /// The day the documents are due.
    pub due_date: Option<NaiveDate>,
    /// The day the documents were received.
    pub received_on: Option<NaiveDate>,
    /// How many reminders the client got.
    pub reminders_sent: i64,
    /// The day of the last reminder.
    pub last_reminded_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RequestList {
    /// The list as a kernel subject: `accounting.request_list:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(REQUEST_LIST_SUBJECT_TYPE, self.id)
    }

    /// Whether it was created and the documents are not received yet.
    pub fn is_open(&self) -> bool {
        self.requested_on.is_some() && self.received_on.is_none()
    }

    /// Whether its period was skipped and no list is created yet.
    pub fn is_skipped(&self) -> bool {
        self.requested_on.is_none()
    }

    /// The lists and skipped periods of engagement `id`, by period.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_engagement(db: &Db, id: i64) -> Result<Vec<RequestList>> {
        RequestList::query()
            .where_(RequestList::ENGAGEMENT_ID.eq(id))
            .order_by(RequestList::PERIOD_END)
            .get(db)
            .await
    }
}

/// Whether `day` is the last day of its month.
pub fn is_month_end(day: NaiveDate) -> bool {
    (day + Duration::days(1)).month() != day.month()
}

/// The month ends after `from` up to and including `until`.
pub fn month_ends(from: NaiveDate, until: NaiveDate) -> Vec<NaiveDate> {
    let mut ends = Vec::new();
    let Some(first) = from.with_day(1) else {
        return ends;
    };
    let mut month = first;
    while let Some(next) = month.checked_add_months(Months::new(1)) {
        let end = next - Duration::days(1);
        if end > until {
            break;
        }
        if end > from {
            ends.push(end);
        }
        month = next;
    }
    ends
}

/// What a period-end run did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PeriodEnd {
    /// The lists created (active engagements).
    pub created: Vec<i64>,
    /// The periods recorded as skipped (suspended engagements).
    pub skipped: Vec<i64>,
}

/// `ACC.RUL.02` for the period ending `period_end`, without the rule run:
/// a list for each `active` engagement (the client told), a skipped period
/// for each `suspended` one. A period already recorded is left as it is.
///
/// # Errors
///
/// The database's and the notifier's errors.
pub async fn period_end_now(state: &AppState, period_end: NaiveDate) -> Result<PeriodEnd> {
    let mut report = PeriodEnd::default();
    let served = Engagement::query()
        .where_(Engagement::STATUS.is_in([EngagementStatus::Active, EngagementStatus::Suspended]))
        .order_by(Engagement::ID)
        .get(&state.db)
        .await?;
    for engagement in served {
        let known = RequestList::query()
            .where_(RequestList::ENGAGEMENT_ID.eq(engagement.id))
            .where_(RequestList::PERIOD_END.eq(period_end))
            .first(&state.db)
            .await?;
        if known.is_some() {
            continue;
        }
        let mut list = RequestList {
            engagement_id: engagement.id,
            client_party_id: engagement.client_party_id,
            period_end,
            ..RequestList::default()
        };
        if engagement.status == EngagementStatus::Suspended {
            list.skipped_on = Some(period_end);
            let list = RequestList::create(&state.db, list).await?;
            report.skipped.push(list.id);
        } else {
            list.requested_on = Some(period_end);
            list.due_date = Some(period_end + Duration::days(LEAD_DAYS));
            let list = RequestList::create(&state.db, list).await?;
            tell_client(state, &engagement, std::slice::from_ref(&list)).await?;
            report.created.push(list.id);
        }
    }
    Ok(report)
}

/// `ACC.RUL.02` for the period ending `period_end`, as a rule run (once per
/// period).
///
/// # Errors
///
/// The database's errors.
pub async fn run_period_end(state: &AppState, period_end: NaiveDate) -> Result<PeriodEnd> {
    let spec = RuleRunSpec::new(RULE, schedule_subject(), occurrence_key(RULE, period_end));
    let seen = Arc::new(Mutex::new(PeriodEnd::default()));
    let out = Arc::clone(&seen);
    run_rule(state, spec, move |state: AppState| async move {
        let report = period_end_now(&state, period_end).await?;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = report;
        Ok(())
    })
    .await?;
    let report = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(report)
}

/// Tells the client of `engagement` that `lists` are open, in one notice.
async fn tell_client(state: &AppState, engagement: &Engagement, lists: &[RequestList]) -> Result {
    let Some(first) = lists.first() else {
        return Ok(());
    };
    let periods = lists
        .iter()
        .map(|l| l.period_end.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let due = first.due_date.map(|d| d.to_string()).unwrap_or_default();
    let mut request = NotifyRequest::new(REQUEST_LIST_NOTICE, engagement.subject())
        .subject_label(&engagement.number())
        .data(json!({
            "number": engagement.number(),
            "periods": periods,
            "due": due,
            "lists": lists.iter().map(|l| l.id).collect::<Vec<_>>(),
        }));
    for recipient in client_recipients(state, engagement.client(), today()).await? {
        request = request.to(recipient);
    }
    notifier(state).notify(state, request).await?;
    Ok(())
}

/// The engagement resumed on `on`: a list for each period skipped while it
/// was suspended, due `on` + [`LEAD_DAYS`], with one notice to the client
/// for all. Answers the lists created.
///
/// # Errors
///
/// The database's and the notifier's errors.
pub async fn create_skipped(
    state: &AppState,
    engagement: &Engagement,
    on: NaiveDate,
) -> Result<Vec<RequestList>> {
    let mut created = Vec::new();
    for mut list in RequestList::of_engagement(&state.db, engagement.id).await? {
        if !list.is_skipped() {
            continue;
        }
        list.requested_on = Some(on);
        list.due_date = Some(on + Duration::days(LEAD_DAYS));
        list.save(&state.db).await?;
        created.push(list);
    }
    tell_client(state, engagement, &created).await?;
    Ok(created)
}

/// The documents of list `id` were received on `on`: it is closed and no
/// reminder follows.
///
/// # Errors
///
/// 404 for an unknown list; the database's errors.
pub async fn mark_received(state: &AppState, id: i64, on: NaiveDate) -> Result<RequestList> {
    let mut list = RequestList::find_or_404(&state.db, id).await?;
    if list.received_on.is_none() {
        list.received_on = Some(on);
        list.save(&state.db).await?;
    }
    Ok(list)
}

/// What a reminder run did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reminded {
    /// The lists whose client was reminded.
    pub reminded: Vec<i64>,
    /// The lists due for a reminder whose engagement is suspended: the
    /// client is not reminded.
    pub held: Vec<i64>,
    /// The lists whose request notice gave up and has no proof of delivery:
    /// no reminder is due yet.
    pub undelivered: Vec<i64>,
}

/// Whether the request notice of `engagement` (one notice lists the periods
/// asked for) gave up without a proof of delivery.
///
/// # Errors
///
/// The notifier's errors.
pub async fn is_undelivered(state: &AppState, engagement: &Engagement) -> Result<bool> {
    Ok(notifier(state)
        .undelivered_notices(state, &engagement.subject())
        .await?
        .iter()
        .any(|n| n.type_key == REQUEST_LIST_NOTICE))
}

/// `POR.RUL.01` on `on`: each open list 3 and 7 days past its due date
/// reminds the client once per step, unless its engagement is suspended.
///
/// # Errors
///
/// The database's and the notifier's errors.
pub async fn remind(state: &AppState, on: NaiveDate) -> Result<Reminded> {
    let mut report = Reminded::default();
    let lists = RequestList::query()
        .where_(RequestList::REQUESTED_ON.is_not_null())
        .where_(RequestList::RECEIVED_ON.is_null())
        .order_by(RequestList::ID)
        .get(&state.db)
        .await?;
    for mut list in lists {
        let (Some(due), Ok(step)) = (list.due_date, usize::try_from(list.reminders_sent)) else {
            continue;
        };
        let Some(after) = REMIND_AFTER_DAYS.get(step) else {
            continue;
        };
        if on < due + Duration::days(*after) {
            continue;
        }
        let Some(engagement) = Engagement::find(&state.db, list.engagement_id).await? else {
            continue;
        };
        if engagement.status != EngagementStatus::Active || !engagement.may_message_client() {
            report.held.push(list.id);
            continue;
        }
        // The request's notice gave up and no proof of delivery is recorded:
        // the reminder deadlines have not started (`NTF.RUL.01`).
        if is_undelivered(state, &engagement).await? {
            report.undelivered.push(list.id);
            continue;
        }
        let mut request = NotifyRequest::new(REQUEST_REMINDER_NOTICE, list.subject())
            .subject_label(&engagement.number())
            .data(json!({
                "number": engagement.number(),
                "period": list.period_end.to_string(),
                "due": due.to_string(),
            }));
        for recipient in client_recipients(state, PartyId(list.client_party_id), on).await? {
            request = request.to(recipient);
        }
        notifier(state).notify(state, request).await?;
        list.reminders_sent += 1;
        list.last_reminded_on = Some(on);
        list.save(&state.db).await?;
        report.reminded.push(list.id);
    }
    Ok(report)
}

/// The notification types.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let subject = english("accounting.request_lists.notice_subject", &[]);
    let body = english("accounting.request_lists.notice_body", &[]);
    let reminder_subject = english("accounting.request_lists.reminder_subject", &[]);
    let reminder_body = english("accounting.request_lists.reminder_body", &[]);
    vec![
        // 04 §9 *Notices*: a document request and its reminders are
        // statutory: on give-up the contact point is marked invalid and the
        // engagement manager gets a task, due in one working day, to deliver
        // it by other means (BL-ACC-24).
        NotificationTypeDef::new(REQUEST_LIST_NOTICE, MODULE)
            .group("statutory")
            .statutory(true)
            .staff_role(MANAGER_ROLE)
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
        NotificationTypeDef::new(REQUEST_REMINDER_NOTICE, MODULE)
            .group("statutory")
            .statutory(true)
            .staff_role(MANAGER_ROLE)
            .template(
                NotifyChannel::Email,
                "en",
                Some(&reminder_subject),
                &reminder_body,
            ),
    ]
}

/// Registers the notification types and the daily run: the lists of the
/// month that ended yesterday, then the reminders.
pub(crate) fn register(app: &mut Registry) {
    for def in notification_types() {
        notify::register_type(def);
    }
    app.schedule()
        .daily_at(RUN_AT, TASK, |state: AppState| async move {
            daily(&state, today()).await
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Create the request lists of the month that ended and remind the clients of open lists (ACC.RUL.02, POR.RUL.01)",
        |_args, state: AppState| async move { daily(&state, today()).await },
    );
}

/// The daily run on `on`: `ACC.RUL.02` when yesterday ended a month, then
/// `POR.RUL.01`.
async fn daily(state: &AppState, on: NaiveDate) -> Result {
    let yesterday = on - Duration::days(1);
    if is_month_end(yesterday) {
        run_period_end(state, yesterday).await?;
    }
    remind(state, on).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn the_month_ends_between_two_days_are_listed() {
        assert!(is_month_end(day("2026-11-30")));
        assert!(!is_month_end(day("2026-11-29")));
        assert_eq!(
            month_ends(day("2026-10-01"), day("2026-12-05")),
            [day("2026-10-31"), day("2026-11-30")]
        );
        assert_eq!(
            month_ends(day("2026-10-31"), day("2026-11-30")),
            [day("2026-11-30")]
        );
        assert!(month_ends(day("2026-11-02"), day("2026-11-29")).is_empty());
    }
}
