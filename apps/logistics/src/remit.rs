//! Building the draft COD remittance and the actions on it (03 Logistics
//! §4.1 step 11, §7.1 *COD remittance*, §7.2 `LOG.RUL.06`, `LOG.ACL.07`;
//! BL-LOG-14).
//!
//! - **The weekly rule** ([`run_weekly`], scheduled on Monday): for every
//!   client with COD collected in the period, or with open adjustment lines,
//!   one draft is created or the client's existing draft is rebuilt
//!   ([`draft_for`]). The accountants are told.
//! - **Which shipments.** Every shipment of the client that is `delivered`
//!   with COD recorded, delivered before the period end, on no remittance
//!   yet, not marked already remitted and not frozen by the delivery module
//!   (under review, or lost after a review: `DeliveryRequester::is_frozen`).
//!   The held shipment's COD is not left out: a held shipment is not
//!   delivered. The selection is cumulative, so a shipment released from
//!   review is on the next draft. The cash handovers are never read: the
//!   amount owed is the COD recorded, whatever happened to the cash after
//!   collection (`DeliveryRequester::remittable_cod` is the cash collected,
//!   which a driver's shortage lowers, so it is not the amount).
//! - **The lines.** COD less the difference the client absorbed, less the
//!   client's COD fee (basis points of that amount, rounded to the fil).
//!   The net is the sum of the rounded lines plus the approved adjustment
//!   lines; a negative net is set to 0 and the rest is carried as an open,
//!   approved adjustment line to the next draft.
//! - **Rebuild.** A draft is rebuilt, never duplicated: its shipment lines
//!   are recomputed, the open adjustment lines of the client are attached,
//!   and the lines already attached stay with their approval.
//! - **Actions** ([`approve`], [`reopen`], [`pay`]): `approve` needs a user
//!   other than the preparer and every adjustment line approved; `pay` records
//!   the payout through the kernel's payments contract (an outgoing transfer
//!   to the client) and then runs the transition; a refused transition
//!   reverses the payout.

use std::collections::BTreeSet;

use renox::chrono::{NaiveDate, TimeDelta, Weekday};
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::LegalEntity;
use wl_kernel::PartyId;
use wl_kernel::delivery::DeliveryId;
use wl_kernel::documents::{DUBAI_OFFSET_SECONDS, today};
use wl_kernel::parties::{self, PartyStatus};
use wl_kernel::payments::{self, PaymentMethod, RecordPayment, ReversalReason};

use crate::adjustment::{AdjustmentLine, AdjustmentStatus, source};
use crate::notices::tell_accountants;
use crate::permissions;
use crate::remittance::{REMITTANCE_SERIES, Remittance, RemittanceLine, RemittanceStatus};
use crate::remittance_machine::{APPROVE, PAY, REOPEN, RemittanceInput};
use crate::reweigh::{self, ClientTerms};
use crate::shipment::{Shipment, ShipmentStatus};
use crate::texts::english;

/// The scheduled task of `LOG.RUL.06`.
pub const TASK: &str = "logistics-cod-remittances";

/// Day D of the rule: Monday (§7.2).
pub const DAY: Weekday = Weekday::Mon;

/// The time the rule runs on day D, Asia/Dubai.
pub const TASK_AT: &str = "06:00";

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// The end of the period of a run on `day`: the start of that day in
/// Asia/Dubai, so everything delivered up to the Sunday 23:59 is in.
pub fn period_end_of(day: NaiveDate) -> DateTime {
    let midnight = day.and_hms_opt(0, 0, 0).expect("midnight exists");
    (midnight - TimeDelta::seconds(i64::from(DUBAI_OFFSET_SECONDS))).and_utc()
}

/// What one shipment brings to a remittance: the COD recorded, the
/// difference the client absorbed with its reason, the fee, and the amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineFigures {
    /// The COD recorded on the shipment.
    pub cod_amount: i64,
    /// The difference the client absorbed.
    pub absorbed: i64,
    /// Its reason.
    pub absorbed_reason: String,
    /// The COD fee on what is paid out.
    pub fee: i64,
    /// COD less absorbed less fee.
    pub amount: i64,
}

