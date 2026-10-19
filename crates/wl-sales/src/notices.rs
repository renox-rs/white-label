//! The module's notifications (#138 A17), sent through the kernel
//! [`Notifier`](wl_kernel::notify::Notifier): transactional, by e-mail, with
//! English texts first, to the campaign's owner. Without wl-notify the
//! kernel's fallback puts them in the bell.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::campaign::Campaign;
use crate::texts::english;

/// The owner is told that the campaign ended.
pub const CAMPAIGN_ENDED: &str = "sales.campaign_ended";

/// `date` as `18 Oct 2026`, empty when unset.
fn date_text(date: Option<NaiveDate>) -> String {
    date.map(|d| d.format("%-d %b %Y").to_string())
        .unwrap_or_default()
}

/// The notification type `sales.campaign_ended`.
pub fn campaign_ended_type() -> NotificationTypeDef {
    let subject = english("wl_sales.notices.ended_subject", &[]);
    let body = english("wl_sales.notices.ended_body", &[]);
    NotificationTypeDef::new(CAMPAIGN_ENDED, "sales")
        .group("sales")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Registers the module's notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(campaign_ended_type());
}

/// Tells the campaign's owner that it ended. A campaign without an owner
/// tells nobody.
pub(crate) async fn campaign_ended(state: &AppState, campaign: &Campaign) -> Result {
    let Some(owner) = campaign.owner_id else {
        return Ok(());
    };
    let request = NotifyRequest::new(CAMPAIGN_ENDED, campaign.campaign_id().subject())
        .to_user(owner)
        .subject_label(&campaign.code)
        .data(json!({
            "name": campaign.name,
            "code": campaign.code,
            "end_date": date_text(campaign.end_date),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
