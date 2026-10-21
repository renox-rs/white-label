//! A cash on delivery collected short (wl-delivery §4.1 `deliver`, §5.2 step
//! 4, `WRK.ACL.09`, issue #159).
//!
//! When the recipient pays less than the cash expected by more than the
//! tolerance (§7: AED 0), the driver either fails the attempt (*refused to
//! pay*, *no cash*) or records the difference with a reason and asks for
//! approval from the phone:
//!
//! - the request is the user transition `deliver_short`; the kernel's policy
//!   `delivery.deliver_short` (always, one step: any holder of
//!   `deliveries.approve_cod_difference`, never the requester) defers it as
//!   an approval request and the delivery stays `out_for_delivery`;
//! - the approver chooses who absorbs the difference (customer, client,
//!   business or driver; the kernel approval carries no choice, so
//!   [`approve`] stores it in [`ApprovalChoice`] just before approving, and
//!   the transition `deliver_short_approved` reads it); without a choice the
//!   reason's default applies, and an approval with no absorber that is
//!   offered is refused;
//! - the driver can withdraw at any time with *no answer: fail attempt*
//!   ([`crate::proof::fail`] withdraws the pending request);
//! - the request lapses after the approval timeout (§7: 10 minutes): the
//!   kernel's `expires_at` is set to the creation time plus the timeout, and
//!   [`lapse_overdue`] (called by every read and action on the delivery and
//!   by a task every minute) expires it, so the phone offers only the failed
//!   attempt and a late approval is refused;
//! - an approver's *no* leaves the delivery `out_for_delivery`; the driver
//!   collects in full or fails the attempt.
//!
//! The driver is the request's requester, so the kernel's segregation of
//! duties keeps the driver from approving their own difference, even as an
//! admin. The difference is not posted here (invoice balance, remittance
//! line and COD loss are for the consumers of `DeliveryDelivered`).

use renox::chrono::Duration;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{
    self, ApprovalEnd, ApprovalRequest, ApprovalStatus, PolicyDefault, decide, submit::REQUEST_KEY,
};
use wl_kernel::delivery::{CodAbsorber, DELIVERY_SUBJECT_TYPE};

use crate::delivery::{Attempt, Delivery};
use crate::permissions;
use crate::settings::settings;
use crate::texts::english;

/// The approval policy's action key of `deliver_short`.
pub const ACTION: &str = "delivery.deliver_short";

/// The rule, as the specification names it.
pub const RULE: &str = "WRK.ACL.09";

/// The task that lapses the requests nobody answered.
pub const TASK: &str = "delivery:cod-lapse";

/// What an approver chose on an approval request of this module: who absorbs
/// a cash difference (#159), or what happens to cash already handed over when
/// a delivery is reversed (#161). The `*_approved` transition reads it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_delivery_approval_choices", unique(request_id))]
pub struct ApprovalChoice {
    /// The primary key.
    pub id: i64,
    /// The approval request.
    pub request_id: i64,
    /// Who absorbs the difference: `customer`, `client`, `business`,
    /// `driver`.
    pub absorbed_by: Option<String>,
    /// What happens to cash already handed over: `refund` or `keep`.
    pub cash_treatment: Option<String>,
    /// The approver.
    pub chosen_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ApprovalChoice {
    /// The choice made on request `request_id`, read inside a transaction.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub(crate) async fn in_tx(
        tx: &mut Transaction,
        request_id: i64,
    ) -> Result<Option<ApprovalChoice>> {
        ApprovalChoice::query()
            .where_(ApprovalChoice::REQUEST_ID.eq(request_id))
            .first(&mut *tx)
            .await
    }

    /// Stores the approver's choice for request `request_id`, replacing an
    /// earlier one of the same request.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub(crate) async fn record(
        db: &Db,
        request_id: i64,
        absorbed_by: Option<String>,
        cash_treatment: Option<String>,
        by: i64,
    ) -> Result<()> {
        let earlier = ApprovalChoice::query()
            .where_(ApprovalChoice::REQUEST_ID.eq(request_id))
            .first(db)
            .await?;
        if earlier.is_some() {
            let chosen_by = Some(by);
            ApprovalChoice::query()
                .where_(ApprovalChoice::REQUEST_ID.eq(request_id))
                .update(
                    db,
                    &[
                        ("absorbed_by", &absorbed_by),
                        ("cash_treatment", &cash_treatment),
                        ("chosen_by", &chosen_by),
                    ],
                )
                .await?;
            return Ok(());
        }
        let mut row = ApprovalChoice {
            request_id,
            absorbed_by,
            cash_treatment,
            chosen_by: Some(by),
            ..ApprovalChoice::default()
        };
        row.insert(db).await
    }
}

