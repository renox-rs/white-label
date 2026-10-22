//! The lapse of an unpaid seat hold (10 §4.1 *The first payment never
//! arrives*, §7.2 `EDU.RUL.11`, BL-EDU-01).
//!
//! An enrolment waiting for its first payment holds its seat until *seat hold
//! until*, 48 hours after it was created. When that time has passed and
//! nothing was paid, the run [`lapse_due`] lapses it: the seat is released, the
//! payment link is cancelled, the student or guardian and the counsellor are
//! told. The machine's guard `no_payment` ([`crate::enrolment_machine`]) holds
//! the rule; this file gathers its facts (what the invoices received, whether
//! a payment is still `pending` at the gateway) and runs the transition.
//!
//! - **A payment still `pending`** at the gateway stops the lapse; when it
//!   fails or times out (`PaymentFailed`) after the hold ended, [`on_payment_failed`]
//!   lapses the enrolment at once.
//! - **A payment that fails before the hold ends** does not end it and does not
//!   extend it: the payer gets a new payment link that expires with the hold.
//! - **A partial first payment** is a payment: the enrolment does not lapse and
//!   the counsellor sees the balance on the enrolment page.
//! - **A payment after the lapse**: `lapsed` is not an active prepaid subject
//!   ([`enrolment_active`], registered for `edtech.enrolment`), so wl-payments'
//!   `late_confirm` refunds it by itself, also when the student has been
//!   enrolled again; the counsellor is told of the refund
//!   ([`on_refund_approved`]). A `lapsed` enrolment never reopens, and a new
//!   enrolment takes its own seat (`EDU.ACL.17`).
//!
//! Decisions: the hold of 48 hours is [`crate::enrolments::HOLD_HOURS`]; the
//! setting page of the shared negative-flow mechanisms (BL-EDU-18) will make it
//! a setting. The run is every 15 minutes, not once a day, so an enrolment lapses
//! near the hour its hold ends.

use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::dubai_text;
use wl_kernel::payments::{
    DEFAULT_CURRENCY, PaymentFailed, PaymentFuture, PaymentLinkRequest, PaymentStatus,
    RefundApproved, RefundSource, payment_links, payment_recorder,
};
use wl_kernel::{LegalEntity, Subject};

use crate::batch::Batch;
use crate::enrolment::{ENROLMENT_SUBJECT_TYPE, Enrolment, EnrolmentStatus};
use crate::enrolment_machine::{EnrolmentInput, LAPSE, LapseFacts};
use crate::enrolments::{HOLD_HOURS, paid_of};
use crate::money_text;
use crate::notices;
use crate::texts::english;

/// The scheduled task and command that lapse the holds that ran out.
pub const TASK: &str = "edtech:lapse";
/// The minutes between two runs.
pub const TASK_EVERY_MINUTES: u64 = 15;

/// Whether the enrolment `subject` still waits for the money paid for it: an
/// enrolment in `pending_payment` does, and so does one with a reopened invoice
/// to collect. wl-payments asks this when a gateway
/// confirms a payment late; a `lapsed` enrolment is never an active subject, so
/// the late payment is refunded.
pub fn enrolment_active<'a>(state: &'a AppState, subject: &'a Subject) -> PaymentFuture<'a, bool> {
    Box::pin(async move {
        let Some(enrolment) = Enrolment::find(&state.db, subject.subject_id).await? else {
            return Ok(false);
        };
        if enrolment.status == EnrolmentStatus::PendingPayment {
            return Ok(true);
        }
        // An enrolment whose paid invoice was reopened by a reversed payment
        // takes the money paid on the link sent to collect it (BL-EDU-06).
        let collecting = matches!(
            enrolment.status,
            EnrolmentStatus::Active | EnrolmentStatus::Frozen | EnrolmentStatus::Completed
        );
        Ok(collecting && crate::chargebacks::is_outstanding(&state.db, enrolment.id).await?)
    })
}

/// Whether a payment of the enrolment is `pending` at the gateway.
async fn pending_at_gateway(state: &AppState, enrolment: &Enrolment) -> Result<bool> {
    let payments = payment_recorder(state)
        .payments_for(&state.db, &enrolment.subject())
        .await?;
    Ok(payments
        .iter()
        .any(|payment| payment.status == PaymentStatus::Pending))
}

/// The facts the guard of `lapse` needs for `enrolment` at `at`.
async fn facts(state: &AppState, enrolment: &Enrolment, at: DateTime) -> Result<LapseFacts> {
    Ok(LapseFacts {
        at,
        paid: paid_of(state, enrolment).await?,
        pending: pending_at_gateway(state, enrolment).await?,
    })
}

