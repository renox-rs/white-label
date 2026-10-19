//! The deductions of a deposit, published to the party, disputed by the party
//! and answered by staff (wl-payments §4 *Deposit* "Deductions dispute flag",
//! §5.4 steps 2 to 4, §7, issue #108).
//!
//! When the settlement of a deposit is pending, a user with `deposits.manage`
//! itemises the deductions with evidence and **publishes** them: a numbered
//! version ([`DepositDeduction`]) with its lines ([`DeductionLine`]). A
//! published version is never changed; a revision publishes the next one. A
//! forfeiture is published the same way, as the itemised deductions it is
//! applied to or as one line for the whole deposit with the reason.
//!
//! Where the vertical allows it (a setting, off by default,
//! `WlPayments::deductions_dispute`), the party may **dispute** one or more
//! lines of the latest version until the end of the dispute window (14 days
//! after publication, to 23:59 Asia/Dubai of the last day): [`dispute`], called
//! by the portal through the kernel's
//! [`DepositDeductions`](wl_kernel::deposits::DepositDeductions). A dispute
//! row per line sets the **flag** (it is not a state): while one is open
//! `refund` and `forfeit` wait ([`gate_problem`]). Staff **answer** with
//! evidence ([`answer`]): the lines named in a change are revised into a new
//! published version (the window starts again), every other disputed line is
//! upheld; an answer with no change upholds all disputed lines, which clears
//! the flag without restarting the window.
//!
//! `refund` and `forfeit` run only after the window of the latest published
//! version has passed, even when no line was disputed, so the party always has
//! the whole window. With the setting off there is no window and no
//! publication is required. A legal case on the source agreement keeps the flag
//! until a staff member records the decision ([`set_legal_case`]).
//!
//! Decisions on what the spec leaves open: a revision removes a line by
//! setting its amount to 0; the response window (7 days) is recorded in the
//! notice to the staff, and its escalation is the vertical's (01
//! `PRP.RUL.11`); the party hears of a publication and of an answer by notice
//! (not while blocked); the staff told of a dispute are the holders of the
//! accountant role; a version may have no line (no deductions) and is then
//! published as such, so the window runs.

use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::deposits::{
    DeductionKind, DeductionLineView, DepositDeductions, DepositDeductionsDisputed,
    DepositDeductionsPublished, DepositDisputeAnswered, DisputeAnswer, DisputeDeductions,
    PublishedDeductions,
};
use wl_kernel::documents::today;
use wl_kernel::PartyId;

use crate::config_of;
use crate::deposit::{Deposit, DepositStatus};
use crate::notices;
use crate::permissions;

