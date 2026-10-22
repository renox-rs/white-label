//! The paper orders of an outage as late entries (06 F&B §4.6 row *Internet or
//! power outage at the outlet*, `FNB.RUL.15`, `PLT.ACL.10`; BL-FNB-14 and
//! BL-FNB-15).
//!
//! While the devices are silent the kitchen works from the aggregators'
//! printouts and paper tickets, and counter sales go on paper. When the outlet
//! is back, each paper order is entered with its **business time** and carries
//! the **late-entry flag** with a reason: the kernel's check (`wl-kernel`
//! §5.13) decides from the business time and the entry time.
//!
//! - More than 30 minutes late (and up to 24 hours): flagged and recorded.
//! - More than 24 hours late, or **into a submitted daily closing**: an
//!   approval request for the area manager (holders of `late_entries.approve`).
//!   On approval the order is recorded and the closing is `reopened` with the
//!   reason *late entries after an outage*, and the kitchen manager is told
//!   ([`crate::closings::on_late_entry`]). A late entry into a closing the
//!   area manager already `approved` is **refused**; the area manager gets a
//!   task to record it as an adjustment in the next day's closing.
//!
//! A paper order is this app's own record (the sales order and the kitchen
//! ticket were never made): it carries the outlet, the channel, the paper
//! reference, the amount and the method, and counts in the closing of its
//! business date. The kernel's period check only knows the business time, so
//! the late-entry type is registered **per outlet code**
//! (`fnb.paper_order_<code>`, [`crate::Fnb::kitchen_devices`]); an outlet not
//! named there uses the general type, which flags and records but cannot see a
//! submitted closing (its approved closing is still refused by the service).

use renox::chrono::{NaiveDate, NaiveTime};
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::StateMachine;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::DUBAI_OFFSET_SECONDS;
use wl_kernel::late_entries::{self, Entered, EntrySpec, Period};
use wl_kernel::machines::BoxFuture;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::{LateEntryDef, NewRecord, Subject};

use crate::catalogue::Outlet;
use crate::closing::{ClosingStatus, DailyClosing, REOPEN_LATE};
use crate::outage::Outage;
use crate::permissions;
use crate::placing::refused;
use crate::texts::english;

/// The general late-entry type of a paper order.
pub const GENERAL_SUBJECT: &str = "fnb.paper_order";

/// The prefix of the per-outlet late-entry types.
pub const SUBJECT_PREFIX: &str = "fnb.paper_order_";

/// The late-entry type of the paper orders of the outlet `code`.
pub fn subject_for(code: &str) -> String {
    format!("{SUBJECT_PREFIX}{code}")
}

/// Whether `subject_type` is a paper order's.
pub fn is_paper_subject(subject_type: &str) -> bool {
    subject_type == GENERAL_SUBJECT || subject_type.starts_with(SUBJECT_PREFIX)
}

/// How a paper order was paid.
pub mod method {
    /// Cash.
    pub const CASH: &str = "cash";
    /// Card.
    pub const CARD: &str = "card";
    /// Charged to the aggregator (its printout).
    pub const AGGREGATOR: &str = "aggregator";
    /// All of them.
    pub const ALL: [&str; 3] = [CASH, CARD, AGGREGATOR];
}

