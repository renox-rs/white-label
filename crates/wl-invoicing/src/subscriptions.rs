//! The subscription services (wl-invoicing §4 *Subscription*, §5.1, §6
//! `Subscriptions`, `FIN.RUL.06`).
//!
//! - [`start`] stores an `active` subscription; [`bill_due`] drafts one cycle
//!   invoice per cycle (source `invoicing.subscription:<id>`, linked by a
//!   [`SubscriptionCycle`]), issued by the system when the client issues
//!   `invoicing.subscription` drafts automatically
//!   ([`crate::WlInvoicing::auto_issue_source`]), else left for review.
//! - [`evaluate`] is `FIN.RUL.06` and the rest of the daily check: user
//!   pauses that reached their end date end, an `active` subscription with
//!   an overdue cycle invoice goes `past_due` (not while the invoice is
//!   disputed or in the grace period after an upheld dispute), a `past_due`
//!   one gets its retries or reminders on the retry days and is suspended
//!   after the last one (when suspension is on and not while the only
//!   overdue invoices are disputed), a paid-up one is settled, and a user or
//!   party cancel whose notice ended is carried out.
//! - [`pause`], [`resume`] and [`cancel`] run the transitions for a user,
//!   the vertical (through the kernel's [`Subscriptions`]) or the system.
//! - Listeners: `InvoiceReopened` by a reversed payment marks the cycle
//!   reopened and makes an `active` subscription `past_due` at once (due or
//!   not); `InvoicePaid` settles; `PartyBlocked` pauses (block effect
//!   *pause*, reason `on_hold`, nothing sent to the party) and
//!   `PartyUnblocked` removes that cause; `Transitioned<Subscription>` emits
//!   the kernel events and the party notices after each commit.
//!
//! Decision: the retry of an automatic charge waits for a charge contract in
//! wl-payments (the kernel has none yet); until then `FIN.RUL.06` counts the
//! retry days and notifies the party each time, for both payment modes.

use renox::auth::permissions::Scope;
use renox::chrono::{Datelike, Days, NaiveDate};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use wl_kernel::block_effects::{self, AgreementKind, BlockEffect};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    CreditLine, DraftLine, InvoiceDraft, InvoicePaid, InvoiceReopened, InvoiceStatus, ReopenReason,
};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::parties::{NEUTRAL_BLOCK_REASON, PartyBlocked, PartyStatus, PartyUnblocked};
use wl_kernel::subscriptions::{
    CancelSubscription, PauseCause, PauseSubscription, SUBSCRIPTION_SUBJECT_TYPE,
    StartSubscription, SubscriptionCancelled, SubscriptionFuture, SubscriptionId,
    SubscriptionPastDue, SubscriptionSettled, SubscriptionStatus, SubscriptionSummary,
    SubscriptionSuspended, Subscriptions,
};
use wl_kernel::{PartyId, Subject};

use crate::allocate::amount_text;
use crate::config_of;
use crate::credit_notes::By;
use crate::invoice::Invoice;
use crate::subscription::{
    Subscription, SubscriptionCycle, SubscriptionPause, pro_rata, unused_days,
};
use crate::subscription_machine::{
    CANCEL, CANCEL_SYSTEM, Facts, MARK_PAST_DUE, PAUSE, PAUSE_SYSTEM, RESUME, RESUME_PAST_DUE,
    RESUME_PAST_DUE_SYSTEM, RESUME_SYSTEM, SETTLE, SUSPEND,
};
use crate::texts::english;

/// The agreement kind of wl-parties §5.6 the module owns.
pub const AGREEMENT_KIND: &str = "subscription";

/// The role that decides for the kind (wl-parties §5.6).
pub const DECIDER: &str = "subscriptions_manager";

/// The rule id of the retry schedule.
pub const RULE: &str = "FIN.RUL.06";

/// The daily task: bill the due cycles, then [`evaluate`].
pub const TASK: &str = "invoicing:subscriptions";

/// When [`TASK`] runs, in Asia/Dubai: after `invoicing:mark-overdue`
/// (00:30), so an invoice that fell due yesterday is already `overdue`.
pub const TASK_AT: &str = "01:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The party notice of `mark_past_due`.
pub const PAST_DUE_NOTICE: &str = "invoicing.subscription_past_due";

/// The party notice of each `FIN.RUL.06` retry or reminder.
pub const RETRY_NOTICE: &str = "invoicing.subscription_retry";

/// The party notice of a pause (not sent for a party block, and suppressed
/// by wl-notify while the party is blocked).
pub const PAUSED_NOTICE: &str = "invoicing.subscription_paused";

/// The module that sends the notices.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the notices belong to.
const GROUP: &str = "invoices";

/// Who cancels a subscription.
#[derive(Clone, Copy)]
pub enum Canceller<'a> {
    /// A user with `subscriptions.manage`, in the back office.
    User(&'a AuthUser),
    /// The party (portal, through the vertical), when the client allows it.
    Party(PartyId),
    /// The system: the vertical's agreement ended or was withdrawn.
    /// `settled`: the vertical's settlement already credited or billed the
    /// unused part.
    Agreement {
        /// Whether the vertical already settled.
        settled: bool,
    },
    /// The system, on the day a user or party cancel's notice ended.
    NoticeEnded,
}

