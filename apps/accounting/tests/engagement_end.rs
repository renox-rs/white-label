//! #330 (BL-ACC-06): the End checklist of an engagement, the write-off of
//! unbilled time above a share of the WIP (`ACC.ACL.09`), and an end
//! scheduled for a later date (04 §7.1 *Engagement* `end`). The fake modules
//! stand in for ticketing and invoicing here; what ending does to the
//! retainer, the tasks and the notice with the real ones is in
//! `engagement_end_real.rs`.

mod support;

use accounting::ending::{run_scheduled, scheduled_key};
use accounting::wip::{WipWriteOff, WriteOffStatus};
use accounting::{Engagement, EngagementStatus};
use wl_kernel::approvals;

use support::{Booted, boot, d};

/// The refusal text of a response.
fn said(response: &renox::testing::TestResponse) -> String {
    response.text()
}

/// An `active` retainer engagement of a fresh client.
async fn active(t: &Booted) -> (support::Client, Engagement) {
    let client = t.client().await;
    let engagement = t.active_engagement(&client).await;
    (client, engagement)
}

/// Done when 1: an `active` engagement with a VAT return in `in_review`: `end`
/// is refused naming the return; once it is cancelled the end goes through.
#[renox::test]
async fn an_open_return_stops_the_end_and_is_named() {
    let t = boot().await;
    let (client, engagement) = active(&t).await;
    let crew = t.crew(&client).await;
    let ret = t.open_return(&engagement, d(11, 28)).await;
    t.return_at(&ret, &crew, accounting::ReturnStatus::InReview)
        .await;

    let refused = t
        .act(
            &crew.partner,
            &engagement,
            "end",
            &[("reason", "client_request")],
        )
        .await;
    refused.assert_status(422);
    let text = said(&refused);
    assert!(text.contains("RET-0001"), "{text}");
    assert!(text.contains("in review"), "{text}");
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
    // The page lists it as an item still open.
    t.get(
        &crew.partner,
        &format!("/accounting/engagements/{}", engagement.id),
    )
    .await
    .assert_ok()
    .assert_see("RET-0001");

    t.ret_act(
        &crew.reviewer,
        &ret,
        "cancel",
        &[("reason", "engagement_ended")],
    )
    .await
    .assert_ok();
    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Ended);
}

/// A filed return does not stop the end; neither does a cancelled one.
#[renox::test]
async fn a_filed_return_does_not_stop_the_end() {
    let t = boot().await;
    let (client, engagement) = active(&t).await;
    let crew = t.crew(&client).await;
    let ret = t.open_return(&engagement, d(11, 28)).await;
    t.return_at(&ret, &crew, accounting::ReturnStatus::Approved)
        .await;
    t.file_return(&crew.partner, &ret).await.assert_ok();
    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
}

/// A final invoice or credit: a retainer's pro rata satisfies it; a fixed
/// fee engagement needs the partner's confirmation that there is nothing to
/// bill, and only the partner gives it.
#[renox::test]
async fn an_engagement_without_a_retainer_needs_the_nothing_to_bill_confirmation() {
    let t = boot().await;
    let (client, engagement) = active(&t).await;
    let crew = t.crew(&client).await;
    let engagement = t.engagement_fixed_fee(&engagement).await;

    let refused = t
        .act(
            &crew.partner,
            &engagement,
            "end",
            &[("reason", "client_request")],
        )
        .await;
    refused.assert_status(422);
    assert!(
        said(&refused).contains("final invoice or credit"),
        "{}",
        said(&refused)
    );

    t.act(&crew.reviewer, &engagement, "nothing-to-bill", &[])
        .await
        .assert_forbidden();
    t.act(&crew.partner, &engagement, "nothing-to-bill", &[])
        .await
        .assert_ok();
    let confirmed = t.reload(&engagement).await;
    assert_eq!(confirmed.nothing_to_bill_by, Some(crew.partner.id));
    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Ended);
}

/// Unbilled time stops the end until it is billed or written off; time
/// billed on invoices satisfies the settlement of a fixed fee.
#[renox::test]
async fn unbilled_time_stops_the_end_until_it_is_written_off() {
    let t = boot().await;
    let (client, engagement) = active(&t).await;
    let crew = t.crew(&client).await;
    t.wip.set(&engagement.subject(), 1_000_000);

    let refused = t
        .act(
            &crew.partner,
            &engagement,
            "end",
            &[("reason", "client_request")],
        )
        .await;
    refused.assert_status(422);
    assert!(
        said(&refused).contains("AED 10,000.00"),
        "{}",
        said(&refused)
    );
    // "Nothing to bill" cannot be confirmed with time left.
    t.act(&crew.partner, &engagement, "nothing-to-bill", &[])
        .await
        .assert_status(422);

    // The partner writes all of it off: no approval, then the end passes.
    t.act(
        &crew.partner,
        &engagement,
        "wip-write-off",
        &[("amount", "10000"), ("reason", "Client dispute settled")],
    )
    .await
    .assert_ok();
    assert_eq!(t.wip.unbilled_of(&engagement.subject()), 0);
    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Ended);
}

