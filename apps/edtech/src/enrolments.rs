//! The enrolment services (10 §4.1, §7.1 *Enrolment*, `EDU.ENR.01`,
//! `EDU.REF.01`): enrolling a student, confirming the terms, activating on the
//! first payment, freezing and unfreezing, completing and withdrawing.
//!
//! The machine ([`crate::enrolment_machine`]) holds the rules a record and its
//! input can answer; this file gathers what it cannot know (the parties, the
//! freeze days used this year, the causes left, the sessions held and what was
//! paid), takes the seat, raises the fees ([`crate::fees`]) and runs the
//! transition. What the issues of the epic add is marked at the item:
//! BL-EDU-01 `lapse`, BL-EDU-02 the atomic seat count, BL-EDU-03 the money
//! documents of `withdraw`, BL-EDU-04 and BL-EDU-05 the block and the
//! membership link of `freeze` and `unfreeze`.
//!
//! Decisions:
//!
//! - The seat of a course enrolment is taken in the transaction that inserts
//!   it, with [`Limit::Room`] for the centre manager (`batches.exceed_capacity`)
//!   and [`Limit::Capacity`] for everyone else (`EDU.ACL.17`).
//! - A guardian must be one of the student's guardians in wl-parties; the
//!   payer is the guardian unless another payer is named, else the student.
//! - A discount above [`DISCOUNT_LIMIT_PERCENT`] of the price waits for
//!   `enrolments.approve` ([`approve_discount`]); `activate` is refused until
//!   then (the limit is the default of the discount settings of wl-sales).
//! - The terms and the refund policy are accepted on the enrolment form or
//!   afterwards with [`accept_terms`]; the signed document itself
//!   (`DOC.SIG.01`) is wl-documents'.
//! - The paid amount `withdraw` works from is what the enrolment's invoices
//!   have received: their total less their balance.

use renox::chrono::{Duration, NaiveDate};
use renox::db::{Json, now};
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionError, Transitioned};
use wl_kernel::approvals::{announce, submit::REQUEST_KEY};
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoicePaid, invoicer};
use wl_kernel::parties::directory;
use wl_kernel::sales::{lead_conversion, package_ledger};

use crate::batch::{Batch, BatchStatus};
use crate::db::refused;
use crate::enrolment::{
    Enrolment, EnrolmentFreeze, EnrolmentKind, EnrolmentStatus, FreezeCause, Instalment,
};
use crate::enrolment_machine::{
    ACTIVATE, ACTIVATE_SYSTEM, COMPLETE, EnrolmentInput, FREEZE, FREEZE_SYSTEM, LAPSE, UNFREEZE,
    UNFREEZE_SYSTEM, WITHDRAW, WITHDRAW_APPROVED, WITHDRAW_SYSTEM, end_cause,
};
use crate::fees;
use crate::permissions;
use crate::refund::{self, Refund, RefundBasis, RefundInput, RefundPolicy};
use crate::seats::{self, Limit};
use crate::session::{Session, SessionStatus};
use crate::sessions;
use crate::texts::english;

/// The hours a seat is held for the first payment (`EDU.RUL.11`, BL-EDU-01
/// makes it a setting).
pub const HOLD_HOURS: i64 = 48;

/// A discount above this share of the price, in percent, needs approval.
pub const DISCOUNT_LIMIT_PERCENT: i64 = 10;

/// The most instalments a course fee is split in.
pub const MAX_INSTALMENTS: i64 = 12;

/// An enrolment to create.
#[derive(Debug, Clone)]
pub struct NewEnrolment {
    /// Batch, package or membership.
    pub kind: EnrolmentKind,
    /// The student, a party.
    pub student: i64,
    /// The guardian, a party.
    pub guardian: Option<i64>,
    /// Who pays, a party; the guardian, else the student, when not named.
    pub payer: Option<i64>,
    /// The batch of a course enrolment.
    pub batch_id: Option<i64>,
    /// The catalogue item of a package or membership.
    pub item_code: Option<String>,
    /// The name of the item as sold.
    pub item_name: Option<String>,
    /// The catalogue price with VAT, in fils.
    pub price: i64,
    /// The discount, in fils.
    pub discount: i64,
    /// The registration fee with VAT, in fils.
    pub registration_fee: i64,
    /// The admin fee with VAT, in fils.
    pub admin_fee: i64,
    /// The credits of a package.
    pub sessions: i64,
    /// The instalments of a course fee.
    pub instalments: i64,
    /// The refund policy accepted.
    pub policy: RefundPolicy,
    /// Whether the terms and the refund policy are accepted now.
    pub terms_accepted: bool,
}