/// The figures of `shipment` for a client whose COD fee is `fee_bp` basis
/// points. Only a difference that the *client* absorbs is deducted (never a
/// driver's, the business's or the customer's), and never more than the COD.
pub fn figures(shipment: &Shipment, fee_bp: i64) -> LineFigures {
    let cod_amount = shipment.cod_amount.max(0);
    let by_client = shipment.cod_absorbed_by.as_deref() == Some("client");
    let absorbed = if by_client {
        shipment.cod_difference.clamp(0, cod_amount)
    } else {
        0
    };
    let paid_out = cod_amount - absorbed;
    let fee = (paid_out * fee_bp.max(0) + 5_000) / 10_000;
    LineFigures {
        cod_amount,
        absorbed,
        absorbed_reason: if absorbed > 0 {
            shipment.cod_difference_reason.clone().unwrap_or_default()
        } else {
            String::new()
        },
        fee,
        amount: paid_out - fee,
    }
}

/// The net to pay and what is carried forward, from the lines' sum and the
/// approved adjustments: never below 0.
pub fn net_and_carried(lines_amount: i64, adjustments: i64) -> (i64, i64) {
    let raw = lines_amount + adjustments;
    if raw < 0 { (0, raw) } else { (raw, 0) }
}

/// The shipments the draft of `client` takes now: see the module docs.
/// `draft` is the client's existing draft, whose own lines do not count as
/// "already on a remittance".
async fn candidates(
    state: &AppState,
    client: PartyId,
    period_end: DateTime,
    draft: Option<i64>,
) -> Result<Vec<Shipment>> {
    let delivered = Shipment::query()
        .where_(Shipment::CLIENT_PARTY_ID.eq(client.0))
        .where_(Shipment::STATUS.eq(ShipmentStatus::Delivered))
        .where_(Shipment::COD_ALREADY_REMITTED.eq(false))
        .order_by(Shipment::ID)
        .get(&state.db)
        .await?;
    let deliveries = wl_kernel::delivery::requester(state);
    let mut taken = Vec::new();
    for shipment in delivered {
        if shipment.cod_amount <= 0 {
            continue;
        }
        if !shipment.delivered_at.is_some_and(|at| at < period_end) {
            continue;
        }
        let line = RemittanceLine::query()
            .where_(RemittanceLine::SHIPMENT_ID.eq(shipment.id))
            .first(&state.db)
            .await?;
        if line.is_some_and(|line| Some(line.remittance_id) != draft) {
            continue;
        }
        if let Some(delivery) = shipment.delivery_id
            && deliveries.is_frozen(state, DeliveryId(delivery)).await?
        {
            continue;
        }
        taken.push(shipment);
    }
    Ok(taken)
}

/// What a build did, for the notice and the tests.
#[derive(Debug, Clone)]
pub struct Built {
    /// The draft, as stored.
    pub remittance: Remittance,
    /// Whether it was created now (else rebuilt).
    pub created: bool,
    /// Whether its figures changed.
    pub changed: bool,
}

