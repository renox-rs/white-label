//! The expiry logs: [`AffectedItem`] rows for planned records already
//! reported as affected by a blocking document, [`ReminderLog`] rows for
//! reminder offsets already sent, [`ExpiredLog`] rows for documents whose
//! expiry was reported, and [`RenewalTask`] rows for the renewal tasks opened.
//!
//! The tables are unique per key, so the inserts ignore a duplicate and
//! report whether a row was written: a repeat run adds nothing.

use renox::chrono::NaiveDate;
use renox::db::{Executor, now, sql};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// A planned record reported as affected by a document's expiry.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_documents_affected_items",
    unique(document_id, guarded, record_type, record_id)
)]
pub struct AffectedItem {
    /// The primary key.
    pub id: i64,
    /// The expiring document.
    #[model(references = "wl_documents_documents")]
    pub document_id: i64,
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// The planned record's type.
    pub record_type: String,
    /// The planned record's id.
    pub record_id: i64,
    /// The date the record is planned for.
    pub planned_date: NaiveDate,
    /// The business date of the run that first reported it.
    pub first_reported_on: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AffectedItem {
    /// Records the item. Returns `false` when the document already has a row
    /// for this entry and record: the item is not new.
    pub async fn insert_new<'c>(&self, db: impl Executor<'c>) -> Result<bool> {
        let at = now();
        let written = sql(format!(
            "INSERT INTO {} (document_id, guarded, record_type, record_id, planned_date, \
             first_reported_on, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (document_id, guarded, record_type, record_id) DO NOTHING",
            Self::TABLE
        ))
        .bind(self.document_id)
        .bind(self.guarded.clone())
        .bind(self.record_type.clone())
        .bind(self.record_id)
        .bind(self.planned_date)
        .bind(self.first_reported_on)
        .bind(at)
        .bind(at)
        .execute(db)
        .await?;
        Ok(written > 0)
    }

    /// Whether the document already has a row for this entry and record.
    pub async fn is_reported(
        db: &Db,
        document_id: i64,
        guarded: &str,
        record_type: &str,
        record_id: i64,
    ) -> Result<bool> {
        Ok(Self::query()
            .where_(Self::DOCUMENT_ID.eq(document_id))
            .where_(Self::GUARDED.eq(guarded))
            .where_(Self::RECORD_TYPE.eq(record_type))
            .where_(Self::RECORD_ID.eq(record_id))
            .first(db)
            .await?
            .is_some())
    }

    /// Every item reported for `document_id`, oldest first.
    pub async fn for_document(db: &Db, document_id: i64) -> Result<Vec<AffectedItem>> {
        Self::query()
            .where_(Self::DOCUMENT_ID.eq(document_id))
            .order_by(Self::ID)
            .get(db)
            .await
    }
}

/// A reminder offset sent for a document.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_documents_reminder_logs", unique(document_id, offset_days))]
pub struct ReminderLog {
    /// The primary key.
    pub id: i64,
    /// The document.
    #[model(references = "wl_documents_documents")]
    pub document_id: i64,
    /// The offset in days before the expiry date.
    pub offset_days: i64,
    /// The business date of the run that sent (or passed) it.
    pub run_date: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ReminderLog {
    /// Records that `offset_days` was handled for `document_id` on
    /// `run_date`. Returns `false` when it was already logged.
    pub async fn insert_new<'c>(
        db: impl Executor<'c>,
        document_id: i64,
        offset_days: i64,
        run_date: NaiveDate,
    ) -> Result<bool> {
        let at = now();
        let written = sql(format!(
            "INSERT INTO {} (document_id, offset_days, run_date, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?) ON CONFLICT (document_id, offset_days) DO NOTHING",
            Self::TABLE
        ))
        .bind(document_id)
        .bind(offset_days)
        .bind(run_date)
        .bind(at)
        .bind(at)
        .execute(db)
        .await?;
        Ok(written > 0)
    }

    /// Forgets every offset logged for `document_id`, so the next run
    /// reaches them again against a changed expiry date (#51).
    pub async fn reset_for<'c>(db: impl Executor<'c>, document_id: i64) -> Result<()> {
        sql(format!("DELETE FROM {} WHERE document_id = ?", Self::TABLE))
            .bind(document_id)
            .execute(db)
            .await?;
        Ok(())
    }

    /// The offsets already logged for `document_id`, ascending.
    pub async fn offsets_for(db: &Db, document_id: i64) -> Result<Vec<i64>> {
        let mut offsets: Vec<i64> = Self::query()
            .where_(Self::DOCUMENT_ID.eq(document_id))
            .get(db)
            .await?
            .into_iter()
            .map(|log| log.offset_days)
            .collect();
        offsets.sort_unstable();
        Ok(offsets)
    }
}