/// Done when 2: with AED 10,000 of unbilled time a write-off of AED 1,500 by
/// the manager needs a partner (above 10%); AED 800 does not. The partner's
/// yes writes the time off.
#[renox::test]
async fn a_write_off_above_ten_percent_of_the_wip_needs_a_partner() {
    let t = boot().await;
    let (client, engagement) = active(&t).await;
    let crew = t.crew(&client).await;
    let subject = engagement.subject();
    t.wip.set(&subject, 1_000_000);

    // AED 800: written off at once.
    t.act(
        &crew.reviewer,
        &engagement,
        "wip-write-off",
        &[("amount", "800"), ("reason", "Rework we caused")],
    )
    .await
    .assert_ok();
    let rows = WipWriteOff::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, WriteOffStatus::WrittenOff);
    assert_eq!(t.wip.written_off().len(), 1);
    assert_eq!(t.wip.written_off()[0].1, 80_000);
    assert_eq!(t.wip.written_off()[0].3, Some(crew.reviewer.id));
    assert_eq!(t.wip.unbilled_of(&subject), 920_000);

    // 10% of the WIP at the time (AED 1,000 of the AED 10,000 first asked
    // is AED 920 now): AED 1,500 of AED 9,200 is above 10%.
    t.act(
        &crew.reviewer,
        &engagement,
        "wip-write-off",
        &[("amount", "1500"), ("reason", "Goodwill")],
    )
    .await
    .assert_ok();
    let rows = WipWriteOff::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[1].status,
        WriteOffStatus::Proposed,
        "waits for a partner"
    );
    assert_eq!(t.wip.written_off().len(), 1, "nothing written off yet");
    let requests = approvals::pending_for(t.db(), &rows[1].subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].requester_id, crew.reviewer.id);

    // The manager cannot decide it; the partner can.
    assert!(
        t.run(approvals::approve(
            t.state(),
            requests[0].id,
            &crew.reviewer,
            "ok"
        ))
        .await
        .is_err()
    );
    t.run(approvals::approve(
        t.state(),
        requests[0].id,
        &crew.partner,
        "Agreed",
    ))
    .await
    .unwrap();
    let rows = WipWriteOff::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(rows[1].status, WriteOffStatus::WrittenOff);
    assert_eq!(t.wip.written_off().len(), 2);
    assert_eq!(t.wip.written_off()[1].1, 150_000);
    assert_eq!(t.wip.unbilled_of(&subject), 770_000);
}

/// The exact limit: AED 1,000 of AED 10,000 is 10%, not above it; a partner's
/// own write-off needs no approval; an amount above the WIP, without a reason,
/// or by anybody without `wip.write_off` is refused.
#[renox::test]
async fn the_limit_is_exactly_ten_percent_and_a_partner_needs_no_approval() {
    let t = boot().await;
    let (client, engagement) = active(&t).await;
    let crew = t.crew(&client).await;
    t.wip.set(&engagement.subject(), 1_000_000);

    t.act(
        &crew.reviewer,
        &engagement,
        "wip-write-off",
        &[("amount", "1000"), ("reason", "Exactly the limit")],
    )
    .await
    .assert_ok();
    assert_eq!(t.wip.written_off().len(), 1);
    // AED 9,000 are left: the partner writes AED 5,000 off by himself.
    t.act(
        &crew.partner,
        &engagement,
        "wip-write-off",
        &[("amount", "5000"), ("reason", "Agreed with the client")],
    )
    .await
    .assert_ok();
    assert_eq!(t.wip.written_off().len(), 2);

    for (user, amount, reason) in [
        (&crew.partner, "99999", "Too much"),
        (&crew.partner, "100", ""),
        (&crew.partner, "0", "Nothing"),
    ] {
        t.act(
            user,
            &engagement,
            "wip-write-off",
            &[("amount", amount), ("reason", reason)],
        )
        .await
        .assert_status(422);
    }
    t.act(
        &crew.preparer,
        &engagement,
        "wip-write-off",
        &[("amount", "100"), ("reason", "No right")],
    )
    .await
    .assert_forbidden();
    assert_eq!(t.wip.written_off().len(), 2);
}