/// One published version of the deductions of a deposit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_deposit_deductions",
    unique(deposit_id, version),
    index(deposit_id)
)]
pub struct DepositDeduction {
    /// The primary key.
    pub id: i64,
    /// The deposit.
    pub deposit_id: i64,
    /// 1, 2, ...: each revision publishes the next.
    pub version: i32,
    /// Deductions, or a forfeiture.
    pub kind: DeductionKind,
    /// The reason of a forfeiture.
    pub reason: Option<String>,
    /// The day it was published (Asia/Dubai).
    pub published_on: NaiveDate,
    /// The last day of the dispute window; `None` without a dispute setting.
    pub window_ends_on: Option<NaiveDate>,
    /// The user who published it.
    pub published_by: Option<i64>,
    /// What it deducts in all, in fils.
    pub total: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One line of a published version.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_payments_deposit_deduction_lines", index(deduction_id))]
pub struct DeductionLine {
    /// The primary key.
    pub id: i64,
    /// The version it belongs to.
    pub deduction_id: i64,
    /// The deposit.
    pub deposit_id: i64,
    /// 0, 1, ...: the order it is shown in.
    pub position: i32,
    /// What it is for, e.g. "Repainting".
    pub description: String,
    /// The amount, in fils; above 0.
    pub amount: i64,
    /// The evidence reference (inspection photos, a quote).
    pub evidence: String,
    /// Whether the deduction is invoiced to the party when billable.
    pub billable: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Where a dispute of a line stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DisputeStatus {
    /// The flag is set: staff have not answered.
    #[default]
    Open,
    /// The deductions were revised into a new version.
    Revised,
    /// The deductions stand.
    Upheld,
}

/// The party's dispute of one line.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_payments_deposit_disputes", index(deposit_id, status))]
pub struct DepositDispute {
    /// The primary key.
    pub id: i64,
    /// The deposit.
    pub deposit_id: i64,
    /// The version disputed.
    pub deduction_id: i64,
    /// The line disputed.
    pub line_id: i64,
    /// The party's words.
    pub comment: String,
    /// The day it was raised (Asia/Dubai).
    pub raised_on: NaiveDate,
    /// Where it stands.
    pub status: DisputeStatus,
    /// Staff's answer.
    pub response: Option<String>,
    /// The evidence staff gave with it.
    pub evidence: Option<String>,
    /// The user who answered.
    pub answered_by: Option<i64>,
    /// The day it was answered.
    pub answered_on: Option<NaiveDate>,
    /// The version a revision published.
    pub revised_in: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DepositDeduction {
    /// The versions of `deposit_id`, oldest first.
    pub async fn of_deposit(db: &Db, deposit_id: i64) -> Result<Vec<DepositDeduction>> {
        DepositDeduction::query()
            .where_(DepositDeduction::DEPOSIT_ID.eq(deposit_id))
            .order_by(DepositDeduction::VERSION)
            .get(db)
            .await
    }

    /// The latest version of `deposit_id`.
    pub async fn latest(db: &Db, deposit_id: i64) -> Result<Option<DepositDeduction>> {
        Ok(Self::of_deposit(db, deposit_id).await?.pop())
    }

    /// The lines of this version, in order.
    pub async fn lines(&self, db: &Db) -> Result<Vec<DeductionLine>> {
        DeductionLine::query()
            .where_(DeductionLine::DEDUCTION_ID.eq(self.id))
            .order_by(DeductionLine::POSITION)
            .get(db)
            .await
    }
}

impl DepositDispute {
    /// The disputes of `deposit_id`, oldest first.
    pub async fn of_deposit(db: &Db, deposit_id: i64) -> Result<Vec<DepositDispute>> {
        DepositDispute::query()
            .where_(DepositDispute::DEPOSIT_ID.eq(deposit_id))
            .order_by(DepositDispute::ID)
            .get(db)
            .await
    }

    /// The open disputes of `deposit_id`: the flag is set while there is one.
    pub async fn open_of_deposit(db: &Db, deposit_id: i64) -> Result<Vec<DepositDispute>> {
        DepositDispute::query()
            .where_(DepositDispute::DEPOSIT_ID.eq(deposit_id))
            .where_(DepositDispute::STATUS.eq(DisputeStatus::Open))
            .order_by(DepositDispute::ID)
            .get(db)
            .await
    }
}

/// What `refund` and `forfeit` need to know about the deductions, gathered by
/// the deposit service ([`gate_of`]) and carried to the guards.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeductionGate {
    /// Whether the party may dispute (the setting).
    pub dispute_on: bool,
    /// Today (Asia/Dubai).
    pub today: NaiveDate,
    /// Whether a version was published.
    pub published: bool,
    /// Whether the latest published version is a forfeiture.
    pub forfeiture: bool,
    /// The last day of the dispute window of the latest version.
    pub window_ends_on: Option<NaiveDate>,
    /// How many disputed lines are unanswered.
    pub open_disputes: i64,
    /// Whether a legal case on the source agreement holds the flag.
    pub legal_case: bool,
}

/// Why `refund` (or `forfeit`) must wait for the deductions, if it must.
pub(crate) fn gate_problem(gate: &DeductionGate, forfeit: bool) -> Option<String> {
    if gate.legal_case {
        return Some(
            "A legal case on the source agreement keeps the deductions dispute open until its decision is recorded."
                .to_owned(),
        );
    }
    if gate.open_disputes > 0 {
        return Some(
            "A deductions dispute is open: answer every disputed line first (revise or uphold)."
                .to_owned(),
        );
    }
    if !gate.dispute_on {
        return None;
    }
    if !gate.published || (forfeit && !gate.forfeiture) {
        return Some(if forfeit {
            "The forfeiture must be published to the party as the deductions first.".to_owned()
        } else {
            "The deductions must be published to the party first.".to_owned()
        });
    }
    if !forfeit && gate.forfeiture {
        return Some(
            "The latest published version is a forfeiture: publish the deductions to refund."
                .to_owned(),
        );
    }
    if let Some(end) = gate.window_ends_on
        && gate.today <= end
    {
        return Some(format!(
            "The dispute window of the latest published version ends on {}: wait until it has passed.",
            end.format("%Y-%m-%d")
        ));
    }
    None
}

/// The gate for `deposit` as it is today.
///
/// # Errors
///
/// The database's errors.
pub async fn gate_of(state: &AppState, deposit: &Deposit) -> Result<DeductionGate> {
    let latest = DepositDeduction::latest(&state.db, deposit.id).await?;
    Ok(DeductionGate {
        dispute_on: config_of(state).deductions_dispute,
        today: today(),
        published: latest.is_some(),
        forfeiture: latest
            .as_ref()
            .is_some_and(|d| d.kind == DeductionKind::Forfeiture),
        window_ends_on: latest.as_ref().and_then(|d| d.window_ends_on),
        open_disputes: DepositDispute::open_of_deposit(&state.db, deposit.id)
            .await?
            .len() as i64,
        legal_case: deposit.dispute_legal_case,
    })
}

/// One line the user gives to publish.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewLine {
    /// What it is for.
    pub description: String,
    /// The amount, in fils; above 0.
    pub amount: i64,
    /// The evidence reference; required.
    pub evidence: String,
    /// Whether it is invoiced to the party.
    pub billable: bool,
}

/// What a user publishes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PublishRequest {
    /// The itemised lines. For a forfeiture, the deductions it is applied to;
    /// empty for one line for the whole deposit.
    pub lines: Vec<NewLine>,
    /// Set to publish a forfeiture, with its reason.
    pub forfeiture_reason: Option<String>,
}