fn unprocessable(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The latest pending request for action `action` about delivery
/// `delivery_id`.
///
/// # Errors
///
/// Database errors.
pub async fn pending_request(
    db: &Db,
    delivery_id: i64,
    action: &str,
) -> Result<Option<ApprovalRequest>> {
    ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(DELIVERY_SUBJECT_TYPE))
        .where_(ApprovalRequest::SUBJECT_ID.eq(delivery_id))
        .where_(ApprovalRequest::ACTION.eq(action))
        .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
        .order_by_desc(ApprovalRequest::ID)
        .first(db)
        .await
}

/// The latest request, of any status, for action `action` about delivery
/// `delivery_id`.
///
/// # Errors
///
/// Database errors.
pub async fn latest_request(
    db: &Db,
    delivery_id: i64,
    action: &str,
) -> Result<Option<ApprovalRequest>> {
    ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(DELIVERY_SUBJECT_TYPE))
        .where_(ApprovalRequest::SUBJECT_ID.eq(delivery_id))
        .where_(ApprovalRequest::ACTION.eq(action))
        .order_by_desc(ApprovalRequest::ID)
        .first(db)
        .await
}

/// The sentence that tells the driver why a short payment cannot be asked
/// again for the visit now under way: the latest request lapsed or was
/// rejected, and no visit was recorded since. `None` when a request may be
/// made.
///
/// # Errors
///
/// Database errors.
pub async fn blocked(db: &Db, delivery: &Delivery) -> Result<Option<String>> {
    let Some(request) = latest_request(db, delivery.id, ACTION).await? else {
        return Ok(None);
    };
    let last_visit = Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(delivery.id))
        .order_by_desc(Attempt::ID)
        .first(db)
        .await?
        .map(|attempt| attempt.occurred_at);
    let current = match (request.created_at, last_visit) {
        (Some(created), Some(visit)) => created > visit,
        _ => true,
    };
    if !current {
        return Ok(None);
    }
    Ok(match request.status {
        ApprovalStatus::Expired => Some(english("wl_delivery.cod.lapsed").to_owned()),
        ApprovalStatus::Rejected => Some(english("wl_delivery.cod.rejected").to_owned()),
        _ => None,
    })
}

/// Sets the lapse time of the new request `request_id` to now plus the
/// approval timeout (the kernel's policy expiry counts hours).
///
/// # Errors
///
/// Database errors.
pub(crate) async fn set_timeout(state: &AppState, request_id: i64) -> Result<()> {
    let minutes = settings(state).cod_approval_minutes();
    let expires = Some(renox::db::now() + Duration::minutes(minutes));
    ApprovalRequest::query()
        .where_(ApprovalRequest::ID.eq(request_id))
        .update(&state.db, &[("expires_at", &expires)])
        .await?;
    Ok(())
}

/// The id of the request a deferred `deliver_short` created.
pub(crate) fn request_id<M: renox::state_machine::StateMachine>(
    outcome: &renox::state_machine::Outcome<M>,
) -> Option<i64> {
    match outcome {
        renox::state_machine::Outcome::Pending(pending) => pending
            .data
            .get(REQUEST_KEY)
            .and_then(renox::serde_json::Value::as_i64),
        _ => None,
    }
}

/// Lapses the pending request of delivery `delivery_id` when its time is up:
/// the request ends *expired*, the delivery stays `out_for_delivery` and the
/// driver can only fail the attempt. Returns whether one lapsed.
///
/// # Errors
///
/// Database errors.
pub async fn lapse_overdue(state: &AppState, delivery_id: i64) -> Result<bool> {
    let Some(request) = pending_request(&state.db, delivery_id, ACTION).await? else {
        return Ok(false);
    };
    if !request.is_past_expiry(renox::db::now()) {
        return Ok(false);
    }
    match approvals::expire(state, request.id).await {
        Ok(_) => Ok(true),
        // Someone decided it meanwhile.
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)) => Ok(false),
        Err(error) => Err(error),
    }
}

/// The task: lapses every request of this module nobody answered in time.
/// Returns how many lapsed.
///
/// # Errors
///
/// Database errors.
pub async fn lapse_all(state: &AppState) -> Result<usize> {
    let pending = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(DELIVERY_SUBJECT_TYPE))
        .where_(ApprovalRequest::ACTION.eq(ACTION))
        .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
        .get(&state.db)
        .await?;
    let mut lapsed = 0;
    for request in pending {
        if lapse_overdue(state, request.subject_id).await? {
            lapsed += 1;
        }
    }
    Ok(lapsed)
}

/// The final re-check of a request about a cash difference: a request whose
/// time is up fails, so a late approval is refused.
pub(crate) fn recheck(request: &ApprovalRequest) -> Option<String> {
    request
        .is_past_expiry(renox::db::now())
        .then(|| english("wl_delivery.cod.timed_out").to_owned())
}

