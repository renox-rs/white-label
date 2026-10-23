//! The insurer declares the vehicle a total loss (08 §2, §4.7 row *The insurer
//! declares a total loss…*, §7.1 *Insurance repair claim* `declare_total_loss`
//! and `invoice`; BL-AUT-09).
//!
//! The coordinator declares it with the insurer's notice attached
//! ([`crate::claims::declare_total_loss`]). What follows the commit is
//! [`follow`], once:
//!
//! 1. **work stops**: the job card is `cancel`led by the system with the reason
//!    *total loss* (the work done is recorded, the parts issued and not fitted
//!    go back to stock), or `decline`d when no work started. The job card makes
//!    **no invoice of its own** ([`crate::stop_billing::bill_closed`] answers
//!    nothing for a total loss): the claim's `invoice` bills both parties, so
//!    nothing is billed twice;
//! 2. **the charges are listed** on the claim for the insurer's approval, one
//!    line per strip-down or labour line done, per part fitted and for the
//!    storage ([`charge_lines`]);
//! 3. the customer is told ([`crate::insurer_mail::TOTAL_LOSS_TYPE`]).
//!
//! The coordinator then records the insurer's approval of the charges, line by
//! line ([`crate::claims::approve_charges`]), and the accountant runs `invoice`:
//! the insurer is billed what it approved, the customer the rest
//! ([`unapproved`]). The vehicle leaves on the insurer's written instruction
//! and a signature, paid or not ([`crate::release`]).
//!
//! Decisions:
//!
//! - **Storage** is charged only here after a total loss: from the check-in
//!   day to the release day (to today while the vehicle is still in), at the
//!   storage rate the insurer agreed ([`crate::insurer::Insurer::storage_rate_daily`];
//!   none or 0: no charge), at least one day. The customer storage fee of
//!   `AUT.RUL.18` is never charged on a job card cancelled for a total loss
//!   ([`crate::storage::fee_applies`]), so storage is not billed twice. A charge
//!   line the insurer approves for less than its price, or not at all, goes to
//!   the customer's invoice for the difference.
//! - The strip-down is a labour line of the job card (the workshop names it
//!   so); the app has no separate strip-down entity.
//! - An insurer who declines every charge is not built: the coordinator
//!   records the approval of the lines it accepts, and the rest is the
//!   customer's (see `DEFERRED.md`).

use std::sync::Arc;

use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::notify::{NotifyRequest, notifier};
use wl_kernel::{ClientConfig, PartyId};

use crate::approval::EstimateApproval;
use crate::claim::{ClaimStatus, InsuranceClaim};
use crate::claim_approval::{ApprovalKind, ApprovalOutcome, ApprovedLine, ClaimApproval};
use crate::claims::RequestLine;
use crate::estimate::Estimate;
use crate::insurer::Insurer;
use crate::insurer_mail::TOTAL_LOSS_TYPE;
use crate::job_card::{JobCard, JobCardStatus};
use crate::job_card_machine::{DECLINE_SYSTEM, JobInput};
use crate::job_cards::run_system;
use crate::job_stops::{self, reason_text};
use crate::labour_line::LineStatus;
use crate::stop::TOTAL_LOSS;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// The days of storage the insurer is charged for a vehicle checked in on
/// `check_in` and released on `release` (today while it is still in): at least
/// one.
pub fn storage_days(check_in: renox::chrono::NaiveDate, release: renox::chrono::NaiveDate) -> i64 {
    (release - check_in).num_days().max(1)
}

/// The charges of the total loss of `claim`, line by line: the labour done
/// (the strip-down among it), the parts fitted, and the storage.
///
/// # Errors
///
/// The database's errors.
pub async fn charge_lines(db: &Db, claim: &InsuranceClaim) -> Result<Vec<RequestLine>> {
    let Some(job_id) = claim.job_card_id else {
        return Ok(Vec::new());
    };
    let card = JobCard::find_or_404(db, job_id).await?;
    let estimate = Estimate::load(db, job_id).await?;
    let mut lines = Vec::new();
    for line in estimate
        .labour
        .iter()
        .filter(|line| line.approval == LineStatus::Approved)
    {
        let price = line.done_value();
        if price > 0 {
            lines.push(RequestLine {
                kind: "labour",
                line_id: line.id,
                description: line.description.clone(),
                price,
            });
        }
    }
    for line in estimate
        .parts
        .iter()
        .filter(|line| line.approval == LineStatus::Approved)
    {
        let price = line.done_value();
        if price > 0 {
            lines.push(RequestLine {
                kind: "part",
                line_id: line.id,
                description: line.description.clone(),
                price,
            });
        }
    }
    let insurer = Insurer::find_or_404(db, claim.insurer_id).await?;
    let rate = insurer.storage_rate_daily.unwrap_or(0);
    if rate > 0 {
        let start = card.created_at.map_or_else(today, dubai_date);
        let end = card.released_at.map_or_else(today, dubai_date);
        let days = storage_days(start, end);
        let rate_text = crate::money_text(rate);
        let description = english(
            "auto.total_loss.storage_line",
            &[("days", &days), ("rate", &rate_text)],
        );
        lines.push(RequestLine {
            kind: "charge",
            line_id: 0,
            description,
            price: rate * days,
        });
    }
    Ok(lines)
}

/// The part of each charge the insurer did not approve, with its line: the
/// customer's invoice carries it. A charge approved in full is left out.
pub fn unapproved(charges: &[RequestLine], approved: &[ApprovedLine]) -> Vec<(RequestLine, i64)> {
    charges
        .iter()
        .filter_map(|charge| {
            let got: i64 = approved
                .iter()
                .filter(|line| line.kind == charge.kind && line.line_id == charge.line_id)
                .map(|line| line.amount)
                .sum();
            let rest = charge.price - got;
            (rest > 0).then(|| (charge.clone(), rest))
        })
        .collect()
}