/// The version `request` publishes for a deposit holding `held` fils: its
/// kind, reason and lines, or what is wrong with it.
pub(crate) fn prepare(
    request: &PublishRequest,
    held: i64,
) -> std::result::Result<(DeductionKind, Option<String>, Vec<NewLine>), String> {
    let reason = request
        .forfeiture_reason
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned);
    if request.forfeiture_reason.is_some() && reason.is_none() {
        return Err("A forfeiture needs a reason.".to_owned());
    }
    let mut lines: Vec<NewLine> = request
        .lines
        .iter()
        .map(|line| NewLine {
            description: line.description.trim().to_owned(),
            evidence: line.evidence.trim().to_owned(),
            ..line.clone()
        })
        .collect();
    if let Some(reason) = &reason
        && lines.is_empty()
    {
        // One line for the whole deposit, with the reason as its evidence.
        lines.push(NewLine {
            description: "Forfeiture of the deposit".to_owned(),
            amount: held,
            evidence: reason.clone(),
            billable: false,
        });
    }
    for (i, line) in lines.iter().enumerate() {
        let n = i + 1;
        if line.description.is_empty() {
            return Err(format!("Line {n} needs a description."));
        }
        if line.amount <= 0 {
            return Err(format!("Line {n} needs an amount above 0."));
        }
        if line.evidence.is_empty() {
            return Err(format!("Line {n} needs its evidence."));
        }
    }
    let kind = if reason.is_some() {
        DeductionKind::Forfeiture
    } else {
        DeductionKind::Deductions
    };
    Ok((kind, reason, lines))
}

fn guard(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

async fn reload(state: &AppState, deposit: &Deposit) -> std::result::Result<Deposit, TransitionError> {
    Deposit::find_or_404(&state.db, deposit.id)
        .await
        .map_err(TransitionError::Failed)
}

/// The window's last day for a version published on `published_on`, when the
/// party may dispute.
pub fn window_end(published_on: NaiveDate, window_days: i64) -> NaiveDate {
    published_on + TimeDelta::days(window_days)
}

/// A version to insert.
struct NewVersion<'a> {
    version: i32,
    kind: DeductionKind,
    reason: Option<String>,
    lines: &'a [NewLine],
    by: Option<i64>,
    /// The disputes this version answers, with their new status.
    answered: Vec<(i64, DisputeStatus)>,
    /// The response and the evidence given with the answer.
    answer: Option<(&'a str, &'a str)>,
}

