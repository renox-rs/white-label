//! Adjustment lines on a COD remittance (03 Logistics §4.1 *Remittance wrong
//! after payment*, §6 *COD remittance*, `LOG.ACL.12`; BL-LOG-15).
//!
//! A paid remittance is never edited. A correction is a signed line with a
//! reason that goes on the client's next draft: one user prepares it, another
//! with `remittances.approve` approves it. A line is *open* until the
//! remittance carrying it is `paid`; one with no draft yet
//! (`remittance_id` none) waits for the next weekly run of `LOG.RUL.06`.
//! Only an approved line counts in a remittance's net; a rejected line stays
//! unapproved and is not counted.

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use crate::permissions;
use crate::texts::english;

/// `approve`: pending → approved.
pub const APPROVE: &str = "approve";
/// `reject`: pending → rejected.
pub const REJECT: &str = "reject";
/// `resubmit`: rejected → pending, after the preparer edited it.
pub const RESUBMIT: &str = "resubmit";

/// Where a line is in its approval.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentStatus {
    /// Prepared, waiting for a second user.
    #[default]
    Pending,
    /// Approved by a user other than its preparer; it counts.
    Approved,
    /// Not approved; it does not count. The preparer edits and resubmits it,
    /// or deletes it.
    Rejected,
}

/// Where an adjustment line comes from.
pub mod source {
    /// Entered by an accountant.
    pub const MANUAL: &str = "manual";
    /// A delivery reversal whose COD was already remitted (BL-LOG-10).
    pub const DELIVERY_REVERSAL: &str = "delivery_reversal";
    /// The outcome of a statement dispute (BL-LOG-18).
    pub const STATEMENT_DISPUTE: &str = "statement_dispute";
    /// A COD difference the client agreed to absorb after the statement
    /// (BL-LOG-08).
    pub const COD_DIFFERENCE: &str = "cod_difference";
    /// The rest of a negative net, carried to the next draft (BL-LOG-14).
    pub const CARRIED_FORWARD: &str = "carried_forward";
    /// Every source.
    pub const ALL: [&str; 5] = [
        MANUAL,
        DELIVERY_REVERSAL,
        STATEMENT_DISPUTE,
        COD_DIFFERENCE,
        CARRIED_FORWARD,
    ];
}

