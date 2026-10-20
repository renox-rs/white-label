//! The services of the bank statement import (wl-payments §4 *Bank statement
//! import*, §5.7): reading the file into lines (`parse`, or `fail` for a file
//! that is not recognised), `complete`, `revert`, and what the accountant does
//! with a line: ignore it with a reason, or mark an outgoing line as an owner
//! drawing (§5.7 step 3).
//!
//! A line marked as a drawing creates the drawings approval request
//! (`FIN.ACL.21`, wl-invoicing §5.16) and stays `suggested`; the approval
//! records the payment and matches the line ([`drawing_recorded`]); a
//! rejection, withdrawal or expiry puts it back to `unmatched` with the reason
//! and tells the accountant who marked it ([`drawing_rejected`]). The import
//! cannot `complete` while such a request is open.
//!
//! Not built here (they come with the match engine of the bank
//! reconciliation feature): the match suggestions, confirming an incoming
//! line into a payment, matching a line to cheques, invoices or bills, and the
//! task for lines left unmatched.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use wl_kernel::bank_statements::{
    BankStatementCompleted, BankStatementFuture, BankStatementImported, DrawingLine,
};
use wl_kernel::drawings::owner_drawings;
use wl_kernel::notify::{self, NotifyRequest};
use wl_kernel::payments::PaymentMethod;

use crate::bank_statement::{BankStatementImport, ImportStatus};
use crate::bank_statement_line::{BankStatementLine, LineStatus};
use crate::bank_statement_machine::{
    COMPLETE, CompleteInput, FAIL, NewLine, PARSE, ParseInput, REVERT, RevertInput,
};
use crate::bank_statement_parse::{self, ParsedLine};
use crate::drawings::{self, DrawingInput, DrawingStatus, PaymentDrawing};
use crate::followup::money_text;
use crate::notices::{BANK_IMPORT_FAILED, BANK_LINE_DRAWING_REJECTED};
use crate::permissions;

fn guard(message: &str) -> TransitionError {
    TransitionError::Guard(message.to_owned())
}

async fn reload(state: &AppState, import: &BankStatementImport) -> Result<BankStatementImport> {
    BankStatementImport::find_or_404(&state.db, import.id).await
}

/// Reads the file of the `uploaded` import `import_id` as the system: its
/// lines are created `unmatched` (a line already imported for the account is
/// skipped), the import goes `in_review`, and a closing balance in the file
/// replaces one entered at upload. A file that is not recognised fails the
/// import with the reason and tells the uploader: a format that cannot be read
/// yet (Excel), rows or tags that are not understood, a transaction outside
/// the period, a closing balance not dated the last day of the period, an
/// account that is not the import's.
///
/// # Errors
///
/// `NotFrom` unless `uploaded`; a database error.
pub async fn parse(
    state: &AppState,
    import_id: i64,
) -> std::result::Result<BankStatementImport, TransitionError> {
    let import = BankStatementImport::find_or_404(&state.db, import_id).await?;
    if import.status != ImportStatus::Uploaded {
        return Err(TransitionError::NotFrom(import.status.as_str().to_owned()));
    }
    let content = import.content.clone().unwrap_or_default();
    // A CSV upload without a file is a balance entered by hand: no lines.
    let read = if content.trim().is_empty() && import.format == "csv" {
        Ok(Default::default())
    } else if content.trim().is_empty() {
        Err("The text of the file is missing.".to_owned())
    } else {
        bank_statement_parse::parse(&import.format, &content)
    };
    let parsed = match read.and_then(|parsed| checked(&import, parsed)) {
        Ok(parsed) => parsed,
        Err(reason) => return fail(state, &import, &reason).await,
    };
    let lines = new_lines(state, &import, &parsed.lines).await?;
    let input = ParseInput {
        lines,
        closing_balance: parsed
            .closing
            .map(|(_, fils)| fils)
            .or(import.closing_balance),
    };
    let _outcome = import
        .transition(PARSE)
        .by_system()
        .input(&input)
        .run(state)
        .await?;
    Ok(reload(state, &import).await?)
}

/// The statement as it is, or why it does not match the import: its account
/// (when the file names one), its dates against the period.
fn checked(
    import: &BankStatementImport,
    parsed: bank_statement_parse::ParsedStatement,
) -> std::result::Result<bank_statement_parse::ParsedStatement, String> {
    if let Some(account) = &parsed.account
        && !account
            .trim()
            .eq_ignore_ascii_case(import.bank_account.trim())
    {
        return Err(format!(
            "The account in the file ({}) is not {}.",
            account.trim(),
            import.bank_account
        ));
    }
    let inside = |date: NaiveDate| date >= import.period_start && date <= import.period_end;
    if let Some(line) = parsed.lines.iter().find(|line| !inside(line.date)) {
        return Err(format!(
            "A transaction is dated {}, outside the period of the statement.",
            line.date
        ));
    }
    if let Some((date, _)) = parsed.closing
        && date != import.period_end
    {
        return Err(format!(
            "The closing balance is dated {date}, not the last day of the period."
        ));
    }
    Ok(parsed)
}

