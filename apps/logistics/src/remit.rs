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
//! - **Offsets.** When the client's contract allows netting, the open,
//!   undisputed invoices of the client are offset against the amount before
//!   offsets, oldest due date first and capped so the net is not negative
//!   ([`crate::netting`]).
//! - **Actions** ([`approve`], [`reopen`], [`pay`]): `approve` needs a user
//!   other than the preparer and every adjustment line approved; `pay`
//!   re-checks each offset (an invoice that changed sends the remittance back
//!   to a draft with a message), records the payout through the kernel's
//!   payments contract (an outgoing transfer to the client) and one payment
//!   per offset allocated to its invoice, and then runs the transition and
//!   publishes the statement; a refused transition reverses every payment it
//!   recorded.

use std::collections::BTreeSet;

use renox::chrono::{NaiveDate, TimeDelta, Weekday};
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::LegalEntity;
use wl_kernel::PartyId;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::block_guard;
use wl_kernel::delivery::DeliveryId;
use wl_kernel::documents::{DUBAI_OFFSET_SECONDS, today};
use wl_kernel::invoicing::{InvoiceId, InvoiceSummary};
use wl_kernel::payments::{
    self, AllocationTarget, PaymentId, PaymentMethod, PaymentSource, RecordPayment, ReversalReason,
};

use crate::adjustment::{AdjustmentLine, AdjustmentStatus, source};
use crate::block;
use crate::booking;
use crate::netting::{self, Offset};
use crate::notices::{STATEMENT_NOTICE, tell_accountants, tell_party_as};
use crate::permissions;
use crate::remittance::{REMITTANCE_SERIES, Remittance, RemittanceLine, RemittanceStatus};
use crate::remittance_machine::{
    APPROVE, PAY, REOPEN, REOPEN_SYSTEM, REPAY, RETURN_PAYMENT, RemittanceInput,
};
use crate::reweigh::{self, ClientTerms};
use crate::shipment::{Shipment, ShipmentStatus};
use crate::statement;
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

/// What a draft's figures come to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Totals {
    cod_total: i64,
    fee_total: i64,
    adjustment_total: i64,
    offset_total: i64,
    net: i64,
}

/// Works out the figures of draft `remittance` from the lines it has now and
/// stores them, inside `tx`: the COD of its shipment lines less the
/// differences the client absorbed, the fees, the approved adjustment lines
/// attached to it (a pending or rejected one is not counted) and the net,
/// never below 0. The rest of a negative net becomes an open, approved
/// adjustment line for the next draft; the one the earlier settle made is
/// replaced, so it is never carried twice. The open invoices `open` are
/// offset against what is left, oldest due date first (none when the
/// client's contract does not allow netting).
async fn settle(
    tx: &mut Transaction,
    remittance: &Remittance,
    open: &[InvoiceSummary],
) -> Result<Totals> {
    AdjustmentLine::query()
        .where_(AdjustmentLine::CARRIED_FROM_ID.eq(remittance.id))
        .where_(AdjustmentLine::REMITTANCE_ID.is_null())
        .delete(&mut *tx)
        .await?;
    let lines = RemittanceLine::query()
        .where_(RemittanceLine::REMITTANCE_ID.eq(remittance.id))
        .get(&mut *tx)
        .await?;
    let cod_total: i64 = lines.iter().map(RemittanceLine::collected_for_client).sum();
    let fee_total: i64 = lines.iter().map(|line| line.fee).sum();
    let lines_amount: i64 = lines.iter().map(|line| line.amount).sum();
    let attached = AdjustmentLine::query()
        .where_(AdjustmentLine::REMITTANCE_ID.eq(remittance.id))
        .get(&mut *tx)
        .await?;
    let adjustment_total: i64 = attached
        .iter()
        .filter(|line| line.counts())
        .map(|line| line.amount)
        .sum();
    let (before_offsets, carried) = net_and_carried(lines_amount, adjustment_total);
    let taken = netting::plan(before_offsets, open);
    let offset_total = netting::store(&mut *tx, remittance.id, &taken).await?;
    let net = before_offsets - offset_total;
    if carried < 0 {
        let note = english("logistics.remittance.carried_note", &[]);
        let mut line = AdjustmentLine {
            client_party_id: remittance.client_party_id,
            amount: carried,
            reason: english(
                "logistics.remittance.carried_reason",
                &[("number", &remittance.number)],
            ),
            source: source::CARRIED_FORWARD.to_owned(),
            carried_from_id: Some(remittance.id),
            decided_at: Some(renox::db::now()),
            decision_note: Some(note.clone()),
            // Derived from lines that were approved: the rest of them.
            status: AdjustmentStatus::Approved,
            ..AdjustmentLine::default()
        };
        // Born approved, as the lines it is derived from were: the state
        // machine's one exception for a row that starts past `pending`.
        line.insert_in_state(&mut *tx, AdjustmentStatus::Approved, &note)
            .await?;
    }
    let carried_forward = -carried;
    Remittance::query()
        .where_(Remittance::ID.eq(remittance.id))
        .update(
            &mut *tx,
            &[
                ("cod_total", &cod_total),
                ("fee_total", &fee_total),
                ("adjustment_total", &adjustment_total),
                ("offset_total", &offset_total),
                ("net", &net),
                ("carried_forward", &carried_forward),
            ],
        )
        .await?;
    Ok(Totals {
        cod_total,
        fee_total,
        adjustment_total,
        offset_total,
        net,
    })
}

