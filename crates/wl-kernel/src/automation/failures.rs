//! The failures of rule runs (wl-kernel §4.2, `PLT.RUL.03`): one row each
//! time a run reaches `failed` (retries exhausted) or `stopped` (chain depth
//! above 3), written by the `fail` and `stop` transitions in their own
//! transaction.
//!
//! A run's `rerun` clears its error and times, so the daily digest of failed
//! runs reads these rows: a run that failed and was rerun before the digest
//! is still listed, marked *rerun, succeeded*.

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::run::RuleRun;
use crate::subject::Subject;

/// How a run ended without doing its work.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FailureKind {
    /// The last attempt failed.
    #[default]
    Failed,
    /// The chain was too deep: nothing ran.
    Stopped,
}

/// One failure of a rule run (`wl_rule_run_failures`).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_rule_run_failures", index(at), index(run_id))]
pub struct RuleRunFailure {
    /// The primary key.
    pub id: i64,
    /// The rule run.
    pub run_id: i64,
    /// The rule id, e.g. `FIN.RUL.01`.
    pub rule: String,
    /// The type of the record the run was about.
    pub subject_type: String,
    /// The record's id.
    pub subject_id: i64,
    /// The occurrence the run was for.
    pub occurrence_key: String,
    /// Failed or stopped.
    pub kind: FailureKind,
    /// The run's last error.
    pub error: Option<String>,
    /// When the run reached `failed` or `stopped`.
    pub at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RuleRunFailure {
    /// The record the run was about.
    pub fn subject(&self) -> Subject {
        Subject::new(&self.subject_type, self.subject_id)
    }

    /// The failures from `from` to `until` (the end not included), oldest
    /// first.
    pub async fn between(db: &Db, from: DateTime, until: DateTime) -> Result<Vec<RuleRunFailure>> {
        RuleRunFailure::query()
            .where_(RuleRunFailure::AT.gte(from))
            .where_(RuleRunFailure::AT.lt(until))
            .order_by(RuleRunFailure::AT)
            .order_by(RuleRunFailure::ID)
            .get(db)
            .await
    }
}

/// Records that run `id` reached `kind` now, inside the transition's
/// transaction (the run's error is read from there).
pub(crate) async fn record(tx: &mut Transaction, id: i64, kind: FailureKind) -> Result {
    let Some(run) = RuleRun::find(&mut *tx, id).await? else {
        return Ok(());
    };
    let at = now();
    let mut failure = RuleRunFailure {
        run_id: run.id,
        rule: run.rule,
        subject_type: run.subject_type,
        subject_id: run.subject_id,
        occurrence_key: run.occurrence_key,
        kind,
        error: run.error,
        at,
        created_at: Some(at),
        updated_at: Some(at),
        ..Default::default()
    };
    failure.insert(&mut *tx).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kinds_are_stored_in_snake_case() {
        let kinds: Vec<_> = FailureKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds, ["failed", "stopped"]);
    }
}