/// The key of a line for the skip of lines imported before.
type Key = (NaiveDate, i64, String, String);

fn key_of(date: NaiveDate, amount: i64, reference: &str, description: &str) -> Key {
    (date, amount, reference.to_owned(), description.to_owned())
}

/// The lines of `parsed` that the account has not got yet: of identical
/// lines (the same day, amount, reference and description) as many as the
/// account already holds are skipped.
async fn new_lines(
    state: &AppState,
    import: &BankStatementImport,
    parsed: &[ParsedLine],
) -> Result<Vec<NewLine>> {
    let mut held: BTreeMap<Key, usize> = BTreeMap::new();
    for line in BankStatementLine::query()
        .where_(BankStatementLine::LEGAL_ENTITY_ID.eq(import.legal_entity_id))
        .where_(BankStatementLine::BANK_ACCOUNT.eq(import.bank_account.clone()))
        .where_(BankStatementLine::LINE_DATE.gte(import.period_start))
        .where_(BankStatementLine::LINE_DATE.lte(import.period_end))
        .get(&state.db)
        .await?
    {
        let key = key_of(
            line.line_date,
            line.amount,
            &line.reference,
            &line.description,
        );
        *held.entry(key).or_default() += 1;
    }
    let mut new = Vec::new();
    for line in parsed {
        let key = key_of(line.date, line.amount, &line.reference, &line.description);
        match held.get_mut(&key) {
            Some(count) if *count > 0 => *count -= 1,
            _ => new.push(NewLine {
                date: line.date,
                amount: line.amount,
                description: line.description.clone(),
                reference: line.reference.clone(),
                counterparty: line.counterparty.clone(),
            }),
        }
    }
    Ok(new)
}