impl Canceller<'_> {
    /// The user's id, if a user cancels.
    fn user_id(&self) -> Option<i64> {
        match self {
            Canceller::User(user) => Some(user.id),
            _ => None,
        }
    }

    /// The stored name of who cancelled.
    fn as_str(&self) -> &'static str {
        match self {
            Canceller::User(_) => "user",
            Canceller::Party(_) => "party",
            Canceller::Agreement { .. } => "agreement",
            Canceller::NoticeEnded => "notice_ended",
        }
    }
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Whether `error` only says the run no longer applies (the record moved, a
/// guard says no): a scheduled run or a listener leaves the record as it is.
fn skipped(error: &TransitionError) -> bool {
    matches!(
        error,
        TransitionError::NotFrom(_) | TransitionError::Guard(_) | TransitionError::Conflict
    )
}

/// `date` plus `days`.
fn plus_days(date: NaiveDate, days: i64) -> NaiveDate {
    date.checked_add_days(Days::new(days.max(0).unsigned_abs()))
        .unwrap_or(date)
}

/// `14 Mar 2027`.
fn date_text(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

/// Stores a new `active` subscription for `start`, made by `by` (a user id,
/// `None` for the system or a vertical). Refused (422) for a blank plan, a
/// negative price, a quantity of 0 or less, a currency that is not a code
/// like `AED`, an anchor day outside 1 to 31, or an end before the start.
pub async fn start(
    state: &AppState,
    start: StartSubscription,
    by: Option<i64>,
) -> Result<Subscription> {
    let plan = start.plan.trim().to_owned();
    if plan.is_empty() {
        return Err(refused("give the subscription a plan"));
    }
    if start.unit_price < 0 {
        return Err(refused("the price cannot be negative"));
    }
    if start.quantity_milli <= 0 {
        return Err(refused("the quantity must be above 0"));
    }
    let currency = start.currency.trim().to_ascii_uppercase();
    if !crate::invoicer::is_currency(&currency) {
        return Err(refused(format!(
            "`{}` is not a currency code like AED",
            start.currency
        )));
    }
    let anchor = start.anchor_day.unwrap_or_else(|| start.start.day());
    if !(1..=31).contains(&anchor) {
        return Err(refused("the anchor day must be from 1 to 31"));
    }
    if start.end.is_some_and(|end| end < start.start) {
        return Err(refused("the end cannot be before the start"));
    }
    let mut subscription = Subscription {
        party_id: start.party.0,
        legal_entity_id: start.legal_entity_id,
        source_type: start.source.subject_type.clone(),
        source_id: start.source.subject_id,
        plan,
        unit_price: start.unit_price,
        vat_category: start.vat_category,
        quantity_milli: start.quantity_milli,
        currency,
        cycle: start.cycle,
        anchor_day: i64::from(anchor),
        start_date: start.start,
        end_date: start.end,
        notice_days: i64::from(start.notice_days),
        payment_mode: start.payment_mode,
        status: SubscriptionStatus::Active,
        next_cycle_date: start.start,
        created_by: by,
        ..Default::default()
    };
    subscription.insert(&state.db).await?;
    Ok(subscription)
}

/// The open pause causes of subscription `id`.
pub async fn causes(db: &Db, id: i64) -> Result<Vec<PauseCause>> {
    Ok(SubscriptionPause::open_of(db, id)
        .await?
        .into_iter()
        .map(|p| p.cause)
        .collect())
}

/// What the kernel sees of subscription `subscription`, as stored now.
pub async fn summary_of(db: &Db, id: i64) -> Result<Option<SubscriptionSummary>> {
    let Some(subscription) = Subscription::find(db, id).await? else {
        return Ok(None);
    };
    let open = causes(db, id).await?;
    Ok(Some(subscription.summary(open)))
}

/// The cycles of subscription `id` with their invoices, oldest first.
pub async fn cycle_invoices(db: &Db, id: i64) -> Result<Vec<(SubscriptionCycle, Invoice)>> {
    let cycles = SubscriptionCycle::of(db, id).await?;
    let ids: Vec<i64> = cycles.iter().filter_map(|c| c.invoice_id).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let invoices = Invoice::query()
        .where_(Invoice::ID.is_in(ids))
        .get(db)
        .await?;
    Ok(cycles
        .into_iter()
        .filter_map(|cycle| {
            let invoice = invoices
                .iter()
                .find(|i| Some(i.id) == cycle.invoice_id)?
                .clone();
            Some((cycle, invoice))
        })
        .collect())
}

/// Whether the cycle invoice is outstanding: open with a balance, and
/// `overdue` or reopened by a reversed payment.
pub fn is_outstanding(cycle: &SubscriptionCycle, invoice: &Invoice) -> bool {
    invoice.status.is_open()
        && invoice.balance > 0
        && (invoice.status == InvoiceStatus::Overdue || cycle.reopened)
}

/// What the guards of `subscription` read on `on`, from its cycle
/// invoices, the settings and the party's status.
pub async fn gather(state: &AppState, subscription: &Subscription, on: NaiveDate) -> Result<Facts> {
    let config = config_of(state);
    let outstanding: Vec<(SubscriptionCycle, Invoice)> = cycle_invoices(&state.db, subscription.id)
        .await?
        .into_iter()
        .filter(|(cycle, invoice)| is_outstanding(cycle, invoice))
        .collect();
    let chased: Vec<&(SubscriptionCycle, Invoice)> = outstanding
        .iter()
        .filter(|(_, invoice)| !invoice.chasing_paused(on))
        .collect();
    let chased_before_pause = match subscription.paused_from {
        Some(from) => chased.iter().any(|(cycle, _)| cycle.period_start < from),
        None => !chased.is_empty(),
    };
    let party_blocked = wl_kernel::parties::directory(state)
        .status(&state.db, subscription.party())
        .await?
        == Some(PartyStatus::Blocked);
    let retries = config.subscription_retry_days.len();
    Ok(Facts {
        chased_unpaid: !chased.is_empty(),
        chased_before_pause,
        outstanding: outstanding.len(),
        retries_done: usize::try_from(subscription.retries_done).unwrap_or(0) >= retries,
        suspension_enabled: config.subscription_suspension,
        reason: None,
        allowance_ok: true,
        causes_left: 0,
        party_blocked,
        notice_respected: true,
    })
}

/// Runs the system transition `name` on `subscription` with `facts` and
/// `meta`, then writes `columns` in the same transaction; emits after the
/// commit.
async fn run_system(
    state: &AppState,
    subscription: &Subscription,
    name: &str,
    facts: &Facts,
    meta: Value,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> std::result::Result<(), TransitionError> {
    let mut tx = state.db.begin().await?;
    let outcome = subscription
        .transition(name)
        .by_system()
        .input(facts)
        .meta(meta)
        .run_in(&mut tx)
        .await?;
    if !columns.is_empty() {
        Subscription::query()
            .where_(Subscription::ID.eq(subscription.id))
            .update(&mut tx, columns)
            .await?;
    }
    tx.commit().await?;
    outcome.emit(state).await.map_err(TransitionError::Failed)
}

/// `mark_past_due` on `subscription` (as stored now) on `on`; `cause` is
/// `overdue` or `reopened`. The retry schedule starts from `on`.
pub async fn mark_past_due(
    state: &AppState,
    subscription: &Subscription,
    on: NaiveDate,
    cause: &str,
) -> std::result::Result<(), TransitionError> {
    let subscription = Subscription::find_or_404(&state.db, subscription.id).await?;
    let facts = gather(state, &subscription, on).await?;
    run_system(
        state,
        &subscription,
        MARK_PAST_DUE,
        &facts,
        json!({ "cause": cause }),
        &[("past_due_on", &Some(on)), ("retries_done", &0_i64)],
    )
    .await
}

/// `settle` on `subscription` when nothing is outstanding on `on`.
pub async fn settle(
    state: &AppState,
    subscription: &Subscription,
    on: NaiveDate,
) -> std::result::Result<(), TransitionError> {
    let subscription = Subscription::find_or_404(&state.db, subscription.id).await?;
    let facts = gather(state, &subscription, on).await?;
    let none: Option<NaiveDate> = None;
    run_system(
        state,
        &subscription,
        SETTLE,
        &facts,
        Value::Null,
        &[("past_due_on", &none), ("retries_done", &0_i64)],
    )
    .await
}

/// Pauses `subscription` for `request`, as `by` (`None`: the system). On a
/// subscription already `paused` only the cause is added; a cause of the
/// same kind already open is not added twice (a user pause gets the new end
/// date). A block pause records `on_hold`. Only a user pause counts against
/// the yearly allowance.
///
/// # Errors
///
/// `Forbidden` without `subscriptions.manage`; `NotFrom` from `suspended` or
/// `cancelled`; `Guard` without a reason (user pause), without an end date
/// (user pause) or beyond the allowance.
pub async fn pause(
    state: &AppState,
    subscription: &Subscription,
    request: &PauseSubscription,
    by: Option<&AuthUser>,
) -> std::result::Result<Subscription, TransitionError> {
    let subscription = Subscription::find_or_404(&state.db, subscription.id).await?;
    let open = SubscriptionPause::open_of(&state.db, subscription.id).await?;
    if let Some(existing) = open.iter().find(|p| p.cause == request.cause) {
        if request.cause == PauseCause::User
            && let Some(until) = request.until
            && existing.until != Some(until)
        {
            SubscriptionPause::query()
                .where_(SubscriptionPause::ID.eq(existing.id))
                .update(&state.db, &[("until", &Some(until))])
                .await?;
        }
        return Ok(subscription);
    }
    let on = request.on;
    let reason = match request.cause {
        PauseCause::PartyBlock => Some(NEUTRAL_BLOCK_REASON.to_owned()),
        PauseCause::User => request.reason.clone(),
        PauseCause::Agreement => request
            .reason
            .clone()
            .filter(|r| !r.trim().is_empty())
            .or_else(|| {
                Some(english("wl_invoicing.subscription.pause_reason.agreement").to_owned())
            }),
    }
    .map(|r| r.trim().to_owned());
    let year = i64::from(on.year());
    let used = if subscription.pause_year == Some(year) {
        subscription.pause_days_used
    } else {
        0
    };
    let days = match (request.cause, request.until) {
        (PauseCause::User, Some(until)) if until > on => (until - on).num_days(),
        (PauseCause::User, _) => {
            return Err(TransitionError::Guard(
                english("wl_invoicing.subscription.problem.no_until").to_owned(),
            ));
        }
        _ => 0,
    };
    let allowance = i64::from(config_of(state).subscription_pause_allowance_days);
    let allowance_ok =
        request.cause != PauseCause::User || allowance == 0 || used + days <= allowance;
    let facts = Facts {
        reason: reason.clone(),
        allowance_ok,
        notice_respected: true,
        ..Default::default()
    };
    let first = subscription.status != SubscriptionStatus::Paused;
    let meta = json!({ "cause": request.cause.as_str(), "first": first });

    let mut tx = state.db.begin().await?;
    let builder = match by {
        Some(user) => subscription.transition(PAUSE).by(user),
        None => subscription.transition(PAUSE_SYSTEM).by_system(),
    };
    let outcome = builder.input(&facts).meta(meta).run_in(&mut tx).await?;
    let mut cause = SubscriptionPause {
        subscription_id: subscription.id,
        cause: request.cause,
        reason: reason.clone().unwrap_or_default(),
        started_on: on,
        until: if request.cause == PauseCause::User {
            request.until
        } else {
            None
        },
        by_user_id: by.map(|u| u.id),
        ..Default::default()
    };
    cause.insert(&mut tx).await?;
    if first {
        Subscription::query()
            .where_(Subscription::ID.eq(subscription.id))
            .update(
                &mut tx,
                &[("paused_from", &Some(on)), ("pause_reason", &reason)],
            )
            .await?;
    }
    if request.cause == PauseCause::User {
        Subscription::query()
            .where_(Subscription::ID.eq(subscription.id))
            .update(
                &mut tx,
                &[
                    ("pause_year", &Some(year)),
                    ("pause_days_used", &(used + days)),
                ],
            )
            .await?;
    }
    tx.commit().await?;
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    Ok(Subscription::find_or_404(&state.db, subscription.id).await?)
}

/// Removes the open `cause` of `subscription` on `on`, as `by` (`None`: the
/// system), and resumes it when no cause is left: to `past_due` when a cycle
/// invoice from before the pause is still overdue (and not disputed), else
/// `active`. The next cycle date is recalculated from `on` and the anchor
/// day; cycles that started during the pause are not billed. Without an
/// open `cause` nothing changes.
///
/// # Errors
///
/// `Forbidden` without `subscriptions.manage`; `Guard` while the party is
/// still blocked. Nothing is written then.
pub async fn resume(
    state: &AppState,
    subscription: &Subscription,
    cause: PauseCause,
    on: NaiveDate,
    by: Option<&AuthUser>,
) -> std::result::Result<Subscription, TransitionError> {
    let subscription = Subscription::find_or_404(&state.db, subscription.id).await?;
    // Everything the run reads, before the transaction.
    let mut facts = gather(state, &subscription, on).await?;
    let mut tx = state.db.begin().await?;
    let closed = SubscriptionPause::query()
        .where_(SubscriptionPause::SUBSCRIPTION_ID.eq(subscription.id))
        .where_(SubscriptionPause::CAUSE.eq(cause))
        .where_(SubscriptionPause::ENDED_ON.is_null())
        .update(&mut tx, &[("ended_on", &Some(on))])
        .await?;
    if closed == 0 {
        return Ok(subscription);
    }
    let left = SubscriptionPause::open_of(&mut tx, subscription.id).await?;
    if subscription.status != SubscriptionStatus::Paused || !left.is_empty() {
        tx.commit().await?;
        return Ok(Subscription::find_or_404(&state.db, subscription.id).await?);
    }
    facts.causes_left = left.len();
    let past_due = facts.chased_before_pause;
    let name = match (by.is_some(), past_due) {
        (true, true) => RESUME_PAST_DUE,
        (true, false) => RESUME,
        (false, true) => RESUME_PAST_DUE_SYSTEM,
        (false, false) => RESUME_SYSTEM,
    };
    let builder = match by {
        Some(user) => subscription.transition(name).by(user),
        None => subscription.transition(name).by_system(),
    };
    let outcome = builder
        .input(&facts)
        .meta(json!({ "cause": cause.as_str() }))
        .run_in(&mut tx)
        .await?;
    let next = subscription.next_on_or_after(on);
    let no_day: Option<NaiveDate> = None;
    let no_text: Option<String> = None;
    let past_due_on = if past_due { Some(on) } else { None };
    Subscription::query()
        .where_(Subscription::ID.eq(subscription.id))
        .update(
            &mut tx,
            &[
                ("next_cycle_date", &next),
                ("paused_from", &no_day),
                ("pause_reason", &no_text),
                ("past_due_on", &past_due_on),
                ("retries_done", &0_i64),
            ],
        )
        .await?;
    tx.commit().await?;
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    Ok(Subscription::find_or_404(&state.db, subscription.id).await?)
}

/// Cancels `subscription` on `on` for `by`.
///
/// - The agreement's cancel ends it at once, without notice, with a pro rata
///   credit note for the unused part of the billed cycle (or a final pro rata
///   invoice for a cycle not billed yet), unless the vertical `settled`.
/// - A user or party cancel with a notice period is scheduled for `on` plus
///   the notice (`cancel_on`) and carried out by [`evaluate`] that day;
///   without one it ends at once. It is prorated only when *pro rata on
///   cancel* is on (§7, default off).
/// - A party cancel needs *party may cancel* (§7, default off).
///
/// No refund is paid here: a credit note on a paid cycle stays as the
/// party's credit, and a refund to a blocked party waits (`PTY.ACL.03`).
///
/// # Errors
///
/// `Forbidden` without `subscriptions.manage`, or for a party when the
/// client does not let parties cancel or the subscription is not theirs;
/// `NotFrom` when already `cancelled`; the credit services' refusals.
pub async fn cancel(
    state: &AppState,
    subscription: &Subscription,
    by: Canceller<'_>,
    on: NaiveDate,
    reason: Option<&str>,
) -> std::result::Result<Subscription, TransitionError> {
    let subscription = Subscription::find_or_404(&state.db, subscription.id).await?;
    let config = config_of(state);
    if let Canceller::Party(party) = by
        && (!config.subscription_party_may_cancel || party != subscription.party())
    {
        return Err(TransitionError::Forbidden);
    }
    if let Canceller::User(user) = by
        && !user.has_permission(crate::permissions::SUBSCRIPTIONS_MANAGE)
    {
        return Err(TransitionError::Forbidden);
    }
    if subscription.status == SubscriptionStatus::Cancelled {
        // A cancel whose settlement failed is settled now; otherwise there
        // is nothing to do again.
        if subscription.settlement_due {
            settle_due(state, &subscription).await?;
            return Ok(Subscription::find_or_404(&state.db, subscription.id).await?);
        }
        return Err(TransitionError::NotFrom(
            SubscriptionStatus::Cancelled.as_str().to_owned(),
        ));
    }
    // The reason given with a cancel whose notice ended is the one stored
    // when it was asked.
    let stored = match by {
        Canceller::NoticeEnded => subscription.cancel_reason.clone(),
        _ => None,
    };
    let reason = reason
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
        .or(stored)
        .unwrap_or_else(|| {
            english(&format!(
                "wl_invoicing.subscription.cancel_reason.{}",
                by.as_str()
            ))
            .to_owned()
        });
    // A user or party cancel with notice ends when the notice does.
    if matches!(by, Canceller::User(_) | Canceller::Party(_)) && subscription.notice_days > 0 {
        if subscription.cancel_on.is_none() {
            let ends = plus_days(on, subscription.notice_days);
            Subscription::query()
                .where_(Subscription::ID.eq(subscription.id))
                .update(
                    &state.db,
                    &[("cancel_on", &Some(ends)), ("cancel_reason", &Some(reason))],
                )
                .await?;
        }
        return Ok(Subscription::find_or_404(&state.db, subscription.id).await?);
    }
    let notice_respected = match by {
        Canceller::NoticeEnded => subscription.cancel_on.is_some_and(|ends| on >= ends),
        _ => true,
    };
    let prorate = match by {
        Canceller::Agreement { settled } => !settled,
        _ => config.subscription_pro_rata_on_cancel,
    };
    let facts = Facts {
        notice_respected,
        ..Default::default()
    };
    let meta = json!({
        "by": by.as_str(),
        "prorate": prorate,
        "party": match by { Canceller::Party(party) => Some(party.0), _ => None },
    });
    let mut tx = state.db.begin().await?;
    let builder = match by {
        Canceller::User(user) => subscription.transition(CANCEL).by(user),
        _ => subscription.transition(CANCEL_SYSTEM).by_system(),
    };
    let outcome = builder.input(&facts).meta(meta).run_in(&mut tx).await?;
    // The settlement owed is recorded with the cancel, in one transaction:
    // if making the credit note or the final invoice fails below, the next
    // `cancel` call or the daily check makes it ([`settle_due`]).
    let paused_from = if subscription.status == SubscriptionStatus::Paused {
        subscription.paused_from
    } else {
        None
    };
    Subscription::query()
        .where_(Subscription::ID.eq(subscription.id))
        .update(
            &mut tx,
            &[
                ("cancelled_on", &Some(on)),
                ("cancel_reason", &Some(reason)),
                ("settlement_due", &prorate),
                ("settlement_paused_from", &paused_from),
                ("settlement_by", &by.user_id()),
            ],
        )
        .await?;
    tx.commit().await?;
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    if prorate {
        let cancelled = Subscription::find_or_404(&state.db, subscription.id).await?;
        settle_due(state, &cancelled).await?;
    }
    Ok(Subscription::find_or_404(&state.db, subscription.id).await?)
}

/// Makes the pro rata settlement a cancel of `subscription` recorded
/// (`settlement_due`): the credit note or final invoice, as the
/// subscription was before the cancel, then clears the flag. A settlement
/// that already made its credit note or final invoice is only cleared, so a
/// retry never credits twice. Nothing when none is due.
///
/// # Errors
///
/// What making the credit note or the invoice answers; the flag then stays
/// for the next try.
pub async fn settle_due(
    state: &AppState,
    subscription: &Subscription,
) -> std::result::Result<(), TransitionError> {
    let subscription = Subscription::find_or_404(&state.db, subscription.id).await?;
    if !subscription.settlement_due {
        return Ok(());
    }
    let made =
        subscription.final_credit_note_id.is_some() || subscription.final_invoice_id.is_some();
    if !made && let Some(on) = subscription.cancelled_on {
        // As it was before the cancel: paused from that day, or running.
        let mut before = subscription.clone();
        before.status = match subscription.settlement_paused_from {
            Some(_) => SubscriptionStatus::Paused,
            None => SubscriptionStatus::Active,
        };
        before.paused_from = subscription.settlement_paused_from;
        settle_unused(state, &before, on, subscription.settlement_by).await?;
    }
    Subscription::query()
        .where_(Subscription::ID.eq(subscription.id))
        .update(&state.db, &[("settlement_due", &false)])
        .await?;
    Ok(())
}

/// The invoice states a credit note can be made against.
const CREDITABLE: [InvoiceStatus; 4] = [
    InvoiceStatus::Issued,
    InvoiceStatus::PartiallyPaid,
    InvoiceStatus::Overdue,
    InvoiceStatus::Paid,
];

/// The pro rata money of a cancel of `subscription` (as it was before the
/// cancel) on `on`: a credit note for the unused days of the billed cycle,
/// or a final invoice for the used days of a cycle not billed yet. Pro rata
/// is by calendar days, the cancel day counted as used; while paused the
/// unused part runs from the pause's first day. The credit note reduces an
/// unpaid cycle invoice (nothing is refunded); on a paid one it stays as the
/// party's credit.
async fn settle_unused(
    state: &AppState,
    subscription: &Subscription,
    on: NaiveDate,
    by: Option<i64>,
) -> std::result::Result<(), TransitionError> {
    let paused_from = if subscription.status == SubscriptionStatus::Paused {
        subscription.paused_from
    } else {
        None
    };
    let day = paused_from.unwrap_or(on);
    let cycle = SubscriptionCycle::containing(&state.db, subscription.id, day).await?;
    match cycle {
        Some(cycle) => credit_unused(state, subscription, &cycle, on, paused_from, by).await,
        None if paused_from.is_none() && on >= subscription.start_date => {
            final_invoice(state, subscription, on).await
        }
        None => Ok(()),
    }
}

/// The credit note for the unused days of `cycle`'s invoice.
async fn credit_unused(
    state: &AppState,
    subscription: &Subscription,
    cycle: &SubscriptionCycle,
    on: NaiveDate,
    paused_from: Option<NaiveDate>,
    by: Option<i64>,
) -> std::result::Result<(), TransitionError> {
    let Some(invoice_id) = cycle.invoice_id else {
        return Ok(());
    };
    let invoice = Invoice::find_or_404(&state.db, invoice_id).await?;
    if !CREDITABLE.contains(&invoice.status) {
        // A draft is reviewed by a user; a credited or written-off invoice
        // owes nothing more.
        return Ok(());
    }
    let (unused, total) = unused_days(cycle.period_start, cycle.period_end, on, paused_from);
    if unused == 0 {
        return Ok(());
    }
    let lines = crate::invoicer::lines_of(&state.db, invoice.id).await?;
    let credited = crate::credit_notes::credited_by_line(&state.db, invoice.id).await?;
    let credit: Vec<CreditLine> = lines
        .iter()
        .filter_map(|line| {
            let remaining = crate::vat::remaining_credit(
                line.net_amount,
                credited.get(&line.id).copied().unwrap_or(0),
            );
            let net = pro_rata(line.net_amount, unused, total).min(remaining);
            (net > 0).then(|| CreditLine::new(line.position, net))
        })
        .collect();
    if credit.is_empty() {
        return Ok(());
    }
    let first_unused = paused_from.unwrap_or_else(|| plus_days(on, 1));
    let reason = english("wl_invoicing.subscription.credit_reason")
        .replace("{{ days }}", &unused.to_string())
        .replace("{{ total }}", &total.to_string())
        .replace("{{ from }}", &date_text(first_unused))
        .replace("{{ to }}", &date_text(cycle.period_end));
    let note =
        crate::credit_notes::credit_partly(state, invoice.id, &credit, &reason, By::System(by))
            .await?;
    Subscription::query()
        .where_(Subscription::ID.eq(subscription.id))
        .update(&state.db, &[("final_credit_note_id", &Some(note.id))])
        .await?;
    Ok(())
}

/// The final pro rata invoice for the used days of the cycle containing
/// `on`, when it was not billed.
async fn final_invoice(
    state: &AppState,
    subscription: &Subscription,
    on: NaiveDate,
) -> std::result::Result<(), TransitionError> {
    let mut start = subscription.next_cycle_date;
    if start > on {
        return Ok(());
    }
    while subscription.after(start) <= on {
        start = subscription.after(start);
    }
    let end = subscription.end_of(start);
    let used = (on - start).num_days() + 1;
    let total = (end - start).num_days() + 1;
    let price = pro_rata(subscription.unit_price, used, total);
    if price <= 0 {
        return Ok(());
    }
    let description = english("wl_invoicing.subscription.final_line")
        .replace("{{ plan }}", &subscription.plan)
        .replace("{{ from }}", &date_text(start))
        .replace("{{ to }}", &date_text(on))
        .replace("{{ days }}", &used.to_string())
        .replace("{{ total }}", &total.to_string());
    let billed = bill_cycle(state, subscription, start, end, price, true, &description).await?;
    if let Some(invoice_id) = billed {
        Subscription::query()
            .where_(Subscription::ID.eq(subscription.id))
            .update(&state.db, &[("final_invoice_id", &Some(invoice_id))])
            .await?;
    }
    Ok(())
}

/// Drafts the invoice of the cycle `start..=end` of `subscription` with one
/// line of `price` fils a unit, and links it. The system issues it when the
/// client issues `invoicing.subscription` drafts automatically; a draft it
/// cannot issue stays for review. `None` when the cycle was billed already.
async fn bill_cycle(
    state: &AppState,
    subscription: &Subscription,
    start: NaiveDate,
    end: NaiveDate,
    price: i64,
    is_final: bool,
    description: &str,
) -> Result<Option<i64>> {
    let mut cycle = SubscriptionCycle {
        subscription_id: subscription.id,
        period_start: start,
        period_end: end,
        is_final,
        ..Default::default()
    };
    match cycle.insert(&state.db).await {
        Ok(()) => {}
        Err(error) if error.is_unique_violation() => return Ok(None),
        Err(error) => return Err(error),
    }
    let mut draft = InvoiceDraft::new(subscription.party(), subscription.subject())
        .currency(&subscription.currency)
        .due_date(start)
        .date_of_supply(start)
        .line(
            DraftLine::new(description, price, subscription.vat_category)
                .quantity_milli(subscription.quantity_milli)
                .origin(cycle.subject()),
        );
    if let Some(entity) = subscription.legal_entity_id {
        draft = draft.legal_entity(entity);
    }
    let invoice = match crate::invoicer::create_draft(&state.db, draft).await {
        Ok(invoice) => invoice,
        Err(error) => {
            cycle.delete(&state.db).await?;
            return Err(error);
        }
    };
    SubscriptionCycle::query()
        .where_(SubscriptionCycle::ID.eq(cycle.id))
        .update(&state.db, &[("invoice_id", &Some(invoice.id))])
        .await?;
    let automatic = config_of(state)
        .auto_issue_sources
        .iter()
        .any(|s| s == SUBSCRIPTION_SUBJECT_TYPE);
    if automatic {
        match crate::issue::issue_by_system(state, &invoice).await {
            Ok(()) => {}
            Err(error) if skipped(&error) => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    Ok(Some(invoice.id))
}

/// Bills every cycle of an `active` or `past_due` subscription that starts
/// on or before `on` (and not after its end date): one cycle invoice per
/// cycle, due on the cycle's first day, the cycle date being its date of
/// supply. Billing stops while `paused`, `suspended` or `cancelled`.
/// Returns how many cycles were billed.
pub async fn bill_due(state: &AppState, on: NaiveDate) -> Result<usize> {
    let due = Subscription::query()
        .where_(
            Subscription::STATUS.is_in([SubscriptionStatus::Active, SubscriptionStatus::PastDue]),
        )
        .where_(Subscription::NEXT_CYCLE_DATE.lte(on))
        .order_by(Subscription::ID)
        .get(&state.db)
        .await?;
    let mut billed = 0;
    for subscription in due {
        let mut next = subscription.next_cycle_date;
        while next <= on && subscription.end_date.is_none_or(|end| next <= end) {
            let end = subscription.end_of(next);
            let description = english("wl_invoicing.subscription.cycle_line")
                .replace("{{ plan }}", &subscription.plan)
                .replace("{{ from }}", &date_text(next))
                .replace("{{ to }}", &date_text(end));
            let made = bill_cycle(
                state,
                &subscription,
                next,
                end,
                subscription.unit_price,
                false,
                &description,
            )
            .await?;
            billed += usize::from(made.is_some());
            next = subscription.after(next);
        }
        if next != subscription.next_cycle_date {
            Subscription::query()
                .where_(Subscription::ID.eq(subscription.id))
                .update(&state.db, &[("next_cycle_date", &next)])
                .await?;
        }
    }
    Ok(billed)
}

/// `FIN.RUL.06` and the daily check on `on` (see the module docs).
pub async fn evaluate(state: &AppState, on: NaiveDate) -> Result<()> {
    // User pauses that reached their end date.
    let ending = SubscriptionPause::query()
        .where_(SubscriptionPause::CAUSE.eq(PauseCause::User))
        .where_(SubscriptionPause::ENDED_ON.is_null())
        .where_(SubscriptionPause::UNTIL.lte(on))
        .order_by(SubscriptionPause::ID)
        .get(&state.db)
        .await?;
    for pause in ending {
        let Some(subscription) = Subscription::find(&state.db, pause.subscription_id).await? else {
            continue;
        };
        match resume(state, &subscription, PauseCause::User, on, None).await {
            Ok(_) => {}
            Err(error) if skipped(&error) => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    // Past due, retries, suspension and settling.
    let watched = Subscription::query()
        .where_(Subscription::STATUS.is_in([
            SubscriptionStatus::Active,
            SubscriptionStatus::PastDue,
            SubscriptionStatus::Suspended,
        ]))
        .order_by(Subscription::ID)
        .get(&state.db)
        .await?;
    for subscription in watched {
        match evaluate_one(state, &subscription, on).await {
            Ok(()) => {}
            Err(error) if skipped(&error) => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    // Cancels whose settlement failed are settled again.
    let unsettled = Subscription::query()
        .where_(Subscription::STATUS.eq(SubscriptionStatus::Cancelled))
        .where_(Subscription::SETTLEMENT_DUE.eq(true))
        .order_by(Subscription::ID)
        .get(&state.db)
        .await?;
    for subscription in unsettled {
        settle_due(state, &subscription)
            .await
            .map_err(TransitionError::into_error)?;
    }
    // User and party cancels whose notice ended.
    let ending = Subscription::query()
        .where_(Subscription::CANCEL_ON.lte(on))
        .where_(Subscription::STATUS.ne(SubscriptionStatus::Cancelled))
        .order_by(Subscription::ID)
        .get(&state.db)
        .await?;
    for subscription in ending {
        let ends = subscription.cancel_on.unwrap_or(on);
        match cancel(state, &subscription, Canceller::NoticeEnded, ends, None).await {
            Ok(_) => {}
            Err(error) if skipped(&error) => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    Ok(())
}

/// The daily check of one subscription.
async fn evaluate_one(
    state: &AppState,
    subscription: &Subscription,
    on: NaiveDate,
) -> std::result::Result<(), TransitionError> {
    let facts = gather(state, subscription, on).await?;
    match subscription.status {
        SubscriptionStatus::Active if facts.chased_unpaid => {
            mark_past_due(state, subscription, on, "overdue").await
        }
        SubscriptionStatus::PastDue | SubscriptionStatus::Suspended if facts.outstanding == 0 => {
            settle(state, subscription, on).await
        }
        SubscriptionStatus::PastDue => retry(state, subscription, &facts, on).await,
        _ => Ok(()),
    }
}

/// `FIN.RUL.06` on a `past_due` subscription: each retry day reached since
/// it went past due notifies the party once (only while an unpaid cycle
/// invoice is chased: a disputed one is not), then, after the last one,
/// `suspend` when suspension is on and not while the only overdue cycle
/// invoices are disputed.
async fn retry(
    state: &AppState,
    subscription: &Subscription,
    facts: &Facts,
    on: NaiveDate,
) -> std::result::Result<(), TransitionError> {
    if !facts.chased_unpaid {
        // Only disputed (or grace-period) invoices are overdue: it stays
        // `past_due` and nobody is chased.
        return Ok(());
    }
    let days = config_of(state).subscription_retry_days.clone();
    let since = subscription.past_due_on.unwrap_or(on);
    let mut done = usize::try_from(subscription.retries_done).unwrap_or(0);
    let before = done;
    while let Some(day) = days.get(done) {
        if on < plus_days(since, i64::from(*day)) {
            break;
        }
        done += 1;
        notify_party(
            state,
            subscription,
            RETRY_NOTICE,
            json!({ "attempt": done }),
        )
        .await?;
    }
    if done != before {
        Subscription::query()
            .where_(Subscription::ID.eq(subscription.id))
            .update(
                &state.db,
                &[("retries_done", &i64::try_from(done).unwrap_or(i64::MAX))],
            )
            .await?;
    }
    if done < days.len() || !facts.suspension_enabled {
        return Ok(());
    }
    let mut facts = facts.clone();
    facts.retries_done = true;
    let subscription = Subscription::find_or_404(&state.db, subscription.id).await?;
    run_system(state, &subscription, SUSPEND, &facts, Value::Null, &[]).await
}

/// Bills the due cycles, then runs [`evaluate`], on `on`.
pub async fn run(state: &AppState, on: NaiveDate) -> Result<()> {
    bill_due(state, on).await?;
    evaluate(state, on).await
}

/// Tells the party of `subscription` with the notice `type_key`.
async fn notify_party(
    state: &AppState,
    subscription: &Subscription,
    type_key: &str,
    extra: Value,
) -> Result {
    let mut data = json!({
        "plan": subscription.plan,
        "amount": amount_text(&subscription.currency, subscription.unit_price),
        "link": format!("/subscriptions/{}", subscription.id),
    });
    if let (Some(data), Value::Object(extra)) = (data.as_object_mut(), extra) {
        data.extend(extra);
    }
    let request = NotifyRequest::new(type_key, subscription.subject())
        .to_party(subscription.party())
        .data(data);
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The kernel events and the party notices of a committed transition.
async fn announce(state: &AppState, event: &Transitioned<Subscription>) -> Result {
    let subscription = Subscription::find_or_404(&state.db, event.key).await?;
    let (id, party, source) = (
        subscription.subscription_id(),
        subscription.party(),
        subscription.source(),
    );
    match event.transition.as_str() {
        MARK_PAST_DUE => {
            state
                .emit(SubscriptionPastDue {
                    subscription: id,
                    party,
                    source,
                })
                .await?;
            notify_party(state, &subscription, PAST_DUE_NOTICE, Value::Null).await
        }
        SUSPEND => {
            state
                .emit(SubscriptionSuspended {
                    subscription: id,
                    party,
                    source,
                })
                .await
        }
        SETTLE => {
            state
                .emit(SubscriptionSettled {
                    subscription: id,
                    party,
                    source,
                })
                .await
        }
        CANCEL | CANCEL_SYSTEM => {
            state
                .emit(SubscriptionCancelled {
                    subscription: id,
                    party,
                    source,
                })
                .await
        }
        PAUSE | PAUSE_SYSTEM => {
            // A block pause sends the party nothing (wl-parties §5.6); a
            // cause added to a paused subscription neither.
            let cause = event.meta.get("cause").and_then(Value::as_str);
            let first = event.from != SubscriptionStatus::Paused.as_str();
            if first && cause != Some(PauseCause::PartyBlock.as_str()) {
                notify_party(state, &subscription, PAUSED_NOTICE, Value::Null).await
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

/// `InvoiceReopened` by a reversed payment on a cycle invoice: the cycle is
/// outstanding again and an `active` subscription goes `past_due` at once.
async fn on_reopened(state: &AppState, event: &InvoiceReopened) -> Result {
    if event.source.subject_type != SUBSCRIPTION_SUBJECT_TYPE
        || event.reason != ReopenReason::PaymentReversed
        || !event.new_status.is_open()
    {
        return Ok(());
    }
    SubscriptionCycle::query()
        .where_(SubscriptionCycle::INVOICE_ID.eq(event.invoice.0))
        .update(&state.db, &[("reopened", &true)])
        .await?;
    let Some(subscription) = Subscription::find(&state.db, event.source.subject_id).await? else {
        return Ok(());
    };
    if subscription.status != SubscriptionStatus::Active {
        return Ok(());
    }
    match mark_past_due(state, &subscription, today(), "reopened").await {
        Ok(()) => Ok(()),
        Err(error) if skipped(&error) => Ok(()),
        Err(error) => Err(error.into_error()),
    }
}

/// `InvoicePaid` on a cycle invoice: settle when nothing else is outstanding.
async fn on_paid(state: &AppState, event: &InvoicePaid) -> Result {
    if event.source.subject_type != SUBSCRIPTION_SUBJECT_TYPE {
        return Ok(());
    }
    let Some(subscription) = Subscription::find(&state.db, event.source.subject_id).await? else {
        return Ok(());
    };
    if !matches!(
        subscription.status,
        SubscriptionStatus::PastDue | SubscriptionStatus::Suspended
    ) {
        return Ok(());
    }
    let facts = gather(state, &subscription, today()).await?;
    if facts.outstanding > 0 {
        return Ok(());
    }
    match settle(state, &subscription, today()).await {
        Ok(()) => Ok(()),
        Err(error) if skipped(&error) => Ok(()),
        Err(error) => Err(error.into_error()),
    }
}

/// `PartyBlocked`: when the block effect for subscriptions is *pause*, each
/// `active`, `past_due` or `paused` subscription of the party is paused for
/// the block, with the reason `on_hold`.
async fn on_blocked(state: &AppState, party: PartyId) -> Result {
    let rule = block_effects::effect_for(AGREEMENT_KIND, &Scope::global());
    if rule.effect != BlockEffect::Pause {
        return Ok(());
    }
    let held = Subscription::query()
        .where_(Subscription::PARTY_ID.eq(party.0))
        .where_(Subscription::STATUS.is_in([
            SubscriptionStatus::Active,
            SubscriptionStatus::PastDue,
            SubscriptionStatus::Paused,
        ]))
        .order_by(Subscription::ID)
        .get(&state.db)
        .await?;
    let request = PauseSubscription::new(PauseCause::PartyBlock, today());
    for subscription in held {
        match pause(state, &subscription, &request, None).await {
            Ok(_) => {}
            Err(error) if skipped(&error) => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    Ok(())
}

/// `PartyUnblocked`: the block cause is removed from the party's
/// subscriptions, which resume only when no other cause is left.
async fn on_unblocked(state: &AppState, party: PartyId) -> Result {
    let held = Subscription::query()
        .where_(Subscription::PARTY_ID.eq(party.0))
        .where_(Subscription::STATUS.ne(SubscriptionStatus::Cancelled))
        .order_by(Subscription::ID)
        .get(&state.db)
        .await?;
    for subscription in held {
        match resume(state, &subscription, PauseCause::PartyBlock, today(), None).await {
            Ok(_) => {}
            Err(error) if skipped(&error) => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    Ok(())
}

/// One party notice type.
fn notice_type(key: &str, texts: &str, category: Category) -> NotificationTypeDef {
    NotificationTypeDef::new(key, MODULE)
        .group(GROUP)
        .category(category)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english(&format!(
                "wl_invoicing.subscription.{texts}.subject"
            ))),
            english(&format!("wl_invoicing.subscription.{texts}.body")),
        )
}

/// The party notices: past due, each retry, paused.
pub fn notification_types() -> [NotificationTypeDef; 3] {
    [
        notice_type(PAST_DUE_NOTICE, "past_due", Category::Transactional),
        notice_type(RETRY_NOTICE, "retry", Category::Reminder),
        notice_type(PAUSED_NOTICE, "paused", Category::Transactional),
    ]
}

/// Registers the agreement kind, the notices, the listeners and the daily
/// task.
pub(crate) fn register(app: &mut Registry) {
    // wl-parties §5.6: a block pauses subscriptions; the subscriptions
    // manager decides when a vertical turns it to a review.
    AgreementKind::new(AGREEMENT_KIND)
        .owner("wl-invoicing")
        .effect(BlockEffect::Pause)
        .decider(DECIDER)
        .register();
    for def in notification_types() {
        notify::register_type(def);
    }
    // The pause notice is suppressed while the party is blocked.
    block_effects::mark_announces_pause(PAUSED_NOTICE);
    app.listen(
        |event: Transitioned<Subscription>, state: AppState| async move {
            announce(&state, &event).await
        },
    );
    app.listen(|event: InvoiceReopened, state: AppState| async move {
        on_reopened(&state, &event).await
    });
    app.listen(|event: InvoicePaid, state: AppState| async move { on_paid(&state, &event).await });
    app.listen(|event: PartyBlocked, state: AppState| async move {
        on_blocked(&state, event.party).await
    });
    app.listen(|event: PartyUnblocked, state: AppState| async move {
        on_unblocked(&state, event.party).await
    });
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await
        })
        .timezone(TASK_ZONE);
}

/// The module's [`Subscriptions`] for the kernel contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubscriptionBook;

/// The summaries of `agreement`'s subscriptions, as stored now.
async fn summaries(db: &Db, agreement: &Subject) -> Result<Vec<SubscriptionSummary>> {
    let mut out = Vec::new();
    for subscription in Subscription::of_source(db, agreement).await? {
        let open = causes(db, subscription.id).await?;
        out.push(subscription.summary(open));
    }
    Ok(out)
}

impl Subscriptions for SubscriptionBook {
    fn start<'a>(
        &'a self,
        state: &'a AppState,
        request: StartSubscription,
    ) -> SubscriptionFuture<'a, Option<SubscriptionId>> {
        Box::pin(async move {
            let subscription = start(state, request, None).await?;
            Ok(Some(subscription.subscription_id()))
        })
    }

    fn find<'a>(
        &'a self,
        db: &'a Db,
        id: SubscriptionId,
    ) -> SubscriptionFuture<'a, Option<SubscriptionSummary>> {
        Box::pin(async move { summary_of(db, id.0).await })
    }

    fn of_agreement<'a>(
        &'a self,
        db: &'a Db,
        agreement: &'a Subject,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        Box::pin(async move { summaries(db, agreement).await })
    }

    fn pause<'a>(
        &'a self,
        state: &'a AppState,
        agreement: &'a Subject,
        request: PauseSubscription,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        Box::pin(async move {
            for subscription in Subscription::of_source(&state.db, agreement).await? {
                if !matches!(
                    subscription.status,
                    SubscriptionStatus::Active
                        | SubscriptionStatus::PastDue
                        | SubscriptionStatus::Paused
                ) {
                    continue;
                }
                pause(state, &subscription, &request, None)
                    .await
                    .map_err(TransitionError::into_error)?;
            }
            summaries(&state.db, agreement).await
        })
    }

    fn resume<'a>(
        &'a self,
        state: &'a AppState,
        agreement: &'a Subject,
        cause: PauseCause,
        on: NaiveDate,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        Box::pin(async move {
            for subscription in Subscription::of_source(&state.db, agreement).await? {
                if subscription.status == SubscriptionStatus::Cancelled {
                    // A retry settles what a failed settlement left.
                    settle_due(state, &subscription)
                        .await
                        .map_err(TransitionError::into_error)?;
                    continue;
                }
                resume(state, &subscription, cause, on, None)
                    .await
                    .map_err(TransitionError::into_error)?;
            }
            summaries(&state.db, agreement).await
        })
    }

    fn cancel<'a>(
        &'a self,
        state: &'a AppState,
        agreement: &'a Subject,
        request: CancelSubscription,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        Box::pin(async move {
            for subscription in Subscription::of_source(&state.db, agreement).await? {
                if subscription.status == SubscriptionStatus::Cancelled {
                    // A retry settles what a failed settlement left.
                    settle_due(state, &subscription)
                        .await
                        .map_err(TransitionError::into_error)?;
                    continue;
                }
                cancel(
                    state,
                    &subscription,
                    Canceller::Agreement {
                        settled: request.settled,
                    },
                    request.on,
                    request.reason.as_deref(),
                )
                .await
                .map_err(TransitionError::into_error)?;
            }
            summaries(&state.db, agreement).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notices_are_the_modules() {
        let keys: Vec<String> = notification_types()
            .iter()
            .map(|def| def.key.clone())
            .collect();
        assert_eq!(keys, [PAST_DUE_NOTICE, RETRY_NOTICE, PAUSED_NOTICE]);
    }

    #[test]
    fn only_overdue_or_reopened_open_invoices_are_outstanding() {
        let cycle = SubscriptionCycle::default();
        let reopened = SubscriptionCycle {
            reopened: true,
            ..Default::default()
        };
        let invoice = |status, balance| Invoice {
            status,
            balance,
            ..Default::default()
        };
        assert!(is_outstanding(&cycle, &invoice(InvoiceStatus::Overdue, 10)));
        assert!(!is_outstanding(&cycle, &invoice(InvoiceStatus::Issued, 10)));
        assert!(is_outstanding(
            &reopened,
            &invoice(InvoiceStatus::Issued, 10)
        ));
        assert!(!is_outstanding(&reopened, &invoice(InvoiceStatus::Paid, 0)));
        assert!(!is_outstanding(&cycle, &invoice(InvoiceStatus::Overdue, 0)));
    }
}
