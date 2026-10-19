//! Owner statements: what the company collected for a landlord in a month,
//! what it spent and kept as its fee, and the balance (`PRP.OWN.01`, 01
//! Property §4.4, §6 *Owner statement*, §7.1 *Owner statement*, §7.2
//! `PRP.RUL.05`; BL-PRP-19).
//!
//! **The statement** belongs to one management agreement and one month. Its
//! balance is *brought forward + collected − expenses − fee + adjustments*,
//! all computed from its lines ([`StatementLine`], [`recalc`]): a line carries
//! the sign of its effect on the owner's balance. *Collected* includes the
//! rent written off (negative) and recovered after a write-off (BL-PRP-18).
//! The status moves only through the machine ([`crate::statement_machine`]).
//!
//! **`PRP.RUL.05`** ([`run`], daily at 02:00 and the command [`TASK`]): on day
//! D (5) of the month the rule generates a draft for every agreement that
//! reports (`active` or `ending`) for the previous month ([`generate`]) and
//! tells the accountants:
//!
//! - *collected*: for each invoice of a charge of a lease of the agreement's
//!   units, what has been paid on it and is not on an earlier statement of the
//!   agreement (so a late payment is counted when it arrives); a line per
//!   invoice, with the invoice as its source;
//! - *fee*: the agreement's percentage of the rent collected in this statement
//!   plus its fixed fee;
//! - *lines waiting for the next draft*: the rent written off and recovered
//!   (BL-PRP-18) and the adjustment lines of a dispute (BL-PRP-21), dated in
//!   or before the period;
//! - *brought forward*: the balance of the latest non-superseded earlier
//!   statement (what it carried forward when it was settled, else its
//!   balance).
//!
//! Expenses (vendor bills including each building's share of the AMC costs)
//! are entered by the accountant as lines of the draft: the supplier bills are
//! not read from here yet. A generated statement is regenerated for no period
//! that already has a non-superseded statement, so the rule can run again.
//!
//! Decided where the spec is silent: the user who generated or last edited the
//! draft is its preparer ([`OwnerStatement::prepared_by`]); a statement the rule
//! generated and nobody edited has none, and any property manager may approve
//! it. Lines are edited only while the statement is a draft.

use std::collections::BTreeSet;

use renox::auth::permissions::users_with_role;
use renox::chrono::{Datelike, Duration, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};

use crate::agreement::{AgreementStatus, ManagementAgreement};
use crate::building::refused;
use crate::charge::ChargeStatus;
use crate::lease::Lease;
use crate::notices;
use crate::permissions::{self, ACCOUNTANT_ROLE};
use crate::texts::english;

/// The subject type of an owner statement.
pub const STATEMENT_SUBJECT_TYPE: &str = "property.owner_statement";

/// The scheduled task and the command.
pub const TASK: &str = "property:owner-statements";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";

/// The rule.
pub const RULE: &str = "PRP.RUL.05";

/// D: the day of the month on which the drafts for the previous month are
/// generated.
pub const GENERATE_DAY: u32 = 5;

/// Where a statement stands (§7.1 *Owner statement*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StatementStatus {
    /// Generated (`PRP.RUL.05`); being checked.
    #[default]
    Draft,
    /// Approved by a property manager (`PRP.ACL.08`).
    Approved,
    /// Sent to the owner.
    Published,
    /// Payout paid, nothing due, or the negative balance carried forward or
    /// invoiced.
    Settled,
    /// Replaced by a revision before payout.
    Superseded,
}

/// What a line of a statement is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineKind {
    /// Rent collected.
    #[default]
    Collected,
    /// A cost paid for the owner (a vendor bill, an AMC share).
    Expense,
    /// The management fee.
    Fee,
    /// A correction of an earlier statement, with its reference.
    Adjustment,
    /// Rent written off as uncollectable (negative to the collected amount).
    WrittenOff,
    /// Rent recovered after a write-off.
    Recovered,
}

impl LineKind {
    /// The kinds an accountant enters by hand on a draft.
    pub const MANUAL: [LineKind; 4] = [
        LineKind::Collected,
        LineKind::Expense,
        LineKind::Fee,
        LineKind::Adjustment,
    ];

