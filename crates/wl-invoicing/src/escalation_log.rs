//! The recipient log of the dispute escalation (wl-invoicing §5.10,
//! `FIN.RUL.15`): who was told at each step.

use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// A user told at one escalation step of a dispute. One row per dispute,
/// step and user: it is inserted before the notice is sent and deleted
/// again when sending fails, so a retry tells only those not yet told.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_dispute_escalations",
    unique(dispute_id, step, user_id),
    index(invoice_id)
)]
pub struct EscalationLog {
    /// The primary key.
    pub id: i64,
    /// The dispute.
    pub dispute_id: i64,
    /// Its invoice.
    pub invoice_id: i64,
    /// The escalation step (1, 2, ...).
    pub step: i64,
    /// The user told.
    pub user_id: i64,
    /// `owner`, `receivables` or `finance_manager`.
    pub role: String,
    /// The first message id the notifier returned; `None` without wl-notify.
    pub message_id: Option<i64>,
    /// When it was sent.
    pub sent_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl EscalationLog {
    /// Logs `user_id` as told at `step`; `None` when they already have a
    /// row (told, or being told by another run).
    pub async fn claim(
        db: &Db,
        dispute_id: i64,
        invoice_id: i64,
        step: i64,
        user_id: i64,
        role: &str,
    ) -> Result<Option<EscalationLog>> {
        let mut log = EscalationLog {
            dispute_id,
            invoice_id,
            step,
            user_id,
            role: role.to_owned(),
            sent_at: renox::db::now(),
            ..Default::default()
        };
        match log.insert(db).await {
            Ok(()) => Ok(Some(log)),
            Err(error) if error.is_unique_violation() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The users already logged for `dispute_id` at `step`, sorted.
    pub async fn told(db: &Db, dispute_id: i64, step: i64) -> Result<Vec<i64>> {
        let mut ids: Vec<i64> = EscalationLog::query()
            .where_(EscalationLog::DISPUTE_ID.eq(dispute_id))
            .where_(EscalationLog::STEP.eq(step))
            .get(db)
            .await?
            .iter()
            .map(|log| log.user_id)
            .collect();
        ids.sort_unstable();
        Ok(ids)
    }
}
