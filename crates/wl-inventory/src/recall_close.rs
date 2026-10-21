//! Closing a recall and its report (wl-inventory §4 *Recall* `close`, §5.7
//! step 6, issue #235): a recall closes when every lot has an outcome (its
//! stock holds are `closed`: every line decided and posted, a `release`
//! approved by a second person for `STK.ACL.05`) and every affected party was
//! told or waived with a reason. The outcomes are the closed hold's.
//!
//! [`close`] (`recalls.manage`) builds the recall report (scope, lots, trace,
//! notices and outcomes), a PDF document, and the `close` transition keeps it
//! on the recall (`GET /recalls/{id}/report`). The report is the evidence
//! pack for the authority (`AU-20`, not verified): nothing is filed with an
//! authority. [`RecallClosed`](wl_kernel::inventory::RecallClosed) follows
//! the commit.
//!
//! Decisions: the document is kept on the recall row, as the return note is on
//! its return, until the kernel has a contract to store a file in
//! wl-documents; a hold that is `open` (a line undecided, or a `release`
//! waiting for its approval) or a line that is `pending` (also one with the
//! party unknown, and a count of walk-in sales) refuses the close with the
//! count named.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use wl_kernel::inventory::{LOT_SUBJECT_TYPE, RecallClosed};
use wl_kernel::Subject;

use crate::hold_decide::closed_event;
use crate::item::{Item, quantity_text};
use crate::location::Location;
use crate::lot::Lot;
use crate::recall::{CLOSE, CloseInput, Recall, RecallScope, RecallStatus, lots_in_scope};
use crate::recall_line::{LineKind, NoticeStatus, RecallLine, lines_of};
use crate::recall_trace::holds_of;
use crate::return_note::pages_document;
use crate::stock_hold::{HoldStatus, lines_of as hold_lines_of};

/// The width of a report line, in characters.
const WIDTH: usize = 92;

/// `text` cut into lines of at most [`WIDTH`] characters.
fn wrapped(size: i64, text: &str) -> Vec<(i64, String)> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > WIDTH {
            lines.push((size, std::mem::take(&mut current)));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push((size, current));
    }
    lines
}

/// The text of one affected line.
async fn line_text(db: &Db, line: &RecallLine) -> Result<String> {
    let lot = match line.lot_id {
        Some(id) => Lot::find(db, id).await?.map(|lot| lot.number),
        None => None,
    };
    let lot = lot.map(|number| format!(" lot {number}")).unwrap_or_default();
    let date = line.occurred_on.map(|d| d.to_string()).unwrap_or_default();
    let notice = match line.notice_status {
        NoticeStatus::Pending => "pending".to_owned(),
        NoticeStatus::Sent => format!(
            "sent ({})",
            line.template.clone().unwrap_or_default()
        ),
        NoticeStatus::Waived => format!(
            "waived: {}",
            line.waive_reason.clone().unwrap_or_default()
        ),
    };
    if line.kind == LineKind::WalkIn {
        let outlet = match line.location_id {
            Some(id) => Location::find(db, id).await?.map(|l| l.name_en),
            None => None,
        };
        return Ok(format!(
            "{} sales, {}, {date}{lot} - {notice}",
            line.sales_count,
            outlet.unwrap_or_default()
        ));
    }
    let subject = line.used_by().map(|s| s.to_string()).unwrap_or_default();
    let party = line
        .party_id
        .map_or_else(|| "party unknown".to_owned(), |id| format!("party {id}"));
    let mark = match line.kind {
        LineKind::Uncertain => ", lot uncertain",
        LineKind::Possible => ", possible",
        LineKind::Dispatch => ", dispatch",
        _ => "",
    };
    let delivery = line
        .delivery_status
        .as_deref()
        .map(|status| format!(", delivery {status}"))
        .unwrap_or_default();
    Ok(format!(
        "{subject} ({party}){lot} {} {date}{mark}{delivery} - {notice}",
        quantity_text(line.quantity_milli)
    ))
}

