//! Ticket category settings (wl-ticketing §7, issue #156): rename, turn off
//! and delete a category, behind `tickets.manage`.
//!
//! A **platform category** ([`Category::platform`]: `billing_dispute`,
//! `labour_case`, `delivery_claim`, and the `task` category) is the one other
//! modules open tickets in by its fixed key. A client renames it (the labels
//! only, the key never changes) but cannot delete it or turn it off. Any
//! category with tickets cannot be deleted either: turn it off to stop new
//! tickets.

use renox::prelude::*;

use crate::config::{Category, InvoiceOn};
use crate::permissions;
use crate::ticket::Ticket;

/// A refusal, as a 422 with its English message.
fn refused(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

/// The refusal for a platform category that is deleted or turned off.
pub const PLATFORM_REFUSAL: &str =
    "This is a platform category other modules open tickets in: rename it, but it cannot be deleted or turned off.";

/// The refusal for a category that has tickets.
pub const HAS_TICKETS_REFUSAL: &str =
    "This category has tickets, so it cannot be deleted: turn it off instead.";

fn manage(user: &AuthUser) -> Result {
    if user.has_permission(permissions::MANAGE) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

async fn category(db: &Db, key: &str) -> Result<Category> {
    Category::by_key(db, key).await?.ok_or(Error::NotFound)
}

/// Renames the category `key`: its English and Arabic names. The key, and
/// so what the opening module relies on, does not change. A blank Arabic
/// name leaves the English one in use.
///
/// # Errors
///
/// `Forbidden` without `tickets.manage`, `NotFound` for an unknown key, 422
/// for a blank English name.
pub async fn rename(
    db: &Db,
    user: &AuthUser,
    key: &str,
    name_en: &str,
    name_ar: &str,
) -> Result<Category> {
    manage(user)?;
    let mut category = category(db, key).await?;
    let name_en = name_en.trim();
    if name_en.is_empty() {
        return Err(refused("A category needs a name."));
    }
    category.name_en = name_en.to_owned();
    category.name_ar = name_ar.trim().to_owned();
    category.save(db).await?;
    Ok(category)
}

/// Sets whether the tickets of the category `key` are chargeable and when
/// they are invoiced: at `resolve` or at close (§7). It applies to tickets
/// opened from now on (a ticket keeps the flag it was opened with).
///
/// # Errors
///
/// `Forbidden` without `tickets.manage`, `NotFound` for an unknown key.
pub async fn set_charging(
    db: &Db,
    user: &AuthUser,
    key: &str,
    chargeable: bool,
    invoice_on: InvoiceOn,
) -> Result<Category> {
    manage(user)?;
    let mut category = category(db, key).await?;
    if category.chargeable != chargeable || category.invoice_on != invoice_on {
        category.chargeable = chargeable;
        category.invoice_on = invoice_on;
        category.save(db).await?;
    }
    Ok(category)
}

/// Marks the category `key` compliance-critical or not (01: fire and life
/// safety, lifts): a covered subject that is not compliant for it gets a
/// compliance flag, and a missed planned visit also tells the manager.
///
/// # Errors
///
/// `Forbidden` without `tickets.manage`, `NotFound` for an unknown key.
pub async fn set_compliance_critical(
    db: &Db,
    user: &AuthUser,
    key: &str,
    critical: bool,
) -> Result<Category> {
    manage(user)?;
    let mut category = category(db, key).await?;
    if category.compliance_critical != critical {
        category.compliance_critical = critical;
        category.save(db).await?;
    }
    Ok(category)
}

/// Sets what the coordinator's task offers for the open visits of the
/// category `key` when a service contract ends (`WRK.RUL.18`): reassign or
/// cancel (`false`), or reassign only (`true`; 01: the compliance-critical
/// categories, whose visits cannot be cancelled).
///
/// # Errors
///
/// `Forbidden` without `tickets.manage`, `NotFound` for an unknown key.
pub async fn set_contract_end_reassign_only(
    db: &Db,
    user: &AuthUser,
    key: &str,
    reassign_only: bool,
) -> Result<Category> {
    manage(user)?;
    let mut category = category(db, key).await?;
    if category.contract_end_reassign_only != reassign_only {
        category.contract_end_reassign_only = reassign_only;
        category.save(db).await?;
    }
    Ok(category)
}

/// Turns the category `key` on or off: an inactive category takes no new
/// tickets, and its tickets stay.
///
/// # Errors
///
/// `Forbidden` without `tickets.manage`, `NotFound` for an unknown key, 422
/// for turning a platform category off.
pub async fn set_active(db: &Db, user: &AuthUser, key: &str, active: bool) -> Result<Category> {
    manage(user)?;
    let mut category = category(db, key).await?;
    if category.platform && !active {
        return Err(refused(PLATFORM_REFUSAL));
    }
    if category.active != active {
        category.active = active;
        category.save(db).await?;
    }
    Ok(category)
}

/// Deletes the category `key`.
///
/// # Errors
///
/// `Forbidden` without `tickets.manage`, `NotFound` for an unknown key, 422
/// for a platform category or one that has tickets.
pub async fn delete(db: &Db, user: &AuthUser, key: &str) -> Result {
    manage(user)?;
    let mut category = category(db, key).await?;
    if category.platform {
        return Err(refused(PLATFORM_REFUSAL));
    }
    let used = Ticket::query()
        .where_(Ticket::CATEGORY_ID.eq(category.id))
        .first(db)
        .await?
        .is_some();
    if used {
        return Err(refused(HAS_TICKETS_REFUSAL));
    }
    category.delete(db).await?;
    Ok(())
}
