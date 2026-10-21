//! The Cash holder and the Cash handover (wl-payments §3, §4, `FIN.CSH.01`;
//! base for #110, #113, #114, #116).
//!
//! A [`CashHolder`] is a staff user who holds cash (per person; a till's
//! session mode comes with BL-PAY-19). A [`CashHandover`] is the cash a
//! holder collected in one period (a run, a shift or a day): `open` while the
//! cash is with the holder, `counted` when another user counted at least the
//! expected amount, `short` when less, then `recovered` or `written_off`.
//! A *later shortage* is a handover raised against a holder after the
//! cash was counted; it is `short` from the start. The status moves only
//! through the machine ([`crate::cash_machine`]). What was collected into a
//! handover is kept as [`CashEntry`] rows, every count as a [`CashCount`], and
//! what a count or a write-off leaves for the accountant as a
//! [`CashPosting`]. Money is `i64` fils of AED.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::custody::{
    CASH_HANDOVER_SUBJECT_TYPE, HandoverId, HandoverKind, HandoverPeriod, HandoverStatus,
    HandoverSummary, PeriodKind,
};

/// How a holder's cash is counted (wl-payments §3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HolderMode {
    /// One handover per person and period.
    #[default]
    PerPerson,
    /// One handover per till session (BL-PAY-19).
    TillSession,
}

/// A staff user who holds cash. Created with the first collection.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_payments_cash_holders", unique(user_id))]
pub struct CashHolder {
    /// The primary key.
    pub id: i64,
    /// The staff user.
    pub user_id: i64,
    /// Per person or till session.
    pub mode: HolderMode,
    /// The period of their handover.
    pub period_kind: PeriodKind,
    /// The outlet or branch that bears a written-off loss.
    pub location: Option<String>,
    /// The legal entity of the cash.
    pub legal_entity_id: Option<i64>,
    /// The holder's manager; empty takes wl-hr's, else the manager role.
    pub manager_user_id: Option<i64>,
    /// Whether the holder still holds cash.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CashHolder {
    /// The holder row of the user `user_id`, made per person and per day
    /// when there is none; `location` and `legal_entity_id` fill what the
    /// row lacks.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn ensure(
        db: &Db,
        user_id: i64,
        location: Option<&str>,
        legal_entity_id: Option<i64>,
    ) -> Result<CashHolder> {
        if let Some(mut holder) = CashHolder::query()
            .where_(CashHolder::USER_ID.eq(user_id))
            .first(db)
            .await?
        {
            let mut changed = false;
            if holder.location.is_none() && location.is_some() {
                holder.location = location.map(str::to_owned);
                changed = true;
            }
            if holder.legal_entity_id.is_none() && legal_entity_id.is_some() {
                holder.legal_entity_id = legal_entity_id;
                changed = true;
            }
            if changed {
                holder.save(db).await?;
            }
            return Ok(holder);
        }
        let mut holder = CashHolder {
            user_id,
            location: location.map(str::to_owned),
            legal_entity_id,
            active: true,
            ..Default::default()
        };
        holder.insert(db).await?;
        Ok(holder)
    }
}