/// Creates the draft COD remittance of `client` for the period ending at
/// `period_end`, or rebuilds the client's existing draft; `by` is the user
/// who asked (none for the weekly rule). Nothing is built, and `None`
/// answered, for a client with no shipment to take, no open adjustment line
/// and no draft.
///
/// # Errors
///
/// Database errors and the delivery module's.
pub async fn draft_for(
    state: &AppState,
    client: PartyId,
    period_end: DateTime,
    by: Option<i64>,
) -> Result<Option<Built>> {
    let existing = Remittance::draft_of(&state.db, client).await?;
    let existing_id = existing.as_ref().map(|r| r.id);
    let shipments = candidates(state, client, period_end, existing_id).await?;
    let waiting: Vec<AdjustmentLine> = crate::adjustment::unattached(&state.db, client)
        .await?
        .into_iter()
        .filter(|line| line.status != AdjustmentStatus::Rejected)
        .filter(|line| line.carried_from_id.is_none() || line.carried_from_id != existing_id)
        .collect();
    if existing.is_none() && shipments.is_empty() && waiting.is_empty() {
        return Ok(None);
    }
    let terms = reweigh::terms(&state.db, client).await?;
    let number = match &existing {
        Some(_) => String::new(),
        None => {
            REMITTANCE_SERIES
                .next_allowing_gaps(&state.db, None, None, today())
                .await?
        }
    };
    let mut tx = state.db.begin().await?;
    let (mut remittance, created) = match existing {
        Some(found) => (found, false),
        None => {
            let mut fresh = Remittance {
                number,
                client_party_id: client.0,
                period_end,
                prepared_by: by,
                status: RemittanceStatus::Draft,
                ..Remittance::default()
            };
            fresh.insert(&mut tx).await?;
            (fresh, true)
        }
    };
    let before = (
        remittance.cod_total,
        remittance.fee_total,
        remittance.adjustment_total,
        remittance.net,
    );
    // Recompute the shipment lines; drop the carry of the earlier build.
    RemittanceLine::query()
        .where_(RemittanceLine::REMITTANCE_ID.eq(remittance.id))
        .delete(&mut tx)
        .await?;
    AdjustmentLine::query()
        .where_(AdjustmentLine::CARRIED_FROM_ID.eq(remittance.id))
        .where_(AdjustmentLine::REMITTANCE_ID.is_null())
        .delete(&mut tx)
        .await?;
    let mut cod_total = 0;
    let mut fee_total = 0;
    let mut lines_amount = 0;
    for shipment in &shipments {
        let figures = figures(shipment, terms.cod_fee_bp);
        let mut line = RemittanceLine {
            remittance_id: remittance.id,
            shipment_id: shipment.id,
            cod_amount: figures.cod_amount,
            absorbed: figures.absorbed,
            absorbed_reason: figures.absorbed_reason.clone(),
            fee: figures.fee,
            amount: figures.amount,
            ..RemittanceLine::default()
        };
        line.insert(&mut tx).await?;
        cod_total += figures.cod_amount - figures.absorbed;
        fee_total += figures.fee;
        lines_amount += figures.amount;
    }
    // Attach what waited for a draft; lines already attached stay as they
    // are, with their approval.
    let draft_id = Some(remittance.id);
    for line in &waiting {
        AdjustmentLine::query()
            .where_(AdjustmentLine::ID.eq(line.id))
            .update(&mut tx, &[("remittance_id", &draft_id)])
            .await?;
    }
    let attached = AdjustmentLine::query()
        .where_(AdjustmentLine::REMITTANCE_ID.eq(remittance.id))
        .get(&mut tx)
        .await?;
    let adjustment_total: i64 = attached
        .iter()
        .filter(|line| line.counts())
        .map(|line| line.amount)
        .sum();
    let (net, carried) = net_and_carried(lines_amount, adjustment_total);
    if carried < 0 {
        let mut line = AdjustmentLine {
            client_party_id: client.0,
            amount: carried,
            reason: english("logistics.remittance.carried_reason", &[("number", &remittance.number)]),
            source: source::CARRIED_FORWARD.to_owned(),
            carried_from_id: Some(remittance.id),
            decided_at: Some(renox::db::now()),
            decision_note: Some(english("logistics.remittance.carried_note", &[])),
            // Derived from lines that were approved: the rest of them.
            status: AdjustmentStatus::Approved,
            ..AdjustmentLine::default()
        };
        line.insert(&mut tx).await?;
    }
    let carried_forward = -carried;
    // A user who builds it becomes its preparer; the weekly rule keeps it.
    let prepared_by = by.or(remittance.prepared_by);
    Remittance::query()
        .where_(Remittance::ID.eq(remittance.id))
        .update(
            &mut tx,
            &[
                ("cod_total", &cod_total),
                ("fee_total", &fee_total),
                ("adjustment_total", &adjustment_total),
                ("net", &net),
                ("carried_forward", &carried_forward),
                ("period_end", &period_end),
                ("prepared_by", &prepared_by),
            ],
        )
        .await?;
    tx.commit().await?;
    remittance = Remittance::find_or_404(&state.db, remittance.id).await?;
    let changed = created || before != (cod_total, fee_total, adjustment_total, net);
    if changed {
        tell_accountants(state, &remittance, created).await?;
    }
    Ok(Some(Built {
        remittance,
        created,
        changed,
    }))
}

/// The clients the weekly rule looks at: with shipments delivered and
/// remittable, with open adjustment lines, or with a draft to rebuild.
async fn clients_to_build(state: &AppState) -> Result<BTreeSet<i64>> {
    let mut clients = BTreeSet::new();
    let delivered = Shipment::query()
        .where_(Shipment::STATUS.eq(ShipmentStatus::Delivered))
        .where_(Shipment::COD_ALREADY_REMITTED.eq(false))
        .where_(Shipment::COD_AMOUNT.gt(0))
        .get(&state.db)
        .await?;
    clients.extend(delivered.iter().map(|s| s.client_party_id));
    let lines = AdjustmentLine::query()
        .where_(AdjustmentLine::REMITTANCE_ID.is_null())
        .get(&state.db)
        .await?;
    clients.extend(lines.iter().map(|l| l.client_party_id));
    let drafts = Remittance::query()
        .where_(Remittance::STATUS.eq(RemittanceStatus::Draft))
        .get(&state.db)
        .await?;
    clients.extend(drafts.iter().map(|r| r.client_party_id));
    Ok(clients)
}

