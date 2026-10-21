//! The invoice dispute behind a `client_dispute` adjustment (wl-attendance §4
//! *Correcting a locked timesheet*, §5.7 point 4, §5.8; BL-ATT-09).
//!
//! When a client disputes billed hours the invoice is flagged `disputed`
//! (wl-invoicing). When the business and the client agree to cut hours, a
//! user with `timesheets.adjust` opens an adjustment of origin
//! `client_dispute` ([`crate::adjustment::open`]) linked to the open dispute.
//! Nothing opens it automatically and no task is raised here: the dispute's
//! own ticket is the work item. The order is: open the adjustment, approve
//! it ([`crate::approval`], by the original's approver kind), the credit
//! note is raised from the approval (wl-invoicing), the dispute is resolved
//! `credited` naming the adjustment.
//!
//! What this module adds to opening:
//!
//! - **The link must be to an open dispute.** A link to a dispute
//!   (`invoicing.dispute:7`) is checked through the kernel's
//!   [`Invoicer::dispute_state`]: an unknown dispute and one already resolved
//!   are refused. A link to a ticket, or to any other record, is kept as it
//!   was given (the spec lets a ticket stand in for a dispute).
//! - **Pay is cut only with evidence.** `affects` defaults to `billing`
//!   (`default_affects` in [`crate::adjustment`]); a `client_dispute` that
//!   affects `payroll` or `both` is refused unless attendance evidence is
//!   attached: attendance records of the owner, or documents
//!   ([`AdjustmentEvidence`]).
//!
//! What happens afterwards is driven by `InvoiceDisputeResolved`
//! ([`on_resolved`]):
//!
//! - `credited`: the approved adjustments linked to that dispute take the
//!   credit note's id (`credit_note_id`); the credit note links to the
//!   adjustment on wl-invoicing's side, so the link is there both ways. The
//!   link is informational: the credit note's amount is wl-invoicing's.
//! - `upheld`: an adjustment of that dispute that is not yet approved gets
//!   `dispute_upheld_at`; its `submit` is then refused by the machine ("dispute
//!   resolved upheld") and its creator is told once. It has no payroll or
//!   billing effect. A dispute that ends `upheld` with no adjustment needs
//!   none, and none is created.
//!   One that is already `submitted` when the dispute ends `upheld` is refused
//!   by `approve` and `approve_on_behalf` too ("cannot be approved"), so it
//!   never emits `TimesheetApproved`.
//!
//! Decisions:
//!
//! - The upheld mark is stored on the adjustment instead of asking the
//!   invoicer at `submit`, because a transition guard has no application
//!   state and because the creator must be told when the dispute ends, not
//!   when they try to submit.
//! - An adjustment that is already `approved` when its dispute ends `upheld`
//!   is left as it is: its approval was the agreed outcome and wl-invoicing
//!   has applied it.

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::attendance::ATTENDANCE_SUBJECT_TYPE;
use wl_kernel::invoicing::{
    DISPUTE_SUBJECT_TYPE, DisputeOutcome, InvoiceDisputeResolved, invoicer,
};

use crate::notices;
use crate::record::AttendanceRecord;
use crate::service::refuse;
use crate::texts::english;
use crate::timesheet::{AdjustmentOrigin, Affects, Timesheet, TimesheetKind, TimesheetStatus};

/// The subject type of a document of wl-documents, which may serve as
/// attendance evidence.
pub const DOCUMENT_SUBJECT_TYPE: &str = "wl_documents.document";

/// A record that shows the attendance was wrong, attached to an adjustment.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_attendance_adjustment_evidence", index(timesheet_id))]
pub struct AdjustmentEvidence {
    /// The primary key.
    pub id: i64,
    /// The adjustment.
    #[model(references = "wl_attendance_timesheets")]
    pub timesheet_id: i64,
    /// The type of the evidence's `Subject`.
    pub evidence_type: String,
    /// The id of the evidence's `Subject`.
    pub evidence_id: i64,
    /// The user who attached it.
    pub attached_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AdjustmentEvidence {
    /// The evidence as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(&self.evidence_type, self.evidence_id)
    }

    /// The evidence attached to the adjustment `timesheet_id`, oldest first.
    pub async fn of_timesheet(db: &Db, timesheet_id: i64) -> Result<Vec<AdjustmentEvidence>> {
        AdjustmentEvidence::query()
            .where_(AdjustmentEvidence::TIMESHEET_ID.eq(timesheet_id))
            .order_by(AdjustmentEvidence::ID)
            .get(db)
            .await
    }
}

/// Reads the evidence of a form: subjects as `type:id`, separated by commas.
///
/// # Errors
///
/// `422` naming the part that is not a subject.
pub fn parse_evidence(text: &str) -> Result<Vec<Subject>> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<Subject>().map_err(|_| {
                refuse(english(
                    "wl_attendance.adjustment.bad_evidence",
                    &[("detail", &part)],
                ))
            })
        })
        .collect()
}

/// Whether an adjustment of `origin` that affects `affects` needs attendance
/// evidence: a client's dispute never cuts pay by itself.
pub fn needs_evidence(origin: AdjustmentOrigin, affects: Affects) -> bool {
    origin == AdjustmentOrigin::ClientDispute && affects != Affects::Billing
}

