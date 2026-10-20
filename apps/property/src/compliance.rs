//! Building compliance flags and expiring certificates, without blocking
//! leasing (01 Property §4.5 rows "A planned visit is missed", "The contract
//! expires or is terminated without a replacement", "A compliance certificate
//! expires", §10; BL-PRP-25).
//!
//! A building is **non-compliant** while it has an open [`BuildingFlag`]:
//!
//! - the flags wl-ticketing raises on a building (`visit_missed`,
//!   `no_contract`, `vendor_document_expired`): Property keeps them from the
//!   events `ComplianceFlagRaised` and `ComplianceFlagCleared` (only subjects
//!   of the type `property.building`; a flag on a building system or an asset
//!   has no building and is left to the ticketing report);
//! - `certificate_expired`: wl-documents emits `DocumentExpired` once, on the
//!   first nightly run after the expiry date of a blocking document, that is
//!   the day after it (`DOC.RUL.01`/`DOC.RUL.03`; the vertical's seed gives
//!   the compliance certificate kinds a block entry). For a certificate kind
//!   of a building ([`CERTIFICATE_KINDS`]) the flag is raised and the company
//!   admin is told. It clears when a renewed certificate is accepted
//!   (`DocumentAccepted` with an expiry date that has not passed).
//!
//! The page `/property/compliance` and the count on the portfolio page list
//! the buildings with the cause and since when ([`crate::pages::compliance`]).
//! **Nothing here is read by a lease transition**: leases, renewals and
//! invoices go on whatever the flags say.

use renox::auth::permissions::users_with_role;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::{DocumentAccepted, DocumentExpired, today};
use wl_kernel::tickets::{ComplianceFlagCleared, ComplianceFlagRaised};
use wl_kernel::{COMPANY_ADMIN_ROLE, Subject};

use crate::building::{BUILDING_SUBJECT_TYPE, Building};
use crate::notices;

/// The cause of a flag raised because a compliance certificate expired.
pub const CERTIFICATE_EXPIRED: &str = "certificate_expired";

/// The document kinds of the compliance certificates of a building: the
/// building safety certificate and the lift certificate (§7.2 *Document
/// kinds*).
pub const CERTIFICATE_KINDS: [&str; 2] = ["building_safety_certificate", "lift_certificate"];

/// An open or cleared compliance flag of a building.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_building_flags", index(building_id, cleared_on))]
pub struct BuildingFlag {
    /// The primary key.
    pub id: i64,
    /// The building.
    #[model(references = "property_buildings")]
    pub building_id: i64,
    /// Why: `visit_missed`, `no_contract`, `vendor_document_expired` or
    /// `certificate_expired`.
    pub cause: String,
    /// The compliance-critical ticket category, or the certificate kind.
    pub category: String,
    /// The day it was raised (Asia/Dubai).
    pub raised_on: NaiveDate,
    /// The day it was cleared.
    pub cleared_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BuildingFlag {
    /// The open flags, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open(db: &Db) -> Result<Vec<BuildingFlag>> {
        BuildingFlag::query()
            .where_(BuildingFlag::CLEARED_ON.is_null())
            .order_by(BuildingFlag::RAISED_ON)
            .order_by(BuildingFlag::ID)
            .get(db)
            .await
    }

    /// Whether it is still open.
    pub fn is_open(&self) -> bool {
        self.cleared_on.is_none()
    }
}