/// Lapses enrolment `id` as of `at` if its hold ran out, nothing was paid and
/// no payment is pending at the gateway. Answers the enrolment as it is after,
/// or `None` when it does not lapse (not `pending_payment`, the hold goes on, a
/// payment came or is pending).
///
/// # Errors
///
/// The database's errors and the contracts'.
pub async fn lapse_one(state: &AppState, id: i64, at: DateTime) -> Result<Option<Enrolment>> {
    let Some(enrolment) = Enrolment::find(&state.db, id).await? else {
        return Ok(None);
    };
    if enrolment.status != EnrolmentStatus::PendingPayment {
        return Ok(None);
    }
    let input = EnrolmentInput {
        on: Some(wl_kernel::documents::dubai_date(at)),
        lapse: Some(facts(state, &enrolment, at).await?),
        ..EnrolmentInput::default()
    };
    let run = enrolment
        .transition(LAPSE)
        .by_system()
        .reason(english("edtech.lapse.reason", &[]))
        .input(&input)
        .run(state)
        .await;
    match run {
        Ok(_outcome) => Ok(Some(Enrolment::find_or_404(&state.db, id).await?)),
        Err(TransitionError::Guard(_) | TransitionError::NotFrom(_) | TransitionError::Conflict) => {
            Ok(None)
        }
        Err(other) => Err(other.into_error()),
    }
}

/// The daily run: lapses every `pending_payment` enrolment whose hold ran out by
/// `at`. Returns the ids lapsed.
///
/// # Errors
///
/// The database's errors and the contracts'.
pub async fn lapse_due(state: &AppState, at: DateTime) -> Result<Vec<i64>> {
    let due = Enrolment::query()
        .where_(Enrolment::STATUS.eq(EnrolmentStatus::PendingPayment))
        .where_(Enrolment::SEAT_HOLD_UNTIL.lte(at))
        .order_by(Enrolment::ID)
        .get(&state.db)
        .await?;
    let mut lapsed = Vec::new();
    for enrolment in due {
        if lapse_one(state, enrolment.id, at).await?.is_some() {
            lapsed.push(enrolment.id);
        }
    }
    Ok(lapsed)
}

/// What follows the commit of `lapse`: the open payment link is cancelled and
/// the guardian (or the student) and the counsellor are told.
///
/// # Errors
///
/// The notifier's errors; a link that cannot be cancelled is not an error (the
/// payment that still arrives is refunded).
pub async fn after_lapse(state: &AppState, enrolment: &Enrolment) -> Result {
    cancel_link(state, enrolment).await;
    let batch = batch_number(state, enrolment).await;
    let hours = HOLD_HOURS;
    let number = enrolment.number();
    let family_title = english("edtech.lapse.family_title", &[("batch", &batch)]);
    let family_body = english(
        "edtech.lapse.family_body",
        &[("enrolment", &number), ("hours", &hours)],
    );
    notices::tell_family(state, enrolment, &family_title, &family_body).await?;
    let student = crate::enrolments::party_name(state, "en", enrolment.student_party_id).await;
    let staff_title = english("edtech.lapse.staff_title", &[("enrolment", &number)]);
    let staff_body = english(
        "edtech.lapse.staff_body",
        &[("student", &student), ("batch", &batch), ("hours", &hours)],
    );
    let users: Vec<i64> = enrolment.counsellor_id.into_iter().collect();
    notices::tell_users(
        state,
        enrolment.subject(),
        &number,
        &users,
        &staff_title,
        &staff_body,
    )
    .await?;
    Ok(())
}

/// The number of the batch of `enrolment`, or a dash.
async fn batch_number(state: &AppState, enrolment: &Enrolment) -> String {
    match enrolment.batch_id {
        Some(id) => Batch::find(&state.db, id)
            .await
            .ok()
            .flatten()
            .map_or_else(|| "-".to_owned(), |batch| batch.number()),
        None => enrolment.item_name.clone().unwrap_or_else(|| "-".to_owned()),
    }
}

/// Withdraws the payment link of `enrolment` so the page no longer takes
/// payment. A failure is logged: the enrolment is over whatever happens here,
/// and a payment that still arrives is refunded.
pub async fn cancel_link(state: &AppState, enrolment: &Enrolment) {
    let Some(link) = enrolment.payment_link() else {
        return;
    };
    if let Err(error) = payment_links(state).cancel(state, &link).await {
        eprintln!(
            "edtech: the payment link of {} was not cancelled: {error}",
            enrolment.number()
        );
    }
}