    /// The signed amount of a line of this kind for the amount `entered` as
    /// people type it (above 0): an expense and the fee reduce the balance, the
    /// rent written off too. An adjustment is entered with its sign.
    pub fn signed(self, entered: i64) -> i64 {
        match self {
            LineKind::Collected | LineKind::Recovered | LineKind::Adjustment => entered,
            LineKind::Expense | LineKind::Fee | LineKind::WrittenOff => -entered.abs(),
        }
    }
}

/// A statement of one agreement for one month.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "property_owner_statements",
    state_machine,
    unique(agreement_id, period_start, revision),
    index(agreement_id)
)]
pub struct OwnerStatement {
    /// The primary key.
    pub id: i64,
    /// The management agreement.
    #[model(references = "property_management_agreements")]
    pub agreement_id: i64,
    /// The owner, a party.
    pub owner_party_id: i64,
    /// The first day of the period.
    pub period_start: NaiveDate,
    /// The last day of the period.
    pub period_end: NaiveDate,
    /// 1, 2, ...: a revision of the same period.
    pub revision: i32,
    /// The statement this revision replaced.
    pub previous_id: Option<i64>,
    /// The balance of the earlier statement, in fils.
    pub brought_forward: i64,
    /// Rent collected, less the rent written off and plus what was recovered.
    pub collected: i64,
    /// What was spent for the owner (a positive amount that is subtracted).
    pub expenses: i64,
    /// The management fee (a positive amount that is subtracted).
    pub fee: i64,
    /// The adjustment lines.
    pub adjustments: i64,
    /// What the next statement brings forward once this one is settled.
    pub carried_forward: i64,
    /// The user who generated or last edited the draft.
    pub prepared_by: Option<i64>,
    /// The property manager who approved it.
    pub approved_by: Option<i64>,
    /// The day it was approved.
    pub approved_on: Option<NaiveDate>,
    /// Why the last approval was returned.
    pub return_reason: Option<String>,
    /// The day it was sent to the owner.
    pub published_on: Option<NaiveDate>,
    /// The user who sent it.
    pub published_by: Option<i64>,
    /// Why it was revised.
    pub revise_reason: Option<String>,
    /// The dispute flag (not a state): an owner disputes a line of a published
    /// or settled statement (BL-PRP-21). While it is set no payout is made.
    pub disputed: bool,
    /// The payout recorded for it, in fils (BL-PRP-20).
    pub payout_amount: Option<i64>,
    /// The outgoing payment of the payout.
    pub payout_payment_id: Option<i64>,
    /// How it was settled: `payout`, `carried_forward` or `invoiced`.
    pub settled_how: Option<String>,
    /// The invoice to the owner for a negative balance.
    pub owner_invoice_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: StatementStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One line of a statement.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "property_statement_lines",
    index(statement_id),
    index(agreement_id, statement_id)
)]
pub struct StatementLine {
    /// The primary key.
    pub id: i64,
    /// The agreement.
    #[model(references = "property_management_agreements")]
    pub agreement_id: i64,
    /// The statement it is on; empty while it waits for the next draft.
    pub statement_id: Option<i64>,
    /// What it is.
    pub kind: LineKind,
    /// The text.
    pub description: String,
    /// The amount in fils with the sign of its effect on the balance.
    pub amount: i64,
    /// What it comes from (`invoicing.invoice:7`), so it is made once.
    pub source: Option<String>,
    /// The line this one is linked to: the line an adjustment corrects, or the
    /// line of the other owner's statement a reallocation is linked to.
    pub linked_to: Option<i64>,
    /// The day it happened.
    pub line_on: NaiveDate,
    /// The user who made it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The sums of the lines of a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    /// Collected, less written off, plus recovered.
    pub collected: i64,
    /// Expenses, as a positive amount.
    pub expenses: i64,
    /// The fee, as a positive amount.
    pub fee: i64,
    /// The adjustments.
    pub adjustments: i64,
}

