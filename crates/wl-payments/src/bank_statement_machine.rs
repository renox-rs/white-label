//! The bank statement import state machine (wl-payments §4 *Bank statement
//! import*).
//!
//! ```text
//! [*] --> uploaded --parse--> in_review --complete--> completed (terminal)
//! uploaded --fail--> failed (terminal)
//! in_review --revert--> reverted (terminal)
//! ```
//!
//! `parse` and `fail` are system transitions ([`crate::bank_statement_flow`]
//! reads the file first and hands the lines in); `complete` and `revert` need
//! `bank.reconcile`. `complete` needs every line matched or ignored: a line
//! left `unmatched` or `suggested` (a drawing waiting for its approval) holds
//! it back. `revert` removes the lines and is refused while a line carries a
//! drawing request that is open or recorded: the payment of a recorded
//! drawing is reversed first, an open request withdrawn.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::bank_statement::{BankStatementImport, ImportStatus};
use crate::bank_statement_line::{BankStatementLine, LineStatus};
use crate::permissions;

/// The `parse` transition's name.
pub const PARSE: &str = "parse";
/// The `fail` transition's name.
pub const FAIL: &str = "fail";
/// The `complete` transition's name.
pub const COMPLETE: &str = "complete";
/// The `revert` transition's name.
pub const REVERT: &str = "revert";

/// One transaction `parse` creates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NewLine {
    /// The day of the transaction.
    pub date: NaiveDate,
    /// The signed amount in fils.
    pub amount: i64,
    /// The bank's description.
    pub description: String,
    /// The bank's reference.
    pub reference: String,
    /// The other side.
    pub counterparty: String,
}

/// What a `parse` run carries: the lines to create and the closing balance
/// the import has afterwards (the file's, else the one entered at upload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ParseInput {
    /// The lines of the file not imported before.
    pub lines: Vec<NewLine>,
    /// The closing balance in fils.
    pub closing_balance: Option<i64>,
}

/// What a `complete` run carries: how many lines still hold it back.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct CompleteInput {
    /// Lines `unmatched` or `suggested`.
    pub open: i64,
    /// Of them, lines waiting for the approval of an owner drawing.
    pub open_drawings: i64,
}

/// What a `revert` run carries: the lines whose drawing is open or recorded.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RevertInput {
    /// Lines marked as drawings, waiting or recorded.
    pub drawings: i64,
}

fn input<T: serde::de::DeserializeOwned + Default>(ctx: &TransitionCtx) -> T {
    ctx.input_as::<T>().unwrap_or_default()
}

impl StateMachine for BankStatementImport {
    type State = ImportStatus;
    const COLUMN: renox::db::Col<BankStatementImport, ImportStatus> = BankStatementImport::STATUS;

    fn machine() -> Machine<Self> {
        use ImportStatus::{Completed, Failed, InReview, Reverted, Uploaded};
        let machine = Machine::new()
            .initial([Uploaded])
            .state(Uploaded)
            .tone("neutral")
            .state(InReview)
            .tone("info")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(Failed)
            .tone("error")
            .terminal()
            .state(Reverted)
            .tone("neutral")
            .terminal();
        [parse(), fail(), complete(), revert()]
            .into_iter()
            .fold(machine, Machine::transition)
    }
}

fn parse() -> Transition<BankStatementImport> {
    Transition::new(PARSE)
        .label("Read the file")
        .tone("primary")
        .system()
        .from([ImportStatus::Uploaded])
        .to(ImportStatus::InReview)
        .guard("file_read", |_import: &BankStatementImport, ctx| {
            let read = ctx.input_as::<ParseInput>().is_ok();
            Box::pin(async move {
                if read {
                    Ok(())
                } else {
                    Err("The file was not read.".to_owned())
                }
            })
        })
        .effect(|import: &BankStatementImport, ctx, tx| {
            let import = import.clone();
            let read = ctx.input_as::<ParseInput>().ok();
            Box::pin(async move {
                let read = read.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a parse without its lines"))
                })?;
                for new in &read.lines {
                    let mut line = BankStatementLine {
                        import_id: import.id,
                        legal_entity_id: import.legal_entity_id,
                        bank_account: import.bank_account.clone(),
                        currency: import.currency.clone(),
                        line_date: new.date,
                        amount: new.amount,
                        description: new.description.clone(),
                        reference: new.reference.clone(),
                        counterparty: new.counterparty.clone(),
                        status: LineStatus::Unmatched,
                        ..Default::default()
                    };
                    line.insert(&mut *tx).await?;
                }
                BankStatementImport::query()
                    .where_(BankStatementImport::ID.eq(import.id))
                    .update(
                        &mut *tx,
                        &[
                            ("line_count", &(read.lines.len() as i32)),
                            ("closing_balance", &read.closing_balance),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

fn fail() -> Transition<BankStatementImport> {
    Transition::new(FAIL)
        .label("File not recognised")
        .tone("danger")
        .system()
        .from([ImportStatus::Uploaded])
        .to(ImportStatus::Failed)
        .reason_required()
}

fn complete() -> Transition<BankStatementImport> {
    Transition::new(COMPLETE)
        .label("Complete")
        .tone("primary")
        .permission(permissions::BANK_RECONCILE)
        .from([ImportStatus::InReview])
        .to(ImportStatus::Completed)
        .guard("no_line_left_open", |_import: &BankStatementImport, ctx| {
            let counts = input::<CompleteInput>(ctx);
            Box::pin(async move {
                if counts.open_drawings > 0 {
                    Err(format!(
                        "{} owner drawing(s) wait for their approval; the import cannot be completed.",
                        counts.open_drawings
                    ))
                } else if counts.open > 0 {
                    Err(format!(
                        "{} line(s) are still unmatched or suggested.",
                        counts.open
                    ))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|import: &BankStatementImport, _ctx, tx| {
            let id = import.id;
            Box::pin(async move {
                BankStatementImport::query()
                    .where_(BankStatementImport::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("completed_at", &Some(renox::db::now())),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

fn revert() -> Transition<BankStatementImport> {
    Transition::new(REVERT)
        .label("Revert")
        .tone("danger")
        .permission(permissions::BANK_RECONCILE)
        .from([ImportStatus::InReview])
        .to(ImportStatus::Reverted)
        .guard("no_drawing_in_flight", |_import: &BankStatementImport, ctx| {
            let counts = input::<RevertInput>(ctx);
            Box::pin(async move {
                if counts.drawings > 0 {
                    Err(format!(
                        "{} line(s) carry an owner drawing that is waiting or recorded; withdraw or reverse it first.",
                        counts.drawings
                    ))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|import: &BankStatementImport, _ctx, tx| {
            let id = import.id;
            Box::pin(async move {
                BankStatementLine::query()
                    .where_(BankStatementLine::IMPORT_ID.eq(id))
                    .delete(&mut *tx)
                    .await?;
                Ok(())
            })
        })
}