/// Works the figures of the draft `id` out again after one of its
/// adjustment lines was decided, edited or deleted. A remittance that is not
/// a draft is left as it is: it is never edited.
///
/// # Errors
///
/// Database errors.
pub async fn refresh_totals(state: &AppState, id: i64) -> Result<Remittance> {
    let remittance = Remittance::find_or_404(&state.db, id).await?;
    if remittance.status != RemittanceStatus::Draft {
        return Ok(remittance);
    }
    let open = netting::open_invoices(state, remittance.client()).await?;
    let mut tx = state.db.begin().await?;
    settle(&mut tx, &remittance, &open).await?;
    tx.commit().await?;
    Remittance::find_or_404(&state.db, id).await
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
    let open = netting::open_invoices(state, client).await?;
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
            wl_kernel::search::index_in(&mut tx, &fresh.subject()).await?;
            (fresh, true)
        }
    };
    let before = (
        remittance.cod_total,
        remittance.fee_total,
        remittance.adjustment_total,
        remittance.offset_total,
        remittance.net,
    );
    // Recompute the shipment lines.
    RemittanceLine::query()
        .where_(RemittanceLine::REMITTANCE_ID.eq(remittance.id))
        .delete(&mut tx)
        .await?;
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
    let totals = settle(&mut tx, &remittance, &open).await?;
    // A user who builds it becomes its preparer; the weekly rule keeps it.
    let prepared_by = by.or(remittance.prepared_by);
    Remittance::query()
        .where_(Remittance::ID.eq(remittance.id))
        .update(
            &mut tx,
            &[("period_end", &period_end), ("prepared_by", &prepared_by)],
        )
        .await?;
    tx.commit().await?;
    remittance = Remittance::find_or_404(&state.db, remittance.id).await?;
    let changed = created
        || before
            != (
                totals.cod_total,
                totals.fee_total,
                totals.adjustment_total,
                totals.offset_total,
                totals.net,
            );
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

/// The kind a payment of an offset carries in its method details: the
/// payment is an incoming transfer of the client's own money that stayed with
/// the courier (method *COD netting*; wl-payments has no method of that name,
/// see the deferred list).
pub const COD_NETTING: &str = "cod_netting";

