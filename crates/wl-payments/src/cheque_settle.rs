//! Settling part of a bounced cheque and replacing it (wl-payments §4
//! `settle_part` and `replace`, §5.3 step 4, `FIN.ACL.02`, issue #104).
//!
//! - **`settle_part`** (`cheques.manage`): the party pays part of a bounced
//!   *received* cheque's open amount in cash, by transfer or by card. The
//!   payment must be below the open amount (equal to it is `replace` with that
//!   payment). It is recorded as a payment of the cheque in the transition's
//!   transaction and allocated to the documents the cheque pays after the
//!   commit; the open amount goes down and the cheque stays where it is, so
//!   there are two transitions: `settle_part` (`bounced` to `bounced`) and
//!   `settle_part_in_legal_case` (`legal_case` to `legal_case`).
//! - **`link_replacement`**: a cheque (same party, direction and currency, not
//!   bounced or ended) or a payment (received and not allocated elsewhere, in
//!   the cheque's direction) is linked to the bounced cheque as its
//!   replacement; it counts for the amount it carries. A replacement belongs to
//!   one cheque only.
//! - **`replace`** (`cheques.manage`, `FIN.ACL.02`): `bounced` or `legal_case`
//!   to `replaced`, only when the linked replacements cover the open amount.
//!   In the transaction the tasks of the bounce and of the legal case are
//!   closed. After the commit, for a received cheque, `ChequeBounceResolved`
//!   (replaced) is emitted, and the original is handed back to the party with
//!   a receipt once every replacement cheque cleared ([`after_clear`]): a task
//!   for the accountant that carries the receipt number
//!   ([`HAND_BACK_SERIES`]). If a replacement cheque bounces, the original
//!   stays replaced and is kept with a note ([`after_bounce`]); the new cheque
//!   follows its own bounce flow. An issued cheque is replaced the same way (a
//!   new issued cheque or an outgoing payment) and needs no hand-back.
//!
//! Decisions on what the spec leaves open: a replacement *payment* is not
//! allocated by `replace`; the accountant allocates it like any payment, and
//! `replace` counts what it carried unallocated when it was linked. A
//! replacement cheque counts at its full amount.

use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::{ChequeOutcome, ChequeStatus};
use wl_kernel::documents::today;
use wl_kernel::payments::{Direction, PaymentMethod};
use wl_kernel::{Reset, Series};

use crate::cheque::Cheque;
use crate::cheque_events;
use crate::cheque_money;
use crate::credit;
use crate::followup::{FollowupKind, FollowupStatus, FollowupTask};
use crate::notices;
use crate::payment::Payment;
use crate::permissions;

/// The `settle_part` transition's name.
pub const SETTLE_PART: &str = "settle_part";
/// `settle_part` from `legal_case`.
pub const SETTLE_PART_IN_LEGAL_CASE: &str = "settle_part_in_legal_case";
/// The `replace` transition's name.
pub const REPLACE: &str = "replace";

/// The receipt numbers of the original cheques handed back:
/// `CHR-2026-0001`, yearly, one count for the client.
pub const HAND_BACK_SERIES: Series = Series {
    key: "cheque_hand_back",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "CHR",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// What replaces a bounced cheque.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReplacementKind {
    /// A new cheque.
    #[default]
    Cheque,
    /// A payment (cash, transfer, card).
    Payment,
}

/// A cheque or payment linked as the replacement of a bounced cheque.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_cheque_replacements",
    unique(kind, ref_id),
    index(cheque_id)
)]
pub struct ChequeReplacement {
    /// The primary key.
    pub id: i64,
    /// The bounced cheque that is replaced.
    pub cheque_id: i64,
    /// Whether the replacement is a cheque or a payment.
    pub kind: ReplacementKind,
    /// The replacement cheque or payment.
    pub ref_id: i64,
    /// What it covers, in fils.
    pub amount: i64,
    /// The user who linked it.
    pub linked_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ChequeReplacement {
    /// The replacements of `cheque_id`, oldest first.
    pub async fn of_cheque(db: &Db, cheque_id: i64) -> Result<Vec<ChequeReplacement>> {
        ChequeReplacement::query()
            .where_(ChequeReplacement::CHEQUE_ID.eq(cheque_id))
            .order_by(ChequeReplacement::ID)
            .get(db)
            .await
    }

    /// The cheques that `cheque_id`, as a replacement cheque, replaces.
    pub async fn replaced_by_cheque(db: &Db, cheque_id: i64) -> Result<Vec<ChequeReplacement>> {
        ChequeReplacement::query()
            .where_(ChequeReplacement::KIND.eq(ReplacementKind::Cheque))
            .where_(ChequeReplacement::REF_ID.eq(cheque_id))
            .order_by(ChequeReplacement::ID)
            .get(db)
            .await
    }
}

/// What the party pays towards a bounced cheque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettleRequest {
    /// Cash, transfer or card.
    pub method: PaymentMethod,
    /// The amount, in fils; below the open amount.
    pub amount: i64,
    /// The transfer's or card slip's reference (required for those two).
    pub reference: Option<String>,
}

