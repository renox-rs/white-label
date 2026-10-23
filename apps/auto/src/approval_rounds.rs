//! Opening and closing the estimate rounds inside the job card's transitions
//! (08 §4.4 step 4, `AUT.JOB.02`): `send_estimate` opens a round with the lines
//! that were drafted, and a job card that ends supersedes the round the
//! customer never answered.

use renox::db::{Transaction, now};
use renox::prelude::*;

use crate::approval::{self, ApprovalStatus, EstimateApproval, MIN_WARRANTY_DAYS};
use crate::job_card::JobCard;
use crate::job_card_machine::JobInput;
use crate::labour_line::{LabourLine, LineStatus};
use crate::part_line::PartLine;

/// Opens round `revision` of `card`: the lines drafted are sent with it, the
/// round carries their total, the promised day, the warranty (at least
/// [`MIN_WARRANTY_DAYS`]) and the token of the customer's link. A round the
/// customer has not answered is superseded.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn open(
    tx: &mut Transaction,
    card: &JobCard,
    revision: i64,
    input: &JobInput,
) -> Result<EstimateApproval> {
    supersede(tx, card.id).await?;
    let labour = LabourLine::query()
        .where_(LabourLine::JOB_CARD_ID.eq(card.id))
        .where_(LabourLine::APPROVAL.eq(LineStatus::Draft))
        .get(&mut *tx)
        .await?;
    let parts = PartLine::query()
        .where_(PartLine::JOB_CARD_ID.eq(card.id))
        .where_(PartLine::APPROVAL.eq(LineStatus::Draft))
        .get(&mut *tx)
        .await?;
    let net_total: i64 = labour.iter().map(LabourLine::price).sum::<i64>()
        + parts.iter().map(PartLine::price).sum::<i64>();
    let mut round = EstimateApproval {
        job_card_id: card.id,
        revision,
        token: approval::new_token(),
        net_total,
        promised_for: input.promised_for,
        warranty_days: input
            .warranty_days
            .unwrap_or(MIN_WARRANTY_DAYS)
            .max(MIN_WARRANTY_DAYS),
        status: ApprovalStatus::Pending,
        sent_by: input.by,
        sent_at: Some(now()),
        ..EstimateApproval::default()
    };
    round.insert(&mut *tx).await?;
    let stamp = now();
    LabourLine::query()
        .where_(LabourLine::JOB_CARD_ID.eq(card.id))
        .where_(LabourLine::APPROVAL.eq(LineStatus::Draft))
        .update(
            &mut *tx,
            &[
                ("approval", &LineStatus::Sent),
                ("approval_id", &Some(round.id)),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    PartLine::query()
        .where_(PartLine::JOB_CARD_ID.eq(card.id))
        .where_(PartLine::APPROVAL.eq(LineStatus::Draft))
        .update(
            &mut *tx,
            &[
                ("approval", &LineStatus::Sent),
                ("approval_id", &Some(round.id)),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    Ok(round)
}

/// Supersedes the round of job card `job_card_id` that the customer has not
/// answered; the lines it sent go back to drafts, so a later round sends them
/// again.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn supersede(tx: &mut Transaction, job_card_id: i64) -> Result {
    let pending = EstimateApproval::query()
        .where_(EstimateApproval::JOB_CARD_ID.eq(job_card_id))
        .where_(EstimateApproval::STATUS.eq(ApprovalStatus::Pending))
        .get(&mut *tx)
        .await?;
    let stamp = now();
    for round in pending {
        EstimateApproval::query()
            .where_(EstimateApproval::ID.eq(round.id))
            .update(
                &mut *tx,
                &[
                    ("status", &ApprovalStatus::Superseded),
                    ("updated_at", &stamp),
                ],
            )
            .await?;
        let no_round: Option<i64> = None;
        LabourLine::query()
            .where_(LabourLine::APPROVAL_ID.eq(Some(round.id)))
            .where_(LabourLine::APPROVAL.eq(LineStatus::Sent))
            .update(
                &mut *tx,
                &[
                    ("approval", &LineStatus::Draft),
                    ("approval_id", &no_round),
                    ("updated_at", &stamp),
                ],
            )
            .await?;
        PartLine::query()
            .where_(PartLine::APPROVAL_ID.eq(Some(round.id)))
            .where_(PartLine::APPROVAL.eq(LineStatus::Sent))
            .update(
                &mut *tx,
                &[
                    ("approval", &LineStatus::Draft),
                    ("approval_id", &no_round),
                    ("updated_at", &stamp),
                ],
            )
            .await?;
    }
    Ok(())
}