/// Inserts `new` as a version of `deposit` in a transaction; returns it.
async fn insert_version(
    state: &AppState,
    deposit: &Deposit,
    new: NewVersion<'_>,
) -> Result<DepositDeduction> {
    let config = config_of(state);
    let published_on = today();
    let mut deduction = DepositDeduction {
        deposit_id: deposit.id,
        version: new.version,
        kind: new.kind,
        reason: new.reason,
        published_on,
        window_ends_on: config
            .deductions_dispute
            .then(|| window_end(published_on, config.dispute_window_days)),
        published_by: new.by,
        total: new.lines.iter().map(|l| l.amount).sum(),
        ..Default::default()
    };
    let mut tx = state.db.begin().await?;
    deduction.insert(&mut tx).await?;
    for (i, line) in new.lines.iter().enumerate() {
        let mut row = DeductionLine {
            deduction_id: deduction.id,
            deposit_id: deposit.id,
            position: i as i32,
            description: line.description.clone(),
            amount: line.amount,
            evidence: line.evidence.clone(),
            billable: line.billable,
            ..Default::default()
        };
        row.insert(&mut tx).await?;
    }
    let (response, evidence) = new.answer.unwrap_or_default();
    for (dispute_id, status) in &new.answered {
        DepositDispute::query()
            .where_(DepositDispute::ID.eq(*dispute_id))
            .update(
                &mut tx,
                &[
                    ("status", &status.as_str().to_owned()),
                    ("response", &Some(response.to_owned())),
                    ("evidence", &Some(evidence.to_owned())),
                    ("answered_by", &new.by),
                    ("answered_on", &Some(published_on)),
                    ("revised_in", &Some(deduction.id)),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    tx.commit().await?;
    Ok(deduction)
}

/// Emits `DepositDeductionsPublished` and tells the party.
async fn announce_published(
    state: &AppState,
    deposit: &Deposit,
    deduction: &DepositDeduction,
) -> Result<()> {
    state
        .emit(DepositDeductionsPublished {
            deposit: deposit.deposit_id(),
            source: deposit.source(),
            party: deposit.party(),
            version: deduction.version,
            kind: deduction.kind,
            total: deduction.total,
            published_on: deduction.published_on,
            window_ends_on: deduction.window_ends_on,
        })
        .await?;
    notices::deposit_deductions_published(state, deposit, deduction).await
}

/// `user` publishes the deductions (or the forfeiture) of the deposit whose
/// settlement is pending as the next version, shown to the party in the portal.
/// The window to dispute starts today.
///
/// # Errors
///
/// `Forbidden` without `deposits.manage`; `NotFrom` unless
/// `settlement_pending`; `Guard` (422) for a line without a description, an
/// amount above 0 or its evidence, a forfeiture without a reason, or while a
/// dispute is open (answer it, which publishes the revision).
pub async fn publish(
    state: &AppState,
    user: &AuthUser,
    deposit: &Deposit,
    request: &PublishRequest,
) -> std::result::Result<DepositDeduction, TransitionError> {
    if !user.has_permission(permissions::DEPOSITS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let deposit = reload(state, deposit).await?;
    if deposit.status != DepositStatus::SettlementPending {
        return Err(TransitionError::NotFrom(deposit.status.as_str().to_owned()));
    }
    if !DepositDispute::open_of_deposit(&state.db, deposit.id)
        .await?
        .is_empty()
    {
        return Err(guard(
            "A dispute is open: answer it (revise or uphold) instead of publishing again.",
        ));
    }
    let (kind, reason, lines) = prepare(request, deposit.held_amount).map_err(guard)?;
    let version = DepositDeduction::latest(&state.db, deposit.id)
        .await?
        .map_or(1, |d| d.version + 1);
    let new = NewVersion {
        version,
        kind,
        reason,
        lines: &lines,
        by: Some(user.id),
        answered: Vec::new(),
        answer: None,
    };
    let deduction = insert_version(state, &deposit, new).await?;
    announce_published(state, &deposit, &deduction).await?;
    Ok(deduction)
}

/// The latest version as the party sees it.
async fn published_of(
    state: &AppState,
    deposit: &Deposit,
) -> Result<Option<PublishedDeductions>> {
    if deposit.status != DepositStatus::SettlementPending {
        return Ok(None);
    }
    let Some(latest) = DepositDeduction::latest(&state.db, deposit.id).await? else {
        return Ok(None);
    };
    let lines = latest.lines(&state.db).await?;
    let open = DepositDispute::open_of_deposit(&state.db, deposit.id).await?;
    let can_dispute = config_of(state).deductions_dispute
        && latest.window_ends_on.is_some_and(|end| today() <= end);
    Ok(Some(PublishedDeductions {
        deposit: deposit.deposit_id(),
        source: deposit.source(),
        version: latest.version,
        kind: latest.kind,
        reason: latest.reason.clone(),
        currency: deposit.currency.clone(),
        published_on: latest.published_on,
        window_ends_on: latest.window_ends_on,
        lines: lines
            .iter()
            .map(|line| DeductionLineView {
                id: line.id,
                description: line.description.clone(),
                amount: line.amount,
                evidence: line.evidence.clone(),
            })
            .collect(),
        disputed_lines: open.iter().map(|d| d.line_id).collect(),
        can_dispute,
    }))
}

/// `party` disputes lines of the latest published version of the deposit. The
/// flag is set: `refund` and `forfeit` wait until staff answer. The staff are
/// told and `DepositDeductionsDisputed` is emitted.
///
/// # Errors
///
/// 404 for a deposit of another party; `Guard` (422) when the setting is
/// off, the deposit is not `settlement_pending`, the version is not the latest
/// published one, the window has passed, no line is named, a line is not on the
/// version or is disputed already, or the comment is blank.
pub async fn dispute(
    state: &AppState,
    party: PartyId,
    request: &DisputeDeductions,
) -> std::result::Result<Vec<DepositDispute>, TransitionError> {
    let Some(deposit) = Deposit::find(&state.db, request.deposit.0).await? else {
        return Err(TransitionError::Failed(Error::NotFound));
    };
    if deposit.party() != party {
        return Err(TransitionError::Failed(Error::NotFound));
    }
    if !config_of(state).deductions_dispute {
        return Err(guard("The deductions cannot be disputed."));
    }
    if deposit.status != DepositStatus::SettlementPending {
        return Err(guard("The deposit is not being settled."));
    }
    let Some(latest) = DepositDeduction::latest(&state.db, deposit.id).await? else {
        return Err(guard("No deductions are published."));
    };
    if latest.version != request.version {
        return Err(guard(
            "These are not the latest published deductions: reload and dispute again.",
        ));
    }
    let today = today();
    if latest.window_ends_on.is_none_or(|end| today > end) {
        return Err(guard("The dispute window has passed."));
    }
    let comment = request.comment.trim();
    if comment.is_empty() {
        return Err(guard("Say why the lines are disputed."));
    }
    let mut ids: Vec<i64> = request.lines.clone();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Err(guard("Choose the lines to dispute."));
    }
    let lines = latest.lines(&state.db).await?;
    let open = DepositDispute::open_of_deposit(&state.db, deposit.id).await?;
    for id in &ids {
        if !lines.iter().any(|l| l.id == *id) {
            return Err(guard("A line is not on the published deductions."));
        }
        if open.iter().any(|d| d.line_id == *id) {
            return Err(guard("A line is disputed already."));
        }
    }
    let mut tx = state.db.begin().await?;
    let mut rows = Vec::new();
    for id in &ids {
        let mut row = DepositDispute {
            deposit_id: deposit.id,
            deduction_id: latest.id,
            line_id: *id,
            comment: comment.to_owned(),
            raised_on: today,
            status: DisputeStatus::Open,
            ..Default::default()
        };
        row.insert(&mut tx).await?;
        rows.push(row);
    }
    tx.commit().await?;
    state
        .emit(DepositDeductionsDisputed {
            deposit: deposit.deposit_id(),
            source: deposit.source(),
            party,
            version: latest.version,
            lines: ids,
            raised_on: today,
        })
        .await?;
    notices::deposit_disputed(state, &deposit, &latest, rows.len()).await?;
    Ok(rows)
}

/// A change to a disputed line in a revision.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LineChange {
    /// The line of the latest version.
    pub line_id: i64,
    /// The new amount, in fils; 0 removes the line.
    pub amount: i64,
    /// The new description, if it changes.
    pub description: Option<String>,
    /// The new evidence reference, if it changes.
    pub evidence: Option<String>,
}

/// What staff answer a dispute with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AnswerRequest {
    /// The answer to the party; required.
    pub response: String,
    /// The evidence for it (inspection photos, quotes); required.
    pub evidence: String,
    /// The revised lines. Empty: the deductions are upheld.
    pub changes: Vec<LineChange>,
}

/// The lines of the revised version: `current` with `changes` applied; or what
/// is wrong with them.
pub(crate) fn revised_lines(
    current: &[DeductionLine],
    changes: &[LineChange],
) -> std::result::Result<Vec<NewLine>, String> {
    for change in changes {
        if !current.iter().any(|l| l.id == change.line_id) {
            return Err("A changed line is not on the latest published version.".to_owned());
        }
        if change.amount < 0 {
            return Err("A revised amount cannot be below 0.".to_owned());
        }
    }
    let mut lines = Vec::new();
    for line in current {
        let change = changes.iter().find(|c| c.line_id == line.id);
        let amount = change.map_or(line.amount, |c| c.amount);
        if amount == 0 {
            continue;
        }
        lines.push(NewLine {
            description: change
                .and_then(|c| c.description.clone())
                .map(|d| d.trim().to_owned())
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| line.description.clone()),
            amount,
            evidence: change
                .and_then(|c| c.evidence.clone())
                .map(|e| e.trim().to_owned())
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| line.evidence.clone()),
            billable: line.billable,
        });
    }
    Ok(lines)
}

