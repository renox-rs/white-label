//! The affected line of a recall (wl-inventory §3 *Recall*, affected lines):
//! one subject (a visit, an order, a job card, a dispatch) that got a
//! recalled lot, with its party, the lot, the quantity, the date and the
//! notice status `pending`, `sent` or `waived`. The trace
//! ([`crate::recall_trace`]) makes them, the notices ([`crate::recall_notice`])
//! and the close ([`crate::recall_close`]) decide them.
//!
//! The notice status is a column of the line, not a lifecycle: `send_notices`
//! of the recall marks the lines it told, `waive` marks a line with a reason,
//! and a give-up of the message returns a `sent` line to `pending`.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::inventory::{
    LOCATION_SUBJECT_TYPE, LOT_SUBJECT_TYPE, RECALL_LINE_SUBJECT_TYPE, TraceMark, TracedHolder,
};
use wl_kernel::{PartyId, Subject};

/// What kind of evidence made a line (§4 forward trace).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineKind {
    /// A movement of the lot to the subject proves it.
    #[default]
    Confirmed,
    /// A pending issue whose lot was assigned when stock arrived (*lot
    /// uncertain*).
    Uncertain,
    /// Theoretical consumption of the item while the lot was on hand there.
    Possible,
    /// A dispatch of the lot to a recipient.
    Dispatch,
    /// A count of walk-in sales at one outlet on one day.
    WalkIn,
}

impl LineKind {
    /// How sure the trace is about a line of this kind.
    pub fn mark(self) -> TraceMark {
        match self {
            LineKind::Confirmed | LineKind::Dispatch | LineKind::WalkIn => TraceMark::Confirmed,
            LineKind::Uncertain => TraceMark::Uncertain,
            LineKind::Possible => TraceMark::Possible,
        }
    }
}

/// Whether the party of a line was told.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NoticeStatus {
    /// Not told yet (also after a notice that could not be delivered).
    #[default]
    Pending,
    /// A notice went out.
    Sent,
    /// The owner decided the party need not be told, with a reason.
    Waived,
}

/// One subject that got a recalled lot.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_recall_lines",
    unique(recall_id, trace_key),
    index(subject_type, subject_id)
)]
pub struct RecallLine {
    /// The primary key.
    pub id: i64,
    /// The recall.
    pub recall_id: i64,
    /// Names the line once per recall: a re-run adds only new keys.
    pub trace_key: String,
    /// What evidence made it.
    pub kind: LineKind,
    /// The type of the visit, order, job card or dispatch.
    pub subject_type: Option<String>,
    /// The id of the visit, order, job card or dispatch.
    pub subject_id: Option<i64>,
    /// The party to tell; `None` when the vertical does not know.
    pub party_id: Option<i64>,
    /// The lot.
    pub lot_id: Option<i64>,
    /// The location the lot left (the outlet of a walk-in count).
    pub location_id: Option<i64>,
    /// The quantity, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// The sales counted, for a walk-in line.
    pub sales_count: i64,
    /// The day it was used (Asia/Dubai).
    pub occurred_on: Option<NaiveDate>,
    /// The delivery status of a dispatch.
    pub delivery_status: Option<String>,
    /// Whether the party was told.
    pub notice_status: NoticeStatus,
    /// The template of the notice that went out.
    pub template: Option<String>,
    /// When the notice went out.
    pub sent_at: Option<DateTime>,
    /// The user who sent it.
    pub sent_by: Option<i64>,
    /// Why the owner decided the party need not be told.
    pub waive_reason: Option<String>,
    /// The user who waived it.
    pub waived_by: Option<i64>,
    /// When it was waived.
    pub waived_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RecallLine {
    /// The line as a [`Subject`]: `inventory.recall_line:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(RECALL_LINE_SUBJECT_TYPE, self.id)
    }

    /// The visit, order, job card or dispatch.
    pub fn used_by(&self) -> Option<Subject> {
        match (&self.subject_type, self.subject_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The kernel's view of the line.
    pub fn holder(&self) -> TracedHolder {
        TracedHolder {
            line: self.subject(),
            subject: self.used_by(),
            party: self.party_id.map(PartyId),
            lot: self.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id)),
            location: self
                .location_id
                .map(|id| Subject::new(LOCATION_SUBJECT_TYPE, id)),
            quantity_milli: self.quantity_milli,
            on: self.occurred_on,
            kind: self.kind.as_str().to_owned(),
            mark: self.kind.mark(),
            sales: self.sales_count,
            delivery_status: self.delivery_status.clone(),
            notice: self.notice_status.as_str().to_owned(),
        }
    }
}

/// The lines of recall `recall_id`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn lines_of<'c>(
    db: impl renox::db::Executor<'c>,
    recall_id: i64,
) -> Result<Vec<RecallLine>> {
    RecallLine::query()
        .where_(RecallLine::RECALL_ID.eq(recall_id))
        .order_by(RecallLine::ID)
        .get(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_marked_by_the_evidence_that_made_it() {
        assert_eq!(LineKind::Confirmed.mark(), TraceMark::Confirmed);
        assert_eq!(LineKind::Dispatch.mark(), TraceMark::Confirmed);
        assert_eq!(LineKind::Uncertain.mark(), TraceMark::Uncertain);
        assert_eq!(LineKind::Possible.mark(), TraceMark::Possible);
    }

    #[test]
    fn the_kernel_sees_the_subject_party_and_notice_of_a_line() {
        let line = RecallLine {
            id: 7,
            subject_type: Some("clinics.visit".into()),
            subject_id: Some(3),
            party_id: Some(11),
            lot_id: Some(9),
            quantity_milli: 2_000,
            kind: LineKind::Possible,
            sales_count: 1,
            ..RecallLine::default()
        };
        let holder = line.holder();
        assert_eq!(holder.line.to_string(), "inventory.recall_line:7");
        assert_eq!(holder.subject.unwrap().to_string(), "clinics.visit:3");
        assert_eq!(holder.party, Some(PartyId(11)));
        assert_eq!(holder.mark, TraceMark::Possible);
        assert_eq!(holder.notice, "pending");
        assert_eq!(holder.kind, "possible");
    }
}
