//! The cash custody contract (wl-payments §3 *Cash holder*, *Cash handover*,
//! §4, §6, `FIN.CSH.01`): what a vertical or `wl-delivery` may do with the
//! cash a person holds without depending on the `wl-payments` crate.
//!
//! `wl-payments` provides a [`CashCustody`] with [`provide_custody`] and
//! emits [`CashShortage`], [`CashHandoverCounted`] and
//! [`CashShortageSettled`]. [`custody`] falls back to [`NoCustody`] when the
//! `payments` module is off (nothing is held then).
//!
//! A holder (a staff user) collects cash during a **period**: a run, a shift
//! or a day ([`HandoverPeriod`]). The first [`CashCollection`] in a period
//! opens the holder's handover for it (`open`); more collections of the same
//! period add to its expected amount. At the end of the period another user
//! counts it (`cash.count`, `FIN.ACL.05`); a short count becomes a shortage
//! that is recounted, recovered or written off.
//!
//! How `wl-delivery` uses it (BL-DLV-03, #159, #165, #164):
//!
//! - at `deliver`, [`CashCustody::collect`] with the driver, the run as the
//!   period ([`HandoverPeriod::run`]) and the delivery as the source. The
//!   amount is the cash collected, except for a COD difference the **driver**
//!   absorbs: the full expected cash is collected then, so the missing part
//!   stays expected in the handover and shows as a shortage at the count
//!   (never billed to the customer, wl-invoicing §5.2 step 4);
//! - when a delivery is reversed while the handover is still `open`,
//!   [`CashCustody::release`] takes its cash out again; once the handover is
//!   counted, the operations manager raises a later shortage instead
//!   (`raise_shortage`, #116) with the delivery as the source;
//! - the end of the run's period ([`CashCustody::end_period`]) starts the
//!   escalation clock (`FIN.RUL.22`); a driver of a vertical that has its
//!   own rule (03: `LOG.RUL.13`) is collected with
//!   [`HandoverPeriod::replaced_escalation`], so the driver gets one
//!   escalation, not two;
//! - a run closes when its handover is counted ([`CashHandoverCounted`],
//!   or [`CashCustody::handover_of`]).
//!
//! Money is `i64` fils of AED.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// The subject type of a cash handover, for [`Subject`] references.
pub const CASH_HANDOVER_SUBJECT_TYPE: &str = "payments.cash_handover";

/// The id of a cash handover.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct HandoverId(pub i64);

impl HandoverId {
    /// The handover as a [`Subject`]: `payments.cash_handover:42`.
    pub fn subject(self) -> Subject {
        Subject::new(CASH_HANDOVER_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for HandoverId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for HandoverId {
    fn from(id: i64) -> Self {
        HandoverId(id)
    }
}

/// The lifecycle state of a cash handover (wl-payments §4); the state of the
/// `wl-payments` Cash handover machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HandoverStatus {
    /// The holder has cash collected in the period.
    #[default]
    Open,
    /// Counted amount equals or exceeds expected (terminal).
    Counted,
    /// Counted less than expected.
    Short,
    /// The shortage was recovered from the holder (terminal).
    Recovered,
    /// The shortage was written off with approval (terminal).
    WrittenOff,
}

impl HandoverStatus {
    /// Whether the handover is over: `counted`, `recovered` or `written_off`.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            HandoverStatus::Counted | HandoverStatus::Recovered | HandoverStatus::WrittenOff
        )
    }
}

/// The kind of a handover: the cash of a period, or a shortage found later
/// (`raise_shortage`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HandoverKind {
    /// The cash a holder collected in a period.
    #[default]
    Period,
    /// A shortage raised after the holder's handover was counted.
    LaterShortage,
}

/// The period a holder's cash belongs to.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PeriodKind {
    /// A delivery run (03).
    Run,
    /// A shift.
    Shift,
    /// A day (the default of a per-person holder).
    #[default]
    Day,
    /// A till session (wl-sales, till session mode).
    TillSession,
}

