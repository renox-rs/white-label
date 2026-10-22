//! The notices of the vertical: to the customer, with a title and a body the
//! caller words (06 F&B §7.1: *customer told*), and to the staff of an outlet.
//! A customer's notice that gives up needs no reaction of its own.

use renox::auth::permissions::users_with_permission_in;
use renox::prelude::*;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};
use wl_kernel::{PartyId, Subject};

use crate::catalogue::Outlet;
use crate::order::FnbOrder;
use crate::texts::{english, template};

/// The notification type of a notice to a customer about their order (the
/// order status message of 06 §7.2, BL-FNB-17).
pub const CUSTOMER_NOTICE: &str = "fnb.order_status";

/// The notification type of the one-time code of the brand's ordering site
/// login (06 §7.2, BL-FNB-17): in the *one-time passwords* group.
pub const LOGIN_OTP: &str = "fnb.login_otp";

/// The notification type of a notice to a member of staff.
pub const STAFF_NOTICE: &str = "fnb.staff_notice";

/// The customer's notice (`fnb.order_status`): a title and a body the caller
/// fills in, in the *marketing and tracking* group (order tracking). A notice
/// that gives up creates no task (06 §7.2).
pub fn customer_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CUSTOMER_NOTICE, "fnb")
        .group(notify::GROUP_TRACKING)
        .category(Category::Transactional)
        .give_up(&[GiveUpReaction::None])
        .channels(&[NotifyChannel::Sms, NotifyChannel::Whatsapp, NotifyChannel::Email])
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("fnb.notice.subject")),
            template("fnb.notice.body"),
        )
}

/// The login code of the ordering site (`fnb.login_otp`): in the *one-time
/// passwords* group, so a code that cannot be delivered needs no reaction (the
/// customer asks for another or orders by phone).
pub fn login_otp_type() -> NotificationTypeDef {
    NotificationTypeDef::new(LOGIN_OTP, "fnb")
        .group(notify::GROUP_OTP)
        .category(Category::Transactional)
        .give_up(&[GiveUpReaction::None])
        .channels(&[NotifyChannel::Sms, NotifyChannel::Whatsapp])
        .template(
            NotifyChannel::Sms,
            "en",
            None,
            template("fnb.login_otp.body"),
        )
}

/// The staff notice: a title and a body the caller fills in.
pub fn staff_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(STAFF_NOTICE, "fnb")
        .group("orders")
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("fnb.notice.subject")),
            template("fnb.notice.body"),
        )
}

/// Registers the notification types.
pub(crate) fn register_types() {
    notify::register_type(customer_notice_type());
    notify::register_type(login_otp_type());
    notify::register_type(staff_notice_type());
}

/// Tells the customer of `order` with `title` and `body`.
///
/// # Errors
///
/// The notifier's errors.
pub(crate) async fn tell_customer(
    state: &AppState,
    order: &FnbOrder,
    title: &str,
    body: &str,
) -> Result {
    let request = NotifyRequest::new(CUSTOMER_NOTICE, order.subject())
        .subject_label(&order.number)
        .to_party(PartyId(order.party_id))
        .data(renox::serde_json::json!({ "title": title, "body": body }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the kitchen managers of the outlet of `order` (the holders of
/// `permission` there, or everywhere) with `title` and `body`. Nobody is told
/// when nobody holds the permission. Returns whom it told.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn tell_managers(
    state: &AppState,
    order: &FnbOrder,
    permission: &str,
    title: &str,
    body: &str,
) -> Result<Vec<i64>> {
    let holders = match order.outlet_id {
        Some(id) => match Outlet::find(&state.db, id).await? {
            Some(outlet) => users_with_permission_in(&state.db, permission, &outlet.scope()).await?,
            None => renox::auth::permissions::users_with_permission(&state.db, permission).await?,
        },
        None => renox::auth::permissions::users_with_permission(&state.db, permission).await?,
    };
    if holders.is_empty() {
        return Ok(Vec::new());
    }
    let mut request = NotifyRequest::new(STAFF_NOTICE, order.subject())
        .subject_label(&order.number)
        .data(renox::serde_json::json!({
            "title": title,
            "body": body,
            "link": format!("/orders/{}", order.id),
        }));
    let mut told = Vec::new();
    for user in holders {
        request = request.to_user(user.id);
        told.push(user.id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(told)
}

/// Tells everyone who holds `permission` (in `scope` when given, else
/// anywhere) about `subject`, with a title, a body and the `link` of the page
/// to open. Nobody is told when nobody holds it. Returns whom it told.
///
/// # Errors
///
/// Database errors and the notifier's.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn tell_holders(
    state: &AppState,
    subject: Subject,
    label: &str,
    permission: &str,
    scope: Option<renox::auth::permissions::Scope>,
    title: &str,
    body: &str,
    link: &str,
) -> Result<Vec<i64>> {
    let holders = match scope {
        Some(scope) => users_with_permission_in(&state.db, permission, &scope).await?,
        None => renox::auth::permissions::users_with_permission(&state.db, permission).await?,
    };
    if holders.is_empty() {
        return Ok(Vec::new());
    }
    let mut request = NotifyRequest::new(STAFF_NOTICE, subject)
        .subject_label(label)
        .data(renox::serde_json::json!({
            "title": title,
            "body": body,
            "link": link,
        }));
    let mut told = Vec::new();
    for user in holders {
        request = request.to_user(user.id);
        told.push(user.id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(told)
}

/// The text `key` for the notice of `order`, with its number filled in.
pub(crate) fn about(key: &str, order: &FnbOrder) -> String {
    english(key, &[("number", &order.number)])
}
