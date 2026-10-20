//! #331 (BL-ACC-07, `ACC.ACL.14`) on the system surfaces with the real
//! wl-ticketing and wl-invoicing. Its own file: the real modules register
//! block effects for the whole test process, which the fakes of
//! `neutral_reasons.rs` must not see.

mod support;

use accounting::{Engagement, EngagementStatus};
use renox::prelude::*;
use renox::state_machine::history;

/// What a restricted reason must never show.
const RESTRICTED: [&str; 4] = ["sanctions", "Sanctions", "aml_concern", "AML"];

fn assert_neutral(what: &str, text: &str) {
    for word in RESTRICTED {
        assert!(!text.contains(word), "{what} shows `{word}`: {text}");
    }
}

/// DW1/DW2 on the system surfaces, with the real wl-ticketing and
/// wl-invoicing: a `sanctions` block suspends the engagement, holds its
/// tasks and pauses its retainer, and nothing shows the restricted reason
/// outside the party's block record: not the audit log, not a notice to
/// staff, and the retainer's pause notice to the client is suppressed
/// (`ACC.ACL.14`, `PTY.ACL.02`). There is no engagement search or API in
/// 04 yet; the list and export are checked in the first test.
#[renox::test]
async fn the_system_surfaces_carry_the_neutral_code_only_and_no_retainer_notice_goes() {
    use wl_kernel::subscriptions::{SubscriptionStatus, subscriptions};

    let t = support::boot_real().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let started = t
        .run(accounting::retainer::start(
            t.state(),
            &engagement,
            support::d(11, 1),
        ))
        .await
        .unwrap();
    assert!(started.is_some(), "the retainer started");

    t.block(client.company, "sanctions").await;

    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    let retainer = subscriptions(t.state())
        .of_agreement(t.db(), &engagement.subject())
        .await
        .unwrap();
    assert_eq!(retainer[0].status, SubscriptionStatus::Paused);

    // No automatic message to the client: no pause notice, no other.
    for party in client.parties() {
        assert!(
            t.notices.to_party(party).is_empty(),
            "a message went to {party}: {:?}",
            t.notices.to_party(party)
        );
    }
    // Nothing recorded for staff names the reason.
    for request in t.notices.all() {
        assert_neutral("a notice", &format!("{request:?}"));
    }
    // The audit log keeps the neutral code only.
    let logs = renox::audit::latest(t.db(), 200).await.unwrap();
    for log in logs {
        assert_neutral("an audit row", &format!("{} {}", log.action, log.data));
    }
    let history = history(t.db(), Engagement::TABLE, engagement.id)
        .await
        .unwrap();
    for row in history {
        assert_neutral("a history row", &row.reason.unwrap_or_default());
    }
}