/// A document whose expiry the run has reported (`DOC.RUL.03`).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_documents_expired_logs", unique(document_id))]
pub struct ExpiredLog {
    /// The primary key.
    pub id: i64,
    /// The expired document.
    #[model(references = "wl_documents_documents")]
    pub document_id: i64,
    /// The business date of the run that reported it.
    pub run_date: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ExpiredLog {
    /// Records that the expiry of `document_id` was reported on `run_date`.
    /// Returns `false` when it was already logged: the event is not emitted
    /// twice.
    pub async fn insert_new<'c>(
        db: impl Executor<'c>,
        document_id: i64,
        run_date: NaiveDate,
    ) -> Result<bool> {
        let at = now();
        let written = sql(format!(
            "INSERT INTO {} (document_id, run_date, created_at, updated_at) \
             VALUES (?, ?, ?, ?) ON CONFLICT (document_id) DO NOTHING",
            Self::TABLE
        ))
        .bind(document_id)
        .bind(run_date)
        .bind(at)
        .bind(at)
        .execute(db)
        .await?;
        Ok(written > 0)
    }

    /// Whether the expiry of `document_id` was already reported.
    pub async fn is_logged(db: &Db, document_id: i64) -> Result<bool> {
        Ok(Self::query()
            .where_(Self::DOCUMENT_ID.eq(document_id))
            .first(db)
            .await?
            .is_some())
    }
}

/// A renewal task opened for an expired document.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_documents_renewal_tasks")]
pub struct RenewalTask {
    /// The primary key.
    pub id: i64,
    /// The subject's type, e.g. `hr.employee`.
    pub subject_type: String,
    /// The subject's id.
    pub subject_id: i64,
    /// The document kind to renew.
    #[model(references = "wl_documents_kinds")]
    pub kind_id: i64,
    /// The expired document that caused the task.
    #[model(references = "wl_documents_documents")]
    pub expired_document_id: i64,
    /// The ticket's id; `None` when ticketing is off.
    pub ticket_id: Option<i64>,
    /// The ticket's number; `None` when ticketing is off.
    pub ticket_number: Option<String>,
    /// When the task was opened.
    pub opened_at: DateTime,
    /// When the task was closed; `None` while it is open.
    pub closed_at: Option<DateTime>,
    /// The accepted document that closed it.
    #[model(references = "wl_documents_documents")]
    pub renewal_document_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RenewalTask {
    /// Records an open task. Returns `false` when the subject already has an
    /// open task for the kind: nothing is written.
    pub async fn insert_new<'c>(&self, db: impl Executor<'c>) -> Result<bool> {
        let at = now();
        let written = sql(format!(
            "INSERT INTO {} (subject_type, subject_id, kind_id, expired_document_id, \
             ticket_id, ticket_number, opened_at, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (subject_type, subject_id, kind_id) WHERE closed_at IS NULL DO NOTHING",
            Self::TABLE
        ))
        .bind(self.subject_type.clone())
        .bind(self.subject_id)
        .bind(self.kind_id)
        .bind(self.expired_document_id)
        .bind(self.ticket_id)
        .bind(self.ticket_number.clone())
        .bind(self.opened_at)
        .bind(at)
        .bind(at)
        .execute(db)
        .await?;
        Ok(written > 0)
    }

    /// The open task for the subject and kind, if any.
    pub async fn find_open(
        db: &Db,
        subject_type: &str,
        subject_id: i64,
        kind_id: i64,
    ) -> Result<Option<RenewalTask>> {
        Self::query()
            .where_(Self::SUBJECT_TYPE.eq(subject_type))
            .where_(Self::SUBJECT_ID.eq(subject_id))
            .where_(Self::KIND_ID.eq(kind_id))
            .where_(Self::CLOSED_AT.is_null())
            .first(db)
            .await
    }

    /// Every task for the subject and kind, oldest first.
    pub async fn for_subject(
        db: &Db,
        subject_type: &str,
        subject_id: i64,
        kind_id: i64,
    ) -> Result<Vec<RenewalTask>> {
        Self::query()
            .where_(Self::SUBJECT_TYPE.eq(subject_type))
            .where_(Self::SUBJECT_ID.eq(subject_id))
            .where_(Self::KIND_ID.eq(kind_id))
            .order_by(Self::ID)
            .get(db)
            .await
    }

    /// Closes the task, recording the accepted `renewal_document_id`.
    pub async fn close<'c>(
        &mut self,
        db: impl Executor<'c>,
        renewal_document_id: i64,
    ) -> Result<()> {
        self.closed_at = Some(now());
        self.renewal_document_id = Some(renewal_document_id);
        self.save(db).await
    }
}

#[cfg(test)]
mod tests {
    use renox::testing::TestApp;

    use super::*;
    use crate::kinds::{KindSeed, install_seeds};
    use crate::{Document, DocumentKind, WlDocuments};

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    fn item(document_id: i64, record_id: i64, planned: &str) -> AffectedItem {
        AffectedItem {
            document_id,
            guarded: "ticket.resolve".to_owned(),
            record_type: "ticket".to_owned(),
            record_id,
            planned_date: date(planned),
            first_reported_on: date("2026-11-15"),
            ..AffectedItem::default()
        }
    }