/// An order written on paper during an outage and entered afterwards.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_paper_orders", index(outlet_id))]
pub struct PaperOrder {
    /// The primary key.
    pub id: i64,
    /// The outlet.
    #[model(references = "fnb_outlets")]
    pub outlet_id: i64,
    /// The channel it came through (`counter`, `phone`, an aggregator's key).
    pub channel: String,
    /// The number on the paper ticket or the aggregator's printout.
    pub reference: String,
    /// What it was worth with VAT, in fils.
    pub amount: i64,
    /// `cash`, `card` or `aggregator`.
    pub method: String,
    /// A note of the person who entered it.
    pub note: String,
    /// The business time: when the order really happened.
    pub occurred_at: Option<DateTime>,
    /// When it was typed in.
    pub entered_at: Option<DateTime>,
    /// Who entered it.
    pub entered_by: Option<i64>,
    /// The trading day it belongs to by the outlet's cut-off.
    pub business_date: Option<NaiveDate>,
    /// The late-entry flag.
    pub late: bool,
    /// Why it is late.
    pub late_reason: Option<String>,
    /// The kernel's late entry.
    pub late_entry_id: Option<i64>,
    /// The outage it belongs to.
    pub outage_id: Option<i64>,
    /// The closing this entry reopened.
    pub reopened_closing_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PaperOrder {
    /// The order as a [`Subject`] of the general type.
    pub fn subject(&self) -> Subject {
        Subject::new(GENERAL_SUBJECT, self.id)
    }

    /// The paper orders of the outlet's business date, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_day(db: &Db, outlet_id: i64, date: NaiveDate) -> Result<Vec<PaperOrder>> {
        PaperOrder::query()
            .where_(PaperOrder::OUTLET_ID.eq(outlet_id))
            .where_(PaperOrder::BUSINESS_DATE.eq(date))
            .order_by(PaperOrder::OCCURRED_AT)
            .get(db)
            .await
    }

    /// The latest paper orders of the outlet, newest business time first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn latest(db: &Db, outlet_id: i64, limit: u64) -> Result<Vec<PaperOrder>> {
        PaperOrder::query()
            .where_(PaperOrder::OUTLET_ID.eq(outlet_id))
            .order_by_desc(PaperOrder::OCCURRED_AT)
            .limit(limit)
            .get(db)
            .await
    }
}

/// The kernel's handler: writes the order in its transaction and, when the
/// closing of its business date is `submitted`, reopens it (the approval was
/// given: the handler runs only then).
fn create_paper_order<'a>(
    tx: &'a mut renox::db::Transaction,
    new: &'a NewRecord,
) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        let outlet_id = new.payload["outlet_id"].as_i64().unwrap_or_default();
        let outlet = Outlet::find(&mut *tx, outlet_id)
            .await?
            .ok_or(Error::NotFound)?;
        let date = outlet.business_date(new.occurred_at);
        let mut reopened = None;
        if let Some(closing) = DailyClosing::query()
            .where_(DailyClosing::OUTLET_ID.eq(outlet_id))
            .where_(DailyClosing::BUSINESS_DATE.eq(date))
            .first(&mut *tx)
            .await?
            && closing.status == ClosingStatus::Submitted
        {
            let _outcome = closing
                .transition(REOPEN_LATE)
                .by_system()
                .reason(english("fnb.closing.reopen_late_reason", &[]))
                .run_in(&mut *tx)
                .await
                .map_err(|e| e.into_error())?;
            reopened = Some(closing.id);
        }
        let row = PaperOrder::create(
            &mut *tx,
            PaperOrder {
                outlet_id,
                channel: new.payload["channel"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                reference: new.payload["reference"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                amount: new.payload["amount"].as_i64().unwrap_or_default(),
                method: new.payload["method"]
                    .as_str()
                    .unwrap_or(method::CASH)
                    .to_owned(),
                note: new.payload["note"].as_str().unwrap_or_default().to_owned(),
                occurred_at: Some(new.occurred_at),
                entered_at: Some(new.entered_at),
                entered_by: Some(new.user_id),
                business_date: Some(date),
                late: new.late,
                late_reason: new.reason.clone(),
                late_entry_id: new.late_entry_id,
                outage_id: new.payload["outage_id"].as_i64(),
                reopened_closing_id: reopened,
                ..PaperOrder::default()
            },
        )
        .await?;
        Ok(row.id)
    })
}