/// Done when 6 (with the fake modules): an end scheduled for 2026-12-31 and a
/// VAT return still `preparing` that day: the engagement stays `active` and
/// the partner has one task naming the return; tried again on the following
/// days it is not doubled; after the return is cancelled on 2027-01-04 the
/// next daily try ends it with end date 2026-12-31 and the task closes.
#[renox::test]
async fn a_scheduled_end_waits_for_the_open_return_and_ends_with_the_scheduled_date() {
    let t = boot().await;
    let (client, engagement) = active(&t).await;
    let crew = t.crew(&client).await;
    let ret = t.open_return(&engagement, d(11, 28)).await;

    // A date after today is scheduled, not run.
    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request"), ("end_date", "2026-12-31")],
    )
    .await
    .assert_ok();
    let scheduled = t.reload(&engagement).await;
    assert_eq!(scheduled.status, EngagementStatus::Active);
    assert_eq!(scheduled.end_scheduled_on, Some(d(12, 31)));
    assert_eq!(scheduled.end_scheduled_by, Some(crew.partner.id));

    // Before the date nothing happens.
    let early = t.run(run_scheduled(t.state(), d(12, 30))).await.unwrap();
    assert!(early.ended.is_empty() && early.refused.is_empty());

    // The day: refused, the partner has a task naming the return.
    let day = t.run(run_scheduled(t.state(), d(12, 31))).await.unwrap();
    assert_eq!(day.refused, [engagement.id]);
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
    let key = scheduled_key(engagement.id);
    let tasks = t.tickets.with_key(&key);
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].open);
    assert!(tasks[0].title.contains("2026-12-31"), "{}", tasks[0].title);
    assert!(
        tasks[0].description.contains("RET-0001"),
        "{}",
        tasks[0].description
    );
    assert!(matches!(
        tasks[0].assignee,
        Some(wl_kernel::tickets::Assignee::User { id }) if id == crew.partner.id
    ));
    // Tried again each day: still one task.
    for day in [d(12, 31), d(12, 31)] {
        t.run(run_scheduled(t.state(), day)).await.unwrap();
    }
    assert_eq!(t.tickets.with_key(&key).len(), 1);

    // The return is cancelled on 2027-01-04; the next try ends it.
    t.ret_act(
        &crew.reviewer,
        &ret,
        "cancel",
        &[("reason", "engagement_ended")],
    )
    .await
    .assert_ok();
    let retry = t
        .run(run_scheduled(t.state(), "2027-01-04".parse().unwrap()))
        .await
        .unwrap();
    assert_eq!(retry.ended, [engagement.id]);
    let ended = t.reload(&engagement).await;
    assert_eq!(ended.status, EngagementStatus::Ended);
    assert_eq!(ended.ended_on, Some(d(12, 31)), "the scheduled date");
    assert_eq!(ended.end_scheduled_on, None);
    assert!(
        !t.tickets.with_key(&key)[0].open,
        "the partner's task closed"
    );
}

/// The partner changes the date (a new date replaces the old), or removes it
/// (the partner's task closes and the daily run leaves the engagement
/// alone); a past date is refused; only the partner schedules.
#[renox::test]
async fn the_partner_changes_or_removes_a_scheduled_end() {
    let t = boot().await;
    let (client, engagement) = active(&t).await;
    let crew = t.crew(&client).await;
    t.open_return(&engagement, d(11, 28)).await;

    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request"), ("end_date", "2026-11-30")],
    )
    .await
    .assert_ok();
    t.run(run_scheduled(t.state(), d(11, 30))).await.unwrap();
    let key = scheduled_key(engagement.id);
    assert!(t.tickets.with_key(&key)[0].open);

    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "non_payment"), ("end_date", "2026-12-15")],
    )
    .await
    .assert_ok();
    let moved = t.reload(&engagement).await;
    assert_eq!(moved.end_scheduled_on, Some(d(12, 15)));
    assert_eq!(moved.end_scheduled_reason.as_deref(), Some("non_payment"));

    t.act(&crew.partner, &engagement, "end/remove", &[])
        .await
        .assert_ok();
    let removed = t.reload(&engagement).await;
    assert_eq!(removed.end_scheduled_on, None);
    assert!(!t.tickets.with_key(&key)[0].open, "the task closed");
    let none = t.run(run_scheduled(t.state(), d(12, 31))).await.unwrap();
    assert!(none.ended.is_empty() && none.refused.is_empty());
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);

    // A past date is refused; a manager cannot schedule.
    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request"), ("end_date", "2026-01-31")],
    )
    .await
    .assert_status(422);
    t.act(
        &crew.reviewer,
        &engagement,
        "end",
        &[("reason", "client_request"), ("end_date", "2027-01-31")],
    )
    .await
    .assert_forbidden();
    assert_eq!(t.reload(&engagement).await.end_scheduled_on, None);
}