/// The ids of the buildings with an open flag.
///
/// # Errors
///
/// The database's errors.
pub async fn non_compliant(db: &Db) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = BuildingFlag::open(db)
        .await?
        .iter()
        .map(|flag| flag.building_id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// The building a subject names, if it is one.
fn building_of(subject: &Subject) -> Option<i64> {
    (subject.subject_type == BUILDING_SUBJECT_TYPE).then_some(subject.subject_id)
}

/// Raises the flag `cause` for `category` on `building_id` on `day`, unless
/// one is open already. Returns the flag when it was raised.
///
/// # Errors
///
/// The database's errors.
pub async fn raise(
    db: &Db,
    building_id: i64,
    cause: &str,
    category: &str,
    day: NaiveDate,
) -> Result<Option<BuildingFlag>> {
    let open = BuildingFlag::query()
        .where_(BuildingFlag::BUILDING_ID.eq(building_id))
        .where_(BuildingFlag::CAUSE.eq(cause))
        .where_(BuildingFlag::CATEGORY.eq(category))
        .where_(BuildingFlag::CLEARED_ON.is_null())
        .exists(db)
        .await?;
    if open {
        return Ok(None);
    }
    let flag = BuildingFlag::create(
        db,
        BuildingFlag {
            building_id,
            cause: cause.to_owned(),
            category: category.to_owned(),
            raised_on: day,
            ..Default::default()
        },
    )
    .await?;
    Ok(Some(flag))
}

/// Clears the open flag `cause` for `category` on `building_id` on `day`.
/// Returns how many flags were cleared.
///
/// # Errors
///
/// The database's errors.
pub async fn clear(
    db: &Db,
    building_id: i64,
    cause: &str,
    category: &str,
    day: NaiveDate,
) -> Result<u64> {
    BuildingFlag::query()
        .where_(BuildingFlag::BUILDING_ID.eq(building_id))
        .where_(BuildingFlag::CAUSE.eq(cause))
        .where_(BuildingFlag::CATEGORY.eq(category))
        .where_(BuildingFlag::CLEARED_ON.is_null())
        .update(
            db,
            &[
                ("cleared_on", &Some(day)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await
}

/// A compliance certificate of a building expired: the flag
/// `certificate_expired` is raised and the company admin is told.
///
/// # Errors
///
/// The database's and the notifier's errors.
pub async fn on_expired(state: &AppState, event: &DocumentExpired) -> Result {
    let Some(building_id) = building_of(&event.subject) else {
        return Ok(());
    };
    if !CERTIFICATE_KINDS.contains(&event.kind.as_str()) {
        return Ok(());
    }
    let raised = raise(
        &state.db,
        building_id,
        CERTIFICATE_EXPIRED,
        &event.kind,
        today(),
    )
    .await?;
    if raised.is_none() {
        return Ok(());
    }
    let Some(building) = Building::find(&state.db, building_id).await? else {
        return Ok(());
    };
    let admins: Vec<i64> = users_with_role(&state.db, COMPANY_ADMIN_ROLE)
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    notices::certificate_expired(state, &building, &event.kind, event.expires_on, &admins).await
}

/// A renewed certificate was accepted: the flag clears when it has not
/// expired.
///
/// # Errors
///
/// The database's errors.
pub async fn on_accepted(state: &AppState, event: &DocumentAccepted) -> Result {
    let Some(building_id) = building_of(&event.subject) else {
        return Ok(());
    };
    if !CERTIFICATE_KINDS.contains(&event.kind.as_str()) {
        return Ok(());
    }
    let day = today();
    if event.expires_on.is_some_and(|expiry| expiry < day) {
        return Ok(());
    }
    clear(
        &state.db,
        building_id,
        CERTIFICATE_EXPIRED,
        &event.kind,
        day,
    )
    .await?;
    Ok(())
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: ComplianceFlagRaised, state: AppState| async move {
        if let Some(building) = building_of(&event.subject) {
            raise(&state.db, building, &event.reason, &event.category, today()).await?;
        }
        Ok(())
    });
    app.listen(|event: ComplianceFlagCleared, state: AppState| async move {
        if let Some(building) = building_of(&event.subject) {
            clear(&state.db, building, &event.reason, &event.category, today()).await?;
        }
        Ok(())
    });
    app.listen(|event: DocumentExpired, state: AppState| async move {
        on_expired(&state, &event).await
    });
    app.listen(|event: DocumentAccepted, state: AppState| async move {
        on_accepted(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_building_subject_names_a_building() {
        assert_eq!(
            building_of(&Subject::new(BUILDING_SUBJECT_TYPE, 4)),
            Some(4)
        );
        assert_eq!(
            building_of(&Subject::new("property.building_system", 4)),
            None
        );
    }

    #[test]
    fn a_flag_is_open_until_it_is_cleared() {
        let mut flag = BuildingFlag::default();
        assert!(flag.is_open());
        flag.cleared_on = NaiveDate::from_ymd_opt(2027, 6, 2);
        assert!(!flag.is_open());
    }

    #[test]
    fn the_certificate_kinds_are_the_two_compliance_certificates() {
        assert_eq!(CERTIFICATE_KINDS.len(), 2);
        assert!(CERTIFICATE_KINDS.contains(&"lift_certificate"));
    }
}