/// The methods a part settlement can be paid by.
pub const SETTLE_METHODS: [PaymentMethod; 3] = [
    PaymentMethod::Cash,
    PaymentMethod::Transfer,
    PaymentMethod::Card,
];

/// What a `settle_part` run carries to its guard and effect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SettleInput {
    /// The method.
    pub method: Option<PaymentMethod>,
    /// The amount, in fils.
    pub amount: i64,
}

fn settle_input(ctx: &TransitionCtx) -> Option<SettleInput> {
    ctx.input_as::<SettleInput>().ok()
}

/// Why `input` cannot settle part of `cheque`, if it cannot.
pub(crate) fn settle_problem(cheque: &Cheque, input: Option<&SettleInput>) -> Option<String> {
    if !cheque.is_received() {
        return Some("Only a received cheque takes a part payment.".to_owned());
    }
    let Some(input) = input else {
        return Some("A method and an amount are required.".to_owned());
    };
    if !input.method.is_some_and(|m| SETTLE_METHODS.contains(&m)) {
        return Some("A part payment is made in cash, by transfer or by card.".to_owned());
    }
    if input.amount <= 0 {
        return Some("The amount must be above 0.".to_owned());
    }
    if input.amount >= cheque.open_amount {
        return Some(format!(
            "The payment must be below the open amount ({} fils): use replace for the whole of it.",
            cheque.open_amount
        ));
    }
    None
}

/// What a `replace` run carries to its guard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ReplaceInput {
    /// What the linked replacements cover now, in fils.
    pub covered: i64,
}

/// Why `cheque` cannot be replaced by `covered` fils of replacements, if it
/// cannot (`FIN.ACL.02`).
pub(crate) fn replace_problem(cheque: &Cheque, covered: Option<i64>) -> Option<String> {
    let covered = covered.unwrap_or(0);
    if cheque.open_amount <= 0 {
        return Some("The cheque has nothing open to replace.".to_owned());
    }
    if covered < cheque.open_amount {
        return Some(format!(
            "The linked cheques and payments cover {covered} fils of the {} fils open: link replacements that cover it (FIN.ACL.02).",
            cheque.open_amount
        ));
    }
    None
}

