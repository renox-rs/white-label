//! The notice of a product recall to a patient (05 §4.6 *Supplier,
//! manufacturer or regulator recalls a lot*; BL-CLN-14): one per patient and
//! recall, to the patient or the guardian of a dependant, listing the visit
//! dates. Its text names the clinic and asks the patient to call; it never
//! names the product, the reason or any clinical detail (`CLN.ACL.01`).
//!
//! ```text
//! [*] --> sent --give_up (system, MessageGaveUp)--> gave_up --called--> called
//! ```
//!
//! - `give_up`: the notice could not be delivered on any channel. The lines
//!   of the recall it told go back to `pending`, and wl-notify's give-up
//!   reaction opens the call task for the front desk of the visit's branch.
//! - `called`: the front desk called the patient and recorded the outcome;
//!   the lines are `sent` again.

use renox::db::{DateTime, Json};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::permissions;
use crate::texts::english;

/// The subject type of a recall notice.
pub const RECALL_NOTICE_SUBJECT_TYPE: &str = "clinics.recall_notice";

/// The notification type of the notice (05 §7.2 `clinic.product_recall_notice`).
pub const NOTICE_TYPE: &str = "clinics.product_recall_notice";

/// The system's transition when the notice could not be delivered.
pub const GIVE_UP: &str = "give_up";

/// The front desk's transition: the patient was called.
pub const CALLED: &str = "called";

/// Where a recall notice stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RecallNoticeStatus {
    /// Handed to the notifier.
    #[default]
    Sent,
    /// It could not be delivered; the front desk has a call to make.
    GaveUp,
    /// The front desk called the patient and recorded the outcome.
    Called,
}

/// The notice of a recall to one patient (or guardian).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "clinics_recall_notices",
    state_machine,
    index(recall_id),
    index(status)
)]
pub struct RecallNotice {
    /// The primary key.
    pub id: i64,
    /// The recall (`inventory.recall:2`), by id.
    pub recall_id: i64,
    /// The party told: the patient, or the guardian of a dependant.
    pub party_id: i64,
    /// The patient the visits were for (the dependant, when it is one).
    pub patient_party_id: Option<i64>,
    /// The visits the notice covers.
    pub visit_ids: Json<Vec<i64>>,
    /// The lines of the recall (`inventory.recall_line`) it tells.
    pub line_ids: Json<Vec<i64>>,
    /// The visit days, comma separated, oldest first (what the text says).
    pub visit_dates: String,
    /// Where it stands.
    pub status: RecallNoticeStatus,
    /// The user who sent it.
    pub sent_by: Option<i64>,
    /// When it was handed to the notifier.
    pub sent_at: Option<DateTime>,
    /// When it gave up.
    pub gave_up_at: Option<DateTime>,
    /// What the call came to.
    pub call_outcome: Option<String>,
    /// The user who called.
    pub called_by: Option<i64>,
    /// When the call was recorded.
    pub called_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RecallNotice {
    /// The notice as a [`Subject`]: `clinics.recall_notice:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(RECALL_NOTICE_SUBJECT_TYPE, self.id)
    }

    /// The recall as a [`Subject`]: `inventory.recall:2`.
    pub fn recall(&self) -> Subject {
        Subject::new(wl_kernel::inventory::RECALL_SUBJECT_TYPE, self.recall_id)
    }

    /// The notices of recall `recall`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_recall(db: &Db, recall: i64) -> Result<Vec<RecallNotice>> {
        RecallNotice::query()
            .where_(RecallNotice::RECALL_ID.eq(recall))
            .order_by(RecallNotice::ID)
            .get(db)
            .await
    }

    /// The notices that gave up and wait for a call, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn waiting_for_a_call(db: &Db) -> Result<Vec<RecallNotice>> {
        RecallNotice::query()
            .where_(RecallNotice::STATUS.eq(RecallNoticeStatus::GaveUp))
            .order_by(RecallNotice::ID)
            .get(db)
            .await
    }
}

/// What a run carries to the effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeInput {
    /// When it happened.
    #[serde(default)]
    pub at: Option<DateTime>,
    /// What the call came to (`called`).
    #[serde(default)]
    pub outcome: Option<String>,
    /// The user who called (`called`).
    #[serde(default)]
    pub by: Option<i64>,
}

fn input_of(ctx: &TransitionCtx) -> NoticeInput {
    ctx.input_as::<NoticeInput>().unwrap_or_default()
}

impl StateMachine for RecallNotice {
    type State = RecallNoticeStatus;
    const COLUMN: renox::db::Col<RecallNotice, RecallNoticeStatus> = RecallNotice::STATUS;

    fn machine() -> Machine<Self> {
        use RecallNoticeStatus::{Called, GaveUp, Sent};
        Machine::new()
            .initial([Sent])
            .state(Sent)
            .tone("info")
            .state(GaveUp)
            .tone("warning")
            .state(Called)
            .tone("success")
            .terminal()
            .transition(give_up())
            .transition(called())
    }
}

/// `give_up`: the notice could not be delivered.
fn give_up() -> Transition<RecallNotice> {
    Transition::new(GIVE_UP)
        .label("Gave up")
        .system()
        .from([RecallNoticeStatus::Sent])
        .to(RecallNoticeStatus::GaveUp)
        .effect(|n: &RecallNotice, ctx, tx| {
            let id = n.id;
            let at = input_of(ctx).at.unwrap_or_else(renox::db::now);
            Box::pin(async move {
                RecallNotice::query()
                    .where_(RecallNotice::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[("gave_up_at", &Some(at)), ("updated_at", &renox::db::now())],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `called`: the front desk called the patient and recorded the outcome.
fn called() -> Transition<RecallNotice> {
    Transition::new(CALLED)
        .label("Record the call")
        .tone("primary")
        .permission(permissions::RECALLS_CALL)
        .from([RecallNoticeStatus::GaveUp])
        .to(RecallNoticeStatus::Called)
        .guard("outcome_given", |_n: &RecallNotice, ctx| {
            let given = input_of(ctx)
                .outcome
                .is_some_and(|outcome| !outcome.trim().is_empty());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("clinics.recall.problem.outcome", &[]))
                }
            })
        })
        .effect(|n: &RecallNotice, ctx, tx| {
            let id = n.id;
            let input = input_of(ctx);
            let at = input.at.unwrap_or_else(renox::db::now);
            let by = input.by;
            let outcome = input.outcome.map(|text| text.trim().to_owned());
            Box::pin(async move {
                RecallNotice::query()
                    .where_(RecallNotice::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("call_outcome", &outcome),
                            ("called_by", &by),
                            ("called_at", &Some(at)),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// Registers the machine for `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<RecallNotice>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition() {
        let machine = RecallNotice::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("sent", Some("info"), false),
                ("gave_up", Some("warning"), false),
                ("called", Some("success"), true),
            ]
        );
        let give_up = machine.find(GIVE_UP).unwrap();
        assert_eq!(give_up.from_states(), vec!["sent"]);
        assert_eq!(give_up.to_state(), Some("gave_up"));
        assert!(give_up.is_system());
        let called = machine.find(CALLED).unwrap();
        assert_eq!(called.from_states(), vec!["gave_up"]);
        assert_eq!(called.to_state(), Some("called"));
        assert!(!called.is_system());
        assert_eq!(machine.transitions().len(), 2);
    }
}