/// Checks the `linked` record of an adjustment: a dispute must exist and be
/// still open.
///
/// # Errors
///
/// `422` for an unknown dispute or one already resolved; the invoicer's
/// errors.
pub async fn check_link(state: &AppState, linked: Option<&Subject>) -> Result {
    let Some(link) = linked.filter(|s| s.subject_type == DISPUTE_SUBJECT_TYPE) else {
        return Ok(());
    };
    match invoicer(state)
        .dispute_state(&state.db, link.subject_id)
        .await?
    {
        None => Err(refuse(english(
            "wl_attendance.adjustment.dispute_unknown",
            &[("dispute", link)],
        ))),
        Some(found) if !found.is_open() => Err(refuse(english(
            "wl_attendance.adjustment.dispute_resolved",
            &[("dispute", link)],
        ))),
        Some(_) => Ok(()),
    }
}

/// Checks the evidence of an adjustment of `original`: an attendance record
/// must be one of the owner's; a document is taken as given; anything else
/// is refused.
///
/// # Errors
///
/// `422` for evidence that is not a record of the owner or a document.
pub async fn check_evidence(
    state: &AppState,
    original: &Timesheet,
    evidence: &[Subject],
) -> Result {
    for item in evidence {
        let ok = match item.subject_type.as_str() {
            ATTENDANCE_SUBJECT_TYPE => AttendanceRecord::find(&state.db, item.subject_id)
                .await?
                .is_some_and(|record| record.employee_id == original.owner_employee_id),
            DOCUMENT_SUBJECT_TYPE => true,
            _ => false,
        };
        if !ok {
            return Err(refuse(english(
                "wl_attendance.adjustment.bad_evidence",
                &[("detail", item)],
            )));
        }
    }
    Ok(())
}

/// Attaches `evidence` to the adjustment `adjustment_id` inside `tx`.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn attach_in(
    tx: &mut Transaction,
    adjustment_id: i64,
    evidence: &[Subject],
    by: Option<i64>,
) -> Result {
    for item in evidence {
        AdjustmentEvidence::create(
            &mut *tx,
            AdjustmentEvidence {
                timesheet_id: adjustment_id,
                evidence_type: item.subject_type.clone(),
                evidence_id: item.subject_id,
                attached_by: by,
                ..AdjustmentEvidence::default()
            },
        )
        .await?;
    }
    Ok(())
}

/// The `client_dispute` adjustments linked to the dispute `dispute`.
async fn adjustments_of(db: &Db, dispute: i64) -> Result<Vec<Timesheet>> {
    Timesheet::query()
        .where_(Timesheet::KIND.eq(TimesheetKind::Adjustment))
        .where_(Timesheet::ORIGIN.eq(AdjustmentOrigin::ClientDispute))
        .where_(Timesheet::LINKED_TYPE.eq(DISPUTE_SUBJECT_TYPE))
        .where_(Timesheet::LINKED_ID.eq(dispute))
        .order_by(Timesheet::ID)
        .get(db)
        .await
}

/// Reacts to a resolved dispute (see the module docs). Run twice for the
/// same event it changes and says nothing more.
///
/// # Errors
///
/// The database's errors; the notifier's.
pub async fn on_resolved(state: &AppState, event: &InvoiceDisputeResolved) -> Result {
    for sheet in adjustments_of(&state.db, event.dispute).await? {
        let approved = matches!(
            sheet.status,
            TimesheetStatus::Approved | TimesheetStatus::Locked
        );
        match event.outcome {
            DisputeOutcome::Credited => {
                let Some(note) = event.credit_note.as_ref() else {
                    continue;
                };
                if approved && sheet.credit_note_id.is_none() {
                    Timesheet::query()
                        .where_(Timesheet::ID.eq(sheet.id))
                        .update(&state.db, &[("credit_note_id", &Some(note.subject_id))])
                        .await?;
                }
            }
            DisputeOutcome::Upheld => {
                if approved {
                    continue;
                }
                // Claimed once: a repeat of the event finds the mark set.
                let claimed = Timesheet::query()
                    .where_(Timesheet::ID.eq(sheet.id))
                    .where_(Timesheet::DISPUTE_UPHELD_AT.is_null())
                    .update(&state.db, &[("dispute_upheld_at", &Some(now()))])
                    .await?;
                if claimed == 1 {
                    notices::adjustment_dispute_upheld(state, &sheet).await?;
                }
            }
        }
    }
    Ok(())
}

/// Registers the listener of `InvoiceDisputeResolved`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: InvoiceDisputeResolved, state: AppState| async move {
            on_resolved(&state, &event).await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_client_dispute_that_touches_pay_needs_evidence() {
        use AdjustmentOrigin::{AttendanceError, ClientDispute};
        assert!(!needs_evidence(ClientDispute, Affects::Billing));
        assert!(needs_evidence(ClientDispute, Affects::Both));
        assert!(needs_evidence(ClientDispute, Affects::Payroll));
        assert!(!needs_evidence(AttendanceError, Affects::Both));
    }

    #[test]
    fn evidence_is_read_as_subjects() {
        let found = parse_evidence("attendance.record:5, wl_documents.document:9").unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], Subject::new("attendance.record", 5));
        assert!(parse_evidence("").unwrap().is_empty());
        assert!(parse_evidence("nonsense").is_err());
    }
}