/// `user` (a holder of `deliveries.approve_cod_difference`, never the
/// driver who asked) approves the pending difference of `delivery`, choosing
/// who absorbs it (`absorbed_by`; blank takes the reason's default), with an
/// optional `comment`. The delivery is `delivered` when this returns.
///
/// # Errors
///
/// 403 for anyone who may not decide the request (the requester included);
/// 422 when no request waits (it lapsed, was withdrawn or decided), the
/// absorber is not one that is offered, none is chosen and the reason has no
/// default that is offered, or the delivery can no longer be delivered.
pub async fn approve(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    absorbed_by: Option<&str>,
    comment: &str,
    instruction: &str,
) -> Result<()> {
    lapse_overdue(state, delivery.id).await?;
    let request = pending_request(&state.db, delivery.id, ACTION)
        .await?
        .ok_or_else(|| unprocessable(english("wl_delivery.cod.no_request")))?;
    decide::ensure_approver(&state.db, &request, user).await?;
    let chosen = match absorbed_by.map(str::trim).filter(|key| !key.is_empty()) {
        None => None,
        Some(key) => {
            let absorber = CodAbsorber::parse(key)
                .filter(|absorber| settings(state).offers_absorber(*absorber))
                .ok_or_else(|| {
                    unprocessable(english("wl_delivery.cod.not_offered").replace("{absorber}", key))
                })?;
            Some(absorber.as_str().to_owned())
        }
    };
    // The source's own rule on the choice (03: the client only on its
    // recorded instruction, never the customer).
    if let (Some(key), Some(rule)) = (&chosen, wl_kernel::delivery::cod_rule(state))
        && let Some(absorber) = CodAbsorber::parse(key)
    {
        rule.check(
            state,
            wl_kernel::delivery::CodDecision {
                delivery: delivery.delivery_id(),
                source: delivery.source(),
                absorber,
                instruction: instruction.trim().to_owned(),
                approver: user.id,
            },
        )
        .await?;
    }
    ApprovalChoice::record(&state.db, request.id, chosen, None, user.id).await?;
    match approvals::approve(state, request.id, user, comment).await? {
        Some(decided) if decided.decision != ApprovalEnd::Approved => Err(unprocessable(
            english("wl_delivery.cod.approval_failed").to_owned(),
        )),
        _ => Ok(()),
    }
}

/// `user` rejects the pending difference of `delivery` with a `comment`
/// (required): the delivery stays `out_for_delivery`, the driver collects in
/// full or fails the attempt.
///
/// # Errors
///
/// 403 for anyone who may not decide the request; 422 without a comment or
/// when no request waits.
pub async fn reject(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    comment: &str,
) -> Result<()> {
    lapse_overdue(state, delivery.id).await?;
    let request = pending_request(&state.db, delivery.id, ACTION)
        .await?
        .ok_or_else(|| unprocessable(english("wl_delivery.cod.no_request")))?;
    approvals::reject(state, request.id, user, comment).await?;
    Ok(())
}

/// Withdraws the pending difference of `delivery` when `user` is the driver
/// who asked (the driver chose *no answer: fail attempt*). Returns whether
/// one was withdrawn.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn withdraw_pending(
    state: &AppState,
    user: &AuthUser,
    delivery_id: i64,
) -> Result<bool> {
    lapse_overdue(state, delivery_id).await?;
    let Some(request) = pending_request(&state.db, delivery_id, ACTION).await? else {
        return Ok(false);
    };
    if request.requester_id != user.id {
        return Ok(false);
    }
    approvals::withdraw(state, request.id, user).await?;
    Ok(true)
}

/// Registers the approval policy and the task that lapses requests.
pub(crate) fn register(app: &mut Registry) {
    // `WRK.ACL.09`: one step, any holder of the permission; the requester (the
    // driver) is excluded by the kernel's segregation of duties. The policy's
    // own expiry is in hours: the minutes are set on each request.
    PolicyDefault::new(ACTION)
        .step_permission(permissions::APPROVE_COD_DIFFERENCE)
        .expiry_hours(Some(1))
        .register();
    app.schedule()
        .every_minute(TASK, |state: AppState| async move {
            lapse_all(&state).await.map(|_| ())
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_past_its_time_fails_at_the_final_check() {
        let late = ApprovalRequest {
            expires_at: Some(renox::db::now() - Duration::minutes(1)),
            ..ApprovalRequest::default()
        };
        assert!(recheck(&late).is_some());
        let open = ApprovalRequest {
            expires_at: Some(renox::db::now() + Duration::minutes(5)),
            ..ApprovalRequest::default()
        };
        assert!(recheck(&open).is_none());
    }
}
