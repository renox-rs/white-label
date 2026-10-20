//! The retainer of an engagement (04 §7.1 *Subscription (retainer)*): the
//! wl-invoicing subscription started when the engagement is activated,
//! through the kernel's [`Subscriptions`](wl_kernel::subscriptions::Subscriptions).
//!
//! Only a retainer engagement with a fee starts one: monthly, the plan named
//! after the engagement, from the activation day, billed to the client. Its
//! pause, resume and cancel follow `suspend`, `resume` and `end` (BL-ACC-03,
//! BL-ACC-05, BL-ACC-06).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::subscriptions::{StartSubscription, SubscriptionId, subscriptions};

use crate::engagement::{Engagement, FeeBasis};
use crate::texts::english;

/// The subscription `engagement` starts on `on`, if it bills a retainer.
pub fn start_request(engagement: &Engagement, on: NaiveDate) -> Option<StartSubscription> {
    if engagement.fee_basis != FeeBasis::Retainer || engagement.retainer_fee <= 0 {
        return None;
    }
    let plan = english("accounting.retainer.plan", &[("title", &engagement.title)]);
    Some(StartSubscription::new(
        engagement.client(),
        engagement.subject(),
        plan,
        engagement.retainer_fee,
        on,
    ))
}

/// Starts the retainer of `engagement` on `on`; `None` when it bills none or
/// invoicing is off.
///
/// # Errors
///
/// The subscription service's errors (a refused plan).
pub async fn start(
    state: &AppState,
    engagement: &Engagement,
    on: NaiveDate,
) -> Result<Option<SubscriptionId>> {
    match start_request(engagement, on) {
        Some(request) => subscriptions(state).start(state, request).await,
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_retainer_with_a_fee_starts_a_subscription() {
        let on = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let mut engagement = Engagement {
            id: 4,
            client_party_id: 9,
            title: "Bookkeeping".into(),
            retainer_fee: 500_000,
            ..Engagement::default()
        };
        let request = start_request(&engagement, on).unwrap();
        assert_eq!(request.unit_price, 500_000);
        assert_eq!(request.start, on);
        assert_eq!(request.source.to_string(), "accounting.engagement:4");
        assert_eq!(request.plan, "Bookkeeping retainer");
        engagement.fee_basis = FeeBasis::Fixed;
        assert!(start_request(&engagement, on).is_none());
    }
}