/// Sends the approved remittance back to a draft because the offsets in
/// `changed` no longer hold: they come off, the draft is worked out again
/// (another open invoice may take their place) and the accountant reads the
/// message. Answers the message.
async fn send_back(
    state: &AppState,
    remittance: &Remittance,
    changed: &[(Offset, &'static str)],
) -> std::result::Result<String, TransitionError> {
    let mut parts = Vec::new();
    for (offset, reason) in changed {
        let why = english(&format!("logistics.remittance.offset_reason.{reason}"), &[]);
        parts.push(english(
            "logistics.remittance.offset_removed",
            &[("number", &offset.invoice_number), ("reason", &why)],
        ));
    }
    let message = parts.join("; ");
    let _outcome = remittance
        .transition(REOPEN_SYSTEM)
        .by_system()
        .run(state)
        .await?;
    let note = Some(message.clone());
    Remittance::query()
        .where_(Remittance::ID.eq(remittance.id))
        .update(&state.db, &[("message", &note)])
        .await?;
    refresh_totals(state, remittance.id).await?;
    Ok(message)
}

/// Reverses the payments `recorded` (ids), newest first, as entered in
/// error: a payment that was recorded for a `pay` that did not go through.
async fn undo(
    state: &AppState,
    user: i64,
    remittance: &Remittance,
    recorded: &[i64],
) -> std::result::Result<(), TransitionError> {
    let recorder = payments::payment_recorder(state);
    for id in recorded.iter().rev() {
        let note = english(
            "logistics.remittance.payout_reversed",
            &[("number", &remittance.number)],
        );
        recorder
            .reverse(
                state,
                PaymentId(*id),
                ReversalReason::EnteredInError,
                Some(note),
                Some(user),
            )
            .await
            .map_err(TransitionError::Failed)?;
    }
    Ok(())
}

/// Records the payment of `offset` (an incoming payment of the client, method
/// details *COD netting*, referenced by the remittance's number) and allocates
/// it in full to its invoice; its id is stored on the offset. A refused
/// allocation reverses the payment again.
async fn record_offset(
    state: &AppState,
    user: i64,
    remittance: &Remittance,
    entity: i64,
    offset: &Offset,
) -> std::result::Result<i64, TransitionError> {
    let recorder = payments::payment_recorder(state);
    let record = RecordPayment::new(
        remittance.client(),
        entity,
        PaymentMethod::Transfer,
        offset.amount,
    )
    .reference(&remittance.number)
    .source(PaymentSource::System)
    .method_details(renox::serde_json::json!({
        "kind": COD_NETTING,
        "remittance": remittance.number,
        "invoice": offset.invoice_number,
    }));
    let payment = recorder
        .record(state, record, Some(user))
        .await
        .map_err(TransitionError::Failed)?;
    let allocated = recorder
        .allocate(
            state,
            payment.id,
            AllocationTarget::Invoice(InvoiceId(offset.invoice_id)),
            offset.amount,
        )
        .await;
    if let Err(error) = allocated {
        undo(state, user, remittance, &[payment.id.0]).await?;
        return Err(TransitionError::Failed(error));
    }
    let payment_id = Some(payment.id.0);
    let payment_number = payment.number.clone();
    Offset::query()
        .where_(Offset::ID.eq(offset.id))
        .update(
            &state.db,
            &[
                ("payment_id", &payment_id),
                ("payment_number", &payment_number),
            ],
        )
        .await?;
    Ok(payment.id.0)
}

/// `user` (`remittances.pay`) pays the approved remittance `id` with the
/// bank reference `reference`: each offset is re-checked, an outgoing
/// transfer of the net to the client's account is recorded through the
/// kernel's payments contract, then one payment per offset allocated to its
/// invoice, then the transition `pay` runs and the statement is published. A
/// transition that is refused leaves no payment: nothing is recorded when a
/// guard would fail, and every payment recorded for a transition that then
/// fails is reversed.
///
/// # Errors
///
/// `Forbidden` without `remittances.pay`; `NotFrom` unless it is approved;
/// `Guard` (422) without a reference or a bank account on file, for a
/// blocked client, for nothing to pay and when an offset no longer holds
/// (the remittance is then a draft again, with the message); the payments
/// module's refusals.
pub async fn pay(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reference: &str,
) -> std::result::Result<Remittance, TransitionError> {
    let remittance = load(state, id).await?;
    if remittance.status == RemittanceStatus::Approved
        && user.has_permission(permissions::REMITTANCES_PAY)
    {
        let changed = netting::changes(state, &remittance).await?;
        if !changed.is_empty() {
            let message = send_back(state, &remittance, &changed).await?;
            return Err(guard(message));
        }
    }
    let offsets = remittance.offsets(&state.db).await?;
    let terms = reweigh::terms(&state.db, remittance.client()).await?;
    let account = terms
        .bank_account
        .clone()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    let reference = reference.trim().to_owned();
    // A blocked client waits for the unblock, or for one approved exception
    // (`PTY.ACL.03`).
    let blocked =
        block::on_hold(state, remittance.client()).await? && !remittance.block_exception_granted;
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
        && (remittance.net > 0 || !offsets.is_empty());
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
    let mut recorded: Vec<i64> = Vec::new();
    if remittance.net > 0 {
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
        let payment = payments::payment_recorder(state)
            .record(state, record, Some(user.id))
            .await
            .map_err(TransitionError::Failed)?;
        recorded.push(payment.id.0);
        input.payment_id = Some(payment.id.0);
        input.payment_number = payment.number.clone();
    }
    for offset in &offsets {
        match record_offset(state, user.id, &remittance, entity.id, offset).await {
            Ok(payment) => recorded.push(payment),
            Err(error) => {
                undo(state, user.id, &remittance, &recorded).await?;
                return Err(error);
            }
        }
    }
    let outcome = remittance
        .transition(PAY)
        .by(user)
        .input(&input)
        .run(state)
        .await;
    if let Err(error) = outcome {
        undo(state, user.id, &remittance, &recorded).await?;
        return Err(error);
    }
    let paid = load(state, id).await?;
    statement::publish(&state.db, &paid).await?;
    tell_statement(state, &paid).await?;
    Ok(paid)
}

/// Tells the client that the statement of the paid remittance is published.
/// It is a statement notice: when it gives up, the client's other finance
/// contact is tried, then the accountant gets a task (`invoices` group).
async fn tell_statement(state: &AppState, remittance: &Remittance) -> Result {
    let title = english(
        "logistics.remittance.statement_title",
        &[("number", &remittance.number)],
    );
    let body = english(
        "logistics.remittance.statement_body",
        &[
            ("number", &remittance.number),
            ("amount", &crate::rates::aed(remittance.net)),
        ],
    );
    tell_party_as(
        state,
        STATEMENT_NOTICE,
        remittance.subject(),
        &remittance.number,
        remittance.client(),
        &title,
        &body,
    )
    .await
}

/// `user` (`remittances.pay`) records that the bank returned the transfer of
/// the paid remittance `id`, with the bank's return `advice` and the `reason`
/// it gave: the voucher is reversed in the payments module as a bank return,
/// the remittance is `payment_returned`, the amount is owed to the client
/// again (the offsets stay allocated) and the client is told and asked to
/// confirm its bank account. Each return keeps its advice
/// ([`crate::remittance::PaymentReturn`]).
///
/// # Errors
///
/// `Forbidden` without `remittances.pay`; `NotFrom` unless it is paid;
/// `Guard` (422) without the advice.
pub async fn return_payment(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    advice: &str,
    reason: &str,
) -> std::result::Result<Remittance, TransitionError> {
    let remittance = load(state, id).await?;
    let advice = advice.trim().to_owned();
    let input = RemittanceInput {
        return_advice: Some(advice.clone()),
        return_reason: Some(reason.trim().to_owned()),
        ..RemittanceInput::default()
    };
    let ready = user.has_permission(permissions::REMITTANCES_PAY)
        && remittance.status == RemittanceStatus::Paid
        && !advice.is_empty();
    if ready && let Some(payment) = remittance.payment_id {
        let note = english(
            "logistics.remittance.return_note",
            &[("number", &remittance.number), ("advice", &advice)],
        );
        payments::payment_recorder(state)
            .reverse(
                state,
                PaymentId(payment),
                ReversalReason::BankReturn,
                Some(note),
                Some(user.id),
            )
            .await
            .map_err(TransitionError::Failed)?;
    }
    let _outcome = remittance
        .transition(RETURN_PAYMENT)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    let returned = load(state, id).await?;
    let title = english(
        "logistics.remittance.returned_title",
        &[("number", &returned.number)],
    );
    let body = english(
        "logistics.remittance.returned_body",
        &[
            ("number", &returned.number),
            ("amount", &crate::rates::aed(returned.net)),
        ],
    );
    tell_party_as(
        state,
        STATEMENT_NOTICE,
        returned.subject(),
        &returned.number,
        returned.client(),
        &title,
        &body,
    )
    .await?;
    Ok(returned)
}

/// `user` (`remittances.pay`) records the client's confirmation that the bank
/// account the transfer of the returned remittance `id` went to is correct
/// (for example a temporary bank issue), with `note` on who confirmed it, when
/// and how. A client user of the company may record it in the portal.
///
/// # Errors
///
/// `Forbidden` for anybody but a holder of `remittances.pay` and a client user
/// of the company; `NotFrom` (422) unless the remittance is returned; `Guard`
/// (422) without the note.
pub async fn confirm_account(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    note: &str,
) -> std::result::Result<Remittance, TransitionError> {
    let remittance = load(state, id).await?;
    let staff = user.has_permission(permissions::REMITTANCES_PAY);
    if !staff && !booking::acts_for(state, user, remittance.client()).await? {
        return Err(TransitionError::Forbidden);
    }
    if remittance.status != RemittanceStatus::PaymentReturned {
        return Err(guard(english("logistics.remittance.not_returned", &[])));
    }
    if note.trim().is_empty() {
        return Err(guard(english(
            "logistics.remittance.confirm_note_needed",
            &[],
        )));
    }
    let at = Some(renox::db::now());
    let by = Some(user.id);
    let note = Some(note.trim().to_owned());
    Remittance::query()
        .where_(Remittance::ID.eq(id))
        .update(
            &state.db,
            &[
                ("account_confirmed_at", &at),
                ("account_confirmed_by", &by),
                ("account_confirmed_note", &note),
            ],
        )
        .await?;
    load(state, id).await
}

/// `user` (`remittances.pay`) pays the returned remittance `id` again with the
/// new bank reference `reference`: to the client's remittance account as it
/// is now. That account is the one the transfer was returned from only when
/// the client's confirmation that it is correct is recorded; a changed account
/// is the client's only after a second user approved it and the client
/// confirmed it (`LOG.ACL.13`, [`crate::account`]). The offsets stay
/// allocated; only the cash part is paid. The statement is published again
/// with the new reference.
///
/// # Errors
///
/// `Forbidden` without `remittances.pay`; `NotFrom` unless it is returned;
/// `Guard` (422) without a new reference, without an account on file, while
/// the account is not changed and not confirmed, for a blocked client and
/// for nothing to pay; the payments module's refusals.
pub async fn repay(
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
    // A blocked client waits for the unblock, or for one approved exception
    // (`PTY.ACL.03`).
    let blocked =
        block::on_hold(state, remittance.client()).await? && !remittance.block_exception_granted;
    let changed = account
        .as_deref()
        .is_some_and(|now| Some(now) != remittance.bank_account.as_deref());
    let mut input = RemittanceInput {
        payment_reference: Some(reference.clone()),
        bank_account: account.clone(),
        client_blocked: blocked,
        account_changed: changed,
        account_confirmed: remittance.account_confirmed_at.is_some(),
        ..RemittanceInput::default()
    };
    let ready = user.has_permission(permissions::REMITTANCES_PAY)
        && remittance.status == RemittanceStatus::PaymentReturned
        && !reference.is_empty()
        && Some(reference.as_str()) != remittance.payment_reference.as_deref()
        && account.is_some()
        && (input.account_changed || input.account_confirmed)
        && !blocked
        && remittance.net > 0;
    if !ready {
        // The transition says why: the permission, the state or the guard.
        let _outcome = remittance
            .transition(REPAY)
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
    let payment = payments::payment_recorder(state)
        .record(state, record, Some(user.id))
        .await
        .map_err(TransitionError::Failed)?;
    input.payment_id = Some(payment.id.0);
    input.payment_number = payment.number.clone();
    let outcome = remittance
        .transition(REPAY)
        .by(user)
        .input(&input)
        .run(state)
        .await;
    if let Err(error) = outcome {
        undo(state, user.id, &remittance, &[payment.id.0]).await?;
        return Err(error);
    }
    let paid = load(state, id).await?;
    statement::publish(&state.db, &paid).await?;
    tell_statement(state, &paid).await?;
    Ok(paid)
}

/// `user` (`remittances.pay`) asks the company admin for an exception that
/// lets the payment of remittance `id` go ahead although its client is
/// blocked (`PTY.ACL.03`): `pay` for an approved remittance, `repay` for a
/// returned one. The company admin, never the requester, approves it; the
/// kernel then marks the remittance, and the next `pay` (or `repay`) goes
/// ahead once.
///
/// # Errors
///
/// `Forbidden` without `remittances.pay`; `Guard` (422) when the remittance is
/// not waiting to be paid, when the client is not blocked and when an
/// exception is already pending.
pub async fn ask_exception(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<ApprovalRequest, TransitionError> {
    if !user.has_permission(permissions::REMITTANCES_PAY) {
        return Err(TransitionError::Forbidden);
    }
    let remittance = load(state, id).await?;
    let transition = match remittance.status {
        RemittanceStatus::Approved => PAY,
        RemittanceStatus::PaymentReturned => REPAY,
        _ => {
            return Err(guard(english(
                "logistics.remittance.exception_not_pay",
                &[],
            )));
        }
    };
    let mut tx = state.db.begin().await?;
    let request = block_guard::request_exception_in(
        state,
        &mut tx,
        user,
        &remittance.subject(),
        transition,
        renox::serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    approvals::announce_request(state, &request).await?;
    Ok(request)
}

/// Sets whether the contract of `client` allows netting open invoices
/// against its COD (off unless set; BL-LOG-16).
///
/// # Errors
///
/// Database errors.
pub async fn set_netting_allowed(db: &Db, client: PartyId, allowed: bool) -> Result {
    let found = ClientTerms::query()
        .where_(ClientTerms::CLIENT_PARTY_ID.eq(client.0))
        .first(db)
        .await?;
    match found {
        Some(mut terms) => {
            terms.netting_allowed = allowed;
            terms.save(db).await?;
        }
        None => {
            let mut terms = ClientTerms {
                client_party_id: client.0,
                netting_allowed: allowed,
                ..ClientTerms::default()
            };
            terms.insert(db).await?;
        }
    }
    Ok(())
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