/// What the kernel's check says about the period of a business time: a
/// submitted closing may be reopened (approval), an approved one may not.
fn period_for<'a>(
    tx: &'a mut renox::db::Transaction,
    code: String,
    at: DateTime,
) -> BoxFuture<'a, Result<Period>> {
    Box::pin(async move {
        let Some(outlet) = Outlet::query()
            .where_(Outlet::CODE.eq(code.as_str()))
            .first(&mut *tx)
            .await?
        else {
            return Ok(Period::Open);
        };
        let date = outlet.business_date(at);
        let closing = DailyClosing::query()
            .where_(DailyClosing::OUTLET_ID.eq(outlet.id))
            .where_(DailyClosing::BUSINESS_DATE.eq(date))
            .first(&mut *tx)
            .await?;
        Ok(match closing.map(|c| c.status) {
            Some(ClosingStatus::Submitted) => Period::Closed,
            Some(ClosingStatus::Approved) => {
                Period::Locked(english("fnb.paper.problem.locked", &[("date", &date)]))
            }
            _ => Period::Open,
        })
    })
}

/// Registers the general late-entry type and one per outlet `code`.
pub(crate) fn register_late_entries(codes: &[String]) {
    LateEntryDef::of(GENERAL_SUBJECT, create_paper_order)
        .business_time("occurred_at")
        .parent("closing")
        .register();
    for code in codes {
        let code = code.clone();
        LateEntryDef::of(&subject_for(&code), create_paper_order)
            .business_time("occurred_at")
            .parent("closing")
            .period(move |tx, at| period_for(tx, code.clone(), at))
            .register();
    }
}

/// What the person enters for a paper order.
#[derive(Debug, Clone, Default)]
pub struct PaperAsk {
    /// The outlet.
    pub outlet_id: i64,
    /// The channel.
    pub channel: String,
    /// The number on the paper ticket or printout.
    pub reference: String,
    /// What it was worth with VAT, in fils.
    pub amount: i64,
    /// `cash`, `card` or `aggregator`; cash when blank.
    pub method: String,
    /// A note.
    pub note: String,
    /// The day it happened (Asia/Dubai).
    pub occurred_on: Option<NaiveDate>,
    /// The time it happened, `HH:MM` (Asia/Dubai).
    pub occurred_time: String,
    /// Why it is late.
    pub reason: String,
    /// The outage reference; the outlet's outage covering the time when blank.
    pub outage_reference: String,
}

/// The business time of the ask, as an instant.
pub fn occurred_at(ask: &PaperAsk) -> Result<DateTime> {
    let Some(day) = ask.occurred_on else {
        return Err(refused(english("fnb.paper.problem.day", &[])));
    };
    let time = NaiveTime::parse_from_str(ask.occurred_time.trim(), "%H:%M")
        .map_err(|_| refused(english("fnb.paper.problem.time", &[])))?;
    let local = day.and_time(time);
    let utc = local - renox::chrono::TimeDelta::seconds(i64::from(DUBAI_OFFSET_SECONDS));
    Ok(utc.and_utc())
}

/// A person with `paper_orders.enter` enters a paper order with its business
/// time (`PLT.ACL.10`).
///
/// # Errors
///
/// 403 without `paper_orders.enter`; see [`enter_as`].
pub async fn enter(state: &AppState, user: &AuthUser, ask: &PaperAsk) -> Result<Entered> {
    if !user.has_permission(permissions::PAPER_ORDERS_ENTER) {
        return Err(Error::Forbidden);
    }
    enter_as(state, user.id, ask).await
}