/// The totals of `lines`.
pub fn totals(lines: &[StatementLine]) -> Totals {
    let mut sums = Totals::default();
    for line in lines {
        match line.kind {
            LineKind::Collected | LineKind::WrittenOff | LineKind::Recovered => {
                sums.collected += line.amount;
            }
            LineKind::Expense => sums.expenses -= line.amount,
            LineKind::Fee => sums.fee -= line.amount,
            LineKind::Adjustment => sums.adjustments += line.amount,
        }
    }
    sums
}

/// The fee of `collected` fils at `fee_bp` basis points plus `fixed` fils,
/// rounded to the fils; nothing on rent collected below 0.
pub fn fee_of(collected: i64, fee_bp: i32, fixed: i64) -> i64 {
    let rate = if collected > 0 {
        (collected * i64::from(fee_bp) + 5_000) / 10_000
    } else {
        0
    };
    rate + fixed.max(0)
}

/// The first and last day of the month before `day`.
pub fn previous_month(day: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first_of_this = day.with_day(1).unwrap_or(day);
    let end = first_of_this - Duration::days(1);
    (end.with_day(1).unwrap_or(end), end)
}

impl OwnerStatement {
    /// The statement as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(STATEMENT_SUBJECT_TYPE, self.id)
    }

    /// The statement shown to people: `OS-0007-202702 r1`.
    pub fn number(&self) -> String {
        format!(
            "OS-{:04}-{} r{}",
            self.agreement_id,
            self.period_start.format("%Y%m"),
            self.revision
        )
    }

    /// The balance: brought forward + collected − expenses − fee +
    /// adjustments.
    pub fn balance(&self) -> i64 {
        self.brought_forward + self.collected - self.expenses - self.fee + self.adjustments
    }

    /// What the next statement brings forward: what it carried when it was
    /// settled, else its balance.
    pub fn forward(&self) -> i64 {
        if self.status == StatementStatus::Settled {
            self.carried_forward
        } else {
            self.balance()
        }
    }

    /// The lines on the statement, in the order they were made.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn lines(&self, db: &Db) -> Result<Vec<StatementLine>> {
        StatementLine::query()
            .where_(StatementLine::STATEMENT_ID.eq(self.id))
            .order_by(StatementLine::ID)
            .get(db)
            .await
    }

    /// The statements of the agreement, newest period first, revisions last
    /// first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_agreement(db: &Db, agreement_id: i64) -> Result<Vec<OwnerStatement>> {
        OwnerStatement::query()
            .where_(OwnerStatement::AGREEMENT_ID.eq(agreement_id))
            .order_by_desc(OwnerStatement::PERIOD_START)
            .order_by_desc(OwnerStatement::REVISION)
            .get(db)
            .await
    }
}