/// `user` answers the open disputes of the deposit with `request.response` and
/// its evidence. With changes, the changed lines are revised into the next
/// published version (the window starts again today) and every other disputed
/// line is upheld; without, all disputed lines are upheld, which clears the
/// flag without restarting the window. The party is told and
/// `DepositDisputeAnswered` is emitted.
///
/// # Errors
///
/// `Forbidden` without `deposits.manage`; `NotFrom` unless
/// `settlement_pending`; `Guard` (422) without a response or evidence, with no
/// open dispute, or for a change to a line that is not on the latest version
/// or an amount below 0.
pub async fn answer(
    state: &AppState,
    user: &AuthUser,
    deposit: &Deposit,
    request: &AnswerRequest,
) -> std::result::Result<DepositDispute, TransitionError> {
    if !user.has_permission(permissions::DEPOSITS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let deposit = reload(state, deposit).await?;
    if deposit.status != DepositStatus::SettlementPending {
        return Err(TransitionError::NotFrom(deposit.status.as_str().to_owned()));
    }
    let response = request.response.trim();
    let evidence = request.evidence.trim();
    if response.is_empty() {
        return Err(guard("Answer the party: a response is required."));
    }
    if evidence.is_empty() {
        return Err(guard("The answer needs its evidence."));
    }
    let open = DepositDispute::open_of_deposit(&state.db, deposit.id).await?;
    let Some(first) = open.first().cloned() else {
        return Err(guard("There is no open dispute to answer."));
    };
    let latest = DepositDeduction::latest(&state.db, deposit.id)
        .await?
        .ok_or_else(|| guard("No deductions are published."))?;
    let answered_version = latest.version;
    let party = deposit.party();
    if request.changes.is_empty() {
        let today = today();
        for dispute in &open {
            DepositDispute::query()
                .where_(DepositDispute::ID.eq(dispute.id))
                .update(
                    &state.db,
                    &[
                        ("status", &DisputeStatus::Upheld.as_str().to_owned()),
                        ("response", &Some(response.to_owned())),
                        ("evidence", &Some(evidence.to_owned())),
                        ("answered_by", &Some(user.id)),
                        ("answered_on", &Some(today)),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
        }
        let deposit = reload(state, &deposit).await?;
        state
            .emit(DepositDisputeAnswered {
                deposit: deposit.deposit_id(),
                source: deposit.source(),
                party,
                version: answered_version,
                answer: DisputeAnswer::Upheld,
                new_version: None,
                still_open: deposit.dispute_legal_case,
            })
            .await?;
        notices::deposit_dispute_answered(state, &deposit, DisputeAnswer::Upheld, response).await?;
    } else {
        let lines = latest.lines(&state.db).await?;
        let revised = revised_lines(&lines, &request.changes).map_err(guard)?;
        let mut statuses: Vec<(i64, DisputeStatus)> = Vec::new();
        for dispute in &open {
            let changed = request.changes.iter().any(|c| c.line_id == dispute.line_id);
            statuses.push((
                dispute.id,
                if changed {
                    DisputeStatus::Revised
                } else {
                    DisputeStatus::Upheld
                },
            ));
        }
        let new = NewVersion {
            version: answered_version + 1,
            kind: latest.kind,
            reason: latest.reason.clone(),
            lines: &revised,
            by: Some(user.id),
            answered: statuses,
            answer: Some((response, evidence)),
        };
        let deduction = insert_version(state, &deposit, new).await?;
        announce_published(state, &deposit, &deduction).await?;
        state
            .emit(DepositDisputeAnswered {
                deposit: deposit.deposit_id(),
                source: deposit.source(),
                party,
                version: answered_version,
                answer: DisputeAnswer::Revised,
                new_version: Some(deduction.version),
                still_open: deposit.dispute_legal_case,
            })
            .await?;
        notices::deposit_dispute_answered(state, &deposit, DisputeAnswer::Revised, response)
            .await?;
    }
    DepositDispute::find_or_404(&state.db, first.id)
        .await
        .map_err(TransitionError::Failed)
}

/// `user` records that a legal case on the source agreement of the deposit is
/// open (`open`) or that its decision is recorded (`!open`): while it is open
/// the deductions dispute flag stays and `refund` and `forfeit` wait.
///
/// # Errors
///
/// `Forbidden` without `deposits.manage`; `NotFrom` unless
/// `settlement_pending`.
pub async fn set_legal_case(
    state: &AppState,
    user: &AuthUser,
    deposit: &Deposit,
    open: bool,
) -> std::result::Result<Deposit, TransitionError> {
    if !user.has_permission(permissions::DEPOSITS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let deposit = reload(state, deposit).await?;
    if deposit.status != DepositStatus::SettlementPending {
        return Err(TransitionError::NotFrom(deposit.status.as_str().to_owned()));
    }
    // Not the status column, so outside the machine's guard.
    Deposit::query()
        .where_(Deposit::ID.eq(deposit.id))
        .update(
            &state.db,
            &[
                ("dispute_legal_case", &open),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    reload(state, &deposit).await
}

/// The module's [`DepositDeductions`] for the portal.
#[derive(Debug, Clone, Copy, Default)]
pub struct Deductions;

impl DepositDeductions for Deductions {
    fn published_for<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> wl_kernel::deposits::DepositFuture<'a, Vec<PublishedDeductions>> {
        Box::pin(async move {
            let mut shown = Vec::new();
            for deposit in Deposit::of_party(&state.db, party).await? {
                if let Some(published) = published_of(state, &deposit).await? {
                    shown.push(published);
                }
            }
            Ok(shown)
        })
    }

    fn dispute<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
        request: DisputeDeductions,
    ) -> wl_kernel::deposits::DepositFuture<'a, ()> {
        Box::pin(async move {
            dispute(state, party, &request)
                .await
                .map(|_| ())
                .map_err(TransitionError::into_error)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, 3, day).unwrap()
    }

    fn line(description: &str, amount: i64, evidence: &str) -> NewLine {
        NewLine {
            description: description.into(),
            amount,
            evidence: evidence.into(),
            billable: false,
        }
    }

    fn gate() -> DeductionGate {
        DeductionGate {
            dispute_on: true,
            today: day(16),
            published: true,
            forfeiture: false,
            window_ends_on: Some(day(15)),
            open_disputes: 0,
            legal_case: false,
        }
    }

    #[test]
    fn the_window_ends_n_days_after_the_day_of_publication() {
        assert_eq!(window_end(day(1), 14), day(15));
        assert_eq!(window_end(day(10), 14), day(24));
    }

    #[test]
    fn a_refund_waits_for_the_whole_window_even_without_a_dispute() {
        let mut g = gate();
        g.today = day(5);
        assert!(gate_problem(&g, false).is_some());
        g.today = day(15);
        assert!(gate_problem(&g, false).is_some(), "the last day is still open");
        g.today = day(16);
        assert_eq!(gate_problem(&g, false), None);
    }

    #[test]
    fn a_revision_restarts_the_window() {
        let mut g = gate();
        g.window_ends_on = Some(day(24));
        g.today = day(24);
        assert!(gate_problem(&g, false).is_some());
        g.today = day(25);
        assert_eq!(gate_problem(&g, false), None);
    }

    #[test]
    fn an_open_dispute_or_a_legal_case_holds_the_flag() {
        let mut g = gate();
        g.open_disputes = 1;
        let message = gate_problem(&g, false).expect("refused");
        assert!(message.contains("dispute is open"), "{message}");
        g.open_disputes = 0;
        g.legal_case = true;
        assert!(gate_problem(&g, false).unwrap().contains("legal case"));
        // Whatever the setting says.
        g.dispute_on = false;
        assert!(gate_problem(&g, true).is_some());
    }

    #[test]
    fn with_the_setting_off_there_is_no_window_and_no_publication() {
        let g = DeductionGate::default();
        assert_eq!(gate_problem(&g, false), None);
        assert_eq!(gate_problem(&g, true), None);
    }

    #[test]
    fn the_deductions_must_be_published_with_the_setting_on() {
        let mut g = gate();
        g.published = false;
        g.window_ends_on = None;
        assert!(gate_problem(&g, false).unwrap().contains("published"));
        assert!(gate_problem(&g, true).unwrap().contains("forfeiture"));
    }

    #[test]
    fn a_forfeit_needs_a_published_forfeiture_and_a_refund_published_deductions() {
        let mut g = gate();
        assert!(gate_problem(&g, true).unwrap().contains("forfeiture must be published"));
        g.forfeiture = true;
        assert_eq!(gate_problem(&g, true), None);
        assert!(gate_problem(&g, false).unwrap().contains("is a forfeiture"));
    }

    #[test]
    fn a_published_version_needs_described_evidenced_lines_above_zero() {
        let ok = PublishRequest {
            lines: vec![line("Repainting", 250_000, "photos-12")],
            forfeiture_reason: None,
        };
        let (kind, reason, lines) = prepare(&ok, 800_000).expect("ok");
        assert_eq!(kind, DeductionKind::Deductions);
        assert_eq!(reason, None);
        assert_eq!(lines.len(), 1);
        for bad in [
            line("", 100, "photos"),
            line("Repainting", 0, "photos"),
            line("Repainting", 100, "  "),
        ] {
            let request = PublishRequest {
                lines: vec![bad],
                forfeiture_reason: None,
            };
            assert!(prepare(&request, 800_000).is_err());
        }
        // No line is a version without deductions.
        assert!(prepare(&PublishRequest::default(), 800_000).unwrap().2.is_empty());
    }

    #[test]
    fn a_forfeiture_has_a_reason_and_defaults_to_one_line_for_the_whole_deposit() {
        let request = PublishRequest {
            lines: Vec::new(),
            forfeiture_reason: Some("  Abandoned the unit  ".into()),
        };
        let (kind, reason, lines) = prepare(&request, 800_000).expect("ok");
        assert_eq!(kind, DeductionKind::Forfeiture);
        assert_eq!(reason.as_deref(), Some("Abandoned the unit"));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].amount, 800_000);
        assert_eq!(lines[0].evidence, "Abandoned the unit");
        let blank = PublishRequest {
            lines: Vec::new(),
            forfeiture_reason: Some("  ".into()),
        };
        assert!(prepare(&blank, 800_000).is_err());
    }

    fn current() -> Vec<DeductionLine> {
        vec![
            DeductionLine {
                id: 1,
                description: "Repainting".into(),
                amount: 250_000,
                evidence: "photos-1".into(),
                ..Default::default()
            },
            DeductionLine {
                id: 2,
                description: "Cleaning".into(),
                amount: 30_000,
                evidence: "photos-2".into(),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn a_revision_changes_the_named_lines_and_keeps_the_others() {
        let changes = [LineChange {
            line_id: 1,
            amount: 80_000,
            ..Default::default()
        }];
        let lines = revised_lines(&current(), &changes).expect("ok");
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].description.as_str(), lines[0].amount), ("Repainting", 80_000));
        assert_eq!(lines[0].evidence, "photos-1");
        assert_eq!(lines[1].amount, 30_000);
    }

    #[test]
    fn a_revised_amount_of_zero_removes_the_line() {
        let changes = [LineChange {
            line_id: 2,
            amount: 0,
            ..Default::default()
        }];
        let lines = revised_lines(&current(), &changes).expect("ok");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].description, "Repainting");
    }

    #[test]
    fn a_change_to_a_line_not_on_the_version_or_below_zero_is_refused() {
        let unknown = [LineChange {
            line_id: 9,
            amount: 10,
            ..Default::default()
        }];
        assert!(revised_lines(&current(), &unknown).is_err());
        let negative = [LineChange {
            line_id: 1,
            amount: -1,
            ..Default::default()
        }];
        assert!(revised_lines(&current(), &negative).is_err());
    }

    #[test]
    fn a_new_dispute_is_open() {
        assert_eq!(DisputeStatus::default(), DisputeStatus::Open);
        assert_eq!(DisputeStatus::Revised.as_str(), "revised");
    }
}
