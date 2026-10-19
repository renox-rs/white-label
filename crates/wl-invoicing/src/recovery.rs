//! A recovery (wl-invoicing §3, #66): a payment set against a written-off
//! invoice. The `recover` service writes one row per recovery, in the same
//! transaction as the allocation, so that a reversal can undo exactly it.
//! Amounts are fils of the invoice's currency; the VAT share is AED fils.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::allocation::Allocation;
use crate::vat_adjustment::AdjustmentStatus;

/// What one recovery did to the invoice and to its bad-debt relief.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_recoveries",
    unique(allocation_id),
    index(invoice_id),
    index(source_type, source_id)
)]
pub struct Recovery {
    /// The primary key.
    pub id: i64,
    /// The written-off invoice.
    pub invoice_id: i64,
    /// The type of the money's source, e.g. `payments.payment`.
    pub source_type: String,
    /// The id of the money's source.
    pub source_id: i64,
    /// The allocation the recovery wrote.
    pub allocation_id: i64,
    /// The amount recovered, in fils; always > 0.
    pub amount: i64,
    /// The total recovered after this one, in fils.
    pub recovered_total: i64,
    /// The VAT share of the amount, in AED fils.
    pub vat_share: i64,
    /// Whether the relief was claimed (`included`) at the time.
    pub relief_claimed: bool,
    /// The status of the bad-debt relief it saw; `None` without a relief.
    pub relief_status: Option<AdjustmentStatus>,
    /// The bad-debt relief adjustment it saw.
    pub relief_adjustment_id: Option<i64>,
    /// The `bad_debt_recovery` adjustment it created.
    pub recovery_adjustment_id: Option<i64>,
    /// The day it was recovered, in Asia/Dubai.
    pub recovered_on: NaiveDate,
    /// When the payment was reversed and the recovery undone (#70 A10);
    /// `None` while it stands. A reversed recovery is never deleted.
    pub reversed_at: Option<DateTime>,
    /// The idempotency key of the unallocation that reversed it.
    pub reversed_key: Option<String>,
    /// What the reversal did to the VAT side ([`ReversalEffect`]).
    pub reversed_effect: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Recovery {
    /// Where the money came from.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// Whether the recovery was reversed.
    pub fn is_reversed(&self) -> bool {
        self.reversed_at.is_some()
    }

    /// The recovery that a reversal of `amount` from `source` undoes on
    /// invoice `invoice_id`: the latest one from that source, for exactly
    /// that amount, not reversed yet, whose allocation is still active.
    /// `None` when there is none, so the money is not a recovery's.
    pub async fn matching(
        db: &Db,
        invoice_id: i64,
        source: &Subject,
        amount: i64,
    ) -> Result<Option<Recovery>> {
        let candidates = Recovery::query()
            .where_(Recovery::INVOICE_ID.eq(invoice_id))
            .where_(Recovery::SOURCE_TYPE.eq(source.subject_type.clone()))
            .where_(Recovery::SOURCE_ID.eq(source.subject_id))
            .where_(Recovery::AMOUNT.eq(amount))
            .where_(Recovery::REVERSED_AT.is_null())
            .order_by_desc(Recovery::ID)
            .get(db)
            .await?;
        for candidate in candidates {
            let allocation = Allocation::find(db, candidate.allocation_id).await?;
            if allocation.is_some_and(|row| row.removed_at.is_none()) {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }

    /// The recoveries of invoice `invoice_id`, oldest first.
    pub async fn of_invoice(db: &Db, invoice_id: i64) -> Result<Vec<Recovery>> {
        Recovery::query()
            .where_(Recovery::INVOICE_ID.eq(invoice_id))
            .order_by(Recovery::ID)
            .get(db)
            .await
    }
}

/// What a reversal does to the VAT side of its recovery (#70 A10), stored
/// as text in `reversed_effect` and shown in the accountant's notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReversalEffect {
    /// Nothing: no relief, or the share was 0.00, or the adjustment was
    /// excluded by a decision.
    None,
    /// A `pending` or `proposed` relief got the share back.
    ReliefRestored,
    /// A relief excluded as "recovered before claim" is `pending` again.
    ReliefReinstated,
    /// An open recovery adjustment was reduced by the share.
    AdjustmentReduced,
    /// An open recovery adjustment reached 0.00 and was excluded.
    AdjustmentExcluded,
    /// A new relief for the share, because the VAT was already included.
    ReliefCreated,
}

impl ReversalEffect {
    /// The text stored in `reversed_effect`.
    pub const fn as_str(self) -> &'static str {
        match self {
            ReversalEffect::None => "none",
            ReversalEffect::ReliefRestored => "relief_restored",
            ReversalEffect::ReliefReinstated => "relief_reinstated",
            ReversalEffect::AdjustmentReduced => "adjustment_reduced",
            ReversalEffect::AdjustmentExcluded => "adjustment_excluded",
            ReversalEffect::ReliefCreated => "relief_created",
        }
    }
}

/// An adjustment as the reversal sees it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    /// Its row id.
    pub id: i64,
    /// Its status now.
    pub status: AdjustmentStatus,
    /// Its amount now, in AED fils.
    pub amount: i64,
    /// For a relief: it was excluded by the system as "recovered before
    /// claim" (no decider), so a reversal can bring it back.
    pub recovered_before_claim: bool,
}