/// The recall report of `recall` as a PDF document: scope, lots, trace,
/// notices and outcomes.
///
/// # Errors
///
/// A database error.
pub async fn report(db: &Db, recall: &Recall) -> Result<String> {
    let number = recall.number.clone().unwrap_or_default();
    let mut lines: Vec<(i64, String)> = vec![
        (16, format!("Recall report {number}")),
        (11, format!("Source: {}", recall.source.as_str())),
    ];
    lines.extend(wrapped(11, &format!("Reason: {}", recall.reason)));
    if let Some(at) = recall.opened_at {
        lines.push((
            11,
            format!("Opened: {}", wl_kernel::documents::dubai_text(at)),
        ));
    }
    if let Some(at) = recall.closed_at.or(Some(renox::db::now())) {
        lines.push((
            11,
            format!("Report made: {}", wl_kernel::documents::dubai_text(at)),
        ));
    }
    lines.push((11, String::new()));
    lines.push((13, "Scope".to_owned()));
    let scope = RecallScope::query()
        .where_(RecallScope::RECALL_ID.eq(recall.id))
        .order_by(RecallScope::ID)
        .get(db)
        .await?;
    for entry in &scope {
        let item = Item::find(db, entry.item_id).await?;
        let code = item.map(|item| item.code).unwrap_or_default();
        let what = match (&entry.lot_number, &entry.supplier_lot) {
            (Some(number), _) => format!("lot {number}"),
            (None, Some(number)) => format!("supplier lot {number}"),
            _ => String::new(),
        };
        lines.push((10, format!("{code}: {what}")));
    }
    lines.push((11, String::new()));
    lines.push((13, "Lots held".to_owned()));
    for lot_id in lots_in_scope(db, recall.id).await? {
        let lot = Lot::find_or_404(db, lot_id).await?;
        let item = Item::find(db, lot.item_id).await?;
        lines.push((
            10,
            format!(
                "{} lot {} ({})",
                item.map(|item| item.code).unwrap_or_default(),
                lot.number,
                lot.status.as_str()
            ),
        ));
    }
    lines.push((11, String::new()));
    lines.push((13, "Trace: who got the goods".to_owned()));
    let found = lines_of(db, recall.id).await?;
    if found.is_empty() {
        lines.push((10, "No subject used the lots.".to_owned()));
    }
    for (position, line) in (1..).zip(&found) {
        let text = line_text(db, line).await?;
        lines.extend(wrapped(10, &format!("{position}. {text}")));
    }
    lines.push((11, String::new()));
    lines.push((13, "Outcomes".to_owned()));
    for hold in holds_of(db, recall.id).await? {
        for held in hold_lines_of(db, hold.id).await? {
            let item = Item::find(db, held.item_id).await?;
            let place = Location::find(db, held.location_id).await?;
            let lot = match held.lot_id {
                Some(id) => Lot::find(db, id).await?.map(|lot| lot.number),
                None => None,
            };
            lines.extend(wrapped(
                10,
                &format!(
                    "{} lot {} at {}: {} - {}",
                    item.map(|item| item.code).unwrap_or_default(),
                    lot.unwrap_or_default(),
                    place.map(|l| l.name_en).unwrap_or_default(),
                    quantity_text(held.quantity_milli),
                    held.outcome.clone().unwrap_or_else(|| "undecided".to_owned()),
                ),
            ));
        }
    }
    Ok(pages_document(&lines))
}

/// `user` (a holder of `recalls.manage`) closes recall `recall_id` (§4
/// `close`).
///
/// # Errors
///
/// `Forbidden` without the permission; `NotFrom` unless the recall is
/// `traced`; `Guard` (422) while a stock hold of the recall is `open` or an
/// affected line is `pending`.
pub async fn close(
    state: &AppState,
    user: &AuthUser,
    recall_id: i64,
) -> std::result::Result<Recall, TransitionError> {
    let recall = Recall::find_or_404(&state.db, recall_id).await?;
    let holds = holds_of(&state.db, recall.id).await?;
    let holds_closed = holds.iter().all(|hold| hold.status == HoldStatus::Closed);
    let pending_lines = lines_of(&state.db, recall.id)
        .await?
        .iter()
        .filter(|line| line.notice_status == NoticeStatus::Pending)
        .count() as i64;
    let report = if recall.status == RecallStatus::Traced && holds_closed && pending_lines == 0 {
        report(&state.db, &recall).await?
    } else {
        String::new()
    };
    let input = CloseInput {
        holds_closed,
        pending_lines,
        by: Some(user.id),
        report,
    };
    let _outcome = recall
        .transition(CLOSE)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(Recall::find_or_404(&state.db, recall_id).await?)
}

/// The event of the closed recall `recall`.
///
/// # Errors
///
/// A database error.
pub async fn closed_event_of(db: &Db, recall: &Recall) -> Result<RecallClosed> {
    let mut outcomes = Vec::new();
    for hold in holds_of(db, recall.id).await? {
        let held = hold_lines_of(db, hold.id).await?;
        outcomes.extend(closed_event(&hold, &held).outcomes);
    }
    Ok(RecallClosed {
        recall: recall.subject(),
        number: recall.number.clone().unwrap_or_default(),
        lots: lots_in_scope(db, recall.id)
            .await?
            .iter()
            .map(|id| Subject::new(LOT_SUBJECT_TYPE, *id))
            .collect(),
        lines: lines_of(db, recall.id)
            .await?
            .iter()
            .map(RecallLine::holder)
            .collect(),
        outcomes,
    })
}

/// Registers the listener that tells the other modules a recall closed,
/// after the commit.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Recall>, state: AppState| async move {
        if event.transition != CLOSE {
            return Ok(());
        }
        let recall = Recall::find_or_404(&state.db, event.key).await?;
        state.emit(closed_event_of(&state.db, &recall).await?).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_text_is_cut_at_words() {
        let text = "word ".repeat(60);
        let lines = wrapped(10, &text);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|(_, line)| line.chars().count() <= WIDTH));
        assert_eq!(wrapped(10, "").len(), 1);
    }
}