/// The period of a holder's cash: which one, the record that is the period
/// (a run), its day and, once known, when it ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverPeriod {
    /// Run, shift, day or till session.
    pub kind: PeriodKind,
    /// The record that is the period (a run), if it is one.
    pub subject: Option<Subject>,
    /// The day of the period (Asia/Dubai); the day itself for a day period.
    pub date: NaiveDate,
    /// When the period ends, if known: a run's planned end, a shift's end.
    /// A day period ends at 23:59 Asia/Dubai of its date.
    pub ends_at: Option<DateTime>,
    /// Whether `FIN.RUL.22` escalates a handover left open. `false` when a
    /// vertical rule replaces it (03: `LOG.RUL.13` for drivers).
    pub escalate: bool,
}

impl HandoverPeriod {
    /// The day `date`.
    pub fn day(date: NaiveDate) -> Self {
        HandoverPeriod {
            kind: PeriodKind::Day,
            subject: None,
            date,
            ends_at: None,
            escalate: true,
        }
    }

    /// The shift on `date`, identified by `shift` (a record of the vertical).
    pub fn shift(shift: Subject, date: NaiveDate, ends_at: Option<DateTime>) -> Self {
        HandoverPeriod {
            kind: PeriodKind::Shift,
            subject: Some(shift),
            date,
            ends_at,
            escalate: true,
        }
    }

    /// The run `run` of `date`, planned to end at `planned_end`.
    pub fn run(run: Subject, date: NaiveDate, planned_end: Option<DateTime>) -> Self {
        HandoverPeriod {
            kind: PeriodKind::Run,
            subject: Some(run),
            date,
            ends_at: planned_end,
            escalate: true,
        }
    }

    /// The same period, but `FIN.RUL.22` does not escalate it: a vertical
    /// rule replaces it (03: `LOG.RUL.13` for driver handovers).
    pub fn replaced_escalation(mut self) -> Self {
        self.escalate = false;
        self
    }
}

/// Cash a holder collected: [`CashCustody::collect`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashCollection {
    /// The holder, a staff user.
    pub holder_user: i64,
    /// The period the cash belongs to.
    pub period: HandoverPeriod,
    /// The amount collected, in fils, above 0.
    pub amount: i64,
    /// What it was collected for (a delivery, an invoice); one source adds
    /// its cash once.
    pub source: Subject,
    /// The holder's outlet or branch, which bears a written-off loss.
    pub location: Option<String>,
    /// The legal entity the cash belongs to.
    pub legal_entity_id: Option<i64>,
}

impl CashCollection {
    /// `amount` fils collected by `holder_user` in `period` for `source`.
    pub fn new(holder_user: i64, period: HandoverPeriod, amount: i64, source: Subject) -> Self {
        CashCollection {
            holder_user,
            period,
            amount,
            source,
            location: None,
            legal_entity_id: None,
        }
    }

    /// The outlet or branch of the holder.
    pub fn location(mut self, location: &str) -> Self {
        self.location = Some(location.to_owned());
        self
    }

    /// The legal entity of the cash.
    pub fn legal_entity(mut self, id: i64) -> Self {
        self.legal_entity_id = Some(id);
        self
    }
}

/// What other modules see of a handover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverSummary {
    /// The handover.
    pub id: HandoverId,
    /// Period or later shortage.
    pub kind: HandoverKind,
    /// Where it stands.
    pub status: HandoverStatus,
    /// The holder, a staff user.
    pub holder_user: i64,
    /// The person recoveries are taken from.
    pub responsible_user: i64,
    /// Run, shift, day or till session.
    pub period_kind: PeriodKind,
    /// The record that is the period, if it is one.
    pub period: Option<Subject>,
    /// The day of the period.
    pub date: NaiveDate,
    /// The expected cash, in fils.
    pub expected: i64,
    /// The counted cash, in fils; 0 until counted.
    pub counted: i64,
    /// Expected less counted while short, in fils; 0 otherwise.
    pub difference: i64,
    /// Recorded recoveries, in fils.
    pub recovered: i64,
    /// When the period was ended by the record that is it (a run's return to
    /// the depot or its abort, [`CashCustody::end_period`]); `None` while it
    /// has not ended, whatever its planned end says.
    #[serde(default)]
    pub ended_at: Option<DateTime>,
    /// Whether `FIN.RUL.22` escalates it when left open; `false` when a
    /// vertical rule replaces it (03: `LOG.RUL.13`).
    #[serde(default)]
    pub escalate: bool,
}