impl NewEnrolment {
    fn base(kind: EnrolmentKind, student: i64, price: i64) -> Self {
        NewEnrolment {
            kind,
            student,
            guardian: None,
            payer: None,
            batch_id: None,
            item_code: None,
            item_name: None,
            price,
            discount: 0,
            registration_fee: 0,
            admin_fee: 0,
            sessions: 0,
            instalments: 1,
            policy: RefundPolicy::default(),
            terms_accepted: false,
        }
    }

    /// A seat in batch `batch_id` at `price` fils with VAT.
    pub fn course(student: i64, batch_id: i64, price: i64) -> Self {
        let mut new = Self::base(EnrolmentKind::Course, student, price);
        new.batch_id = Some(batch_id);
        new
    }

    /// A package of `credits` sessions of the item `item_code` at `price`.
    pub fn package(student: i64, item_code: &str, name: &str, credits: i64, price: i64) -> Self {
        let mut new = Self::base(EnrolmentKind::Package, student, price);
        new.item_code = Some(item_code.to_owned());
        new.item_name = Some(name.to_owned());
        new.sessions = credits;
        new
    }

    /// A membership of the item `item_code` at `monthly_price` a month.
    pub fn membership(student: i64, item_code: &str, name: &str, monthly_price: i64) -> Self {
        let mut new = Self::base(EnrolmentKind::Membership, student, monthly_price);
        new.item_code = Some(item_code.to_owned());
        new.item_name = Some(name.to_owned());
        new
    }

    /// With a guardian (a party who is a guardian of the student).
    pub fn guardian(mut self, party: i64) -> Self {
        self.guardian = Some(party);
        self
    }

    /// Paid by `party`.
    pub fn payer(mut self, party: i64) -> Self {
        self.payer = Some(party);
        self
    }

    /// With a discount of `fils`.
    pub fn discount(mut self, fils: i64) -> Self {
        self.discount = fils;
        self
    }

    /// With a registration fee of `fils`.
    pub fn registration_fee(mut self, fils: i64) -> Self {
        self.registration_fee = fils;
        self
    }

    /// With an admin fee of `fils`.
    pub fn admin_fee(mut self, fils: i64) -> Self {
        self.admin_fee = fils;
        self
    }

    /// Paid in `count` instalments.
    pub fn instalments(mut self, count: i64) -> Self {
        self.instalments = count;
        self
    }

    /// With the item's name as sold.
    pub fn item_name(mut self, name: &str) -> Self {
        self.item_name = Some(name.to_owned());
        self
    }

    /// With the terms and the refund policy accepted.
    pub fn terms_accepted(mut self) -> Self {
        self.terms_accepted = true;
        self
    }
}

