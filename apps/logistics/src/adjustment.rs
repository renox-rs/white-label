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
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use crate::permissions;
use crate::remittance::{Remittance, RemittanceStatus};
use crate::shipment::Shipment;
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
            let note = ctx
                .reason()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned);
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
            let note = ctx
                .reason()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned);
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

// ---------------------------------------------------------------------------
// The services (BL-LOG-15)

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

async fn load(state: &AppState, id: i64) -> std::result::Result<AdjustmentLine, TransitionError> {
    AdjustmentLine::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// Dirhams with a sign as fils: `70`, `-40`, `+12.5` are 7000, -4000 and 1250.
pub fn parse_signed_dirhams(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    if digits.trim().is_empty() {
        return None;
    }
    let fils = crate::booking::parse_dirhams(digits)?;
    Some(if negative { -fils } else { fils })
}

/// What a user enters for an adjustment line.
#[derive(Debug, Clone, Default)]
pub struct NewAdjustment {
    /// The client.
    pub client: PartyId,
    /// Signed, in fils; not 0.
    pub amount: i64,
    /// Why; the client sees it.
    pub reason: String,
    /// One of [`source`]; blank is [`source::MANUAL`].
    pub source: String,
    /// The shipment it corrects.
    pub shipment_id: Option<i64>,
    /// The remittance it corrects (the paid one that was wrong).
    pub corrects_remittance_id: Option<i64>,
    /// A draft to put it on now; none: the client's draft if it has one,
    /// else it waits for the next weekly run.
    pub remittance_id: Option<i64>,
}

/// `user` (`remittances.prepare`) prepares an adjustment line for a client.
/// It is pending until another user approves it. It goes on `remittance_id`
/// when that is a draft of the client, else on the client's draft if there is
/// one, else it is left open for the next weekly run (`LOG.RUL.06`). A paid
/// or approved remittance is never edited: naming one as the target is
/// refused (an approved one is reopened first).
///
/// # Errors
///
/// `Forbidden` without `remittances.prepare`; `Guard` (422) for an amount of
/// 0, a blank reason, an unknown source, shipment or remittance, and a target
/// that is not a draft of the client.
pub async fn add(
    state: &AppState,
    user: &AuthUser,
    new: &NewAdjustment,
) -> std::result::Result<AdjustmentLine, TransitionError> {
    if !user.has_permission(permissions::REMITTANCES_PREPARE) {
        return Err(TransitionError::Forbidden);
    }
    if new.amount == 0 {
        return Err(guard(english("logistics.adjustment.amount_needed", &[])));
    }
    let reason = new.reason.trim();
    if reason.is_empty() {
        return Err(guard(english("logistics.adjustment.reason_needed", &[])));
    }
    let source: &str = match new.source.trim() {
        "" => source::MANUAL,
        known if known != source::CARRIED_FORWARD && source::ALL.contains(&known) => known,
        _ => return Err(guard(english("logistics.adjustment.source_unknown", &[]))),
    };
    if let Some(id) = new.shipment_id {
        let shipment = Shipment::find(&state.db, id).await?;
        if shipment.is_none_or(|s| s.client_party_id != new.client.0) {
            return Err(guard(english("logistics.adjustment.shipment_unknown", &[])));
        }
    }
    if let Some(id) = new.corrects_remittance_id {
        let corrected = Remittance::find(&state.db, id).await?;
        if corrected.is_none_or(|r| r.client_party_id != new.client.0) {
            return Err(guard(english("logistics.adjustment.remittance_unknown", &[])));
        }
    }
    let target = match new.remittance_id {
        Some(id) => {
            let remittance = Remittance::find(&state.db, id)
                .await?
                .filter(|r| r.client_party_id == new.client.0)
                .ok_or_else(|| guard(english("logistics.adjustment.remittance_unknown", &[])))?;
            match remittance.status {
                RemittanceStatus::Draft => Some(remittance.id),
                RemittanceStatus::Approved => {
                    return Err(guard(english("logistics.adjustment.reopen_first", &[])));
                }
                _ => {
                    return Err(guard(english(
                        "logistics.adjustment.paid_not_edited",
                        &[("number", &remittance.number)],
                    )));
                }
            }
        }
        None => Remittance::draft_of(&state.db, new.client)
            .await?
            .map(|draft| draft.id),
    };
    let mut line = AdjustmentLine {
        client_party_id: new.client.0,
        remittance_id: target,
        amount: new.amount,
        reason: reason.to_owned(),
        source: source.to_owned(),
        shipment_id: new.shipment_id,
        corrects_remittance_id: new.corrects_remittance_id,
        prepared_by: Some(user.id),
        status: AdjustmentStatus::Pending,
        ..AdjustmentLine::default()
    };
    line.insert(&state.db).await?;
    // A pending line is not counted; the figures of the draft stay.
    Ok(line)
}

/// `user` (`remittances.approve`, not the preparer) approves line `id`; the
/// draft carrying it counts it from now.
///
/// # Errors
///
/// `Forbidden` without the permission; `NotFrom` unless it is pending;
/// `Guard` (422) for the preparer.
pub async fn approve_line(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    note: &str,
) -> std::result::Result<AdjustmentLine, TransitionError> {
    decide(state, user, id, APPROVE, note).await
}

/// `user` (`remittances.approve`, not the preparer) rejects line `id`: it
/// stays unapproved and is not counted.
///
/// # Errors
///
/// As [`approve_line`].
pub async fn reject_line(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    note: &str,
) -> std::result::Result<AdjustmentLine, TransitionError> {
    decide(state, user, id, REJECT, note).await
}

async fn decide(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    transition: &str,
    note: &str,
) -> std::result::Result<AdjustmentLine, TransitionError> {
    let line = load(state, id).await?;
    let _outcome = line
        .transition(transition)
        .by(user)
        .reason(note.trim())
        .run(state)
        .await?;
    let line = load(state, id).await?;
    if let Some(remittance) = line.remittance_id {
        crate::remit::refresh_totals(state, remittance).await?;
    }
    Ok(line)
}

/// The preparer edits a rejected line (`amount` in fils, `reason`) and sends
/// it for a new decision. An approved line is never edited: the transition
/// refuses it, and a wrong line is corrected by another line.
///
/// # Errors
///
/// `Forbidden` without `remittances.prepare`; `NotFrom` unless the line is
/// rejected; `Guard` (422) for another user than the preparer and for an
/// amount of 0 or a blank reason.
pub async fn resubmit_line(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    amount: i64,
    reason: &str,
) -> std::result::Result<AdjustmentLine, TransitionError> {
    let line = load(state, id).await?;
    let editable = line.status == AdjustmentStatus::Rejected
        && line.prepared_by == Some(user.id)
        && user.has_permission(permissions::REMITTANCES_PREPARE);
    if editable {
        if amount == 0 {
            return Err(guard(english("logistics.adjustment.amount_needed", &[])));
        }
        if reason.trim().is_empty() {
            return Err(guard(english("logistics.adjustment.reason_needed", &[])));
        }
        let reason = reason.trim().to_owned();
        AdjustmentLine::query()
            .where_(AdjustmentLine::ID.eq(id))
            .update(&state.db, &[("amount", &amount), ("reason", &reason)])
            .await?;
    }
    // Not editable: the transition says why (the state, the permission, the
    // preparer).
    let line = load(state, id).await?;
    let _outcome = line.transition(RESUBMIT).by(user).run(state).await?;
    load(state, id).await
}

/// The preparer deletes line `id` while it is unapproved (pending or
/// rejected); an approved line stays.
///
/// # Errors
///
/// `Forbidden` without `remittances.prepare`; `Guard` (422) for another user
/// than the preparer and for an approved line.
pub async fn delete_line(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<(), TransitionError> {
    if !user.has_permission(permissions::REMITTANCES_PREPARE) {
        return Err(TransitionError::Forbidden);
    }
    let line = load(state, id).await?;
    if line.prepared_by != Some(user.id) {
        return Err(guard(english("logistics.adjustment.not_your_line", &[])));
    }
    if line.status == AdjustmentStatus::Approved {
        return Err(guard(english("logistics.adjustment.approved_kept", &[])));
    }
    line.delete(&state.db).await?;
    if let Some(remittance) = line.remittance_id {
        crate::remit::refresh_totals(state, remittance).await?;
    }
    Ok(())
}

/// One adjustment line as the client reads it on its statement: the date,
/// the amount, the reason and what it corrects. The preparer, the approver
/// and the approver's note are internal and not here.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StatementLine {
    /// The day it was approved (Asia/Dubai).
    pub date: String,
    /// Signed, in fils.
    pub amount: i64,
    /// Why.
    pub reason: String,
    /// The shipment or remittance it corrects, by number; blank for none.
    pub corrects: String,
}

/// The adjustment lines on `remittance` that count, as its statement shows
/// them (`LOG.ACL.12`: the client sees each line and its reason on the next
/// statement). A line that is pending or rejected is not shown. The portal
/// screen and the PDF are POR.SLF.01 and DOC.GEN.01.
///
/// # Errors
///
/// Database errors.
pub async fn statement_lines(db: &Db, remittance: &Remittance) -> Result<Vec<StatementLine>> {
    let mut out = Vec::new();
    for line in remittance.adjustments(db).await? {
        if !line.counts() {
            continue;
        }
        let mut corrects = String::new();
        if let Some(id) = line.shipment_id
            && let Some(shipment) = Shipment::find(db, id).await?
        {
            corrects = shipment.number;
        } else if let Some(id) = line.corrects_remittance_id
            && let Some(corrected) = Remittance::find(db, id).await?
        {
            corrects = corrected.number;
        }
        out.push(StatementLine {
            date: line
                .decided_at
                .map(|at| wl_kernel::documents::dubai_date(at).to_string())
                .unwrap_or_default(),
            amount: line.amount,
            reason: line.reason.clone(),
            corrects,
        });
    }
    Ok(out)
}