/// The holder's cash for a period was counted (`count`, `recount`): a run
/// closes on it (wl-delivery §4.2 `close`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashHandoverCounted {
    /// The handover.
    pub handover: HandoverId,
    /// The holder.
    pub holder_user: i64,
    /// The period's record, if it is one (the run).
    pub period: Option<Subject>,
    /// The expected cash, in fils.
    pub expected: i64,
    /// The counted cash, in fils.
    pub counted: i64,
    /// `counted` or `short`.
    pub status: HandoverStatus,
    /// The user who counted.
    pub counted_by: i64,
}

impl Event for CashHandoverCounted {}

/// A handover is `short` (`CashShortage`, wl-payments §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashShortage {
    /// The handover.
    pub handover: HandoverId,
    /// The holder.
    pub holder_user: i64,
    /// The person recoveries are taken from.
    pub responsible_user: i64,
    /// Run, shift, day or till session.
    pub period_kind: PeriodKind,
    /// The record that is the period, if it is one.
    pub period: Option<Subject>,
    /// The till session, in till mode.
    pub till_session: Option<Subject>,
    /// The difference, in fils.
    pub difference: i64,
    /// The cheques missing (their ids); empty until cheques are in handovers.
    pub missing_cheques: Vec<i64>,
    /// The record that caused a later shortage (the reversed delivery).
    pub source: Option<Subject>,
}

impl Event for CashShortage {}

/// A shortage ended: recovered in full, or written off after the recoveries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashShortageSettled {
    /// The handover.
    pub handover: HandoverId,
    /// The holder.
    pub holder_user: i64,
    /// `recovered` or `written_off`.
    pub status: HandoverStatus,
    /// What was recovered from the holder, in fils.
    pub recovered: i64,
    /// What was written off as a loss of the holder's branch, in fils.
    pub written_off: i64,
}

impl Event for CashShortageSettled {}

/// The future the [`CashCustody`] methods return.
pub type CustodyFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What other modules ask of cash custody. `wl-payments` provides it.
pub trait CashCustody: Send + Sync + 'static {
    /// Adds `collection` to the holder's handover for its period, opening it
    /// when it is the first. One source adds its cash once: a second call
    /// for the same source answers the same handover and changes nothing.
    /// Refused (422) when the handover of the period is no longer `open`
    /// (the cash would be missing from the count). `None` when custody is
    /// off.
    fn collect<'a>(
        &'a self,
        _state: &'a AppState,
        _collection: CashCollection,
    ) -> CustodyFuture<'a, Option<HandoverId>> {
        Box::pin(async { Ok(None) })
    }

    /// Takes the cash collected for `source` out of its handover again, while
    /// that is still `open`. Refused (422) once it was counted: raise a
    /// later shortage instead. `false` when nothing was collected for it.
    fn release<'a>(&'a self, _state: &'a AppState, _source: Subject) -> CustodyFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// Sets when the holder's period ends (a run's return to the depot or
    /// abort, or its planned end, whichever is earlier), which starts the
    /// escalation clock of `FIN.RUL.22`. Nothing happens when the holder has
    /// no open handover for `period`.
    fn end_period<'a>(
        &'a self,
        _state: &'a AppState,
        _holder_user: i64,
        _period: Subject,
        _ends_at: DateTime,
    ) -> CustodyFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    /// The holder's handover whose period is the record `period`.
    fn handover_of<'a>(
        &'a self,
        _db: &'a Db,
        _holder_user: i64,
        _period: &'a Subject,
    ) -> CustodyFuture<'a, Option<HandoverSummary>> {
        Box::pin(async { Ok(None) })
    }

    /// The `open` handovers of a run (a period record of the vertical) with
    /// cash expected whose escalation a vertical rule replaces
    /// ([`HandoverPeriod::replaced_escalation`]), oldest first: what 03's
    /// `LOG.RUL.13` watches. The default has none.
    fn replaced_open<'a>(&'a self, _db: &'a Db) -> CustodyFuture<'a, Vec<HandoverSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The handover with this id.
    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: HandoverId,
    ) -> CustodyFuture<'a, Option<HandoverSummary>> {
        Box::pin(async { Ok(None) })
    }
}