/// The cash a holder collected in one period. Money is `i64` fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_cash_handovers",
    state_machine,
    index(status),
    index(holder_user_id, period_date),
    index(period_type, period_id),
    index(source_type, source_id)
)]
pub struct CashHandover {
    /// The primary key.
    pub id: i64,
    /// The cash of a period, or a shortage found later.
    pub kind: HandoverKind,
    /// The holder row.
    pub holder_id: i64,
    /// The holder, a staff user.
    pub holder_user_id: i64,
    /// The person recoveries are taken from.
    pub responsible_user_id: i64,
    /// Run, shift, day or till session.
    pub period_kind: PeriodKind,
    /// The type of the record that is the period (a run).
    pub period_type: Option<String>,
    /// The id of the record that is the period.
    pub period_id: Option<i64>,
    /// The day of the period (Asia/Dubai).
    pub period_date: NaiveDate,
    /// When the period ends; a day ends at 23:59 Asia/Dubai of its date.
    pub period_end: Option<DateTime>,
    /// Whether `FIN.RUL.22` escalates it when left open.
    pub escalate: bool,
    /// What the holder is expected to hand over.
    pub expected: i64,
    /// What was counted (the latest count).
    pub counted: i64,
    /// Expected less counted while `short`; 0 otherwise.
    pub difference: i64,
    /// Counted above expected.
    pub overage: i64,
    /// The recoveries recorded on a shortage.
    pub recovered: i64,
    /// What was written off.
    pub written_off: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The holder's outlet or branch.
    pub location: Option<String>,
    /// The legal entity of the cash.
    pub legal_entity_id: Option<i64>,
    /// The type of the record that caused a later shortage.
    pub source_type: Option<String>,
    /// The id of the record that caused a later shortage.
    pub source_id: Option<i64>,
    /// Why a later shortage was raised.
    pub reason: Option<String>,
    /// Who counted first.
    pub counted_by: Option<i64>,
    /// When it was first counted.
    pub counted_at: Option<DateTime>,
    /// Who recounted last.
    pub recounted_by: Option<i64>,
    /// Whether the holder was present at the last recount.
    pub holder_present: Option<bool>,
    /// Whether the last count was made without the holder.
    pub holder_absent: bool,
    /// When `FIN.RUL.22` gave the manager the task.
    pub task_at: Option<DateTime>,
    /// When `FIN.RUL.22` told the company admin.
    pub admin_notified_at: Option<DateTime>,
    /// Who raised a later shortage.
    pub created_by: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: HandoverStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CashHandover {
    /// The id as the kernel contract names it.
    pub fn handover_id(&self) -> HandoverId {
        HandoverId(self.id)
    }

    /// The handover as a subject.
    pub fn subject(&self) -> Subject {
        Subject::new(CASH_HANDOVER_SUBJECT_TYPE, self.id)
    }

    /// The record that is the period, if it is one.
    pub fn period_subject(&self) -> Option<Subject> {
        match (&self.period_type, self.period_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// The record that caused a later shortage.
    pub fn source(&self) -> Option<Subject> {
        match (&self.source_type, self.source_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// What is left of the difference after the recoveries recorded (never
    /// below 0).
    pub fn rest(&self) -> i64 {
        (self.difference - self.recovered).max(0)
    }

    /// When the holder's period ends: the recorded end, else 23:59
    /// Asia/Dubai (19:59 UTC) of the period's date.
    pub fn ends_at(&self) -> DateTime {
        self.period_end
            .unwrap_or_else(|| end_of_day(self.period_date))
    }

    /// What other modules see of it.
    pub fn summary(&self) -> HandoverSummary {
        HandoverSummary {
            id: self.handover_id(),
            kind: self.kind,
            status: self.status,
            holder_user: self.holder_user_id,
            responsible_user: self.responsible_user_id,
            period_kind: self.period_kind,
            period: self.period_subject(),
            date: self.period_date,
            expected: self.expected,
            counted: self.counted,
            difference: self.difference,
            recovered: self.recovered,
        }
    }

    /// The handover of the period `period` held by `holder_user`, if there is
    /// one: of the record that is the period, else of the period's day.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_period(
        db: &Db,
        holder_user: i64,
        period: &HandoverPeriod,
    ) -> Result<Option<CashHandover>> {
        let query = CashHandover::query()
            .where_(CashHandover::HOLDER_USER_ID.eq(holder_user))
            .where_(CashHandover::KIND.eq(HandoverKind::Period))
            .where_(CashHandover::PERIOD_KIND.eq(period.kind));
        let query = match &period.subject {
            Some(subject) => query
                .where_(CashHandover::PERIOD_TYPE.eq(subject.subject_type.clone()))
                .where_(CashHandover::PERIOD_ID.eq(subject.subject_id)),
            None => query.where_(CashHandover::PERIOD_DATE.eq(period.date)),
        };
        query.order_by(CashHandover::ID).first(db).await
    }

    /// The handover of `holder_user` whose period is the record `period`.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_period_record(
        db: &Db,
        holder_user: i64,
        period: &Subject,
    ) -> Result<Option<CashHandover>> {
        CashHandover::query()
            .where_(CashHandover::HOLDER_USER_ID.eq(holder_user))
            .where_(CashHandover::KIND.eq(HandoverKind::Period))
            .where_(CashHandover::PERIOD_TYPE.eq(period.subject_type.clone()))
            .where_(CashHandover::PERIOD_ID.eq(period.subject_id))
            .order_by(CashHandover::ID)
            .first(db)
            .await
    }
}

/// 23:59 Asia/Dubai (UTC+4, no daylight saving) of `date`, as an instant.
pub fn end_of_day(date: NaiveDate) -> DateTime {
    date.and_hms_opt(19, 59, 0)
        .expect("19:59 is a time")
        .and_utc()
}

/// Cash collected into a handover (negative: taken out again).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_cash_entries",
    index(handover_id),
    index(source_type, source_id)
)]
pub struct CashEntry {
    /// The primary key.
    pub id: i64,
    /// The handover.
    pub handover_id: i64,
    /// The amount in fils; negative when the cash was released again.
    pub amount: i64,
    /// What it was collected for.
    pub source_type: String,
    /// The id of what it was collected for.
    pub source_id: i64,
    /// A note.
    pub note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CashEntry {
    /// The entries of `handover_id`, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_handover(db: &Db, handover_id: i64) -> Result<Vec<CashEntry>> {
        CashEntry::query()
            .where_(CashEntry::HANDOVER_ID.eq(handover_id))
            .order_by(CashEntry::ID)
            .get(db)
            .await
    }