/// `Forbidden` unless `user` holds `permission`.
fn require(user: &AuthUser, permission: &str) -> Result {
    if user.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// The name of party `id` in `lang`, else `#id`.
pub async fn party_name(state: &AppState, lang: &str, id: i64) -> String {
    directory(state)
        .display_name(&state.db, PartyId(id), lang)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{id}"))
}

/// Refuses unless party `id` exists.
async fn require_party(state: &AppState, id: i64, key: &str) -> Result {
    let found = directory(state)
        .display_name(&state.db, PartyId(id), "en")
        .await?;
    if found.is_none() {
        return Err(refused(english(key, &[])));
    }
    Ok(())
}

/// The amounts of a new enrolment make sense.
fn check_amounts(new: &NewEnrolment) -> Result {
    let bad = new.price < 0
        || new.discount < 0
        || new.discount > new.price
        || new.registration_fee < 0
        || new.admin_fee < 0;
    if bad {
        return Err(refused(english("edtech.enrolments.problem.amounts", &[])));
    }
    if new.kind == EnrolmentKind::Course && !(1..=MAX_INSTALMENTS).contains(&new.instalments) {
        return Err(refused(english("edtech.enrolments.problem.instalments", &[])));
    }
    match new.kind {
        EnrolmentKind::Course if new.batch_id.is_none() => {
            Err(refused(english("edtech.enrolments.problem.batch", &[])))
        }
        EnrolmentKind::Package if new.item_code.is_none() || new.sessions < 1 => {
            Err(refused(english("edtech.enrolments.problem.package", &[])))
        }
        EnrolmentKind::Membership if new.item_code.is_none() => {
            Err(refused(english("edtech.enrolments.problem.membership", &[])))
        }
        _ => Ok(()),
    }
}

/// Enrols a student: checks the parties and the batch, takes the seat and
/// inserts the enrolment `pending_payment` in one transaction, then raises the
/// fees ([`fees::raise`]).
///
/// # Errors
///
/// A 403 without `enrolments.manage`; a 422 for an unknown student, guardian
/// or payer, a guardian the student does not have, a batch that is not
/// `open`, no seat free (`EDU.ACL.17`) or amounts that make no sense.
pub async fn create(state: &AppState, by: &AuthUser, new: NewEnrolment) -> Result<Enrolment> {
    require(by, permissions::ENROLMENTS_MANAGE)?;
    check_amounts(&new)?;
    require_party(state, new.student, "edtech.enrolments.problem.student").await?;
    if let Some(guardian) = new.guardian {
        require_party(state, guardian, "edtech.enrolments.problem.guardian").await?;
        let guardians = directory(state)
            .guardians(&state.db, PartyId(new.student))
            .await?;
        if !guardians.contains(&PartyId(guardian)) {
            return Err(refused(english("edtech.enrolments.problem.not_guardian", &[])));
        }
    }
    let payer = new.payer.or(new.guardian).unwrap_or(new.student);
    if payer != new.student && Some(payer) != new.guardian {
        require_party(state, payer, "edtech.enrolments.problem.payer").await?;
    }
    let batch = match new.batch_id {
        Some(id) if new.kind == EnrolmentKind::Course => {
            Some(Batch::find_or_404(&state.db, id).await?)
        }
        _ => None,
    };
    if let Some(batch) = &batch
        && batch.status != BatchStatus::Open
    {
        return Err(refused(english("edtech.enrolments.problem.batch_not_open", &[])));
    }
    let sessions_total = match &batch {
        Some(batch) => {
            let planned = Session::of_batch(&state.db, batch.id).await?;
            let counted = planned
                .iter()
                .filter(|s| s.status != SessionStatus::Cancelled)
                .count();
            i64::try_from(counted).unwrap_or(i64::MAX)
        }
        None => new.sessions,
    };
    let needs_approval = new.discount * 100 > new.price * DISCOUNT_LIMIT_PERCENT;
    let limit = if by.has_permission(permissions::SEATS_EXCEED) {
        Limit::Room
    } else {
        Limit::Capacity
    };
    let mut enrolment = Enrolment {
        kind: new.kind,
        batch_id: batch.as_ref().map(|b| b.id),
        student_party_id: new.student,
        guardian_party_id: new.guardian,
        payer_party_id: payer,
        campus_type: batch.as_ref().and_then(|b| b.campus_type.clone()),
        campus_id: batch.as_ref().and_then(|b| b.campus_id),
        counsellor_id: Some(by.id),
        item_code: new.item_code.clone(),
        item_name: new.item_name.clone(),
        price: new.price,
        discount: new.discount,
        discount_approved: !needs_approval || by.has_permission(permissions::ENROLMENTS_APPROVE),
        registration_fee: new.registration_fee,
        admin_fee: new.admin_fee,
        sessions_total,
        instalments: if new.kind == EnrolmentKind::Course {
            new.instalments
        } else {
            1
        },
        policy: Json(new.policy.clone()),
        terms_accepted_at: new.terms_accepted.then(now),
        seat_held: batch.is_some(),
        seat_hold_until: Some(now() + Duration::hours(HOLD_HOURS)),
        created_by: Some(by.id),
        status: EnrolmentStatus::PendingPayment,
        ..Enrolment::default()
    };
    // `begin_immediate`: a second taker of the last seat waits here and then
    // sees the full count (`EDU.ACL.17`).
    let mut tx = state.db.begin_immediate().await?;
    if let Some(batch) = &batch
        && let Err(error) = seats::take(&mut tx, batch.id, 1, limit).await
    {
        tx.rollback().await?;
        return Err(seats::offer(&state.db, batch.id, error).await);
    }
    enrolment
        .insert_in_state(&mut tx, EnrolmentStatus::PendingPayment, "create")
        .await?;
    tx.commit().await?;
    fees::raise(state, &enrolment).await
}

/// The terms and the refund policy are accepted (`DOC.SIG.01`).
///
/// # Errors
///
/// A 403 without `enrolments.manage`; a 422 unless the enrolment is
/// `pending_payment`.
pub async fn accept_terms(state: &AppState, by: &AuthUser, id: i64) -> Result<Enrolment> {
    require(by, permissions::ENROLMENTS_MANAGE)?;
    let enrolment = Enrolment::find_or_404(&state.db, id).await?;
    if enrolment.status != EnrolmentStatus::PendingPayment {
        return Err(refused(english("edtech.enrolments.problem.not_pending", &[])));
    }
    Enrolment::query()
        .where_(Enrolment::ID.eq(id))
        .update(
            &state.db,
            &[("terms_accepted_at", &Some(now())), ("updated_at", &now())],
        )
        .await?;
    Enrolment::find_or_404(&state.db, id).await
}

/// The centre manager approves a discount above the limit.
///
/// # Errors
///
/// A 403 without `enrolments.approve`; a 422 unless the enrolment is
/// `pending_payment`.
pub async fn approve_discount(state: &AppState, by: &AuthUser, id: i64) -> Result<Enrolment> {
    require(by, permissions::ENROLMENTS_APPROVE)?;
    let enrolment = Enrolment::find_or_404(&state.db, id).await?;
    if enrolment.status != EnrolmentStatus::PendingPayment {
        return Err(refused(english("edtech.enrolments.problem.not_pending", &[])));
    }
    Enrolment::query()
        .where_(Enrolment::ID.eq(id))
        .update(
            &state.db,
            &[("discount_approved", &true), ("updated_at", &now())],
        )
        .await?;
    Enrolment::find_or_404(&state.db, id).await
}

/// Runs the transition `user_name` as `by`, or `system_name` as the system
/// when nobody acts.
async fn run(
    state: &AppState,
    by: Option<&AuthUser>,
    id: i64,
    (user_name, system_name): (&str, &str),
    reason: Option<&str>,
    input: EnrolmentInput,
) -> Result<Enrolment> {
    let enrolment = Enrolment::find_or_404(&state.db, id).await?;
    let builder = match by {
        Some(user) => enrolment.transition(user_name).by(user),
        None => enrolment.transition(system_name).by_system(),
    };
    let builder = match reason {
        Some(reason) => builder.reason(reason),
        None => builder,
    };
    // What the listeners after the commit need to know: which cause, which day.
    let meta = renox::serde_json::json!({
        "cause": input.cause.map(FreezeCause::as_str),
        "on": input.on.map(|day| day.to_string()),
    });
    let _outcome = builder
        .input(&input)
        .meta(meta)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Enrolment::find_or_404(&state.db, id).await
}

/// `activate`: the first payment came (`by` is `None`), or staff confirm.
/// A lead of the student is won and a membership starts after the commit.
///
/// # Errors
///
/// The machine's: a 403 without `enrolments.manage`, a 422 unless the
/// enrolment is `pending_payment`, its terms accepted and its discount
/// approved.
pub async fn activate(state: &AppState, by: Option<&AuthUser>, id: i64) -> Result<Enrolment> {
    let input = EnrolmentInput {
        on: Some(today()),
        by: by.map(|user| user.id),
        ..EnrolmentInput::default()
    };
    run(state, by, id, (ACTIVATE, ACTIVATE_SYSTEM), None, input).await
}

/// The days from `on` to `until`, both counted.
fn days_between(on: NaiveDate, until: NaiveDate) -> i64 {
    (until - on).num_days() + 1
}

/// `freeze`: the enrolment is paused for `cause`, up to `until` (needed for
/// the student's own freeze, which counts against the allowance of 30 days a
/// year). A freeze on a `frozen` enrolment only adds its cause. A user needs
/// `enrolments.manage`; `by` is `None` for the system (BL-EDU-04, BL-EDU-05).
///
/// # Errors
///
/// The machine's; a 422 without `until` for a counted freeze, or when the
/// enrolment is already frozen for this cause.
pub async fn freeze(
    state: &AppState,
    by: Option<&AuthUser>,
    id: i64,
    cause: FreezeCause,
    until: Option<NaiveDate>,
) -> Result<Enrolment> {
    let enrolment = Enrolment::find_or_404(&state.db, id).await?;
    let on = today();
    let counted_days = if cause.counts_against_allowance() {
        let Some(until) = until.filter(|u| *u >= on) else {
            return Err(refused(english("edtech.enrolments.problem.until", &[])));
        };
        days_between(on, until)
    } else {
        0
    };
    let open = EnrolmentFreeze::open_of(&state.db, id).await?;
    if open.iter().any(|row| row.cause == cause) {
        return Err(refused(english("edtech.enrolments.problem.already_frozen", &[])));
    }
    let used_days =
        EnrolmentFreeze::counted_in_year(&state.db, enrolment.student_party_id, on).await?;
    let input = EnrolmentInput {
        on: Some(on),
        by: by.map(|user| user.id),
        cause: Some(cause),
        until,
        counted_days,
        used_days,
        ..EnrolmentInput::default()
    };
    run(state, by, id, (FREEZE, FREEZE_SYSTEM), None, input).await
}

/// `unfreeze`: the freeze of `cause` ends. Only that cause is removed; the
/// enrolment is active again when none is left, else it stays `frozen`.
///
/// # Errors
///
/// The machine's; a 422 when the enrolment is not frozen for `cause`.
pub async fn unfreeze(
    state: &AppState,
    by: Option<&AuthUser>,
    id: i64,
    cause: FreezeCause,
) -> Result<Enrolment> {
    let enrolment = Enrolment::find_or_404(&state.db, id).await?;
    let open = EnrolmentFreeze::open_of(&state.db, id).await?;
    if enrolment.status != EnrolmentStatus::Frozen || !open.iter().any(|row| row.cause == cause) {
        return Err(refused(english("edtech.enrolments.problem.not_frozen", &[])));
    }
    let left = open.iter().filter(|row| row.cause != cause).count();
    let on = today();
    if left > 0 {
        if let Some(user) = by {
            require(user, permissions::ENROLMENTS_MANAGE)?;
        }
        let mut tx = state.db.begin().await?;
        end_cause(&mut tx, id, cause, on).await?;
        tx.commit().await?;
        return Enrolment::find_or_404(&state.db, id).await;
    }
    let input = EnrolmentInput {
        on: Some(on),
        by: by.map(|user| user.id),
        cause: Some(cause),
        causes_left: Some(0),
        ..EnrolmentInput::default()
    };
    run(state, by, id, (UNFREEZE, UNFREEZE_SYSTEM), None, input).await
}

/// `complete`, as the system: the course ended or the credits are used up.
/// The certificate is evaluated after the commit ([`crate::certificates`]).
///
/// # Errors
///
/// A 422 unless the enrolment is `active`.
pub async fn complete(state: &AppState, id: i64) -> Result<Enrolment> {
    let input = EnrolmentInput {
        on: Some(today()),
        ..EnrolmentInput::default()
    };
    run(state, None, id, (COMPLETE, COMPLETE), None, input).await
}

/// What the enrolment's invoices have received, in fils: their total less
/// their balance.
///
/// # Errors
///
/// The database's errors and the contract's.
pub async fn paid_of(state: &AppState, enrolment: &Enrolment) -> Result<i64> {
    let mut ids: Vec<i64> = enrolment.first_invoice_id.into_iter().collect();
    // What moved here with a `transfer` counts as paid.
    for row in Instalment::of_enrolment(&state.db, enrolment.id).await? {
        if let Some(id) = row.invoice_id
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    let books = invoicer(state);
    let mut paid = enrolment.carried_paid;
    for id in ids {
        if let Some(summary) = books.find(&state.db, InvoiceId(id)).await? {
            paid += (summary.gross_total - summary.balance).max(0);
        }
    }
    Ok(paid)
}

/// The refund and the amount owed if the student left on `on` (`EDU.REF.01`):
/// the facts gathered, then [`refund::calculate`].
///
/// # Errors
///
/// The database's errors and the contracts'.
pub async fn refund_for(state: &AppState, enrolment: &Enrolment, on: NaiveDate) -> Result<Refund> {
    let paid = paid_of(state, enrolment).await?;
    let (basis, held, started) = match enrolment.kind {
        EnrolmentKind::Course => {
            let held = match enrolment.batch_id {
                Some(batch) => sessions::held_until(&state.db, batch, on).await?,
                None => 0,
            };
            let started = match enrolment.batch_id {
                Some(batch) => Batch::find(&state.db, batch)
                    .await?
                    .is_none_or(|b| b.start_date <= on),
                None => true,
            };
            (RefundBasis::Course, held, started)
        }
        EnrolmentKind::Package => {
            let left = match &enrolment.item_code {
                Some(item) => {
                    package_ledger(state)
                        .balance(&state.db, enrolment.student(), item)
                        .await?
                }
                None => enrolment.sessions_total,
            };
            (
                RefundBasis::Package {
                    single_session_price: refund::ratio(
                        enrolment.price,
                        1,
                        enrolment.sessions_total,
                    ),
                },
                (enrolment.sessions_total - left).max(0),
                true,
            )
        }
        EnrolmentKind::Membership => (RefundBasis::Membership, 0, true),
    };
    Ok(refund::calculate(&RefundInput {
        basis,
        policy: enrolment.policy.0.clone(),
        price: enrolment.fee(),
        paid,
        registration_fee: enrolment.registration_fee,
        admin_fee: enrolment.admin_fee,
        sessions_total: enrolment.sessions_total,
        sessions_held: held,
        started,
        pending_payment: enrolment.status == EnrolmentStatus::PendingPayment,
    }))
}

/// What a request to withdraw came to.
#[derive(Debug, Clone)]
pub struct Withdrawal {
    /// The enrolment as it is now: `withdrawn`, or unchanged while another
    /// user decides the refund.
    pub enrolment: Enrolment,
    /// The approval request that waits for the refund's approval
    /// (`FIN.ACL.04`), when there is one.
    pub approval_request: Option<i64>,
}

/// `withdraw`: the student leaves for `reason`. The refund the policy gives
/// (`EDU.REF.01`) is calculated and kept in the input. A user needs
/// `enrolments.manage`; when the refund is above nothing the kernel defers the
/// run as a request another user approves (`FIN.ACL.04`, [`withdraw_policy`]
/// in the machine) and the enrolment stays as it is; the run at the approval,
/// or at once when nothing is refunded, stores the refund, cancels the
/// instalments not yet invoiced and releases the seat, and
/// [`crate::settlement::settle`] makes the money documents after the commit.
/// `by` is `None` for a system withdrawal.
///
/// # Errors
///
/// The machine's: a 403 without `enrolments.manage`, a 422 without a reason or
/// unless the enrolment is `pending_payment`, `active` or `frozen`.
pub async fn request_withdrawal(
    state: &AppState,
    by: Option<&AuthUser>,
    id: i64,
    reason: &str,
) -> Result<Withdrawal> {
    let enrolment = Enrolment::find_or_404(&state.db, id).await?;
    // A blocked payer or student: only the centre manager withdraws (BL-EDU-04).
    if let Some(user) = by {
        crate::blocked::check_withdraw(state, user, &enrolment).await?;
    }
    let on = today();
    let refund = refund_for(state, &enrolment, on).await?;
    let input = EnrolmentInput {
        on: Some(on),
        by: by.map(|user| user.id),
        amount: refund.refund,
        reason: Some(reason.trim().to_owned()),
        refund: Some(refund),
        ..EnrolmentInput::default()
    };
    let builder = match by {
        Some(user) => enrolment.transition(WITHDRAW).by(user),
        None => enrolment.transition(WITHDRAW_SYSTEM).by_system(),
    };
    let outcome = builder
        .reason(reason)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    announce(state, &outcome).await?;
    let approval_request = match &outcome {
        Outcome::Pending(pending) => pending
            .data
            .get(REQUEST_KEY)
            .and_then(renox::serde_json::Value::as_i64),
        Outcome::Done(_) => None,
    };
    Ok(Withdrawal {
        enrolment: Enrolment::find_or_404(&state.db, id).await?,
        approval_request,
    })
}

/// [`request_withdrawal`], answering the enrolment.
///
/// # Errors
///
/// As [`request_withdrawal`].
pub async fn withdraw(
    state: &AppState,
    by: Option<&AuthUser>,
    id: i64,
    reason: &str,
) -> Result<Enrolment> {
    Ok(request_withdrawal(state, by, id, reason).await?.enrolment)
}

/// What follows a transition once it committed: a lead is won and a
/// membership starts when the enrolment activates, the subscription ends when
/// the student withdraws, the certificate is evaluated when it completes.
async fn after(state: &AppState, event: &Transitioned<Enrolment>) -> Result {
    let Some(enrolment) = Enrolment::find(&state.db, event.key).await? else {
        return Ok(());
    };
    match event.transition.as_str() {
        ACTIVATE | ACTIVATE_SYSTEM => {
            // A lead of the student, if there is one, is won (`SAL.CRM.01`).
            let _won = lead_conversion(state)
                .win_for_party(state, enrolment.student(), enrolment.subject())
                .await?;
            fees::start_membership(state, &enrolment).await?;
        }
        WITHDRAW | WITHDRAW_SYSTEM | WITHDRAW_APPROVED => {
            // The money documents follow the commit; a step that fails is kept
            // on the enrolment and repeated, it does not undo the withdrawal.
            crate::settlement::settle_quietly(state, enrolment.id).await;
        }
        UNFREEZE | UNFREEZE_SYSTEM => {
            let on = event
                .meta
                .get("on")
                .and_then(renox::serde_json::Value::as_str)
                .and_then(|text| text.parse::<NaiveDate>().ok())
                .unwrap_or_else(today);
            crate::frozen::after_unfreeze(state, &enrolment, on).await;
        }
        LAPSE => {
            crate::lapse::after_lapse(state, &enrolment).await?;
        }
        COMPLETE => {
            crate::certificates::evaluate(state, &enrolment).await?;
        }
        _ => {}
    }
    Ok(())
}

/// The first payment of an enrolment arrived: it activates. One whose terms
/// are not accepted or whose discount is not approved stays
/// `pending_payment` for staff to finish.
async fn on_invoice_paid(state: &AppState, event: &InvoicePaid) -> Result {
    let Some(enrolment) = Enrolment::of_first_invoice(&state.db, event.invoice.0).await? else {
        return Ok(());
    };
    if enrolment.status != EnrolmentStatus::PendingPayment {
        return Ok(());
    }
    let input = EnrolmentInput {
        on: Some(today()),
        ..EnrolmentInput::default()
    };
    let run = enrolment
        .transition(ACTIVATE_SYSTEM)
        .by_system()
        .input(&input)
        .run(state)
        .await;
    match run {
        Ok(_) | Err(TransitionError::Guard(_) | TransitionError::NotFrom(_)) => Ok(()),
        Err(other) => Err(other.into_error()),
    }
}

/// Registers the machine, the listeners and the daily run of the instalments.
pub(crate) fn register(app: &mut Registry) {
    crate::enrolment_machine::register();
    fees::register(app);
    crate::lapse::register(app);
    crate::blocked::register();
    crate::blocked::listen(app);
    app.listen(|event: Transitioned<Enrolment>, state: AppState| async move {
        after(&state, &event).await
    });
    app.listen(|event: InvoicePaid, state: AppState| async move {
        on_invoice_paid(&state, &event).await
    });
}