/// Custody when the `payments` module is off: nothing is held.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCustody;

impl CashCustody for NoCustody {}

/// Makes `custody` the app's [`CashCustody`]; `wl-payments` calls it in
/// `register`.
pub fn provide_custody(app: &mut Registry, custody: impl CashCustody) {
    let custody: Arc<dyn CashCustody> = Arc::new(custody);
    app.provide(custody);
}

/// The app's [`CashCustody`], or [`NoCustody`] when none is provided.
pub fn custody(state: &AppState) -> Arc<dyn CashCustody> {
    match state.provided::<Arc<dyn CashCustody>>() {
        Some(custody) => Arc::clone(&custody),
        None => Arc::new(NoCustody),
    }
}

/// Whether a [`CashCustody`] is provided (the `payments` module is on):
/// without one nothing is held, so a caller that waits for a count must not.
pub fn custody_provided(state: &AppState) -> bool {
    state.provided::<Arc<dyn CashCustody>>().is_some()
}

/// A salary deduction passed to wl-hr as an adjustment of type *cash
/// shortage recovery* (`FIN.ACL.19`, `⚠ open: lawyer L-20`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortageDeduction {
    /// The handover the shortage is on.
    pub handover: HandoverId,
    /// The recovery row (the key of the adjustment: passing it twice
    /// adjusts once).
    pub recovery: i64,
    /// The person the deduction is taken from, a staff user.
    pub responsible_user: i64,
    /// The amount, in fils.
    pub amount: i64,
    /// The user who approved it (`FIN.ACL.19`).
    pub approved_by: Option<i64>,
    /// Whether the holder acknowledged it.
    pub acknowledged: bool,
    /// The shortage, for the payslip line.
    pub reason: String,
}

/// What wl-payments asks of wl-hr to deduct a shortage from wages. `wl-hr`
/// will provide it; the deduction caps (`HRM.PAY...`) are its own. The
/// default refuses, so a deduction is never recorded without the module that
/// can carry it out. No code decides whether wages are touched: the
/// `wl-payments` setting is off by default.
pub trait ShortageRecovery: Send + Sync + 'static {
    /// Passes `deduction` and answers the reference of the adjustment.
    fn deduct<'a>(
        &'a self,
        _state: &'a AppState,
        _deduction: ShortageDeduction,
    ) -> CustodyFuture<'a, String> {
        Box::pin(async {
            Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "A salary deduction needs the HR module to carry it out.".to_owned(),
            ))
        })
    }
}

/// Shortage recovery when the `hr` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoShortageRecovery;

impl ShortageRecovery for NoShortageRecovery {}

/// Makes `recovery` the app's [`ShortageRecovery`]; `wl-hr` calls it.
pub fn provide_shortage_recovery(app: &mut Registry, recovery: impl ShortageRecovery) {
    let recovery: Arc<dyn ShortageRecovery> = Arc::new(recovery);
    app.provide(recovery);
}

/// The app's [`ShortageRecovery`], or [`NoShortageRecovery`].
pub fn shortage_recovery(state: &AppState) -> Arc<dyn ShortageRecovery> {
    match state.provided::<Arc<dyn ShortageRecovery>>() {
        Some(recovery) => Arc::clone(&recovery),
        None => Arc::new(NoShortageRecovery),
    }
}

/// A later shortage another module asks the cash custody to raise (03: a
/// delivery reversed days later with the driver liable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaterShortage {
    /// The holder the shortage is raised against, a staff user.
    pub holder_user: i64,
    /// The amount, in fils.
    pub amount: i64,
    /// Why.
    pub reason: String,
    /// The record that caused it (03: the shipment).
    pub source: Subject,
}

/// How a vertical raises a later cash shortage against a holder on behalf of
/// the user who decides it (`FIN.ACL.05`). `wl-payments` provides it; the
/// default refuses, so a shortage is never recorded without the module that
/// can carry it.
pub trait ShortageRaiser: Send + Sync + 'static {
    /// `user` raises `shortage`: a handover of kind *later shortage*, `short`
    /// from the start; the user must hold the count permission and is never
    /// the holder.
    fn raise<'a>(
        &'a self,
        _state: &'a AppState,
        _user: &'a AuthUser,
        _shortage: LaterShortage,
    ) -> CustodyFuture<'a, HandoverId> {
        Box::pin(async {
            Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "A later shortage needs the payments module.".to_owned(),
            ))
        })
    }
}