    /// The entries made for `source`, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_source(db: &Db, source: &Subject) -> Result<Vec<CashEntry>> {
        CashEntry::query()
            .where_(CashEntry::SOURCE_TYPE.eq(source.subject_type.clone()))
            .where_(CashEntry::SOURCE_ID.eq(source.subject_id))
            .order_by(CashEntry::ID)
            .get(db)
            .await
    }
}

/// Whether a count is the first or a recount.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CountKind {
    /// The first count.
    #[default]
    Count,
    /// A recount of a short handover.
    Recount,
}

/// One count of a handover.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_payments_cash_counts", index(handover_id))]
pub struct CashCount {
    /// The primary key.
    pub id: i64,
    /// The handover.
    pub handover_id: i64,
    /// Count or recount.
    pub kind: CountKind,
    /// The user who counted.
    pub counter_id: i64,
    /// What they counted, in fils.
    pub amount: i64,
    /// Whether the holder was present (a recount).
    pub holder_present: Option<bool>,
    /// Whether the count was made without the holder (`FIN.RUL.22`).
    pub holder_absent: bool,
    /// A note.
    pub note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CashCount {
    /// The counts of `handover_id`, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_handover(db: &Db, handover_id: i64) -> Result<Vec<CashCount>> {
        CashCount::query()
            .where_(CashCount::HANDOVER_ID.eq(handover_id))
            .order_by(CashCount::ID)
            .get(db)
            .await
    }
}

/// What a count or a write-off leaves for the accountant.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PostingKind {
    /// An overage, held as a liability and not as income until the
    /// accountant decides (§12 question 4).
    #[default]
    OverageLiability,
    /// A written-off shortage, a loss of the holder's outlet or branch.
    ShortageLoss,
}

/// An overage liability or a shortage loss.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_payments_cash_postings", index(handover_id))]
pub struct CashPosting {
    /// The primary key.
    pub id: i64,
    /// The handover.
    pub handover_id: i64,
    /// Overage liability or shortage loss.
    pub kind: PostingKind,
    /// The amount in fils.
    pub amount: i64,
    /// The holder's outlet or branch that bears a loss.
    pub location: Option<String>,
    /// The legal entity.
    pub legal_entity_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CashPosting {
    /// The postings of `handover_id`, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_handover(db: &Db, handover_id: i64) -> Result<Vec<CashPosting>> {
        CashPosting::query()
            .where_(CashPosting::HANDOVER_ID.eq(handover_id))
            .order_by(CashPosting::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handover() -> CashHandover {
        CashHandover {
            id: 4,
            period_date: "2027-05-01".parse().unwrap(),
            expected: 90_000,
            difference: 50_000,
            recovered: 20_000,
            ..Default::default()
        }
    }

    #[test]
    fn a_day_ends_at_23_59_in_dubai() {
        let end = handover().ends_at();
        assert_eq!(end.to_rfc3339(), "2027-05-01T19:59:00+00:00");
    }

    #[test]
    fn a_recorded_end_wins_over_the_end_of_the_day() {
        let mut h = handover();
        let end: DateTime = "2027-05-01T11:00:00Z".parse().unwrap();
        h.period_end = Some(end);
        assert_eq!(h.ends_at(), end);
    }

    #[test]
    fn the_rest_is_the_difference_less_the_recoveries_and_never_below_zero() {
        let mut h = handover();
        assert_eq!(h.rest(), 30_000);
        h.recovered = 70_000;
        assert_eq!(h.rest(), 0);
    }

    #[test]
    fn a_handover_is_a_subject_and_has_a_summary() {
        let h = handover();
        assert_eq!(h.subject(), Subject::new("payments.cash_handover", 4));
        let summary = h.summary();
        assert_eq!(summary.expected, 90_000);
        assert_eq!(summary.difference, 50_000);
        assert_eq!(summary.status, HandoverStatus::Open);
        assert_eq!(h.period_subject(), None);
        assert_eq!(h.source(), None);
    }
}