/// A signed correction on a client's remittance.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "logistics_adjustment_lines",
    state_machine,
    index(client_party_id),
    index(remittance_id)
)]
pub struct AdjustmentLine {
    /// The primary key.
    pub id: i64,
    /// The client.
    pub client_party_id: i64,
    /// The draft carrying it; none while it waits for the next weekly run.
    pub remittance_id: Option<i64>,
    /// Positive: owed to the client. Negative: taken back. In fils.
    pub amount: i64,
    /// Why; the client sees it on the statement.
    pub reason: String,
    /// One of [`source`].
    pub source: String,
    /// The shipment it corrects.
    pub shipment_id: Option<i64>,
    /// The remittance it corrects.
    pub corrects_remittance_id: Option<i64>,
    /// The remittance whose negative net this line carries forward.
    pub carried_from_id: Option<i64>,
    /// The user who prepared it; none for a line the system derived.
    pub prepared_by: Option<i64>,
    /// The user who approved or rejected it.
    pub decided_by: Option<i64>,
    /// When.
    pub decided_at: Option<DateTime>,
    /// The approver's note.
    pub decision_note: Option<String>,
    /// The approval state; changed only by a transition.
    pub status: AdjustmentStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AdjustmentLine {
    /// The client.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// Whether the line is approved and so counts.
    pub fn counts(&self) -> bool {
        self.status == AdjustmentStatus::Approved
    }
}

/// The lines of `client` that wait for a draft: attached to none.
///
/// # Errors
///
/// Database errors.
pub async fn unattached(db: &Db, client: PartyId) -> Result<Vec<AdjustmentLine>> {
    AdjustmentLine::query()
        .where_(AdjustmentLine::CLIENT_PARTY_ID.eq(client.0))
        .where_(AdjustmentLine::REMITTANCE_ID.is_null())
        .order_by(AdjustmentLine::ID)
        .get(db)
        .await
}

/// `approve` (`LOG.ACL.12`): another user with `remittances.approve` than the
/// one who prepared the line.
fn approve() -> Transition<AdjustmentLine> {
    Transition::new(APPROVE)
        .label("Approve")
        .tone("primary")
        .permission(permissions::REMITTANCES_APPROVE)
        .from([AdjustmentStatus::Pending])
        .to(AdjustmentStatus::Approved)
        .guard("not_the_preparer", |line: &AdjustmentLine, ctx| {
            let same = ctx
                .actor()
                .is_some_and(|user| line.prepared_by == Some(user.id));
            Box::pin(async move {
                if same {
                    Err(english("logistics.adjustment.own_line", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|line: &AdjustmentLine, ctx, tx| {
            let id = line.id;
            let actor = ctx.actor().map(|user| user.id);
            let note = ctx.reason().map(str::to_owned);
            Box::pin(async move { decided(&mut *tx, id, actor, note).await })
        })
}

/// `reject`: the line stays unapproved and is not counted.
fn reject() -> Transition<AdjustmentLine> {
    Transition::new(REJECT)
        .label("Reject")
        .tone("danger")
        .permission(permissions::REMITTANCES_APPROVE)
        .from([AdjustmentStatus::Pending])
        .to(AdjustmentStatus::Rejected)
        .guard("not_the_preparer", |line: &AdjustmentLine, ctx| {
            let same = ctx
                .actor()
                .is_some_and(|user| line.prepared_by == Some(user.id));
            Box::pin(async move {
                if same {
                    Err(english("logistics.adjustment.own_line", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|line: &AdjustmentLine, ctx, tx| {
            let id = line.id;
            let actor = ctx.actor().map(|user| user.id);
            let note = ctx.reason().map(str::to_owned);
            Box::pin(async move { decided(&mut *tx, id, actor, note).await })
        })
}

/// `resubmit`: the preparer sends an edited, rejected line for a new
/// decision.
fn resubmit() -> Transition<AdjustmentLine> {
    Transition::new(RESUBMIT)
        .label("Resubmit")
        .tone("primary")
        .permission(permissions::REMITTANCES_PREPARE)
        .from([AdjustmentStatus::Rejected])
        .to(AdjustmentStatus::Pending)
        .guard("the_preparer", |line: &AdjustmentLine, ctx| {
            let other = ctx
                .actor()
                .is_some_and(|user| line.prepared_by != Some(user.id));
            Box::pin(async move {
                if other {
                    Err(english("logistics.adjustment.not_your_line", &[]))
                } else {
                    Ok(())
                }
            })
        })
}

/// Records who decided the line, when and with which note.
async fn decided(
    tx: &mut renox::db::Transaction,
    id: i64,
    actor: Option<i64>,
    note: Option<String>,
) -> Result {
    let at = Some(renox::db::now());
    AdjustmentLine::query()
        .where_(AdjustmentLine::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("decided_by", &actor),
                ("decided_at", &at),
                ("decision_note", &note),
            ],
        )
        .await?;
    Ok(())
}

impl StateMachine for AdjustmentLine {
    type State = AdjustmentStatus;
    const COLUMN: renox::db::Col<AdjustmentLine, AdjustmentStatus> = AdjustmentLine::STATUS;

    fn machine() -> Machine<Self> {
        use AdjustmentStatus::{Approved, Pending, Rejected};
        Machine::new()
            .initial([Pending])
            .state(Pending)
            .tone("neutral")
            .state(Approved)
            .tone("success")
            .state(Rejected)
            .tone("error")
            .transition(approve())
            .transition(reject())
            .transition(resubmit())
    }
}