/// The shortage raiser when the `payments` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoShortageRaiser;

impl ShortageRaiser for NoShortageRaiser {}

/// Makes `raiser` the app's [`ShortageRaiser`]; `wl-payments` calls it in
/// `register`.
pub fn provide_shortage_raiser(app: &mut Registry, raiser: impl ShortageRaiser) {
    let raiser: Arc<dyn ShortageRaiser> = Arc::new(raiser);
    app.provide(raiser);
}

/// The app's [`ShortageRaiser`], or [`NoShortageRaiser`].
pub fn shortage_raiser(state: &AppState) -> Arc<dyn ShortageRaiser> {
    match state.provided::<Arc<dyn ShortageRaiser>>() {
        Some(raiser) => Arc::clone(&raiser),
        None => Arc::new(NoShortageRaiser),
    }
}

#[cfg(test)]
mod tests {
    use renox::serde_json::{json, to_value};

    use super::*;

    fn day() -> NaiveDate {
        "2027-04-01".parse().unwrap()
    }

    #[test]
    fn a_handover_is_a_subject_and_prints_bare() {
        let id = HandoverId::from(5);
        assert_eq!(id.subject(), Subject::new("payments.cash_handover", 5));
        assert_eq!(id.to_string(), "5");
        assert_eq!(to_value(id).unwrap(), json!(5));
    }

    #[test]
    fn statuses_keep_their_keys_and_three_are_terminal() {
        assert_eq!(HandoverStatus::WrittenOff.as_str(), "written_off");
        assert_eq!(
            to_value(HandoverStatus::WrittenOff).unwrap(),
            json!("written_off")
        );
        assert!(HandoverStatus::Counted.is_terminal());
        assert!(HandoverStatus::Recovered.is_terminal());
        assert!(HandoverStatus::WrittenOff.is_terminal());
        assert!(!HandoverStatus::Open.is_terminal());
        assert!(!HandoverStatus::Short.is_terminal());
        assert_eq!(HandoverKind::LaterShortage.as_str(), "later_shortage");
        assert_eq!(PeriodKind::TillSession.as_str(), "till_session");
    }

    #[test]
    fn a_run_period_can_give_up_the_escalation() {
        let run = HandoverPeriod::run(Subject::new("delivery.run", 3), day(), None);
        assert!(run.escalate);
        assert!(!run.replaced_escalation().escalate);
        assert!(HandoverPeriod::day(day()).escalate);
        assert_eq!(HandoverPeriod::day(day()).kind, PeriodKind::Day);
    }

    #[test]
    fn a_collection_is_built_with_its_builder() {
        let collection = CashCollection::new(
            7,
            HandoverPeriod::day(day()),
            230_000,
            Subject::new("delivery.delivery", 9),
        )
        .location("Dubai branch")
        .legal_entity(2);
        assert_eq!(collection.location.as_deref(), Some("Dubai branch"));
        assert_eq!(collection.legal_entity_id, Some(2));
    }

    #[renox::test]
    async fn without_payments_nothing_is_held_and_a_deduction_is_refused() {
        let app = renox::testing::TestApp::new(App::new()).await;
        let state = app.state();
        let collection = CashCollection::new(
            7,
            HandoverPeriod::day(day()),
            1_000,
            Subject::new("delivery.delivery", 9),
        );
        assert_eq!(
            custody(state).collect(state, collection).await.unwrap(),
            None
        );
        assert!(
            !custody(state)
                .release(state, HandoverId(1).subject())
                .await
                .unwrap()
        );
        assert_eq!(
            custody(state).find(&state.db, HandoverId(1)).await.unwrap(),
            None
        );
        let deduction = ShortageDeduction {
            handover: HandoverId(1),
            recovery: 1,
            responsible_user: 7,
            amount: 1_000,
            approved_by: Some(2),
            acknowledged: true,
            reason: "short".to_owned(),
        };
        assert!(matches!(
            shortage_recovery(state).deduct(state, deduction).await,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
    }
}