    async fn document(db: &Db) -> i64 {
        install_seeds(db, [&KindSeed::new("expiry_logs_permit", "Permit")])
            .await
            .expect("a kind");
        let kind = DocumentKind::by_key(db, "expiry_logs_permit")
            .await
            .unwrap()
            .unwrap();
        let mut document = Document {
            kind_id: kind.id,
            ..Document::default()
        };
        document.save(db).await.expect("a document");
        document.id
    }

    #[renox::test]
    async fn an_item_is_written_once_per_document_entry_and_record() {
        let app = TestApp::new(App::new().module(WlDocuments)).await;
        let db = app.db();
        let id = document(db).await;
        let other = document(db).await;

        assert!(item(id, 1, "2026-12-20").insert_new(db).await.unwrap());
        // A repeat, even with another planned date, adds nothing.
        assert!(!item(id, 1, "2027-01-02").insert_new(db).await.unwrap());
        assert!(item(id, 2, "2027-01-10").insert_new(db).await.unwrap());
        assert!(item(other, 1, "2026-12-20").insert_new(db).await.unwrap());

        let rows = AffectedItem::for_document(db, id).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].planned_date, date("2026-12-20"));
        assert!(rows[0].created_at.is_some());
        assert!(
            AffectedItem::is_reported(db, id, "ticket.resolve", "ticket", 2)
                .await
                .unwrap()
        );
        assert!(
            !AffectedItem::is_reported(db, id, "ticket.resolve", "ticket", 3)
                .await
                .unwrap()
        );
    }

    #[renox::test]
    async fn a_reminder_offset_is_logged_once_per_document() {
        let app = TestApp::new(App::new().module(WlDocuments)).await;
        let db = app.db();
        let id = document(db).await;

        let day = date("2026-11-15");
        assert!(ReminderLog::insert_new(db, id, 30, day).await.unwrap());
        assert!(
            !ReminderLog::insert_new(db, id, 30, date("2026-11-16"))
                .await
                .unwrap()
        );
        assert!(ReminderLog::insert_new(db, id, 90, day).await.unwrap());

        assert_eq!(ReminderLog::offsets_for(db, id).await.unwrap(), [30, 90]);
        assert!(
            ReminderLog::offsets_for(db, id + 1)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[renox::test]
    async fn an_expiry_is_logged_once_per_document() {
        let app = TestApp::new(App::new().module(WlDocuments)).await;
        let db = app.db();
        let id = document(db).await;
        let other = document(db).await;

        assert!(!ExpiredLog::is_logged(db, id).await.unwrap());
        assert!(
            ExpiredLog::insert_new(db, id, date("2026-11-15"))
                .await
                .unwrap()
        );
        assert!(
            !ExpiredLog::insert_new(db, id, date("2026-11-16"))
                .await
                .unwrap()
        );
        assert!(ExpiredLog::is_logged(db, id).await.unwrap());
        assert!(!ExpiredLog::is_logged(db, other).await.unwrap());
        let row = ExpiredLog::query().first(db).await.unwrap().unwrap();
        assert_eq!(row.run_date, date("2026-11-15"));
        assert!(row.created_at.is_some());
    }

    fn task(document_id: i64, kind_id: i64, subject_id: i64) -> RenewalTask {
        RenewalTask {
            subject_type: "hr.employee".to_owned(),
            subject_id,
            kind_id,
            expired_document_id: document_id,
            opened_at: now(),
            ..RenewalTask::default()
        }
    }

    #[renox::test]
    async fn one_renewal_task_is_open_per_subject_and_kind() {
        let app = TestApp::new(App::new().module(WlDocuments)).await;
        let db = app.db();
        let id = document(db).await;
        let kind_id = Document::find(db, id).await.unwrap().unwrap().kind_id;

        assert!(task(id, kind_id, 7).insert_new(db).await.unwrap());
        // A second open task for the same subject and kind adds nothing.
        assert!(!task(id, kind_id, 7).insert_new(db).await.unwrap());
        // Another subject is independent.
        assert!(task(id, kind_id, 8).insert_new(db).await.unwrap());

        let mut open = RenewalTask::find_open(db, "hr.employee", 7, kind_id)
            .await
            .unwrap()
            .expect("an open task");
        assert!(open.ticket_id.is_none() && open.closed_at.is_none());

        open.close(db, id).await.unwrap();
        assert!(
            RenewalTask::find_open(db, "hr.employee", 7, kind_id)
                .await
                .unwrap()
                .is_none()
        );
        // Once closed, a new expiry may open another task.
        assert!(task(id, kind_id, 7).insert_new(db).await.unwrap());
        let all = RenewalTask::for_subject(db, "hr.employee", 7, kind_id)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].renewal_document_id, Some(id));
        assert!(all[1].closed_at.is_none());
    }
}
