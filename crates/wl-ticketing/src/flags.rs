//! Compliance flags (wl-ticketing §3 *Compliance flag*, issue #148): a
//! covered subject (a building, a building system, an asset) that is not
//! compliant for a compliance-critical category.
//!
//! There are three reasons ([`ComplianceFlagReason`]): `no_contract` (a
//! compliance contract ended with no replacement), `visit_missed`
//! (`WRK.RUL.14`, [`crate::visit_check`]) and `vendor_document_expired`. This
//! module holds the flag, [`raise`] and [`clear`]; each cause calls them. The
//! flag of a `visit_missed` cause is cleared here, when that visit ticket is
//! resolved ([`register_events`]); cancelling the visit does not clear it.
//!
//! **One open flag per subject, category and reason**: raising it again while
//! it is open does nothing (also enforced by a unique index). A flag is raised
//! only for a compliance-critical category, and cleared only by its own
//! clearing event. [`ComplianceFlagRaised`] and [`ComplianceFlagCleared`] are
//! emitted after the change is stored.

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::tickets::{ComplianceFlagCleared, ComplianceFlagRaised, ComplianceFlagReason};

use crate::config::Category;
use crate::ticket::Ticket;
use crate::work::{COMPLETE, RESOLVE, RESOLVE_FROM_SOURCE};

/// Why a `visit_missed` flag cleared.
pub const VISIT_RESOLVED: &str = "visit resolved";

/// Why a `no_contract` or `vendor_document_expired` flag cleared when another
/// contract covering the subject and category became `active`.
pub const REPLACEMENT_ACTIVE: &str = "replacement contract active";

/// Why a `vendor_document_expired` flag cleared when its contract ended
/// (`WRK.RUL.18`, which raises `no_contract` instead).
pub const CONTRACT_ENDED: &str = "contract ended";

/// Why a `vendor_document_expired` flag cleared when a renewed document was
/// accepted.
pub const DOCUMENT_RENEWED: &str = "document renewed";

/// A covered subject that is not compliant for a compliance-critical
/// category.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_ticketing_compliance_flags",
    index(subject_type, subject_id)
)]
pub struct ComplianceFlag {
    /// The primary key.
    pub id: i64,
    /// The subject's type, e.g. `property.building_system`.
    pub subject_type: String,
    /// The subject's id.
    pub subject_id: i64,
    /// The compliance-critical category.
    #[model(references = "wl_ticketing_categories")]
    pub category_id: i64,
    /// Why it is raised.
    pub reason: ComplianceFlagReason,
    /// The service contract, when the flag is about one.
    pub contract_id: Option<i64>,
    /// The visit ticket of a `visit_missed` flag.
    pub ticket_id: Option<i64>,
    /// When it was raised.
    pub raised_at: DateTime,
    /// When it was cleared; `None` while open.
    pub cleared_at: Option<DateTime>,
    /// What cleared it: `visit resolved`, `replacement contract active`,
    /// `document renewed`.
    pub cleared_by: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ComplianceFlag {
    /// The subject the flag is on.
    pub fn subject(&self) -> Subject {
        Subject::new(&self.subject_type, self.subject_id)
    }

    /// Whether it is still open.
    pub fn is_open(&self) -> bool {
        self.cleared_at.is_none()
    }

    /// The open flags, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open(db: &Db) -> Result<Vec<ComplianceFlag>> {
        ComplianceFlag::query()
            .where_(ComplianceFlag::CLEARED_AT.is_null())
            .order_by(ComplianceFlag::RAISED_AT)
            .order_by(ComplianceFlag::ID)
            .get(db)
            .await
    }

    /// The open flags on `subject`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open_for(db: &Db, subject: &Subject) -> Result<Vec<ComplianceFlag>> {
        ComplianceFlag::query()
            .where_(ComplianceFlag::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(ComplianceFlag::SUBJECT_ID.eq(subject.subject_id))
            .where_(ComplianceFlag::CLEARED_AT.is_null())
            .order_by(ComplianceFlag::RAISED_AT)
            .order_by(ComplianceFlag::ID)
            .get(db)
            .await
    }
}

/// What a flag is raised for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raise {
    /// The building, building system or asset.
    pub subject: Subject,
    /// The category's key.
    pub category: String,
    /// Why.
    pub reason: ComplianceFlagReason,
    /// The service contract, when the cause is about one.
    pub contract: Option<i64>,
    /// The visit ticket of a `visit_missed` flag.
    pub ticket: Option<i64>,
}

/// Raises the flag `raise` describes at `at` and emits
/// [`ComplianceFlagRaised`]. Returns `None` and does nothing when the
/// category is not compliance-critical (or unknown), or when the same flag
/// (subject, category and reason) is already open.
///
/// # Errors
///
/// The database's errors.
pub async fn raise(
    state: &AppState,
    raise: Raise,
    at: DateTime,
) -> Result<Option<ComplianceFlag>> {
    let Some(category) = Category::by_key(&state.db, &raise.category).await? else {
        return Ok(None);
    };
    if !category.compliance_critical {
        return Ok(None);
    }
    let open = ComplianceFlag::query()
        .where_(ComplianceFlag::SUBJECT_TYPE.eq(raise.subject.subject_type.as_str()))
        .where_(ComplianceFlag::SUBJECT_ID.eq(raise.subject.subject_id))
        .where_(ComplianceFlag::CATEGORY_ID.eq(category.id))
        .where_(ComplianceFlag::REASON.eq(raise.reason))
        .where_(ComplianceFlag::CLEARED_AT.is_null())
        .first(&state.db)
        .await?;
    if open.is_some() {
        return Ok(None);
    }
    let flag = ComplianceFlag {
        subject_type: raise.subject.subject_type.clone(),
        subject_id: raise.subject.subject_id,
        category_id: category.id,
        reason: raise.reason,
        contract_id: raise.contract,
        ticket_id: raise.ticket,
        raised_at: at,
        ..ComplianceFlag::default()
    };
    // The unique index settles a race between two runs: the loser stores
    // nothing and raises nothing.
    let flag = match ComplianceFlag::create(&state.db, flag).await {
        Ok(flag) => flag,
        Err(error) if error.is_unique_violation() => return Ok(None),
        Err(error) => return Err(error),
    };
    state
        .emit(ComplianceFlagRaised {
            subject: raise.subject,
            category: category.key,
            reason: raise.reason.as_str().to_owned(),
            contract: raise.contract,
        })
        .await?;
    Ok(Some(flag))
}