/// Enters a paper order for the user `user_id` (a seed has no signed-in user;
/// the permission is [`enter`]'s).
///
/// # Errors
///
/// 422 without a reference or a channel, for a negative amount, an unknown
/// method, a day or a time that is not one, a late entry without its reason
/// (the kernel's words) and a business time in a closing the area manager
/// approved (the area manager gets a task to record it in the next day's
/// closing); 404 for an unknown outlet.
pub async fn enter_as(state: &AppState, user_id: i64, ask: &PaperAsk) -> Result<Entered> {
    let outlet = Outlet::find_or_404(&state.db, ask.outlet_id).await?;
    let reference = ask.reference.trim().to_owned();
    let channel = ask.channel.trim().to_owned();
    if reference.is_empty() || channel.is_empty() {
        return Err(refused(english("fnb.paper.problem.identity", &[])));
    }
    if ask.amount < 0 {
        return Err(refused(english("fnb.paper.problem.amount", &[])));
    }
    let how = if ask.method.trim().is_empty() {
        method::CASH.to_owned()
    } else {
        ask.method.trim().to_owned()
    };
    if !method::ALL.contains(&how.as_str()) {
        return Err(refused(english("fnb.paper.problem.method", &[])));
    }
    let at = occurred_at(ask)?;
    let date = outlet.business_date(at);
    let closing = DailyClosing::of_day(&state.db, outlet.id, date).await?;
    if let Some(found) = &closing
        && found.status == ClosingStatus::Approved
    {
        adjustment_task(state, &outlet, found, &reference, ask.amount).await?;
        return Err(refused(english(
            "fnb.paper.problem.locked",
            &[("date", &date)],
        )));
    }
    let outage = Outage::covering(&state.db, outlet.id, at).await?;
    let reference_text = if ask.outage_reference.trim().is_empty() {
        outage
            .as_ref()
            .map(|o| {
                english(
                    "fnb.paper.outage_reference",
                    &[("id", &o.id), ("outlet", &outlet.name)],
                )
            })
            .unwrap_or_default()
    } else {
        ask.outage_reference.clone()
    };
    let subject_type = {
        let own = subject_for(&outlet.code);
        if late_entries::get(&own).is_some() {
            own
        } else {
            GENERAL_SUBJECT.to_owned()
        }
    };
    let mut spec = EntrySpec::new(&subject_type, user_id, at)
        .scope(outlet.scope())
        .reason(&ask.reason)
        .outage_reference(&reference_text)
        .payload(json!({
            "outlet_id": outlet.id,
            "channel": channel,
            "reference": reference,
            "amount": ask.amount,
            "method": how,
            "note": ask.note.trim(),
            "outage_id": outage.as_ref().map(|o| o.id),
        }));
    if let Some(found) = &closing {
        spec = spec.parent(found.subject());
    }
    late_entries::enter(state, spec).await
}

/// The area manager's task to record an entry into an approved closing as an
/// adjustment in the next day's closing.
async fn adjustment_task(
    state: &AppState,
    outlet: &Outlet,
    closing: &DailyClosing,
    reference: &str,
    amount: i64,
) -> Result {
    let date = closing
        .business_date
        .unwrap_or_else(wl_kernel::documents::today);
    let next = date.succ_opt().unwrap_or(date);
    let title = english(
        "fnb.paper.task_title",
        &[("outlet", &outlet.name), ("date", &date), ("next", &next)],
    );
    let description = english(
        "fnb.paper.task_body",
        &[
            ("outlet", &outlet.name),
            ("reference", &reference),
            ("amount", &crate::money::aed(amount)),
            ("date", &date),
            ("next", &next),
            ("link", &format!("/closings/{}", closing.id)),
        ],
    );
    role_tasks::open(
        state,
        RoleTask::new(
            closing.subject(),
            permissions::AREA_MANAGER_ROLE,
            &title,
            &format!("fnb.closing_adjustment:{}:{}", closing.id, reference),
        )
        .description(&description),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subject_of_an_outlet_is_a_valid_late_entry_type() {
        assert_eq!(subject_for("marina"), "fnb.paper_order_marina");
        let _ = Subject::new(&subject_for("marina"), 1);
        assert!(is_paper_subject("fnb.paper_order_marina"));
        assert!(is_paper_subject(GENERAL_SUBJECT));
        assert!(!is_paper_subject("fnb.late_order"));
    }
}
