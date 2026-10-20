//! The bank statement contract (ARCHITECTURE §5, wl-payments §3 *Bank
//! statement import*, §6): what other modules may read of the uploaded bank
//! statements without depending on the `wl-payments` crate.
//!
//! `wl-payments` provides [`BankStatements`] with [`provide_bank_statements`].
//! The cash-flow forecast of wl-invoicing (`FIN.CFL.01`) reads the latest
//! known balance of each bank account from it. [`bank_statements`] falls back
//! to [`NoBankStatements`] when the `payments` module is off: no balance is
//! known.
//!
//! Money is `i64` in fils of the import's currency.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// The latest known balance of one bank account: the closing balance of the
/// completed import whose period ends last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankBalance {
    /// The legal entity the account belongs to.
    pub legal_entity_id: i64,
    /// The bank account, as the statement import names it.
    pub bank_account: String,
    /// The balance, in fils.
    pub amount: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The day it is as of: the last day of the import's period. The balance
    /// already includes every movement dated on that day.
    pub as_of: NaiveDate,
    /// The import it comes from.
    pub import_id: i64,
}

/// A statement line that was marked as an owner drawing (`FIN.BUD.01`): an
/// outgoing line the owner spent for personal use (company card, ATM).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawingLine {
    /// The statement line.
    pub line_id: i64,
    /// The import it belongs to.
    pub import_id: i64,
    /// The legal entity the account belongs to.
    pub legal_entity_id: i64,
    /// The bank account, as the statement import names it.
    pub bank_account: String,
    /// The day of the bank transaction.
    pub date: NaiveDate,
    /// The amount taken out, in fils (above 0).
    pub amount: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The code of the drawings category it was marked with.
    pub category: String,
    /// Whether the drawing is approved and its payment recorded (`matched`);
    /// until then the line is `suggested` and the drawing waits for its
    /// approval.
    pub recorded: bool,
}

/// A statement was read into lines (`parse`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankStatementImported {
    /// The import.
    pub import_id: i64,
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The bank account.
    pub bank_account: String,
    /// How many lines were created.
    pub lines: i32,
}

impl Event for BankStatementImported {}

/// A statement was reconciled (`complete`): the closing balance, if the
/// import has one, is now the account's balance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankStatementCompleted {
    /// The import.
    pub import_id: i64,
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The bank account.
    pub bank_account: String,
    /// The closing balance in fils, when the import has one.
    pub closing_balance: Option<i64>,
    /// The last day of the period.
    pub as_of: NaiveDate,
}

impl Event for BankStatementCompleted {}

/// The future a [`BankStatements`] method returns.
pub type BankStatementFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Reads the bank statement imports for other modules. `wl-payments`
/// provides it; without that module, [`bank_statements`] gives
/// [`NoBankStatements`].
pub trait BankStatements: Send + Sync + 'static {
    /// The latest closing balance of each bank account of `legal_entity_id`,
    /// ordered by bank account. Only a `completed` import with a closing
    /// balance counts (an import without one is skipped, a `reverted` or
    /// `failed` one never used); of an account's imports the one whose period
    /// ends last wins, not the one imported last, and on the same period end
    /// the one completed last. Read only.
    fn latest_closing_balances<'a>(
        &'a self,
        db: &'a Db,
        legal_entity_id: i64,
    ) -> BankStatementFuture<'a, Vec<BankBalance>>;

    /// The outgoing lines marked as owner drawings (`FIN.BUD.01`) of
    /// `entities` dated from `from` to `to` (both included), whose
    /// drawing is waiting for its approval or recorded (a line put back to
    /// `unmatched` by a rejection is not returned), oldest first. Read only.
    /// Without the statement lines (the default) nothing is returned.
    fn drawing_lines<'a>(
        &'a self,
        _db: &'a Db,
        _entities: &'a [i64],
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> BankStatementFuture<'a, Vec<DrawingLine>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The statements when the `payments` module is off: no balance is known.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBankStatements;

impl BankStatements for NoBankStatements {
    fn latest_closing_balances<'a>(
        &'a self,
        _db: &'a Db,
        _legal_entity_id: i64,
    ) -> BankStatementFuture<'a, Vec<BankBalance>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Makes `statements` the app's [`BankStatements`]; `wl-payments` calls it in
/// `register`.
pub fn provide_bank_statements(app: &mut Registry, statements: impl BankStatements) {
    let statements: Arc<dyn BankStatements> = Arc::new(statements);
    app.provide(statements);
}

/// The app's [`BankStatements`], or [`NoBankStatements`] when none is
/// provided.
pub fn bank_statements(state: &AppState) -> Arc<dyn BankStatements> {
    match state.provided::<Arc<dyn BankStatements>>() {
        Some(statements) => Arc::clone(&statements),
        None => Arc::new(NoBankStatements),
    }
}