/// `LOG.RUL.06`: builds or rebuilds the draft remittance of every client
/// that has something for the period ending at `period_end`. Running it again
/// leaves one draft per client.
///
/// # Errors
///
/// Database errors and the delivery module's.
pub async fn run_weekly(state: &AppState, period_end: DateTime) -> Result<Vec<Built>> {
    let mut built = Vec::new();
    for client in clients_to_build(state).await? {
        if let Some(one) = draft_for(state, PartyId(client), period_end, None).await? {
            built.push(one);
        }
    }
    Ok(built)
}

/// Sets the COD terms of `client`: the fee in basis points of the COD paid
/// out (a stand-in for the rate card, `FIN.RAT.01`) and the bank account the
/// remittances go to. The control of a later change of the account is
/// BL-LOG-17 (`LOG.ACL.13`).
///
/// # Errors
///
/// Database errors.
pub async fn set_cod_terms(
    db: &Db,
    client: PartyId,
    fee_bp: i64,
    bank_account: Option<&str>,
) -> Result {
    let account = bank_account
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let found = ClientTerms::query()
        .where_(ClientTerms::CLIENT_PARTY_ID.eq(client.0))
        .first(db)
        .await?;
    match found {
        Some(mut terms) => {
            terms.cod_fee_bp = fee_bp;
            terms.bank_account = account;
            terms.save(db).await?;
        }
        None => {
            let mut terms = ClientTerms {
                client_party_id: client.0,
                cod_fee_bp: fee_bp,
                bank_account: account,
                ..ClientTerms::default()
            };
            terms.insert(db).await?;
        }
    }
    Ok(())
}

/// Registers the weekly rule on the scheduler.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .weekly_on(DAY, TASK_AT, TASK, |state: AppState| async move {
            run_weekly(&state, period_end_of(today())).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
}