/// Fails the `uploaded` `import` with `reason` and tells the uploader.
async fn fail(
    state: &AppState,
    import: &BankStatementImport,
    reason: &str,
) -> std::result::Result<BankStatementImport, TransitionError> {
    let _outcome = import
        .transition(FAIL)
        .by_system()
        .reason(reason.to_owned())
        .run(state)
        .await?;
    if let Some(user) = import.imported_by {
        let request = NotifyRequest::new(
            BANK_IMPORT_FAILED,
            wl_kernel::Subject::new("payments.bank_statement_import", import.id),
        )
        .data(json!({
            "file": import.file_name,
            "reason": reason,
            "link": "/bank-statements",
        }))
        .to_user(user);
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(reload(state, import).await?)
}

/// `user` completes the `in_review` `import`: every line is matched or
/// ignored, so the import's closing balance becomes the account's balance as
/// of the end of its period. `BankStatementCompleted` follows.
///
/// # Errors
///
/// `Forbidden` without `bank.reconcile`; `NotFrom` unless `in_review`;
/// `Guard` (422) while a line is `unmatched` or `suggested` (a drawing waiting
/// for its approval is named).
pub async fn complete(
    state: &AppState,
    user: &AuthUser,
    import: &BankStatementImport,
) -> std::result::Result<BankStatementImport, TransitionError> {
    let import = reload(state, import).await?;
    let lines = BankStatementLine::of_import(&state.db, import.id).await?;
    let open: Vec<_> = lines
        .iter()
        .filter(|line| matches!(line.status, LineStatus::Unmatched | LineStatus::Suggested))
        .collect();
    let input = CompleteInput {
        open: open.len() as i64,
        open_drawings: open
            .iter()
            .filter(|line| {
                line.status == LineStatus::Suggested && line.drawing_request_id.is_some()
            })
            .count() as i64,
    };
    let _outcome = import
        .transition(COMPLETE)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(reload(state, &import).await?)
}

/// `user` reverts the `in_review` `import`: its lines are removed and no
/// balance is ever read from it. The payments an import creates (none are
/// created before the match engine confirms incoming lines) would be
/// reversed here.
///
/// # Errors
///
/// `Forbidden` without `bank.reconcile`; `NotFrom` unless `in_review`;
/// `Guard` (422) while a line carries an owner drawing that is waiting for its
/// approval or recorded.
pub async fn revert(
    state: &AppState,
    user: &AuthUser,
    import: &BankStatementImport,
) -> std::result::Result<BankStatementImport, TransitionError> {
    let import = reload(state, import).await?;
    let drawings = BankStatementLine::of_import(&state.db, import.id)
        .await?
        .iter()
        .filter(|line| line.drawing_request_id.is_some())
        .filter(|line| matches!(line.status, LineStatus::Suggested | LineStatus::Matched))
        .count() as i64;
    let _outcome = import
        .transition(REVERT)
        .by(user)
        .input(&RevertInput { drawings })
        .run(state)
        .await?;
    Ok(reload(state, &import).await?)
}

/// The line `id` of an import that is `in_review`, still `unmatched`.
async fn open_line(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<(BankStatementLine, BankStatementImport), TransitionError> {
    if !user.has_permission(permissions::BANK_RECONCILE) {
        return Err(TransitionError::Forbidden);
    }
    let line = BankStatementLine::find_or_404(&state.db, id).await?;
    let import = BankStatementImport::find_or_404(&state.db, line.import_id).await?;
    if import.status != ImportStatus::InReview {
        return Err(TransitionError::NotFrom(import.status.as_str().to_owned()));
    }
    if line.status != LineStatus::Unmatched {
        return Err(guard("The line is not unmatched."));
    }
    Ok((line, import))
}

/// `user` leaves the `unmatched` line `id` out with a `reason` (bank charges,
/// an internal transfer).
///
/// # Errors
///
/// `Forbidden` without `bank.reconcile`; `NotFrom` unless the import is
/// `in_review`; `Guard` (422) for a line that is not `unmatched` or without a
/// reason.
pub async fn ignore(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<BankStatementLine, TransitionError> {
    let (mut line, _import) = open_line(state, user, id).await?;
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(guard("A reason is required to ignore a line."));
    }
    line.status = LineStatus::Ignored;
    line.ignore_reason = Some(reason.to_owned());
    line.save(&state.db).await?;
    Ok(line)
}

/// `user` marks the outgoing `unmatched` line `id` as an owner drawing of the
/// drawings category `category` (§5.7 step 3): the drawings approval request
/// is created with the line's date and amount (a transfer from the import's
/// bank account, the bank's reference), and the line stays `suggested`. A sole
/// owner approves at once: the payment is recorded and the line matched.
///
/// # Errors
///
/// `Forbidden` without `bank.reconcile`; `NotFrom` unless the import is
/// `in_review`; `Guard` (422) for a line that is not `unmatched`, not money
/// out, in a currency other than AED, or for a category that is unknown or has
/// no owner party; the refusals of the drawing request.
pub async fn mark_drawing(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    category: &str,
) -> std::result::Result<BankStatementLine, TransitionError> {
    let (mut line, import) = open_line(state, user, id).await?;
    if line.amount >= 0 {
        return Err(guard("Only money out can be an owner drawing."));
    }
    let Some(found) = owner_drawings(state)
        .category(&state.db, category.trim())
        .await?
    else {
        return Err(guard("Choose an owner drawings category."));
    };
    let Some(owner) = found.owner_party else {
        return Err(guard(
            "The drawings category has no owner party yet: set it first.",
        ));
    };
    let reference = if line.reference.trim().is_empty() {
        format!("LINE-{}", line.id)
    } else {
        line.reference.trim().to_owned()
    };
    let input = DrawingInput {
        legal_entity_id: line.legal_entity_id,
        category: found.code,
        party: owner,
        method: PaymentMethod::Transfer,
        method_details: json!({ "reference": reference }),
        amount: -line.amount,
        currency: line.currency.clone(),
        date: Some(line.line_date),
        reference: Some(reference),
        bank_account: Some(import.bank_account.clone()),
    };
    // Suggested before the request: a sole owner approves inside it and the
    // line is matched there.
    line.status = LineStatus::Suggested;
    line.marked_by = Some(user.id);
    line.reason = None;
    line.save(&state.db).await?;
    let drawing = match drawings::request_for_line(state, user, &input, line.id).await {
        Ok(drawing) => drawing,
        Err(error) => {
            let mut back = BankStatementLine::find_or_404(&state.db, line.id).await?;
            back.status = LineStatus::Unmatched;
            back.marked_by = None;
            back.save(&state.db).await?;
            return Err(TransitionError::Failed(error));
        }
    };
    let mut line = BankStatementLine::find_or_404(&state.db, line.id).await?;
    line.drawing_request_id = Some(drawing.id);
    line.save(&state.db).await?;
    Ok(line)
}

/// The drawing marked on a statement line was recorded: the line is matched to
/// its payment.
pub(crate) async fn drawing_recorded(state: &AppState, row: &PaymentDrawing) -> Result<()> {
    let Some(id) = row.statement_line_id else {
        return Ok(());
    };
    let Some(mut line) = BankStatementLine::find(&state.db, id).await? else {
        return Ok(());
    };
    line.status = LineStatus::Matched;
    line.match_kind = Some("payment".to_owned());
    line.match_id = row.payment_id;
    line.drawing_request_id = Some(row.id);
    line.reason = None;
    line.save(&state.db).await?;
    Ok(())
}

/// The drawing marked on a statement line was not approved (rejected,
/// withdrawn or expired): the line goes back to `unmatched` with the reason
/// and the accountant who marked it is told.
pub(crate) async fn drawing_rejected(state: &AppState, row: &PaymentDrawing) -> Result<()> {
    let Some(id) = row.statement_line_id else {
        return Ok(());
    };
    let Some(mut line) = BankStatementLine::find(&state.db, id).await? else {
        return Ok(());
    };
    let reason = row
        .reason
        .clone()
        .filter(|reason| !reason.trim().is_empty())
        .unwrap_or_else(|| "The owner drawing was not approved.".to_owned());
    line.status = LineStatus::Unmatched;
    line.drawing_request_id = None;
    line.reason = Some(reason.clone());
    line.save(&state.db).await?;
    if let Some(user) = line.marked_by {
        let request = NotifyRequest::new(
            BANK_LINE_DRAWING_REJECTED,
            wl_kernel::Subject::new("payments.bank_statement_import", line.import_id),
        )
        .data(json!({
            "amount": money_text(&line.currency, -line.amount),
            "date": line.line_date.to_string(),
            "reason": reason,
            "link": format!("/bank-statements/{}", line.import_id),
        }))
        .to_user(user);
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// The lines marked as owner drawings of `entities` dated in `from..=to`,
/// waiting for their approval or recorded (a kernel
/// [`BankStatements::drawing_lines`](wl_kernel::bank_statements::BankStatements::drawing_lines)).
///
/// # Errors
///
/// A database error.
pub(crate) fn drawing_lines<'a>(
    db: &'a Db,
    entities: &'a [i64],
    from: NaiveDate,
    to: NaiveDate,
) -> BankStatementFuture<'a, Vec<DrawingLine>> {
    Box::pin(async move {
        if entities.is_empty() {
            return Ok(Vec::new());
        }
        let lines = BankStatementLine::query()
            .where_(BankStatementLine::LEGAL_ENTITY_ID.is_in(entities.to_vec()))
            .where_(BankStatementLine::DRAWING_REQUEST_ID.is_not_null())
            .where_(BankStatementLine::LINE_DATE.gte(from))
            .where_(BankStatementLine::LINE_DATE.lte(to))
            .order_by(BankStatementLine::LINE_DATE)
            .order_by(BankStatementLine::ID)
            .get(db)
            .await?;
        let mut found = Vec::new();
        for line in lines {
            if !matches!(line.status, LineStatus::Suggested | LineStatus::Matched) {
                continue;
            }
            let Some(request) = line.drawing_request_id else {
                continue;
            };
            let Some(drawing) = PaymentDrawing::find(db, request).await? else {
                continue;
            };
            found.push(DrawingLine {
                line_id: line.id,
                import_id: line.import_id,
                legal_entity_id: line.legal_entity_id,
                bank_account: line.bank_account.clone(),
                date: line.line_date,
                amount: -line.amount,
                currency: line.currency.clone(),
                category: drawing.category.clone(),
                recorded: drawing.status == DrawingStatus::Recorded,
            });
        }
        Ok(found)
    })
}

/// Registers the listener: after `parse` commits `BankStatementImported`,
/// after `complete` `BankStatementCompleted`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<BankStatementImport>, state: AppState| async move {
            let name = event.transition.as_str();
            if name != PARSE && name != COMPLETE {
                return Ok(());
            }
            let import = BankStatementImport::find_or_404(&state.db, event.key).await?;
            if name == PARSE {
                return state
                    .emit(BankStatementImported {
                        import_id: import.id,
                        legal_entity_id: import.legal_entity_id,
                        bank_account: import.bank_account.clone(),
                        lines: import.line_count,
                    })
                    .await;
            }
            state
                .emit(BankStatementCompleted {
                    import_id: import.id,
                    legal_entity_id: import.legal_entity_id,
                    bank_account: import.bank_account.clone(),
                    closing_balance: import.closing_balance,
                    as_of: import.period_end,
                })
                .await
        },
    );
}
