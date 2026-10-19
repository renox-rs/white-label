//! The SOP records (wl-documents §3 *SOP*).

use renox::auth::permissions::Scope;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::DocumentStatus;

/// Where an SOP stands. Derived from its current version, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SopStatus {
    /// No version accepted yet.
    Draft,
    /// The current version is accepted.
    Active,
    /// The current version was archived.
    Withdrawn,
}

impl SopStatus {
    /// The status of an SOP whose current version has `current` status
    /// (`None` when it has no current version).
    pub fn of(current: Option<DocumentStatus>) -> SopStatus {
        match current {
            Some(DocumentStatus::Accepted) => SopStatus::Active,
            Some(DocumentStatus::Archived) => SopStatus::Withdrawn,
            _ => SopStatus::Draft,
        }
    }

    /// The status as a key (`draft`, `active`, `withdrawn`).
    pub fn key(self) -> &'static str {
        match self {
            SopStatus::Draft => "draft",
            SopStatus::Active => "active",
            SopStatus::Withdrawn => "withdrawn",
        }
    }
}

/// A standard operating procedure. Its versions and translations are
/// documents with this record as subject.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_documents_sops",
    unique(code, legal_entity_type, legal_entity_id)
)]
pub struct Sop {
    /// The primary key.
    pub id: i64,
    /// The code, e.g. `SOP-HSE-01`; unique per legal entity.
    pub code: String,
    /// The English title.
    pub title_en: String,
    /// The Arabic title.
    pub title_ar: String,
    /// The category, e.g. `hse`.
    pub category: String,
    /// The legal entity's type; empty when the SOP applies to every entity.
    pub legal_entity_type: String,
    /// The legal entity's id; 0 when the SOP applies to every entity.
    pub legal_entity_id: i64,
    /// The role whose holders accept, reject and review it.
    pub owner_role: String,
    /// Months between reviews.
    pub review_interval_months: i64,
    /// Days a reader has to acknowledge a new version.
    pub ack_due_days: i64,
    /// The current version document: accepted, or archived once withdrawn.
    #[model(references = "wl_documents_documents")]
    pub current_document_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Sop {
    /// The subject its documents attach to.
    pub fn subject(&self) -> Subject {
        Subject::new(super::SUBJECT_TYPE, self.id)
    }

    /// The legal entity it belongs to; `None` when it applies to every one.
    pub fn legal_entity(&self) -> Option<i64> {
        (self.legal_entity_id != 0).then_some(self.legal_entity_id)
    }
}

/// A required reader of an SOP: everyone, or a role in a scope.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[model(
    table = "wl_documents_sop_readers",
    unique(sop_id, all_staff, role_key, scope_type, scope_id)
)]
pub struct SopReader {
    /// The primary key.
    pub id: i64,
    /// The SOP.
    #[model(references = "wl_documents_sops")]
    pub sop_id: i64,
    /// Every member of staff, whatever the role.
    pub all_staff: bool,
    /// The role or trade key; empty for an all-staff reader.
    pub role_key: String,
    /// The scope type; empty with `scope_id` for every scope.
    pub scope_type: String,
    /// The scope id; empty for every scope.
    pub scope_id: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SopReader {
    /// The scope the reader works in; global when none.
    pub fn scope(&self) -> Scope {
        if self.scope_type.is_empty() {
            Scope::global()
        } else {
            Scope::new(&self.scope_type, &self.scope_id)
        }
    }
}

/// Links a translation document to the version document it translates.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_documents_sop_translations",
    unique(document_id),
    index(version_document_id, language)
)]
pub struct SopTranslation {
    /// The primary key.
    pub id: i64,
    /// The translation document.
    #[model(references = "wl_documents_documents")]
    pub document_id: i64,
    /// The language, e.g. `ar`.
    pub language: String,
    /// The version document it translates.
    #[model(references = "wl_documents_documents")]
    pub version_document_id: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_derived_from_the_current_version() {
        assert_eq!(SopStatus::of(None), SopStatus::Draft);
        assert_eq!(
            SopStatus::of(Some(DocumentStatus::PendingReview)),
            SopStatus::Draft
        );
        assert_eq!(
            SopStatus::of(Some(DocumentStatus::Accepted)),
            SopStatus::Active
        );
        assert_eq!(
            SopStatus::of(Some(DocumentStatus::Archived)),
            SopStatus::Withdrawn
        );
        assert_eq!(SopStatus::Withdrawn.key(), "withdrawn");
    }

    #[test]
    fn a_sop_without_entity_applies_to_every_one() {
        let sop = Sop {
            id: 4,
            ..Default::default()
        };
        assert_eq!(sop.legal_entity(), None);
        assert_eq!(sop.subject(), Subject::new("documents.sop", 4));
        let sop = Sop {
            legal_entity_id: 2,
            ..sop
        };
        assert_eq!(sop.legal_entity(), Some(2));
    }

    #[renox::test]
    async fn the_code_is_unique_per_legal_entity_and_the_kinds_install() {
        use renox::testing::TestApp;

        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let db = app.db();
        let seeds = crate::sops::kind_seeds();
        assert_eq!(
            crate::kinds::install_seeds(db, seeds.iter()).await.unwrap(),
            2
        );
        let kind = crate::DocumentKind::by_key(db, "sop")
            .await
            .unwrap()
            .unwrap();
        assert!(kind.has_expiry);

        let new = |entity: i64| Sop {
            code: "SOP-HSE-01".into(),
            title_en: "Working at height".into(),
            owner_role: "hse_officer".into(),
            review_interval_months: 12,
            ack_due_days: 7,
            legal_entity_type: if entity == 0 {
                String::new()
            } else {
                "legal_entity".into()
            },
            legal_entity_id: entity,
            ..Default::default()
        };
        let first = Sop::create(db, new(0)).await.unwrap();
        assert_eq!(first.legal_entity(), None);
        assert!(Sop::create(db, new(0)).await.is_err());
        assert!(Sop::create(db, new(1)).await.is_ok());
        assert!(Sop::create(db, new(2)).await.is_ok());
        assert!(Sop::create(db, new(1)).await.is_err());
    }

    #[test]
    fn a_reader_without_scope_is_global() {
        assert!(SopReader::default().scope().is_global());
        let r = SopReader {
            scope_type: "site".into(),
            scope_id: "7".into(),
            ..Default::default()
        };
        assert!(!r.scope().is_global());
    }
}