/// What to write on the VAT side when a recovery is reversed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReliefReversal {
    /// Nothing.
    Nothing,
    /// A `pending` or `proposed` relief: its amount becomes `amount`.
    Restore {
        /// The relief.
        id: i64,
        /// Its new amount.
        amount: i64,
    },
    /// An `excluded` ("recovered before claim") relief goes back to
    /// `pending` with `amount`.
    Reinstate {
        /// The relief.
        id: i64,
        /// Its restored amount.
        amount: i64,
    },
    /// An open recovery adjustment becomes `amount`; at 0 it is excluded.
    Reduce {
        /// The recovery adjustment.
        id: i64,
        /// Its new amount.
        amount: i64,
    },
    /// A new bad-debt relief for `share`, because the VAT was included.
    NewRelief {
        /// Its amount, in AED fils.
        share: i64,
    },
}

impl ReliefReversal {
    /// The effect to record for this plan.
    pub const fn effect(self) -> ReversalEffect {
        match self {
            ReliefReversal::Nothing => ReversalEffect::None,
            ReliefReversal::Restore { .. } => ReversalEffect::ReliefRestored,
            ReliefReversal::Reinstate { .. } => ReversalEffect::ReliefReinstated,
            ReliefReversal::Reduce { amount: 0, .. } => ReversalEffect::AdjustmentExcluded,
            ReliefReversal::Reduce { .. } => ReversalEffect::AdjustmentReduced,
            ReliefReversal::NewRelief { .. } => ReversalEffect::ReliefCreated,
        }
    }
}