/// Makes the payment link for the first payment of `enrolment` (`amount`
/// fils), expiring when the seat hold does, and stores it on the enrolment.
/// Without the payments module, or a gateway, there is no link.
///
/// # Errors
///
/// The database's errors; a link the payments module refuses is not an error.
pub async fn open_link(state: &AppState, enrolment: &Enrolment, amount: i64) -> Result<Enrolment> {
    let Some(expires_at) = enrolment.seat_hold_until else {
        return Ok(enrolment.clone());
    };
    if amount <= 0 {
        return Ok(enrolment.clone());
    }
    let scope = enrolment.campus_type.as_deref().zip(enrolment.campus_id);
    let Ok(entity) = LegalEntity::for_scope(&state.db, scope).await else {
        return Ok(enrolment.clone());
    };
    let request = PaymentLinkRequest {
        party: enrolment.payer(),
        legal_entity_id: entity.id,
        amount,
        currency: DEFAULT_CURRENCY.to_owned(),
        prepaid: enrolment.subject(),
        expires_at,
        created_by: enrolment.counsellor_id,
    };
    let made = payment_links(state).create(state, request).await;
    let Ok(Some(link)) = made else {
        return Ok(enrolment.clone());
    };
    Enrolment::query()
        .where_(Enrolment::ID.eq(enrolment.id))
        .update(
            &state.db,
            &[
                ("payment_link_type", &Some(link.link.subject_type.clone())),
                ("payment_link_id", &Some(link.link.subject_id)),
                ("payment_link_path", &Some(link.path.clone())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Enrolment::find_or_404(&state.db, enrolment.id).await
}

/// `PaymentFailed` for the enrolment: after the hold ended, it lapses at once
/// (no payment can be pending now); before, the hold goes on unchanged, the
/// failed link is replaced and the payer gets the new one.
///
/// # Errors
///
/// The database's errors, the contracts' and the notifier's.
pub async fn on_payment_failed(state: &AppState, event: &PaymentFailed) -> Result {
    let Some(prepaid) = &event.prepaid else {
        return Ok(());
    };
    if prepaid.subject_type != ENROLMENT_SUBJECT_TYPE {
        return Ok(());
    }
    let Some(enrolment) = Enrolment::find(&state.db, prepaid.subject_id).await? else {
        return Ok(());
    };
    if enrolment.status != EnrolmentStatus::PendingPayment {
        return Ok(());
    }
    let at = now();
    if enrolment.seat_hold_until.is_some_and(|until| at >= until) {
        lapse_one(state, enrolment.id, at).await?;
        return Ok(());
    }
    cancel_link(state, &enrolment).await;
    let renewed = open_link(state, &enrolment, event.amount).await?;
    let Some(path) = renewed.payment_link_path.clone() else {
        return Ok(());
    };
    let until = renewed.seat_hold_until.map_or_else(String::new, dubai_text);
    let number = renewed.number();
    let title = english("edtech.lapse.retry_title", &[]);
    let body = english(
        "edtech.lapse.retry_body",
        &[("enrolment", &number), ("until", &until), ("link", &path)],
    );
    notices::tell_family(state, &renewed, &title, &body).await
}

/// `RefundApproved` for a late payment (the system approved it at once): the
/// counsellor of the lapsed enrolment the payment was for, and of an enrolment
/// of the same student in the same batch made since, is told of the refund.
///
/// # Errors
///
/// The database's errors, the contracts' and the notifier's.
pub async fn on_refund_approved(state: &AppState, event: &RefundApproved) -> Result {
    let RefundSource::Payment(payment) = &event.source else {
        return Ok(());
    };
    if event.approved_by.is_some() {
        return Ok(());
    }
    let recorder = payment_recorder(state);
    let candidates = Enrolment::query()
        .where_(Enrolment::PAYER_PARTY_ID.eq(event.party.0))
        .where_(Enrolment::STATUS.eq(EnrolmentStatus::Lapsed))
        .order_by(Enrolment::ID)
        .get(&state.db)
        .await?;
    for lapsed in candidates {
        let paid_for = recorder.payments_for(&state.db, &lapsed.subject()).await?;
        if !paid_for.iter().any(|p| p.id == *payment) {
            continue;
        }
        let mut users: Vec<i64> = lapsed.counsellor_id.into_iter().collect();
        // The student may have been enrolled again meanwhile.
        let again = Enrolment::query()
            .where_(Enrolment::STUDENT_PARTY_ID.eq(lapsed.student_party_id))
            .where_(Enrolment::ID.ne(lapsed.id))
            .get(&state.db)
            .await?;
        users.extend(
            again
                .iter()
                .filter(|e| e.batch_id == lapsed.batch_id && e.status.takes_a_seat())
                .filter_map(|e| e.counsellor_id),
        );
        let number = lapsed.number();
        let amount = money_text(event.amount);
        let title = english("edtech.lapse.refund_title", &[]);
        let body = english(
            "edtech.lapse.refund_body",
            &[("amount", &amount), ("enrolment", &number)],
        );
        notices::tell_users(state, lapsed.subject(), &number, &users, &title, &body).await?;
    }
    Ok(())
}

/// Registers the prepaid check, the listeners, the run and its command.
pub(crate) fn register(app: &mut Registry) {
    wl_kernel::payments::register_prepaid_subject_type(ENROLMENT_SUBJECT_TYPE, enrolment_active);
    app.listen(|event: PaymentFailed, state: AppState| async move {
        on_payment_failed(&state, &event).await
    });
    app.listen(|event: RefundApproved, state: AppState| async move {
        on_refund_approved(&state, &event).await
    });
    app.schedule()
        .every_minutes(TASK_EVERY_MINUTES, TASK, |state: AppState| async move {
            lapse_due(&state, now()).await.map(|_| ())
        });
    app.command(
        TASK,
        "Lapse the seat holds whose hours ran out without a payment",
        |_args, state: AppState| async move {
            let lapsed = lapse_due(&state, now()).await?;
            println!("{} enrolments lapsed.", lapsed.len());
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_comes_often_enough_for_a_48_hour_hold() {
        const { assert!(TASK_EVERY_MINUTES <= 60) };
        assert_eq!(HOLD_HOURS, 48);
    }
}
