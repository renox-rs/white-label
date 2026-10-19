//! The rule runs "already fired, review" of a late entry (wl-kernel §5.13
//! step 5).
//!
//! The kernel never undoes a run that happened before the late record was
//! entered. It lists the runs whose subject is the late record itself or the
//! record named by its parent-subject field, and whose trigger time (the
//! time the run was queued) falls between the business time and the entry
//! time, both ends included. An entity that names no parent only looks at
//! runs on itself.

use renox::db::{DateTime, Transaction};
use renox::prelude::*;

use super::events::FiredRun;
use crate::automation::RuleRun;
use crate::subject::Subject;

/// The runs to review for a record of `subject_type` (`record_id` once it
/// exists) whose parent subject is `parent`, between `occurred_at` and
/// `entered_at`, oldest first.
pub async fn find(
    tx: &mut Transaction,
    subject_type: &str,
    record_id: Option<i64>,
    parent: Option<&Subject>,
    occurred_at: DateTime,
    entered_at: DateTime,
) -> Result<Vec<FiredRun>> {
    let mut subjects: Vec<Subject> = Vec::new();
    if let Some(id) = record_id {
        subjects.push(Subject::new(subject_type, id));
    }
    if let Some(parent) = parent {
        subjects.push(parent.clone());
    }
    if subjects.is_empty() {
        return Ok(Vec::new());
    }
    let runs = RuleRun::query()
        .where_(RuleRun::CREATED_AT.gte(occurred_at))
        .where_(RuleRun::CREATED_AT.lte(entered_at))
        .where_any(move |mut query| {
            for subject in subjects {
                query = query.where_all(move |inner| {
                    inner
                        .where_eq("subject_type", subject.subject_type)
                        .where_eq("subject_id", subject.subject_id)
                });
            }
            query
        })
        .order_by(RuleRun::CREATED_AT)
        .order_by(RuleRun::ID)
        .get(&mut *tx)
        .await?;
    Ok(runs
        .into_iter()
        .map(|run| FiredRun {
            run_id: run.id,
            rule: run.rule.clone(),
            subject: run.subject(),
            at: run.created_at.unwrap_or(occurred_at),
        })
        .collect())
}
