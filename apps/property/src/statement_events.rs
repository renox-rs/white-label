//! What follows the commit of an owner statement transition (01 Property
//! §7.1 *Owner statement*): the preparer is told of a `return`, `publish`
//! sends the statement to the owner by e-mail, and `revise` tells the owner
//! that a corrected statement follows.
//!
//! The settling transitions (`pay_out`, `invoice_owner` and their approved
//! twins) record the payout or make the invoice
//! ([`crate::statement_payout::after`]).
//!
//! The PDF of the statement is `DOC.GEN.01` (wl-documents), which has no
//! contract in the kernel yet: the e-mail carries the statement as text, the
//! lines included.

use renox::auth::permissions::users_with_role;
use renox::prelude::*;
use renox::state_machine::Transitioned;

use crate::money_text;
use crate::notices;
use crate::permissions::ACCOUNTANT_ROLE;
use crate::statement::{OwnerStatement, StatementLine};
use crate::statement_machine::{PUBLISH, RETURN, REVISE};
use crate::statement_payout;

/// The lines of `statement` as text: `Rent L-000012: AED 12,000.00; ...`.
async fn lines_text(db: &Db, statement: &OwnerStatement) -> Result<String> {
    let lines: Vec<StatementLine> = statement.lines(db).await?;
    Ok(lines
        .iter()
        .map(|line| format!("{}: {}", line.description, money_text(line.amount)))
        .collect::<Vec<_>>()
        .join("; "))
}

/// The users to tell that `statement` was returned: its preparer, else the
/// accountants.
async fn preparers(state: &AppState, statement: &OwnerStatement) -> Result<Vec<i64>> {
    if let Some(preparer) = statement.prepared_by {
        return Ok(vec![preparer]);
    }
    Ok(users_with_role(&state.db, ACCOUNTANT_ROLE)
        .await?
        .iter()
        .map(|user| user.id)
        .collect())
}

/// Runs what follows `event`, a transition of an owner statement.
///
/// # Errors
///
/// The database's and the notifier's errors.
pub async fn after(state: &AppState, event: &Transitioned<OwnerStatement>) -> Result {
    let Some(found) = OwnerStatement::find(&state.db, event.key).await? else {
        return Ok(());
    };
    match event.transition.as_str() {
        RETURN => {
            let users = preparers(state, &found).await?;
            let reason = event.reason.clone().unwrap_or_default();
            notices::statement_returned(state, &found, &users, &reason).await
        }
        PUBLISH => {
            let text = lines_text(&state.db, &found).await?;
            notices::statement_published(state, &found, &text).await
        }
        REVISE => {
            notices::statement_revised(state, &found, found.revision + 1).await?;
            crate::statement_dispute::after_revise(state, &found).await
        }
        _ => statement_payout::after(state, event).await,
    }
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<OwnerStatement>, state: AppState| async move {
            after(&state, &event).await
        },
    );
}