/// `settle_part` for a state: the cheque stays in it.
fn settling(transition: Transition<Cheque>, state: ChequeStatus) -> Transition<Cheque> {
    transition
        .label("Settle part")
        .tone("primary")
        .permission(permissions::CHEQUES_MANAGE)
        .from([state])
        .to(state)
        .guard("part_payment_below_open", |cheque: &Cheque, ctx| {
            let problem = settle_problem(cheque, settle_input(ctx).as_ref());
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|cheque: &Cheque, ctx, tx| {
            let id = cheque.id;
            let settled = cheque.settled_amount;
            let open = cheque.open_amount;
            let input = settle_input(ctx);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a part payment without its input"))
                })?;
                // Not the status column, so outside the machine's guard.
                Cheque::query()
                    .where_(Cheque::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("settled_amount", &(settled + input.amount)),
                            ("open_amount", &(open - input.amount)),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `settle_part`, `settle_part_in_legal_case` and `replace`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Cheque>> {
    vec![
        settling(Transition::new(SETTLE_PART), ChequeStatus::Bounced),
        settling(
            Transition::new(SETTLE_PART_IN_LEGAL_CASE),
            ChequeStatus::LegalCase,
        ),
        Transition::new(REPLACE)
            .label("Replace")
            .tone("primary")
            .permission(permissions::CHEQUES_MANAGE)
            .from([ChequeStatus::Bounced, ChequeStatus::LegalCase])
            .to(ChequeStatus::Replaced)
            .guard(
                "replacements_cover_the_open_amount",
                |cheque: &Cheque, ctx| {
                    let covered = ctx.input_as::<ReplaceInput>().ok().map(|i| i.covered);
                    let problem = replace_problem(cheque, covered);
                    Box::pin(async move { problem.map_or(Ok(()), Err) })
                },
            )
            .effect(|cheque: &Cheque, _ctx, tx| {
                let id = cheque.id;
                Box::pin(async move {
                    // The tasks of the bounce and of the legal case are done.
                    for kind in [
                        FollowupKind::FollowUpCheque,
                        FollowupKind::ReplaceCheque,
                        FollowupKind::LegalCase,
                        FollowupKind::DecideCheque,
                    ] {
                        FollowupTask::query()
                            .where_(FollowupTask::CHEQUE_ID.eq(id))
                            .where_(FollowupTask::KIND.eq(kind))
                            .where_(FollowupTask::STATUS.eq(FollowupStatus::Open))
                            .update(
                                &mut *tx,
                                &[
                                    ("status", &FollowupStatus::Done.as_str().to_owned()),
                                    ("done_at", &Some(renox::db::now())),
                                    ("updated_at", &renox::db::now()),
                                ],
                            )
                            .await?;
                    }
                    Cheque::query()
                        .where_(Cheque::ID.eq(id))
                        .update(&mut *tx, &[("updated_at", &renox::db::now())])
                        .await?;
                    Ok(())
                })
            }),
    ]
}

async fn reload(state: &AppState, cheque: &Cheque) -> std::result::Result<Cheque, TransitionError> {
    Cheque::find_or_404(&state.db, cheque.id)
        .await
        .map_err(TransitionError::Failed)
}

fn guard(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// `user` records that the party paid part of the bounced (or `legal_case`)
/// received `cheque` ([`SettleRequest`]): a payment of the cheque is
/// recorded and allocated to the documents it pays, and the open amount goes
/// down. The cheque stays where it is.
///
/// # Errors
///
/// `Forbidden` without `cheques.manage`; `NotFrom` unless `bounced` or
/// `legal_case`; `Guard` (422) for an issued cheque, another method than cash,
/// transfer or card, an amount of 0 or less or not below the open amount, or
/// when the payment is refused (a transfer or card without a reference).
pub async fn settle_part(
    state: &AppState,
    user: &AuthUser,
    cheque: &Cheque,
    request: &SettleRequest,
) -> std::result::Result<Cheque, TransitionError> {
    let cheque = reload(state, cheque).await?;
    let name = match cheque.status {
        ChequeStatus::Bounced => SETTLE_PART,
        ChequeStatus::LegalCase => SETTLE_PART_IN_LEGAL_CASE,
        other => return Err(TransitionError::NotFrom(other.as_str().to_owned())),
    };
    let input = SettleInput {
        method: Some(request.method),
        amount: request.amount,
    };
    // The guard says why; the payment is prepared only for a good request.
    let prepared = if settle_problem(&cheque, Some(&input)).is_none() {
        let mut payment = cheque_money::payment_for(
            &cheque,
            request.amount,
            request.method,
            request.reference.as_deref(),
        );
        if request.method == PaymentMethod::Cash {
            payment = payment.collected_by(user.id);
        }
        Some(cheque_money::prepare(state, &payment, Some(user.id)).await?)
    } else {
        None
    };
    let mut tx = state.db.begin().await?;
    let outcome = cheque
        .transition(name)
        .by(user)
        .input(&input)
        .run_in(&mut tx)
        .await?;
    let Some(prepared) = prepared else {
        return Err(guard("The part payment could not be prepared."));
    };
    let (payment, received) = cheque_money::record_in(&mut tx, prepared).await?;
    tx.commit().await?;
    received
        .emit(state)
        .await
        .map_err(TransitionError::Failed)?;
    cheque_money::allocate_to_links(state, &cheque, &payment, payment.amount, Some(user.id))
        .await?;
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    reload(state, &cheque).await
}

/// Why `candidate` cannot replace `cheque`, if it cannot: the same cheque,
/// another party, direction or currency, or a cheque already bounced or
/// ended.
pub(crate) fn replacement_cheque_problem(cheque: &Cheque, candidate: &Cheque) -> Option<String> {
    if candidate.id == cheque.id {
        return Some("A cheque cannot replace itself.".to_owned());
    }
    if candidate.party_id != cheque.party_id {
        return Some("The replacement cheque belongs to another party.".to_owned());
    }
    if candidate.direction != cheque.direction {
        return Some("The replacement must go the same way: received or issued.".to_owned());
    }
    if !candidate.currency.eq_ignore_ascii_case(&cheque.currency) {
        return Some(format!(
            "The replacement is in {}, the cheque in {}.",
            candidate.currency, cheque.currency
        ));
    }
    if !matches!(
        candidate.status,
        ChequeStatus::Received
            | ChequeStatus::DueForDeposit
            | ChequeStatus::Deposited
            | ChequeStatus::Cleared
    ) {
        return Some(format!(
            "A {} cheque cannot replace another.",
            candidate.status.as_str().replace('_', " ")
        ));
    }
    None
}

/// Why `payment` cannot replace `cheque`, if it cannot, when `free` fils of
/// it are not allocated: another party, direction or currency, not `received`,
/// or nothing free.
pub(crate) fn replacement_payment_problem(
    cheque: &Cheque,
    payment: &Payment,
    free: i64,
) -> Option<String> {
    if payment.party_id != cheque.party_id {
        return Some("The replacement payment belongs to another party.".to_owned());
    }
    if payment.direction != cheque.direction {
        return Some("The replacement must go the same way: money in or out.".to_owned());
    }
    if !payment.currency.eq_ignore_ascii_case(&cheque.currency) {
        return Some(format!(
            "The replacement is in {}, the cheque in {}.",
            payment.currency, cheque.currency
        ));
    }
    if !payment.status.is_received() {
        return Some("Only a received payment can replace a cheque.".to_owned());
    }
    if free <= 0 {
        return Some("The payment has nothing left that is not allocated.".to_owned());
    }
    None
}

/// `user` links the cheque or payment `ref_id` (`kind`) as a replacement of
/// the bounced `cheque`.
///
/// # Errors
///
/// `Forbidden` without `cheques.manage`; `NotFrom` unless `bounced` or
/// `legal_case`; 404 for an unknown replacement; `Guard` (422) for one the
/// checks above refuse, or one already linked to a cheque.
pub async fn link_replacement(
    state: &AppState,
    user: &AuthUser,
    cheque: &Cheque,
    kind: ReplacementKind,
    ref_id: i64,
) -> std::result::Result<ChequeReplacement, TransitionError> {
    if !user.has_permission(permissions::CHEQUES_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let cheque = reload(state, cheque).await?;
    if !cheque.status.is_owed() {
        return Err(TransitionError::NotFrom(cheque.status.as_str().to_owned()));
    }
    let amount = match kind {
        ReplacementKind::Cheque => {
            let candidate = Cheque::find(&state.db, ref_id)
                .await?
                .ok_or(TransitionError::Failed(Error::NotFound))?;
            if let Some(problem) = replacement_cheque_problem(&cheque, &candidate) {
                return Err(guard(problem));
            }
            candidate.amount
        }
        ReplacementKind::Payment => {
            let payment = Payment::find(&state.db, ref_id)
                .await?
                .ok_or(TransitionError::Failed(Error::NotFound))?;
            let free = credit::unallocated(&state.db, &payment).await?;
            if let Some(problem) = replacement_payment_problem(&cheque, &payment, free) {
                return Err(guard(problem));
            }
            free
        }
    };
    let taken = ChequeReplacement::query()
        .where_(ChequeReplacement::KIND.eq(kind))
        .where_(ChequeReplacement::REF_ID.eq(ref_id))
        .count(&state.db)
        .await?;
    if taken > 0 {
        return Err(guard("It already replaces a cheque."));
    }
    let mut link = ChequeReplacement {
        cheque_id: cheque.id,
        kind,
        ref_id,
        amount,
        linked_by: Some(user.id),
        ..Default::default()
    };
    link.insert(&state.db).await?;
    Ok(link)
}

/// What the replacements of `cheque` cover now, in fils: a replacement cheque
/// counts at its amount while it is still good, a payment at what it carried
/// unallocated when it was linked while it is still received.
pub async fn covered_by_replacements(db: &Db, cheque: &Cheque) -> Result<i64> {
    let mut covered = 0;
    for link in ChequeReplacement::of_cheque(db, cheque.id).await? {
        match link.kind {
            ReplacementKind::Cheque => {
                if let Some(candidate) = Cheque::find(db, link.ref_id).await? {
                    if replacement_cheque_problem(cheque, &candidate).is_none() {
                        covered += link.amount;
                    }
                }
            }
            ReplacementKind::Payment => {
                if let Some(payment) = Payment::find(db, link.ref_id).await? {
                    if payment.status.is_received() {
                        covered += link.amount;
                    }
                }
            }
        }
    }
    Ok(covered)
}

/// `user` marks the bounced or `legal_case` `cheque` replaced, when the
/// linked replacements cover its open amount (`FIN.ACL.02`). The tasks of the
/// bounce and of the legal case are closed; for a received cheque
/// `ChequeBounceResolved` (replaced) follows, and the original is handed back
/// once its replacements cleared.
///
/// # Errors
///
/// `Forbidden` without `cheques.manage`; `NotFrom` unless `bounced` or
/// `legal_case`; `Guard` (422) when the replacements do not cover the open
/// amount.
pub async fn replace(
    state: &AppState,
    user: &AuthUser,
    cheque: &Cheque,
) -> std::result::Result<Cheque, TransitionError> {
    let cheque = reload(state, cheque).await?;
    let input = ReplaceInput {
        covered: covered_by_replacements(&state.db, &cheque).await?,
    };
    let _outcome = cheque
        .transition(REPLACE)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    reload(state, &cheque).await
}

/// Whether every replacement cheque of the replaced `original` cleared, and it
/// has at least the cleared ones or payments only: it can be handed back.
async fn hand_back_ready(db: &Db, original: &Cheque) -> Result<bool> {
    if original.status != ChequeStatus::Replaced || !original.is_received() {
        return Ok(false);
    }
    for link in ChequeReplacement::of_cheque(db, original.id).await? {
        if link.kind == ReplacementKind::Cheque {
            let cleared = Cheque::find(db, link.ref_id)
                .await?
                .is_some_and(|c| c.status == ChequeStatus::Cleared);
            if !cleared {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Makes the hand-back task of `original` with its receipt number, when it
/// can be handed back and no task was made yet, and tells the staff.
async fn hand_back(state: &AppState, original: &Cheque) -> Result<()> {
    if !hand_back_ready(&state.db, original).await? {
        return Ok(());
    }
    let mut task = FollowupTask::hand_back(original);
    let receipt = HAND_BACK_SERIES
        .next_allowing_gaps(&state.db, None, None, today())
        .await?;
    task.text = format!(
        "Hand back cheque {} to the party with receipt {receipt}",
        original.number
    );
    task.evidence = renox::db::Json(json!({
        "receipt": receipt,
        "cheque": original.number,
        "bank": original.bank,
        "amount": original.amount,
    }));
    if let Some(task) = FollowupTask::create_once(&state.db, task).await? {
        notices::cheque_task(state, original, &task).await?;
    }
    Ok(())
}

/// After a committed `replace` of `cheque_id`: for a received cheque
/// `ChequeBounceResolved` (replaced), and the hand-back when its replacements
/// are all in.
///
/// # Errors
///
/// The database's errors, or a notice that failed.
pub(crate) async fn after_replace(state: &AppState, cheque_id: i64) -> Result<()> {
    let cheque = Cheque::find_or_404(&state.db, cheque_id).await?;
    if cheque.direction == Direction::In {
        cheque_events::resolved(state, &cheque, ChequeOutcome::Replaced).await?;
    }
    hand_back(state, &cheque).await
}

/// After a committed `clear` of `cheque_id`: if it is the replacement of
/// replaced cheques, they are handed back once all their replacements
/// cleared.
///
/// # Errors
///
/// The database's errors, or a notice that failed.
pub(crate) async fn after_clear(state: &AppState, cheque_id: i64) -> Result<()> {
    for link in ChequeReplacement::replaced_by_cheque(&state.db, cheque_id).await? {
        if let Some(original) = Cheque::find(&state.db, link.cheque_id).await? {
            hand_back(state, &original).await?;
        }
    }
    Ok(())
}

/// After a committed `bounce` of `cheque_id`: if it is the replacement of
/// replaced cheques, each original stays replaced and is kept with a note.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn after_bounce(state: &AppState, cheque_id: i64) -> Result<()> {
    let Some(replacement) = Cheque::find(&state.db, cheque_id).await? else {
        return Ok(());
    };
    for link in ChequeReplacement::replaced_by_cheque(&state.db, cheque_id).await? {
        let note = format!(
            "Replacement cheque {} bounced: the original is kept.",
            replacement.number
        );
        Cheque::query()
            .where_(Cheque::ID.eq(link.cheque_id))
            .where_(Cheque::STATUS.eq(ChequeStatus::Replaced))
            .update(
                &state.db,
                &[
                    ("custody_note", &Some(note)),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::TransitionError;

    fn bounced(open: i64) -> Cheque {
        Cheque {
            id: 1,
            party_id: 7,
            amount: 2_000_000,
            open_amount: open,
            currency: "AED".into(),
            status: ChequeStatus::Bounced,
            ..Default::default()
        }
    }

    fn input(method: PaymentMethod, amount: i64) -> SettleInput {
        SettleInput {
            method: Some(method),
            amount,
        }
    }

    #[test]
    fn settle_part_keeps_the_cheque_in_its_state() {
        let machine = Cheque::machine();
        let found = machine.find(SETTLE_PART).expect("transition");
        assert_eq!(found.from_states(), ["bounced"]);
        assert_eq!(found.to_state(), Some("bounced"));
        let legal = machine.find(SETTLE_PART_IN_LEGAL_CASE).expect("transition");
        assert_eq!(legal.from_states(), ["legal_case"]);
        assert_eq!(legal.to_state(), Some("legal_case"));
        assert_eq!(found.guard_names(), legal.guard_names());
    }

    #[test]
    fn replace_leaves_bounced_and_legal_case() {
        let machine = Cheque::machine();
        let found = machine.find(REPLACE).expect("transition");
        assert!(found.check_from(ChequeStatus::Bounced).is_ok());
        assert!(found.check_from(ChequeStatus::LegalCase).is_ok());
        assert!(found.check_from(ChequeStatus::Deposited).is_err());
        assert!(found.check_from(ChequeStatus::Replaced).is_err());
        assert_eq!(found.to_state(), Some("replaced"));
    }

    #[test]
    fn a_part_payment_is_below_the_open_amount() {
        let cheque = bounced(1_400_000);
        assert_eq!(
            settle_problem(&cheque, Some(&input(PaymentMethod::Cash, 400_000))),
            None
        );
        assert_eq!(
            settle_problem(&cheque, Some(&input(PaymentMethod::Transfer, 1_399_999))),
            None
        );
        assert!(settle_problem(&cheque, Some(&input(PaymentMethod::Cash, 1_400_000))).is_some());
        assert!(settle_problem(&cheque, Some(&input(PaymentMethod::Cash, 0))).is_some());
        assert!(settle_problem(&cheque, Some(&input(PaymentMethod::Cash, -5))).is_some());
        assert!(settle_problem(&cheque, None).is_some());
    }

    #[test]
    fn a_part_payment_is_cash_transfer_or_card_on_a_received_cheque() {
        let cheque = bounced(1_400_000);
        for method in [
            PaymentMethod::Cheque,
            PaymentMethod::Gateway,
            PaymentMethod::SetOff,
        ] {
            assert!(
                settle_problem(&cheque, Some(&input(method, 100))).is_some(),
                "{method:?}"
            );
        }
        assert!(
            settle_problem(
                &cheque,
                Some(&SettleInput {
                    method: None,
                    amount: 100
                })
            )
            .is_some()
        );
        let issued = Cheque {
            direction: Direction::Out,
            ..cheque
        };
        assert!(settle_problem(&issued, Some(&input(PaymentMethod::Cash, 100))).is_some());
    }

    #[test]
    fn replacements_must_cover_the_open_amount() {
        let cheque = bounced(1_000_000);
        assert_eq!(replace_problem(&cheque, Some(1_000_000)), None);
        assert_eq!(replace_problem(&cheque, Some(1_200_000)), None);
        assert!(replace_problem(&cheque, Some(900_000)).is_some());
        assert!(replace_problem(&cheque, None).is_some());
        assert!(replace_problem(&bounced(0), Some(5)).is_some());
        let message = replace_problem(&cheque, Some(900_000)).expect("refused");
        assert!(message.contains("FIN.ACL.02"), "{message}");
    }

    #[renox::test]
    async fn the_replace_guard_reads_the_covered_amount() {
        let machine = Cheque::machine();
        let found = machine.find(REPLACE).expect("transition");
        let ctx = |covered| {
            TransitionCtx::new(None)
                .with_input(renox::serde_json::to_value(ReplaceInput { covered }).expect("json"))
        };
        let cheque = bounced(1_000_000);
        assert!(found.check_guards(&cheque, &ctx(1_000_000)).await.is_ok());
        assert!(matches!(
            found.check_guards(&cheque, &ctx(900_000)).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            found.check_guards(&cheque, &TransitionCtx::new(None)).await,
            Err(TransitionError::Guard(_))
        ));
    }

    fn candidate() -> Cheque {
        Cheque {
            id: 2,
            party_id: 7,
            amount: 1_000_000,
            currency: "AED".into(),
            status: ChequeStatus::Received,
            ..Default::default()
        }
    }

    #[test]
    fn a_replacement_cheque_is_good_while_it_is_not_bounced_or_ended() {
        let original = bounced(1_000_000);
        assert_eq!(replacement_cheque_problem(&original, &candidate()), None);
        for status in [
            ChequeStatus::Deposited,
            ChequeStatus::Cleared,
            ChequeStatus::DueForDeposit,
        ] {
            let ok = Cheque {
                status,
                ..candidate()
            };
            assert_eq!(
                replacement_cheque_problem(&original, &ok),
                None,
                "{status:?}"
            );
        }
        for status in [
            ChequeStatus::Bounced,
            ChequeStatus::LegalCase,
            ChequeStatus::Replaced,
            ChequeStatus::WrittenOff,
            ChequeStatus::Returned,
        ] {
            let bad = Cheque {
                status,
                ..candidate()
            };
            assert!(
                replacement_cheque_problem(&original, &bad).is_some(),
                "{status:?}"
            );
        }
    }

    #[test]
    fn a_replacement_cheque_is_of_the_same_party_direction_and_currency() {
        let original = bounced(1_000_000);
        let same = Cheque {
            id: 1,
            ..candidate()
        };
        assert!(replacement_cheque_problem(&original, &same).is_some());
        let party = Cheque {
            party_id: 8,
            ..candidate()
        };
        assert!(replacement_cheque_problem(&original, &party).is_some());
        let issued = Cheque {
            direction: Direction::Out,
            ..candidate()
        };
        assert!(replacement_cheque_problem(&original, &issued).is_some());
        let usd = Cheque {
            currency: "USD".into(),
            ..candidate()
        };
        assert!(replacement_cheque_problem(&original, &usd).is_some());
    }

    #[test]
    fn a_replacement_payment_is_received_free_and_of_the_same_party_and_direction() {
        let original = bounced(1_000_000);
        let payment = Payment {
            id: 3,
            party_id: 7,
            amount: 1_000_000,
            currency: "AED".into(),
            status: wl_kernel::payments::PaymentStatus::Received,
            ..Default::default()
        };
        assert_eq!(
            replacement_payment_problem(&original, &payment, 1_000_000),
            None
        );
        assert!(replacement_payment_problem(&original, &payment, 0).is_some());
        let out = Payment {
            direction: Direction::Out,
            ..payment.clone()
        };
        assert!(replacement_payment_problem(&original, &out, 1).is_some());
        let other = Payment {
            party_id: 9,
            ..payment.clone()
        };
        assert!(replacement_payment_problem(&original, &other, 1).is_some());
        let reversed = Payment {
            status: wl_kernel::payments::PaymentStatus::Reversed,
            ..payment
        };
        assert!(replacement_payment_problem(&original, &reversed, 1).is_some());
    }

    #[test]
    fn the_hand_back_receipt_numbers_are_yearly_and_not_per_entity() {
        assert_eq!(HAND_BACK_SERIES.prefix, "CHR");
        assert!(!HAND_BACK_SERIES.per_entity);
        assert_eq!(HAND_BACK_SERIES.reset, Reset::Yearly);
    }
}
