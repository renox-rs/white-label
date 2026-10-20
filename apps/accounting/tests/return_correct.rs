//! #342 (BL-ACC-18, `ACC.ACL.04`) with the real wl-ticketing: a filed return
//! is locked and a correction is a new version. A VAT error above AED 10,000
//! of tax, and any corporate tax error, needs a voluntary disclosure with a
//! task due 20 business days after the error was found; a VAT error of AED
//! 10,000 or less goes in the next return, which the new version names and
//! which cancels the new version as *not required* once it is filed.

mod support;

use accounting::corrections::{DISCLOSURE_TASK, routes_for};
use accounting::tax_return::CorrectionRoute;
use accounting::{Engagement, Return, ReturnStatus, ReturnType};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::deadlines::{DeadlineTaskStatus, DeadlineTaskSummary, deadline_tasks};

use support::{Booted, Client, boot_real};

fn day(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

/// A client with an `active` engagement and a `filed` VAT return for the
/// quarter ending 2026-09-30, on 2026-11-05 at 10:00.
async fn filed_vat(t: &Booted) -> (Client, Engagement, Return) {
    t.set_clock(5, 10, 0);
    let client = t.client().await;
    let engagement = t.active_engagement(&client).await;
    let ret = t
        .return_in(
            &engagement,
            ReturnType::Vat,
            day("2026-09-30"),
            day("2026-10-28"),
            ReturnStatus::Filed,
        )
        .await;
    (client, engagement, ret)
}

/// `user` corrects `ret` through the page.
async fn correct(
    t: &Booted,
    user: &User,
    ret: &Return,
    amount: &str,
    route: &str,
    extra: &[(&str, &str)],
) -> renox::testing::TestResponse {
    let mut pairs = vec![
        ("reason", "Input VAT on one invoice was counted twice"),
        ("error_amount", amount),
        ("found_on", "2026-11-05"),
        ("route", route),
    ];
    pairs.extend_from_slice(extra);
    t.ret_act(user, ret, "correct", &pairs).await
}

/// The new version that superseded `old`.
async fn successor(t: &Booted, old: &Return) -> Return {
    Return::query()
        .where_(Return::SUPERSEDES_ID.eq(old.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("a new version exists")
}

/// The disclosure tasks of `engagement`.
async fn disclosures(t: &Booted, engagement: &Engagement) -> Vec<DeadlineTaskSummary> {
    deadline_tasks(t.state())
        .of_subject(t.db(), &engagement.subject())
        .await
        .unwrap()
        .into_iter()
        .filter(|task| task.task_type == DISCLOSURE_TASK)
        .collect()
}

/// A filed VAT return and an error of AED 12,500 found on 2026-11-05: only
/// the voluntary disclosure is offered; the old version is `superseded`, a new
/// `preparing` version copies the figures, and a task is due 2026-12-07 (the
/// holidays of 2026-12-02 and 2026-12-03 in the calendar). The task cannot be
/// completed without the reference, and the superseded version cannot be
/// edited.
#[renox::test]
async fn an_error_above_the_threshold_is_a_voluntary_disclosure_with_a_task() {
    let t = boot_real().await;
    let preparer = t.accountant().await;
    let manager = t.manager().await;
    let (_, engagement, filed) = filed_vat(&t).await;
    assert_eq!(
        routes_for(ReturnType::Vat, 1_250_000),
        [CorrectionRoute::VoluntaryDisclosure],
        "only the voluntary disclosure is offered"
    );

    // The next return is refused for AED 12,500, even when it is named.
    let later = t
        .return_in(
            &engagement,
            ReturnType::Vat,
            day("2026-12-31"),
            day("2027-01-28"),
            ReturnStatus::Preparing,
        )
        .await;
    let next = later.id.to_string();
    correct(
        &t,
        &preparer,
        &filed,
        "12500.00",
        "next_return",
        &[("next_return", &next)],
    )
    .await
    .assert_status(422);
    assert_eq!(t.return_status(&filed).await, ReturnStatus::Filed);
    assert!(disclosures(&t, &engagement).await.is_empty());

    correct(
        &t,
        &preparer,
        &filed,
        "12500.00",
        "voluntary_disclosure",
        &[],
    )
    .await
    .assert_ok();
    assert_eq!(t.return_status(&filed).await, ReturnStatus::Superseded);
    let old = t.return_now(&filed).await;
    assert_eq!(old.error_amount, Some(1_250_000));
    assert_eq!(old.error_found_on, Some(day("2026-11-05")));
    assert_eq!(
        old.filed_on, filed.filed_on,
        "the filed version is not edited"
    );
    let new = successor(&t, &filed).await;
    assert_eq!(new.status, ReturnStatus::Preparing);
    assert_eq!(new.version, 2);
    assert_eq!(new.figures, filed.figures);
    assert!(new.workpapers_attached);
    assert_eq!(new.period_end, filed.period_end);
    assert_eq!(
        new.correction_route,
        Some(CorrectionRoute::VoluntaryDisclosure)
    );
    assert!(!new.is_corrected_via_next_return());

    // The task: 20 business days after Thursday 2026-11-05 over the holidays.
    let tasks = disclosures(&t, &engagement).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].due_date, day("2026-12-07"));
    assert_eq!(new.disclosure_task_id, Some(tasks[0].id.0));
    t.get(&manager, &format!("/accounting/returns/{}", new.id))
        .await
        .assert_ok()
        .assert_see("Voluntary disclosure")
        .assert_see("2026-12-07");

    // It cannot be completed without the reference.
    let id = tasks[0].id;
    t.task_act(&manager, id, "start", &[]).await.assert_ok();
    t.task_act(&manager, id, "complete", &[])
        .await
        .assert_status(422);
    assert_eq!(t.task_now(id).await.status, DeadlineTaskStatus::InProgress);
    t.task_act(&manager, id, "complete", &[("evidence", "VD-2026-0042")])
        .await
        .assert_ok();
    assert_eq!(t.task_now(id).await.status, DeadlineTaskStatus::Completed);

    // The superseded version cannot be edited, submitted or corrected again.
    t.ret_act(
        &preparer,
        &filed,
        "figures",
        &[("figures", "Output VAT 1"), ("workpapers", "on")],
    )
    .await
    .assert_status(422);
    t.ret_act(&preparer, &filed, "submit", &[])
        .await
        .assert_status(422);
    correct(&t, &preparer, &filed, "100.00", "voluntary_disclosure", &[])
        .await
        .assert_status(422);
    assert_eq!(t.return_now(&filed).await.figures, filed.figures);
}

/// An error of AED 10,000.00 may go in the next return; AED 10,000.01 may
/// not. Naming the Q4 2026 return attaches the adjustment to it, and when
/// that return is filed the new version is `cancelled` as *not required*; it
/// has no disclosure task and cannot be filed itself.
#[renox::test]
async fn an_error_up_to_the_threshold_goes_in_the_next_return() {
    let t = boot_real().await;
    let (client, engagement, filed) = filed_vat(&t).await;
    let crew = t.crew(&client).await;
    let (preparer, partner) = (&crew.preparer, &crew.partner);
    let q4 = t
        .return_in(
            &engagement,
            ReturnType::Vat,
            day("2026-12-31"),
            day("2027-01-28"),
            ReturnStatus::Approved,
        )
        .await;
    assert_eq!(
        routes_for(ReturnType::Vat, 1_000_000),
        [
            CorrectionRoute::VoluntaryDisclosure,
            CorrectionRoute::NextReturn
        ]
    );
    let next = q4.id.to_string();

    // One fils more is a voluntary disclosure.
    correct(
        &t,
        preparer,
        &filed,
        "10000.01",
        "next_return",
        &[("next_return", &next)],
    )
    .await
    .assert_status(422);
    // The route needs the return it goes into, a later one, not yet filed.
    correct(&t, preparer, &filed, "10000.00", "next_return", &[])
        .await
        .assert_status(422);
    correct(
        &t,
        preparer,
        &filed,
        "10000.00",
        "next_return",
        &[("next_return", &filed.id.to_string())],
    )
    .await
    .assert_status(422);
    assert_eq!(t.return_status(&filed).await, ReturnStatus::Filed);

    correct(
        &t,
        preparer,
        &filed,
        "10000.00",
        "next_return",
        &[("next_return", &next)],
    )
    .await
    .assert_ok();
    assert_eq!(t.return_status(&filed).await, ReturnStatus::Superseded);
    let new = successor(&t, &filed).await;
    assert_eq!(new.status, ReturnStatus::Preparing);
    assert!(new.is_corrected_via_next_return());
    assert_eq!(new.correction_return_id, Some(q4.id), "attached to Q4 2026");
    assert_eq!(new.error_amount, Some(1_000_000));
    assert!(disclosures(&t, &engagement).await.is_empty());
    t.get(partner, &format!("/accounting/returns/{}", new.id))
        .await
        .assert_ok()
        .assert_see("Adjustment in return");

    // Approved by the client, it is still never filed itself.
    let approved = t.return_at(&new, &crew, ReturnStatus::Approved).await;
    t.file_return(partner, &approved)
        .await
        .assert_status(422)
        .assert_see("never filed itself");
    assert_eq!(t.return_status(&new).await, ReturnStatus::Approved);

    // When Q4 2026 is filed, the new version is cancelled as not required.
    t.file_return(partner, &q4).await.assert_ok();
    assert_eq!(t.return_status(&q4).await, ReturnStatus::Filed);
    let cancelled = t.return_now(&new).await;
    assert_eq!(cancelled.status, ReturnStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("not_required"));
}

/// A later return named only by its period: the adjustment attaches when a
/// return for that period is opened (not before, and not to another period),
/// and the new version is cancelled when that return is filed.
#[renox::test]
async fn a_return_named_by_period_takes_the_adjustment_when_it_is_opened() {
    let t = boot_real().await;
    let (client, engagement, filed) = filed_vat(&t).await;
    let crew = t.crew(&client).await;

    // A period that is not later than the filed return is refused.
    correct(
        &t,
        &crew.preparer,
        &filed,
        "800.00",
        "next_return",
        &[("next_period", "2026-06-30")],
    )
    .await
    .assert_status(422);
    correct(
        &t,
        &crew.preparer,
        &filed,
        "800.00",
        "next_return",
        &[("next_period", "2026-12-31")],
    )
    .await
    .assert_ok();
    let new = successor(&t, &filed).await;
    assert_eq!(new.correction_return_id, None);
    assert_eq!(new.correction_period_end, Some(day("2026-12-31")));

    // Another period takes nothing; the return for the named period does.
    let other = accounting::returns::open(
        t.state(),
        None,
        accounting::returns::NewReturn::new(
            engagement.id,
            ReturnType::Vat,
            day("2027-03-31"),
            day("2027-04-28"),
        ),
    )
    .await
    .unwrap();
    assert_eq!(t.return_now(&new).await.correction_return_id, None);
    let q4 = accounting::returns::open(
        t.state(),
        None,
        accounting::returns::NewReturn::new(
            engagement.id,
            ReturnType::Vat,
            day("2026-12-31"),
            day("2027-01-28"),
        ),
    )
    .await
    .unwrap();
    assert_eq!(t.return_now(&new).await.correction_return_id, Some(q4.id));

    // Filing the other period leaves the correction; filing Q4 cancels it.
    let _ = other;
    let approved = t.return_at(&q4, &crew, ReturnStatus::Approved).await;
    t.file_return(&crew.partner, &approved).await.assert_ok();
    let cancelled = t.return_now(&new).await;
    assert_eq!(cancelled.status, ReturnStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("not_required"));
}

/// A corporate tax return: only the voluntary disclosure.
#[renox::test]
async fn a_corporate_tax_correction_is_always_a_voluntary_disclosure() {
    let t = boot_real().await;
    let preparer = t.accountant().await;
    t.set_clock(5, 10, 0);
    let client = t.client().await;
    let engagement = t.active_engagement(&client).await;
    let ct = t
        .return_in(
            &engagement,
            ReturnType::Ct,
            day("2025-12-31"),
            day("2026-09-30"),
            ReturnStatus::Filed,
        )
        .await;
    assert_eq!(
        routes_for(ReturnType::Ct, 100),
        [CorrectionRoute::VoluntaryDisclosure]
    );
    let later = t
        .return_in(
            &engagement,
            ReturnType::Vat,
            day("2026-12-31"),
            day("2027-01-28"),
            ReturnStatus::Preparing,
        )
        .await;
    correct(
        &t,
        &preparer,
        &ct,
        "100.00",
        "next_return",
        &[("next_return", &later.id.to_string())],
    )
    .await
    .assert_status(422);
    assert_eq!(t.return_status(&ct).await, ReturnStatus::Filed);

    correct(
        &t,
        &preparer,
        &ct,
        "100.00",
        "voluntary_disclosure",
        &[("reference", "VD-7")],
    )
    .await
    .assert_ok();
    let new = successor(&t, &ct).await;
    assert_eq!(new.correction_reference.as_deref(), Some("VD-7"));
    assert_eq!(disclosures(&t, &engagement).await.len(), 1);
}