/// Plans the VAT side of reversing `recovery`, from the adjustments as they
/// are now: `relief` is the invoice's own relief and `recovery_adjustment`
/// the one the recovery created (A10).
///
/// - a claimed recovery: its adjustment is reduced by the share while open
///   (0.00 excludes it), and a new relief is raised when it was already
///   `included`; an excluded one needs nothing;
/// - an unclaimed one: a `pending` or `proposed` relief gets the share
///   back, a relief excluded as "recovered before claim" is reinstated, and
///   an `included` one is covered by a new relief;
/// - no relief or a share of 0.00: nothing.
pub fn plan_reversal(
    recovery: &Recovery,
    relief: Option<Seen>,
    recovery_adjustment: Option<Seen>,
) -> ReliefReversal {
    let share = recovery.vat_share;
    if recovery.recovery_adjustment_id.is_some() {
        return match recovery_adjustment {
            Some(seen)
                if matches!(
                    seen.status,
                    AdjustmentStatus::Pending | AdjustmentStatus::Proposed
                ) =>
            {
                ReliefReversal::Reduce {
                    id: seen.id,
                    amount: (seen.amount - share).max(0),
                }
            }
            Some(seen) if seen.status == AdjustmentStatus::Included && share > 0 => {
                ReliefReversal::NewRelief { share }
            }
            _ => ReliefReversal::Nothing,
        };
    }
    let Some(seen) = relief else {
        return ReliefReversal::Nothing;
    };
    if recovery.relief_adjustment_id != Some(seen.id) || recovery.relief_claimed {
        return ReliefReversal::Nothing;
    }
    match seen.status {
        AdjustmentStatus::Pending | AdjustmentStatus::Proposed if share > 0 => {
            ReliefReversal::Restore {
                id: seen.id,
                amount: seen.amount + share,
            }
        }
        AdjustmentStatus::Excluded if seen.recovered_before_claim => ReliefReversal::Reinstate {
            id: seen.id,
            amount: seen.amount + share,
        },
        AdjustmentStatus::Included if share > 0 => ReliefReversal::NewRelief { share },
        _ => ReliefReversal::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recovery_keeps_its_source() {
        let recovery = Recovery {
            source_type: "payments.payment".into(),
            source_id: 7,
            ..Default::default()
        };
        assert_eq!(recovery.source(), Subject::new("payments.payment", 7));
    }

    fn seen(id: i64, status: AdjustmentStatus, amount: i64, rbc: bool) -> Seen {
        Seen {
            id,
            status,
            amount,
            recovered_before_claim: rbc,
        }
    }

    fn recovered(share: i64, claimed: bool) -> Recovery {
        Recovery {
            id: 9,
            vat_share: share,
            relief_claimed: claimed,
            relief_adjustment_id: Some(1),
            recovery_adjustment_id: claimed.then_some(2),
            ..Default::default()
        }
    }

    #[test]
    fn an_open_relief_gets_the_share_back() {
        use AdjustmentStatus::{Pending, Proposed};
        for status in [Pending, Proposed] {
            assert_eq!(
                plan_reversal(
                    &recovered(2_500, false),
                    Some(seen(1, status, 2_500, false)),
                    None
                ),
                ReliefReversal::Restore {
                    id: 1,
                    amount: 5_000
                }
            );
        }
        assert_eq!(
            plan_reversal(
                &recovered(0, false),
                Some(seen(1, Pending, 5_000, false)),
                None
            ),
            ReliefReversal::Nothing
        );
    }

    #[test]
    fn a_relief_excluded_before_the_claim_is_reinstated() {
        let excluded = seen(1, AdjustmentStatus::Excluded, 0, true);
        assert_eq!(
            plan_reversal(&recovered(5_000, false), Some(excluded), None),
            ReliefReversal::Reinstate {
                id: 1,
                amount: 5_000
            }
        );
        // Excluded by a person: left alone.
        let decided = seen(1, AdjustmentStatus::Excluded, 0, false);
        assert_eq!(
            plan_reversal(&recovered(5_000, false), Some(decided), None),
            ReliefReversal::Nothing
        );
    }

    #[test]
    fn an_open_recovery_adjustment_is_reduced_and_excluded_at_zero() {
        let open = seen(2, AdjustmentStatus::Pending, 2_500, false);
        assert_eq!(
            plan_reversal(&recovered(2_500, true), None, Some(open)),
            ReliefReversal::Reduce { id: 2, amount: 0 }
        );
        assert_eq!(
            ReliefReversal::Reduce { id: 2, amount: 0 }.effect(),
            ReversalEffect::AdjustmentExcluded
        );
        let more = seen(2, AdjustmentStatus::Proposed, 4_000, false);
        let plan = plan_reversal(&recovered(2_500, true), None, Some(more));
        assert_eq!(
            plan,
            ReliefReversal::Reduce {
                id: 2,
                amount: 1_500
            }
        );
        assert_eq!(plan.effect(), ReversalEffect::AdjustmentReduced);
    }

    #[test]
    fn an_included_adjustment_is_answered_by_a_new_relief() {
        let included = seen(2, AdjustmentStatus::Included, 2_500, false);
        let plan = plan_reversal(&recovered(2_500, true), None, Some(included));
        assert_eq!(plan, ReliefReversal::NewRelief { share: 2_500 });
        assert_eq!(plan.effect().as_str(), "relief_created");
        let excluded = seen(2, AdjustmentStatus::Excluded, 2_500, false);
        assert_eq!(
            plan_reversal(&recovered(2_500, true), None, Some(excluded)),
            ReliefReversal::Nothing
        );
        // An unclaimed relief that was included since: also a new relief.
        let relief = seen(1, AdjustmentStatus::Included, 2_500, false);
        assert_eq!(
            plan_reversal(&recovered(2_500, false), Some(relief), None),
            ReliefReversal::NewRelief { share: 2_500 }
        );
        assert_eq!(
            plan_reversal(&recovered(2_500, false), None, None),
            ReliefReversal::Nothing
        );
    }
}