/// Recomputes the totals of statement `id` from its lines.
///
/// # Errors
///
/// The database's errors.
pub async fn recalc(db: &Db, id: i64) -> Result {
    let lines = StatementLine::query()
        .where_(StatementLine::STATEMENT_ID.eq(id))
        .get(db)
        .await?;
    let sums = totals(&lines);
    // Not the status column, so outside the machine's guard.
    OwnerStatement::query()
        .where_(OwnerStatement::ID.eq(id))
        .update(
            db,
            &[
                ("collected", &sums.collected),
                ("expenses", &sums.expenses),
                ("fee", &sums.fee),
                ("adjustments", &sums.adjustments),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// Whether `user` may prepare statements.
pub(crate) fn may_prepare(user: &AuthUser) -> bool {
    user.has_permission(permissions::OWNER_STATEMENTS_PREPARE)
}

/// `user` adds a line to the draft statement `id`: `amount` is as people type
/// it (above 0, except an adjustment, which is signed). The user becomes its
/// preparer.
///
/// # Errors
///
/// 403 without `owner_statements.prepare`; 422 unless the statement is a
/// draft (an approved one is returned first), for a blank description, an
/// amount of 0, or a kind that is not entered by hand.
pub async fn add_line(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    kind: LineKind,
    description: &str,
    amount: i64,
) -> Result<StatementLine> {
    if !may_prepare(user) {
        return Err(Error::Forbidden);
    }
    let statement = OwnerStatement::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if statement.status != StatementStatus::Draft {
        return Err(refused(english("property.statements.not_draft", &[])));
    }
    let description = description.trim();
    if description.is_empty() || amount == 0 || !LineKind::MANUAL.contains(&kind) {
        return Err(refused(english("property.statements.line_invalid", &[])));
    }
    let line = StatementLine::create(
        &state.db,
        StatementLine {
            agreement_id: statement.agreement_id,
            statement_id: Some(statement.id),
            kind,
            description: description.to_owned(),
            amount: kind.signed(amount),
            line_on: today(),
            created_by: Some(user.id),
            ..Default::default()
        },
    )
    .await?;
    touch(&state.db, statement.id, user.id).await?;
    Ok(line)
}

/// `user` removes line `line_id` of the draft statement `id`.
///
/// # Errors
///
/// 403 without `owner_statements.prepare`; 404 for a line that is not on the
/// statement; 422 unless the statement is a draft.
pub async fn remove_line(state: &AppState, user: &AuthUser, id: i64, line_id: i64) -> Result {
    if !may_prepare(user) {
        return Err(Error::Forbidden);
    }
    let statement = OwnerStatement::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if statement.status != StatementStatus::Draft {
        return Err(refused(english("property.statements.not_draft", &[])));
    }
    let mut line = StatementLine::find(&state.db, line_id)
        .await?
        .filter(|line| line.statement_id == Some(statement.id))
        .ok_or(Error::NotFound)?;
    line.delete(&state.db).await?;
    touch(&state.db, statement.id, user.id).await
}

/// `user` is now the last to have edited the draft `id`; the totals follow.
async fn touch(db: &Db, id: i64, user: i64) -> Result {
    OwnerStatement::query()
        .where_(OwnerStatement::ID.eq(id))
        .update(db, &[("prepared_by", &Some(user))])
        .await?;
    recalc(db, id).await
}

/// A line to make for an agreement without a user at hand (a write-off, a
/// recovery, an adjustment of a dispute): see [`post_line`].
#[derive(Debug, Clone)]
pub struct PostLine {
    /// What it is.
    pub kind: LineKind,
    /// The text.
    pub description: String,
    /// The amount in fils with the sign of its effect on the balance.
    pub amount: i64,
    /// What it comes from, so it is made once per kind.
    pub source: Option<String>,
    /// The day it happened.
    pub line_on: NaiveDate,
    /// The line it corrects or is linked to.
    pub linked_to: Option<i64>,
    /// The user who made it, if any.
    pub by: Option<i64>,
}

impl PostLine {
    /// A line of `kind` for `amount` fils (already signed) on `line_on`.
    pub fn new(kind: LineKind, description: &str, amount: i64, line_on: NaiveDate) -> Self {
        PostLine {
            kind,
            description: description.to_owned(),
            amount,
            source: None,
            line_on,
            linked_to: None,
            by: None,
        }
    }

    /// Linked to the line `id`: the line an adjustment corrects.
    pub fn linked_to(mut self, id: Option<i64>) -> Self {
        self.linked_to = id;
        self
    }

    /// Made once for `source`.
    pub fn source(mut self, source: &str) -> Self {
        self.source = Some(source.to_owned());
        self
    }

    /// Made by the user `by`.
    pub fn by(mut self, by: Option<i64>) -> Self {
        self.by = by;
        self
    }
}

/// Makes the line `line` for the agreement `agreement_id`: it goes on the
/// draft statement whose period holds its day, if there is one, else it waits
/// for the next draft. A line with a source is made once per kind: the line
/// made before is returned. Published and approved statements are never edited
/// (`PRP.ACL.21`).
///
/// # Errors
///
/// The database's errors.
pub async fn post_line(db: &Db, agreement_id: i64, line: PostLine) -> Result<StatementLine> {
    if let Some(source) = line.source.as_deref()
        && let Some(found) = StatementLine::query()
            .where_(StatementLine::AGREEMENT_ID.eq(agreement_id))
            .where_(StatementLine::KIND.eq(line.kind))
            .where_(StatementLine::SOURCE.eq(source))
            .first(db)
            .await?
    {
        return Ok(found);
    }
    let draft = OwnerStatement::query()
        .where_(OwnerStatement::AGREEMENT_ID.eq(agreement_id))
        .where_(OwnerStatement::STATUS.eq(StatementStatus::Draft))
        .where_(OwnerStatement::PERIOD_START.lte(line.line_on))
        .where_(OwnerStatement::PERIOD_END.gte(line.line_on))
        .first(db)
        .await?;
    let made = StatementLine::create(
        db,
        StatementLine {
            agreement_id,
            statement_id: draft.as_ref().map(|d| d.id),
            kind: line.kind,
            description: line.description,
            amount: line.amount,
            source: line.source,
            linked_to: line.linked_to,
            line_on: line.line_on,
            created_by: line.by,
            ..Default::default()
        },
    )
    .await?;
    if let Some(draft) = draft {
        recalc(db, draft.id).await?;
    }
    Ok(made)
}

/// The statements of the agreement that still count: not superseded.
async fn live_statement_ids(db: &Db, agreement_id: i64) -> Result<BTreeSet<i64>> {
    Ok(OwnerStatement::query()
        .where_(OwnerStatement::AGREEMENT_ID.eq(agreement_id))
        .where_(OwnerStatement::STATUS.ne(StatementStatus::Superseded))
        .get(db)
        .await?
        .iter()
        .map(|s| s.id)
        .collect())
}

/// What was already counted as collected on the invoice `source` on the
/// statements that count (and the lines waiting), in fils.
async fn collected_before(db: &Db, agreement_id: i64, source: &str) -> Result<i64> {
    let live = live_statement_ids(db, agreement_id).await?;
    let lines = StatementLine::query()
        .where_(StatementLine::AGREEMENT_ID.eq(agreement_id))
        .where_(StatementLine::KIND.eq(LineKind::Collected))
        .where_(StatementLine::SOURCE.eq(source))
        .get(db)
        .await?;
    Ok(lines
        .iter()
        .filter(|line| line.statement_id.is_none_or(|id| live.contains(&id)))
        .map(|line| line.amount)
        .sum())
}

/// The collected lines a statement ending `end` for `agreement` gets: one per
/// invoice of the units' leases, for what was paid and not counted before.
async fn collected_lines(
    state: &AppState,
    agreement: &ManagementAgreement,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<(String, i64, String)>> {
    let mut found: Vec<(String, i64, String)> = Vec::new();
    for unit in agreement.units(&state.db).await? {
        if !unit.covers(start, end) {
            continue;
        }
        let leases = Lease::query()
            .where_(Lease::UNIT_ID.eq(unit.unit_id))
            .order_by(Lease::ID)
            .get(&state.db)
            .await?;
        for lease in leases {
            for charge in lease.charges(&state.db).await? {
                let Some(invoice) = charge.invoice_id else {
                    continue;
                };
                if charge.status != ChargeStatus::Invoiced || charge.due_date > end {
                    continue;
                }
                let source = InvoiceId(invoice).subject().to_string();
                if found.iter().any(|(_, _, s)| *s == source) {
                    continue;
                }
                let Some(summary) = invoicer(state)
                    .find(&state.db, InvoiceId(invoice))
                    .await?
                else {
                    continue;
                };
                if summary.status == InvoiceStatus::Draft {
                    continue;
                }
                let paid = summary.gross_total - summary.balance;
                let new = paid - collected_before(&state.db, agreement.id, &source).await?;
                if new > 0 {
                    found.push((
                        format!("{} · {}", charge.description, lease.number()),
                        new,
                        source,
                    ));
                }
            }
        }
    }
    Ok(found)
}

/// Generates the draft statement of `agreement_id` for `start..=end` as the
/// system (`PRP.RUL.05`) and tells the accountants. `None` when the agreement
/// does not report, or the period has a statement that counts already.
///
/// # Errors
///
/// The database's errors and the invoicer's.
pub async fn generate(
    state: &AppState,
    agreement_id: i64,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Option<OwnerStatement>> {
    let Some(agreement) = ManagementAgreement::find(&state.db, agreement_id).await? else {
        return Ok(None);
    };
    if !AgreementStatus::REPORTING.contains(&agreement.status) {
        return Ok(None);
    }
    let counts = OwnerStatement::query()
        .where_(OwnerStatement::AGREEMENT_ID.eq(agreement.id))
        .where_(OwnerStatement::PERIOD_START.eq(start))
        .where_(OwnerStatement::STATUS.ne(StatementStatus::Superseded))
        .exists(&state.db)
        .await?;
    if counts {
        return Ok(None);
    }
    let earlier = OwnerStatement::query()
        .where_(OwnerStatement::AGREEMENT_ID.eq(agreement.id))
        .where_(OwnerStatement::PERIOD_END.lt(start))
        .where_(OwnerStatement::STATUS.ne(StatementStatus::Superseded))
        .order_by_desc(OwnerStatement::PERIOD_END)
        .order_by_desc(OwnerStatement::REVISION)
        .first(&state.db)
        .await?;
    let statement = OwnerStatement::create(
        &state.db,
        OwnerStatement {
            agreement_id: agreement.id,
            owner_party_id: agreement.owner_party_id,
            period_start: start,
            period_end: end,
            revision: 1,
            brought_forward: earlier.as_ref().map_or(0, OwnerStatement::forward),
            status: StatementStatus::Draft,
            ..Default::default()
        },
    )
    .await?;
    // The lines that waited for this draft.
    StatementLine::query()
        .where_(StatementLine::AGREEMENT_ID.eq(agreement.id))
        .where_(StatementLine::STATEMENT_ID.is_null())
        .where_(StatementLine::LINE_ON.lte(end))
        .update(
            &state.db,
            &[
                ("statement_id", &Some(statement.id)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    let mut rent = 0;
    for (description, amount, source) in collected_lines(state, &agreement, start, end).await? {
        rent += amount;
        StatementLine::create(
            &state.db,
            StatementLine {
                agreement_id: agreement.id,
                statement_id: Some(statement.id),
                kind: LineKind::Collected,
                description,
                amount,
                source: Some(source),
                line_on: end,
                ..Default::default()
            },
        )
        .await?;
    }
    let fee = fee_of(rent, agreement.fee_bp, agreement.fee_fixed);
    if fee > 0 {
        let label = english(
            "property.statements.fee_line",
            &[("percent", &format!("{:.2}", f64::from(agreement.fee_bp) / 100.0))],
        );
        StatementLine::create(
            &state.db,
            StatementLine {
                agreement_id: agreement.id,
                statement_id: Some(statement.id),
                kind: LineKind::Fee,
                description: label,
                amount: -fee,
                line_on: end,
                ..Default::default()
            },
        )
        .await?;
    }
    recalc(&state.db, statement.id).await?;
    let statement = OwnerStatement::find(&state.db, statement.id)
        .await?
        .ok_or(Error::NotFound)?;
    let accountants: Vec<i64> = users_with_role(&state.db, ACCOUNTANT_ROLE)
        .await?
        .iter()
        .map(|u| u.id)
        .collect();
    notices::statement_drafted(state, &statement, &accountants).await?;
    Ok(Some(statement))
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Drafts generated.
    pub generated: usize,
    /// Agreements left alone: a statement counts already, or it does not report.
    pub skipped: usize,
    /// Agreements that failed; the run moves on.
    pub failed: usize,
}

/// `PRP.RUL.05` for `day`: on day [`GENERATE_DAY`] a draft for the previous
/// month of every agreement that reports; on any other day nothing.
///
/// # Errors
///
/// The database's errors.
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let mut report = Report::default();
    if day.day() != GENERATE_DAY {
        return Ok(report);
    }
    let (start, end) = previous_month(day);
    for agreement in ManagementAgreement::reporting(&state.db).await? {
        match generate(state, agreement.id, start, end).await {
            Ok(Some(_)) => report.generated += 1,
            Ok(None) => report.skipped += 1,
            Err(error) => {
                report.failed += 1;
                eprintln!("{TASK}: agreement {}: {error:?}", agreement.id);
            }
        }
    }
    Ok(report)
}

/// Registers the monthly task and its command.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Generate the draft owner statements of the previous month on day 5 (--date YYYY-MM-DD runs another day)",
        |args, state: AppState| async move {
            let on = match args.value("--date") {
                None => today(),
                Some(text) => text.parse().map_err(|_| {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("--date must be YYYY-MM-DD, got `{text}`"),
                    )
                })?,
            };
            let report = run(&state, on).await?;
            println!(
                "{} statements generated, {} left alone, {} failed.",
                report.generated, report.skipped, report.failed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn line(kind: LineKind, amount: i64) -> StatementLine {
        StatementLine {
            kind,
            amount,
            ..Default::default()
        }
    }

    #[test]
    fn the_lines_add_up_to_the_totals_with_their_signs() {
        let sums = totals(&[
            line(LineKind::Collected, 1_000_000),
            line(LineKind::WrittenOff, -600_000),
            line(LineKind::Recovered, 200_000),
            line(LineKind::Expense, -120_000),
            line(LineKind::Fee, -30_000),
            line(LineKind::Adjustment, -420_000),
        ]);
        assert_eq!(
            sums,
            Totals {
                collected: 600_000,
                expenses: 120_000,
                fee: 30_000,
                adjustments: -420_000
            }
        );
    }

    #[test]
    fn the_balance_is_brought_forward_plus_collected_less_expenses_and_fee_plus_adjustments() {
        let statement = OwnerStatement {
            brought_forward: 100_000,
            collected: 1_000_000,
            expenses: 200_000,
            fee: 50_000,
            adjustments: -30_000,
            ..Default::default()
        };
        assert_eq!(statement.balance(), 820_000);
        assert_eq!(statement.forward(), 820_000);
        let settled = OwnerStatement {
            status: StatementStatus::Settled,
            carried_forward: -350_000,
            ..statement
        };
        assert_eq!(settled.forward(), -350_000);
    }

    #[test]
    fn a_kind_decides_the_sign_of_what_is_typed() {
        assert_eq!(LineKind::Expense.signed(5_000), -5_000);
        assert_eq!(LineKind::Fee.signed(5_000), -5_000);
        assert_eq!(LineKind::Collected.signed(5_000), 5_000);
        assert_eq!(LineKind::Adjustment.signed(-5_000), -5_000);
        assert_eq!(LineKind::WrittenOff.signed(5_000), -5_000);
    }

    #[test]
    fn the_fee_is_a_share_of_the_rent_collected_plus_the_fixed_fee() {
        assert_eq!(fee_of(1_000_000, 500, 0), 50_000);
        assert_eq!(fee_of(1_000_001, 500, 0), 50_000);
        assert_eq!(fee_of(1_000_010, 500, 0), 50_001);
        assert_eq!(fee_of(1_000_000, 500, 10_000), 60_000);
        assert_eq!(fee_of(0, 500, 10_000), 10_000);
        assert_eq!(fee_of(-5, 500, 0), 0);
    }

    #[test]
    fn the_previous_month_is_whole() {
        assert_eq!(
            previous_month(day(2027, 3, 5)),
            (day(2027, 2, 1), day(2027, 2, 28))
        );
        assert_eq!(
            previous_month(day(2027, 1, 5)),
            (day(2026, 12, 1), day(2026, 12, 31))
        );
        assert_eq!(
            previous_month(day(2028, 3, 5)),
            (day(2028, 2, 1), day(2028, 2, 29))
        );
    }

    #[test]
    fn the_number_names_the_agreement_the_month_and_the_revision() {
        let statement = OwnerStatement {
            id: 3,
            agreement_id: 7,
            period_start: day(2027, 2, 1),
            revision: 2,
            ..Default::default()
        };
        assert_eq!(statement.number(), "OS-0007-202702 r2");
        assert_eq!(statement.subject().to_string(), "property.owner_statement:3");
    }
}