/// The system stops the job card: `cancel` (reason *total loss*), or `decline`
/// when nothing was approved yet.
async fn stop_job(
    state: &AppState,
    claim: &InsuranceClaim,
    card: &JobCard,
    by: Option<i64>,
) -> Result {
    let can_decline = matches!(
        card.status,
        JobCardStatus::CheckedIn | JobCardStatus::Inspecting | JobCardStatus::AwaitingApproval
    ) && !card.work_approved();
    if can_decline {
        let round = EstimateApproval::pending_of(&state.db, card.id).await?;
        let input = JobInput {
            by,
            on: Some(today()),
            approval_id: round.map(|round| round.id),
            code: Some(TOTAL_LOSS.to_owned()),
            lines_recorded: Some(0),
            ..JobInput::default()
        };
        let reason = reason_text(TOTAL_LOSS, &None);
        run_system(state, card, DECLINE_SYSTEM, Some(reason.as_str()), input).await?;
    } else {
        let notice = claim.total_loss_notice.clone().unwrap_or_default();
        job_stops::cancel_by_system(state, card.id, TOTAL_LOSS, &notice).await?;
    }
    Ok(())
}

/// The request to the insurer for the approval of the charges: one is made per
/// claim, and kept pending until the insurer answers.
async fn list_charges(state: &AppState, claim: &InsuranceClaim) -> Result {
    let known = ClaimApproval::of_claim(&state.db, claim.id).await?;
    if known.iter().any(|row| row.kind == ApprovalKind::Charges) {
        return Ok(());
    }
    let lines = charge_lines(&state.db, claim).await?;
    let mut request = ClaimApproval {
        claim_id: claim.id,
        kind: ApprovalKind::Charges,
        round_no: i64::try_from(known.len()).unwrap_or(0) + 1,
        estimate_amount: lines.iter().map(|line| line.price).sum(),
        outcome: ApprovalOutcome::Pending,
        requested_at: Some(now()),
        ..ClaimApproval::default()
    };
    request.insert(&state.db).await?;
    Ok(())
}

/// The workshop's name for a message.
fn workshop_name(state: &AppState) -> String {
    state
        .provided::<Arc<ClientConfig>>()
        .map(|config| config.client.name.clone())
        .unwrap_or_default()
}

/// Tells the customer the insurer declared a total loss. A message that cannot
/// be queued is the advisor's to follow up; the declaration stands.
async fn tell_customer(state: &AppState, claim: &InsuranceClaim) -> Result {
    let plate = Vehicle::find(&state.db, claim.vehicle_id)
        .await?
        .map_or_else(String::new, |vehicle| vehicle.plate());
    let request = NotifyRequest::new(TOTAL_LOSS_TYPE, claim.subject())
        .subject_label(&claim.number())
        .to_party(PartyId(claim.customer_party_id))
        .data(json!({
            "plate": plate,
            "claim": claim.number(),
            "claim_no": claim.claim_no,
            "workshop": workshop_name(state),
        }));
    let _queued = notifier(state).notify(state, request).await.is_ok();
    Ok(())
}

/// What follows a total loss that committed: the job card stops, the charges
/// are listed for the insurer's approval, and the customer is told. A step
/// already done is left as it is.
///
/// # Errors
///
/// The database's, the machine's, the stock's and the scheduling contract's
/// errors. `by` is the user who declared it.
pub async fn follow(state: &AppState, claim_id: i64, by: Option<i64>) -> Result {
    let claim = InsuranceClaim::find_or_404(&state.db, claim_id).await?;
    if claim.status != ClaimStatus::TotalLoss {
        return Ok(());
    }
    if let Some(job_id) = claim.job_card_id {
        let card = JobCard::find_or_404(&state.db, job_id).await?;
        if !card.status.is_closed() {
            stop_job(state, &claim, &card, by).await?;
        }
    }
    list_charges(state, &claim).await?;
    tell_customer(state, &claim).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    #[test]
    fn storage_is_at_least_a_day_from_the_check_in_to_the_release() {
        assert_eq!(storage_days(day(1), day(1)), 1);
        assert_eq!(storage_days(day(1), day(6)), 5);
    }

    fn charge(kind: &'static str, id: i64, price: i64) -> RequestLine {
        RequestLine {
            kind,
            line_id: id,
            description: "x".to_owned(),
            price,
        }
    }

    fn approved(kind: &str, id: i64, amount: i64) -> ApprovedLine {
        ApprovedLine {
            kind: kind.to_owned(),
            line_id: id,
            description: "x".to_owned(),
            amount,
        }
    }

    #[test]
    fn the_customer_pays_what_the_insurer_did_not_approve() {
        let charges = [
            charge("labour", 1, 80_000),
            charge("labour", 2, 60_000),
            charge("charge", 0, 10_000),
        ];
        let rest = unapproved(
            &charges,
            &[approved("labour", 1, 80_000), approved("labour", 2, 40_000)],
        );
        let amounts: Vec<(i64, i64)> = rest.iter().map(|(c, r)| (c.line_id, *r)).collect();
        assert_eq!(amounts, [(2, 20_000), (0, 10_000)]);
        assert!(
            unapproved(&charges[..1], &[approved("labour", 1, 80_000)]).is_empty(),
            "a charge approved in full leaves nothing for the customer"
        );
    }
}