async fn load(state: &AppState, id: i64) -> std::result::Result<Remittance, TransitionError> {
    Remittance::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` (`remittances.prepare`) builds or rebuilds the draft of `client`
/// now, for the shipments delivered up to now.
///
/// # Errors
///
/// `Forbidden` without `remittances.prepare`; `Guard` (422) when there is
/// nothing to put on a draft.
pub async fn build(
    state: &AppState,
    user: &AuthUser,
    client: PartyId,
) -> std::result::Result<Remittance, TransitionError> {
    if !user.has_permission(permissions::REMITTANCES_PREPARE) {
        return Err(TransitionError::Forbidden);
    }
    match draft_for(state, client, renox::db::now(), Some(user.id)).await? {
        Some(built) => Ok(built.remittance),
        None => Err(guard(english("logistics.remittance.nothing_to_draft", &[]))),
    }
}

/// `user` (`remittances.approve`) approves the draft `id`.
///
/// # Errors
///
/// `Forbidden` without the permission; `NotFrom` unless it is a draft;
/// `Guard` (422) for the preparer and while an adjustment line is
/// unapproved.
pub async fn approve(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Remittance, TransitionError> {
    let remittance = load(state, id).await?;
    let unapproved = AdjustmentLine::query()
        .where_(AdjustmentLine::REMITTANCE_ID.eq(id))
        .where_(AdjustmentLine::STATUS.ne(AdjustmentStatus::Approved))
        .count(&state.db)
        .await?;
    let _outcome = remittance
        .transition(APPROVE)
        .by(user)
        .input(&RemittanceInput {
            unapproved_lines: i64::try_from(unapproved).unwrap_or(i64::MAX),
            ..RemittanceInput::default()
        })
        .run(state)
        .await?;
    load(state, id).await
}

/// `user` (`remittances.approve`) takes the approved remittance `id` back to
/// a draft.
///
/// # Errors
///
/// `Forbidden` without the permission; `NotFrom` unless it is approved.
pub async fn reopen(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Remittance, TransitionError> {
    let remittance = load(state, id).await?;
    let _outcome = remittance.transition(REOPEN).by(user).run(state).await?;
    load(state, id).await
}

/// `user` (`remittances.pay`) pays the approved remittance `id` with the
/// bank reference `reference`: an outgoing transfer to the client's account
/// is recorded through the kernel's payments contract, then the transition
/// `pay` runs. A transition that is refused leaves no payout: nothing is
/// recorded when a guard would fail, and a payout recorded for a transition
/// that then fails is reversed.
///
/// # Errors
///
/// `Forbidden` without `remittances.pay`; `NotFrom` unless it is approved;
/// `Guard` (422) without a reference or a bank account on file, for a
/// blocked client and for a net of 0; the payments module's refusals.
pub async fn pay(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reference: &str,
) -> std::result::Result<Remittance, TransitionError> {
    let remittance = load(state, id).await?;
    let terms = reweigh::terms(&state.db, remittance.client()).await?;
    let account = terms
        .bank_account
        .clone()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    let reference = reference.trim().to_owned();
    let blocked = parties::directory(state)
        .status(&state.db, remittance.client())
        .await?
        == Some(PartyStatus::Blocked);
    let mut input = RemittanceInput {
        payment_reference: Some(reference.clone()),
        bank_account: account.clone(),
        client_blocked: blocked,
        ..RemittanceInput::default()
    };
    let ready = user.has_permission(permissions::REMITTANCES_PAY)
        && remittance.status == RemittanceStatus::Approved
        && !reference.is_empty()
        && account.is_some()
        && !blocked
        && remittance.net > 0;
    if !ready {
        // The transition says why: the permission, the state or the guard.
        let _outcome = remittance
            .transition(PAY)
            .by(user)
            .input(&input)
            .run(state)
            .await?;
        return load(state, id).await;
    }
    let entity = LegalEntity::default_entity(&state.db).await?;
    let mut record = RecordPayment::new(
        remittance.client(),
        entity.id,
        PaymentMethod::Transfer,
        remittance.net,
    )
    .outgoing()
    .reference(&reference);
    if let Some(account) = &account {
        record = record.bank_account(account);
    }
    let recorder = payments::payment_recorder(state);
    let payment = recorder
        .record(state, record, Some(user.id))
        .await
        .map_err(TransitionError::Failed)?;
    input.payment_id = Some(payment.id.0);
    input.payment_number = payment.number.clone();
    let outcome = remittance
        .transition(PAY)
        .by(user)
        .input(&input)
        .run(state)
        .await;
    if let Err(error) = outcome {
        let note = english("logistics.remittance.payout_reversed", &[("number", &remittance.number)]);
        recorder
            .reverse(
                state,
                payment.id,
                ReversalReason::EnteredInError,
                Some(note),
                Some(user.id),
            )
            .await
            .map_err(TransitionError::Failed)?;
        return Err(error);
    }
    load(state, id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipment(cod: i64, difference: i64, by: Option<&str>) -> Shipment {
        Shipment {
            cod_amount: cod,
            cod_difference: difference,
            cod_difference_reason: Some("agreed discount".into()),
            cod_absorbed_by: by.map(str::to_owned),
            ..Shipment::default()
        }
    }

    #[test]
    fn a_difference_the_client_absorbs_is_deducted_with_its_reason() {
        let f = figures(&shipment(30_000, 5_000, Some("client")), 0);
        assert_eq!(f.absorbed, 5_000);
        assert_eq!(f.absorbed_reason, "agreed discount");
        assert_eq!(f.amount, 25_000);
    }

    #[test]
    fn a_difference_anyone_else_absorbs_never_reduces_the_client() {
        for by in [Some("driver"), Some("business"), Some("customer"), None] {
            let f = figures(&shipment(30_000, 5_000, by), 0);
            assert_eq!(f.absorbed, 0, "{by:?}");
            assert_eq!(f.amount, 30_000);
        }
    }

    #[test]
    fn the_fee_is_basis_points_of_what_is_paid_out_rounded_to_the_fil() {
        assert_eq!(figures(&shipment(20_000, 0, None), 200).fee, 400);
        assert_eq!(figures(&shipment(999, 0, None), 200).fee, 20);
        let f = figures(&shipment(30_000, 5_000, Some("client")), 200);
        assert_eq!(f.fee, 500);
        assert_eq!(f.amount, 24_500);
    }

    #[test]
    fn a_negative_net_is_zero_and_the_rest_is_carried() {
        assert_eq!(net_and_carried(0, -4_000), (0, -4_000));
        assert_eq!(net_and_carried(10_000, -4_000), (6_000, 0));
        assert_eq!(net_and_carried(1_000, -4_000), (0, -3_000));
    }

    #[test]
    fn the_period_ends_at_dubai_midnight_of_the_run_day() {
        let monday = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let end = period_end_of(monday);
        assert_eq!(end.to_rfc3339(), "2026-11-01T20:00:00+00:00");
    }
}