/// Clears the open flag `id` at `at`, because of `cleared_by`, and emits
/// [`ComplianceFlagCleared`]. Returns `None` when it is not open.
///
/// # Errors
///
/// The database's errors.
pub async fn clear(
    state: &AppState,
    id: i64,
    cleared_by: &str,
    at: DateTime,
) -> Result<Option<ComplianceFlag>> {
    let Some(mut flag) = ComplianceFlag::find(&state.db, id).await? else {
        return Ok(None);
    };
    if !flag.is_open() {
        return Ok(None);
    }
    flag.cleared_at = Some(at);
    flag.cleared_by = Some(cleared_by.to_owned());
    flag.save(&state.db).await?;
    let category = Category::find(&state.db, flag.category_id)
        .await?
        .map(|c| c.key)
        .unwrap_or_default();
    state
        .emit(ComplianceFlagCleared {
            subject: flag.subject(),
            category,
            reason: flag.reason.as_str().to_owned(),
            contract: flag.contract_id,
        })
        .await?;
    Ok(Some(flag))
}

/// Clears the open flags of `reasons` on `subject` for the category
/// `category_id`, because of `cleared_by`. Returns how many were cleared.
///
/// # Errors
///
/// The database's errors.
pub async fn clear_matching(
    state: &AppState,
    subject: &Subject,
    category_id: i64,
    reasons: &[ComplianceFlagReason],
    cleared_by: &str,
    at: DateTime,
) -> Result<usize> {
    let open = ComplianceFlag::query()
        .where_(ComplianceFlag::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
        .where_(ComplianceFlag::SUBJECT_ID.eq(subject.subject_id))
        .where_(ComplianceFlag::CATEGORY_ID.eq(category_id))
        .where_(ComplianceFlag::CLEARED_AT.is_null())
        .get(&state.db)
        .await?;
    let mut cleared = 0;
    for flag in open.into_iter().filter(|f| reasons.contains(&f.reason)) {
        if clear(state, flag.id, cleared_by, at).await?.is_some() {
            cleared += 1;
        }
    }
    Ok(cleared)
}

/// Clears the `no_contract` and `vendor_document_expired` flags of a covered
/// subject and category: a replacement contract covering them became
/// `active`. A `visit_missed` flag stays until its visit is resolved.
///
/// # Errors
///
/// The database's errors.
pub async fn clear_covered(
    state: &AppState,
    subject: &Subject,
    category_id: i64,
    cleared_by: &str,
    at: DateTime,
) -> Result<usize> {
    clear_matching(
        state,
        subject,
        category_id,
        &[
            ComplianceFlagReason::NoContract,
            ComplianceFlagReason::VendorDocumentExpired,
        ],
        cleared_by,
        at,
    )
    .await
}

/// Clears the `visit_missed` flags raised for the visit `ticket`: the visit
/// is resolved. Returns how many were cleared.
///
/// # Errors
///
/// The database's errors.
pub async fn clear_for_visit(state: &AppState, ticket: i64, at: DateTime) -> Result<usize> {
    let open = ComplianceFlag::query()
        .where_(ComplianceFlag::TICKET_ID.eq(ticket))
        .where_(ComplianceFlag::REASON.eq(ComplianceFlagReason::VisitMissed))
        .where_(ComplianceFlag::CLEARED_AT.is_null())
        .get(&state.db)
        .await?;
    let mut cleared = 0;
    for flag in open {
        if clear(state, flag.id, VISIT_RESOLVED, at).await?.is_some() {
            cleared += 1;
        }
    }
    Ok(cleared)
}

/// Registers the listener that clears the `visit_missed` flag of a visit
/// once it is resolved (`resolve`, or resolved or completed by its source). A
/// cancelled visit does not clear it.
pub(crate) fn register_events(app: &mut Registry) {
    app.listen(|event: Transitioned<Ticket>, state: AppState| async move {
        let name = event.transition.as_str();
        if ![RESOLVE, RESOLVE_FROM_SOURCE, COMPLETE].contains(&name) {
            return Ok(());
        }
        clear_for_visit(&state, event.key, renox::db::now()).await?;
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flag_is_open_until_it_is_cleared() {
        let mut flag = ComplianceFlag {
            subject_type: "property.building_system".into(),
            subject_id: 4,
            ..ComplianceFlag::default()
        };
        assert!(flag.is_open());
        assert_eq!(flag.subject(), Subject::new("property.building_system", 4));
        flag.cleared_at = Some(renox::db::now());
        assert!(!flag.is_open());
    }

    #[test]
    fn the_reasons_are_the_three_of_the_spec() {
        let names: Vec<_> = ComplianceFlagReason::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(names, ["no_contract", "visit_missed", "vendor_document_expired"]);
    }
}
