//! The notices of the company budget (wl-invoicing §6, `FIN.RUL.24`,
//! `FIN.RUL.26`): who is told and the notification types. Without wl-notify
//! the kernel puts a notice in the bell.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel};

use crate::texts::english;

/// The notification type of `FIN.RUL.26`, no approved budget for the coming
/// fiscal year.
pub const REMINDER: &str = "invoicing.budget_reminder";

/// The notification type of `FIN.RUL.24`, an actual reached its budget.
pub const VARIANCE: &str = "invoicing.budget_variance";

/// The notification type of a drawing recorded by the sole owner without a
/// second approver (`FIN.ACL.23`).
pub const DRAWING_SELF_APPROVED: &str = "invoicing.drawing_self_approved";

/// The notification type of a payment drawing recorded by the sole owner
/// without a second approver (`FIN.ACL.23`).
pub const DRAWING_PAYMENT_SELF_APPROVED: &str = "invoicing.drawing_payment_self_approved";

/// The notification type of a bill with an approved drawing that was voided
/// (`FIN.ACL.21`).
pub const DRAWING_CORRECTED: &str = "invoicing.drawing_corrected";

/// The module that sends the notices.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the types belong to.
const GROUP: &str = "budgets";

/// The type of `FIN.RUL.26`: staff, transactional, by e-mail.
pub fn reminder_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REMINDER, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.budget_reminder.subject")),
            english("wl_invoicing.budget_reminder.body"),
        )
}

/// The type of `FIN.RUL.24`: staff, transactional, by e-mail.
pub fn variance_type() -> NotificationTypeDef {
    NotificationTypeDef::new(VARIANCE, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.budget_variance.subject")),
            english("wl_invoicing.budget_variance.body"),
        )
}

/// The type of `FIN.ACL.23`: staff, transactional, by e-mail.
pub fn self_approved_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DRAWING_SELF_APPROVED, MODULE)
        .group("drawings")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.drawing_self_approved.subject")),
            english("wl_invoicing.drawing_self_approved.body"),
        )
}

/// The type of `FIN.ACL.23` for a payment: staff, transactional, by e-mail.
pub fn payment_self_approved_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DRAWING_PAYMENT_SELF_APPROVED, MODULE)
        .group("drawings")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.drawing_payment_self_approved.subject")),
            english("wl_invoicing.drawing_payment_self_approved.body"),
        )
}

/// The type of a corrected drawing: staff, transactional, by e-mail.
pub fn corrected_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DRAWING_CORRECTED, MODULE)
        .group("drawings")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.drawing_corrected.subject")),
            english("wl_invoicing.drawing_corrected.body"),
        )
}

/// Registers the types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(reminder_type());
    notify::register_type(variance_type());
    notify::register_type(self_approved_type());
    notify::register_type(payment_self_approved_type());
    notify::register_type(corrected_type());
}

/// The ids of the users who hold any of `roles` globally, once each, in id
/// order.
pub(crate) async fn holders_of(db: &Db, roles: &[String]) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = Vec::new();
    for role in roles {
        ids.extend(
            users_with_role_in(db, role, &Scope::global())
                .await?
                .iter()
                .map(|user| user.id),
        );
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}
