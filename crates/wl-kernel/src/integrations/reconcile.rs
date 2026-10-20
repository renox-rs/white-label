//! The reconciliation task of a recovered connection (wl-kernel §5.14
//! step 4): what the late sync and the work done by hand disagree about.
//!
//! One [`Reconciliation`] per failure run of a connection (the time of its
//! first failed call), listing every line: a queued call its module reported
//! obsolete (the record changed while it was queued), and an inbound item
//! whose values differ from the record done by hand (for example
//! `AED 250 manual vs AED 205 gateway`). The first lines open one task for
//! the connection's owner role; later lines are added to that task as
//! comments. No line, no task.

use renox::db::{DateTime, Json, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::SUBJECT_TYPE;
use super::connection::IntegrationConnection;
use crate::role_tasks::{self, RoleTask};
use crate::subject::Subject;
use crate::texts;
use crate::tickets::{self, TaskComment, TicketId};

/// What kind of difference a line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// The manual record and the late sync result differ.
    Mismatch,
    /// A queued call was not sent: its record changed while it was queued.
    Obsolete,
}

/// One thing to reconcile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// Mismatch or obsolete.
    pub kind: LineKind,
    /// The natural key, if the item has one (`TX-889`).
    pub natural_key: Option<String>,
    /// What it is about, in words (`refund payments.refund:3`).
    pub reference: String,
    /// How it differs (`AED 250 manual vs AED 205 gateway`).
    pub detail: String,
}

impl Line {
    /// A manual record and an inbound item that differ.
    pub fn mismatch(natural_key: &str, subject: &Subject, detail: &str) -> Self {
        Line {
            kind: LineKind::Mismatch,
            natural_key: Some(natural_key.to_owned()),
            reference: subject.to_string(),
            detail: detail.to_owned(),
        }
    }

    /// A queued call that was not sent.
    pub fn obsolete(kind: &str, natural_key: Option<&str>, subject: Option<&Subject>) -> Self {
        Line {
            kind: LineKind::Obsolete,
            natural_key: natural_key.map(str::to_owned),
            reference: match subject {
                Some(subject) => format!("{kind} {subject}"),
                None => kind.to_owned(),
            },
            detail: texts::en("wl_kernel.integrations.reconcile.obsolete"),
        }
    }

    /// The line as the task lists it: `TX-889: AED 250 manual vs AED 205
    /// gateway`.
    pub fn text(&self) -> String {
        let name = self.natural_key.as_deref().unwrap_or(&self.reference);
        format!("{name}: {}", self.detail)
    }
}

/// The task of one failure run of a connection (`wl_integration_reconciliations`).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_integration_reconciliations",
    unique(connection_key, outage_since)
)]
pub struct Reconciliation {
    /// The primary key.
    pub id: i64,
    /// The connection's key.
    pub connection_key: String,
    /// The first failed call of the failure run it is about.
    pub outage_since: DateTime,
    /// Everything to reconcile.
    pub lines: Json<Vec<Line>>,
    /// The task opened for the owner role, if ticketing is on.
    pub ticket_id: Option<i64>,
    /// The task's number.
    pub ticket_number: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Reconciliation {
    /// The reconciliation of `connection_key` for the failure run that began
    /// at `outage_since`.
    pub async fn find(
        db: &Db,
        connection_key: &str,
        outage_since: DateTime,
    ) -> Result<Option<Reconciliation>> {
        Reconciliation::query()
            .where_(Reconciliation::CONNECTION_KEY.eq(connection_key))
            .where_(Reconciliation::OUTAGE_SINCE.eq(outage_since))
            .first(db)
            .await
    }

    /// Every reconciliation of `connection_key`, oldest first.
    pub async fn of_connection(db: &Db, connection_key: &str) -> Result<Vec<Reconciliation>> {
        Reconciliation::query()
            .where_(Reconciliation::CONNECTION_KEY.eq(connection_key))
            .order_by(Reconciliation::ID)
            .get(db)
            .await
    }
}

/// The failure run lines belong to: the one that last began, else the time
/// the connection was stored.
fn outage_of(connection: &IntegrationConnection) -> DateTime {
    connection
        .outage_since
        .or(connection.failed_since)
        .or(connection.created_at)
        .unwrap_or_else(now)
}

/// The source key of the task of a failure run.
pub fn source_key(connection_key: &str, outage_since: DateTime) -> String {
    format!(
        "kernel.reconcile:{connection_key}:{}",
        outage_since.timestamp()
    )
}

/// Adds `lines` to the reconciliation of `connection`'s failure run. The
/// first lines open the task for the owner role (one task per recovery);
/// later lines are added to it as a comment. Does nothing for no lines.
///
/// Returns the reconciliation as it is then.
pub async fn add_lines(
    state: &AppState,
    connection: &IntegrationConnection,
    lines: Vec<Line>,
) -> Result<Option<Reconciliation>> {
    if lines.is_empty() {
        return Ok(None);
    }
    let db = &state.db;
    let outage = outage_of(connection);
    let mut tx = db.begin().await?;
    let found = Reconciliation::query()
        .where_(Reconciliation::CONNECTION_KEY.eq(connection.key.as_str()))
        .where_(Reconciliation::OUTAGE_SINCE.eq(outage))
        .lock_for_update()
        .first(&mut tx)
        .await?;
    let mut row = match found {
        Some(row) => row,
        None => {
            Reconciliation::create(
                &mut tx,
                Reconciliation {
                    connection_key: connection.key.clone(),
                    outage_since: outage,
                    ..Default::default()
                },
            )
            .await?
        }
    };
    row.lines.0.extend(lines.iter().cloned());
    Reconciliation::where_eq("id", row.id)
        .update(
            &mut tx,
            &[("lines", &row.lines), ("updated_at", &Some(now()))],
        )
        .await?;
    tx.commit().await?;

    let creator = tickets::task_creator(state);
    match row.ticket_id {
        Some(ticket) => {
            let text = lines.iter().map(Line::text).collect::<Vec<_>>().join("\n");
            creator
                .comment(state, TicketId(ticket), TaskComment::internal(&text))
                .await?;
        }
        None => {
            let title = texts::en_with(
                "wl_kernel.integrations.reconcile.title",
                &[("connection", &connection.key)],
            );
            let description = row
                .lines
                .iter()
                .map(Line::text)
                .collect::<Vec<_>>()
                .join("\n");
            let task = RoleTask::new(
                Subject::new(SUBJECT_TYPE, connection.id),
                &connection.owner_role,
                &title,
                &source_key(&connection.key, outage),
            )
            .description(&description);
            if let Some(ticket) = role_tasks::open(state, task).await? {
                row.ticket_id = Some(ticket.id.0);
                row.ticket_number = Some(ticket.number.clone());
                Reconciliation::where_eq("id", row.id)
                    .update(
                        db,
                        &[
                            ("ticket_id", &row.ticket_id),
                            ("ticket_number", &row.ticket_number),
                            ("updated_at", &Some(now())),
                        ],
                    )
                    .await?;
            }
        }
    }
    Ok(Some(row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mismatch_is_listed_by_its_natural_key() {
        let line = Line::mismatch(
            "TX-889",
            &Subject::new("payments.payment", 4),
            "AED 250 manual vs AED 205 gateway",
        );
        assert_eq!(line.text(), "TX-889: AED 250 manual vs AED 205 gateway");
        assert_eq!(line.kind, LineKind::Mismatch);
        assert_eq!(line.reference, "payments.payment:4");
    }

    #[test]
    fn an_obsolete_call_is_listed_by_what_it_was() {
        let line = Line::obsolete("refund", None, Some(&Subject::new("payments.refund", 3)));
        assert_eq!(line.reference, "refund payments.refund:3");
        assert_eq!(
            line.text(),
            "refund payments.refund:3: Not sent: the record changed while the call was queued."
        );
        let keyed = Line::obsolete("refund", Some("R-1"), None);
        assert!(keyed.text().starts_with("R-1: "));
    }

    #[test]
    fn the_task_is_found_by_connection_and_failure_run() {
        let since: DateTime = "2026-11-03T06:00:00Z".parse().unwrap();
        assert_eq!(
            source_key("gateway.acme", since),
            format!("kernel.reconcile:gateway.acme:{}", since.timestamp())
        );
    }

    #[test]
    fn lines_belong_to_the_failure_run_that_began_last() {
        let first: DateTime = "2026-11-03T06:00:00Z".parse().unwrap();
        let second: DateTime = "2026-11-03T06:40:00Z".parse().unwrap();
        let connection = IntegrationConnection {
            outage_since: Some(second),
            failed_since: Some(second),
            created_at: Some(first),
            ..Default::default()
        };
        assert_eq!(outage_of(&connection), second);
        let never_failed = IntegrationConnection {
            created_at: Some(first),
            ..Default::default()
        };
        assert_eq!(outage_of(&never_failed), first);
    }
}
