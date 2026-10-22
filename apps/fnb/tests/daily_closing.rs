//! BL-FNB-15 (#379), 06 §4.6 step 4, §7.1 *Daily closing* (`submit`, `reopen`),
//! §7.2 business-day cut-off, `FNB.ACL.05`, `PLT.ACL.10`, wl-sales *Till
//! session* and BL-PAY-19: an order belongs to the business day of its outlet's
//! cut-off and appears in that day's closing; the closing is submitted only
//! after the cut-off, counts the till (a shortage makes the till's cash
//! handover), a late entry into a submitted closing waits for the area manager
//! and reopens it, a late entry into an approved closing is refused with a task
//! for the area manager, and only the area manager reopens. The modules are the
//! real wl-sales (till sessions), wl-payments (cash handover), wl-kernel (late
//! entries and approvals), wl-inventory and wl-delivery.

mod support;

use fnb::closing::{ClosingStatus, DailyClosing};
use fnb::closings::{self, SubmitAsk};
use fnb::{FnbOrder, Outlet, PaperOrder};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::custody::HandoverStatus;
use wl_kernel::late_entries::LateEntry;
use wl_kernel::sales::{OpenTill, TillView, till_desk};
use wl_payments::cash_till;

use support::World;
use support::ops::{Ops, boot_ops, day_after, dubai_at, go_to, now_of};

const NOTICE: &str = "fnb.staff_notice";

/// The closing of Marina for `day`.
async fn closing_of(w: &World, o: &Ops, day: NaiveDate) -> DailyClosing {
    closings::ensure(w.db(), o.marina.id, day).await.unwrap()
}

/// The cashier opens Marina's till for `day` with a float of `float` fils.
async fn open_till(w: &World, o: &Ops, day: NaiveDate, float: i64) -> TillView {
    till_desk(w.state())
        .open(
            w.state(),
            None,
            OpenTill {
                till: o.marina.till_key(),
                location: Some(o.marina.code.clone()),
                legal_entity_id: None,
                business_date: day,
                cashier_user: o.till.id,
                float,
            },
        )
        .await
        .unwrap()
        .expect("the sales module provides the till desk")
}

/// `by` submits the closing with the cash counted in AED.
async fn submit(
    w: &World,
    by: &User,
    o: &Ops,
    closing: &DailyClosing,
    counted: &str,
    checklist: bool,
) -> TestResponse {
    let cashier = o.till.id.to_string();
    let mut fields = vec![("counted", counted), ("cashier", cashier.as_str())];
    if checklist {
        fields.push(("checklist", "on"));
    }
    w.post(by, &format!("/closings/{}/submit", closing.id), &fields)
        .await
}

async fn reload(w: &World, closing: &DailyClosing) -> DailyClosing {
    DailyClosing::find_or_404(w.db(), closing.id).await.unwrap()
}

/// A paper order of the day `day` at `time`, posted by the cashier.
async fn paper(w: &World, o: &Ops, day: NaiveDate, time: &str, reason: &str) -> TestResponse {
    let outlet = o.marina.id.to_string();
    let day = day.to_string();
    w.post(
        &o.till,
        "/paper-orders",
        &[
            ("outlet", outlet.as_str()),
            ("channel", "counter"),
            ("reference", "P-0007"),
            ("amount", "35.00"),
            ("method", "cash"),
            ("occurred_on", day.as_str()),
            ("occurred_time", time),
            ("reason", reason),
        ],
    )
    .await
}

/// A closing of `day` counted and submitted the next morning at 04:05, as the
/// kitchen manager does. The clock is left there.
async fn submitted(w: &World, o: &Ops, day: NaiveDate, days_ahead: i64) -> DailyClosing {
    let closing = closing_of(w, o, day).await;
    open_till(w, o, day, 10_000).await;
    go_to(w, dubai_at(days_ahead, "04:05")).await;
    submit(w, &o.kitchen, o, &closing, "100.00", true)
        .await
        .assert_status(303);
    let closing = reload(w, &closing).await;
    assert_eq!(closing.status, ClosingStatus::Submitted);
    closing
}

/// AC1: with the cut-off 04:00, an order at 01:30 has the business date of the
/// day before and appears in that day's closing; one at 04:10 belongs to the new
/// day.
#[renox::test]
async fn an_order_before_the_cut_off_belongs_to_the_day_before_and_its_closing() {
    let (w, o) = boot_ops().await;
    let customer = w.customer("Night Owl").await;
    go_to(&w, dubai_at(2, "01:30")).await;
    let early = w
        .place(
            customer,
            "cash_on_delivery",
            &[("shawarma-plate", "1", "60.00")],
        )
        .await;
    let early = FnbOrder::find_or_404(w.db(), early.id).await.unwrap();
    assert_eq!(early.outlet_id, Some(o.marina.id));
    assert_eq!(early.business_date, Some(day_after(1)));

    go_to(&w, dubai_at(2, "04:10")).await;
    let later = w
        .place(
            customer,
            "cash_on_delivery",
            &[("falafel-wrap", "1", "18.00")],
        )
        .await;
    let later = FnbOrder::find_or_404(w.db(), later.id).await.unwrap();
    assert_eq!(later.business_date, Some(day_after(2)));

    let first = DailyClosing::of_day(w.db(), o.marina.id, day_after(1))
        .await
        .unwrap()
        .expect("the order made the closing of its business day");
    let totals = closings::totals(w.db(), &first).await.unwrap();
    assert_eq!((totals.orders, totals.gross), (1, 6_000));
    let second = DailyClosing::of_day(w.db(), o.marina.id, day_after(2))
        .await
        .unwrap()
        .expect("and the later one the next day's");
    assert_eq!(closings::totals(w.db(), &second).await.unwrap().orders, 1);
    // The page shows the day's orders.
    let page = w.get(&o.kitchen, &format!("/closings/{}", first.id)).await;
    page.assert_ok();
    assert!(page.text().contains("Marina Kitchen"), "{}", page.text());
}

/// A cut-off change applies from the next business day only.
#[renox::test]
async fn a_cut_off_change_applies_from_the_next_business_day() {
    let (w, o) = boot_ops().await;
    // The pure rule: 04:00 until the 16th, 02:00 from the 17th.
    let date = |text: &str| NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap();
    let at = |text: &str| fnb::business_day::at_local(date(&text[..10]), &text[11..]).unwrap();
    let mut outlet = o.marina.clone();
    outlet.cutoff_minutes = Some(240);
    outlet.pending_cutoff_minutes = Some(120);
    outlet.pending_cutoff_from = Some(date("2026-10-17"));
    assert_eq!(
        outlet.business_date(at("2026-10-16 03:00")),
        date("2026-10-15")
    );
    assert_eq!(
        outlet.business_date(at("2026-10-17 03:00")),
        date("2026-10-17")
    );
    assert_eq!(outlet.cutoff_for(date("2026-10-16")), 240);
    assert_eq!(outlet.cutoff_for(date("2026-10-17")), 120);

    // Through the screen: the area manager sets it, a kitchen manager cannot.
    let path = format!("/outlets/{}/cutoff", o.marina.id);
    w.post(&o.kitchen, &path, &[("time", "02:00")])
        .await
        .assert_forbidden();
    w.post(&o.area, &path, &[("time", "late")])
        .await
        .assert_status(422);
    w.post(&o.area, &path, &[("time", "02:00")])
        .await
        .assert_status(303);
    let saved = Outlet::find_or_404(w.db(), o.marina.id).await.unwrap();
    assert_eq!(saved.cutoff_minutes, None, "today's cut-off is kept");
    assert_eq!(saved.pending_cutoff_minutes, Some(120));
    let today = saved.business_date(now_of(&w).await);
    assert_eq!(saved.pending_cutoff_from, Some(today.succ_opt().unwrap()));
}

/// AC2: submitting the 15th's closing at 03:00 on the 16th is refused with the
/// time it opens (and the till is not counted); at 04:05 it succeeds.
#[renox::test]
async fn the_closing_is_refused_before_the_cut_off_and_submitted_after_it() {
    let (w, o) = boot_ops().await;
    let day = day_after(1);
    let closing = closing_of(&w, &o, day).await;
    let till = open_till(&w, &o, day, 10_000).await;

    // The checklist and the count come first.
    go_to(&w, dubai_at(2, "03:00")).await;
    let refused = submit(&w, &o.kitchen, &o, &closing, "100.00", true).await;
    refused.assert_status(422);
    assert!(refused.text().contains("04:00"), "{}", refused.text());
    assert!(
        refused
            .text()
            .contains(&dubai_at(2, "04:00").format("%Y").to_string())
    );
    assert_eq!(reload(&w, &closing).await.status, ClosingStatus::Open);
    let still = till_desk(w.state())
        .find(w.state(), till.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.status, "open", "the till was not counted");

    go_to(&w, dubai_at(2, "04:05")).await;
    submit(&w, &o.kitchen, &o, &closing, "100.00", false)
        .await
        .assert_status(422);
    submit(&w, &o.kitchen, &o, &closing, "", true)
        .await
        .assert_status(422);
    w.post(
        &o.till,
        &format!("/closings/{}/submit", closing.id),
        &[("counted", "100.00"), ("checklist", "on")],
    )
    .await
    .assert_forbidden();
    submit(&w, &o.kitchen, &o, &closing, "100.00", true)
        .await
        .assert_status(303);
    let done = reload(&w, &closing).await;
    assert_eq!(done.status, ClosingStatus::Submitted);
    assert_eq!(done.counted_cash, Some(10_000));
    assert_eq!(done.variance, Some(0));
    assert_eq!(done.till_session_id, Some(till.id));
    let counted = till_desk(w.state())
        .find(w.state(), till.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counted.status, "closed");
}

/// AC4: a closing with a AED 35.00 shortage creates a till handover with
/// AED 35.00 short, held by the outlet's till for the cashier.
#[renox::test]
async fn a_shortage_at_closing_creates_a_till_handover_with_the_difference() {
    let (w, o) = boot_ops().await;
    let day = day_after(1);
    let closing = closing_of(&w, &o, day).await;
    let till = open_till(&w, &o, day, 10_000).await;
    go_to(&w, dubai_at(2, "04:05")).await;
    submit(&w, &o.kitchen, &o, &closing, "65.00", true)
        .await
        .assert_status(303);
    let done = reload(&w, &closing).await;
    assert_eq!(
        (done.expected_cash, done.counted_cash, done.variance),
        (10_000, Some(6_500), Some(-3_500))
    );
    let handover = cash_till::of_session(w.db(), till.id)
        .await
        .unwrap()
        .expect("the closed till session made a handover");
    assert_eq!(handover.status, HandoverStatus::Short);
    assert_eq!(handover.difference, 3_500, "AED 35.00 short");
    assert_eq!(
        handover.holder_till.as_deref(),
        Some(o.marina.till_key().as_str())
    );
    assert_eq!(handover.responsible_user_id, o.till.id);
}

/// AC3: a late entry with business time 21:00 of the 15th entered on the 16th at
/// 09:00 into the submitted closing creates an approval for the area manager; on
/// approval the closing is reopened and the kitchen manager is notified.
#[renox::test]
async fn a_late_entry_into_a_submitted_closing_waits_for_the_area_manager_and_reopens_it() {
    let (w, o) = boot_ops().await;
    let day = day_after(1);
    let closing = submitted(&w, &o, day, 2).await;
    go_to(&w, dubai_at(2, "09:00")).await;
    // The reason is required for a late entry.
    paper(&w, &o, day, "21:00", "").await.assert_status(422);
    paper(&w, &o, day, "21:00", "Printout found after the outage")
        .await
        .assert_status(303);

    // Nothing is recorded and the closing is still locked.
    assert_eq!(PaperOrder::query().count(w.db()).await.unwrap(), 0);
    assert_eq!(reload(&w, &closing).await.status, ClosingStatus::Submitted);
    let entry = LateEntry::query()
        .first(w.db())
        .await
        .unwrap()
        .expect("the late entry is stored");
    assert!(entry.needs_approval);
    let pending = ApprovalRequest::pending_about(w.db(), &entry.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "one approval for the area manager");

    // The requester cannot approve it; the area manager does.
    assert!(
        wl_kernel::approvals::approve(w.state(), pending[0].id, &o.till, "")
            .await
            .is_err()
    );
    wl_kernel::approvals::approve(w.state(), pending[0].id, &o.area, "")
        .await
        .expect("the area manager approves");

    let paper = PaperOrder::query()
        .first(w.db())
        .await
        .unwrap()
        .expect("recorded on approval");
    assert!(paper.late);
    assert_eq!(paper.business_date, Some(day));
    assert_eq!(paper.reopened_closing_id, Some(closing.id));
    let reopened = reload(&w, &closing).await;
    assert_eq!(reopened.status, ClosingStatus::Reopened);
    assert_eq!(
        reopened.reopen_reason.as_deref(),
        Some("Late entries after an outage")
    );
    assert_eq!(reopened.reopen_count, 1);
    // The kitchen manager is told, and the till can be counted again.
    assert!(w.notices.users_of_type(NOTICE).contains(&o.kitchen.id));
    let till = till_desk(w.state())
        .find(w.state(), reopened.till_session_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(till.status, "open");
    // The paper order is in the closing's sales.
    let totals = closings::totals(w.db(), &reopened).await.unwrap();
    assert_eq!(
        (totals.paper_orders, totals.late_entries, totals.gross),
        (1, 1, 3_500)
    );
    // And it is submitted again.
    submit(&w, &o.kitchen, &o, &reopened, "100.00", true)
        .await
        .assert_status(303);
    assert_eq!(reload(&w, &closing).await.status, ClosingStatus::Submitted);
}

/// AC5: given the closing `approved`, a late entry with business time 22:00 is
/// refused, no `reopen` is offered, and the area manager gets a task to record
/// it as an adjustment in the next day's closing.
#[renox::test]
async fn a_late_entry_into_an_approved_closing_is_refused_with_a_task_for_the_area_manager() {
    let (w, o) = boot_ops().await;
    let day = day_after(1);
    let closing = submitted(&w, &o, day, 2).await;
    w.post(
        &o.area,
        &format!("/closings/{}/approve", closing.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    assert_eq!(reload(&w, &closing).await.status, ClosingStatus::Approved);

    go_to(&w, dubai_at(2, "09:00")).await;
    let answer = paper(&w, &o, day, "22:00", "Found in the drawer").await;
    answer.assert_status(422);
    assert!(answer.text().contains("approved"), "{}", answer.text());
    assert_eq!(PaperOrder::query().count(w.db()).await.unwrap(), 0);
    assert_eq!(LateEntry::query().count(w.db()).await.unwrap(), 0);

    // No reopen is offered: the closing is terminal.
    w.post(
        &o.area,
        &format!("/closings/{}/reopen", closing.id),
        &[("reason", "One more")],
    )
    .await
    .assert_status(422);
    let page = w.get(&o.area, &format!("/closings/{}", closing.id)).await;
    page.assert_ok();
    assert!(!page.text().contains("closing-reopen"), "no reopen sheet");
    assert_eq!(reload(&w, &closing).await.status, ClosingStatus::Approved);

    // The area manager's task names the next day.
    let key = format!("fnb.closing_adjustment:{}:P-0007", closing.id);
    let tasks = w.tickets.opened_with(&key);
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0].title.contains(&day_after(2).to_string()),
        "{}",
        tasks[0].title
    );
}

/// Must not let anyone but the area manager reopen a submitted closing; the
/// accountant approves but does not reopen; the reason is required, the till
/// goes back to open, and the kitchen manager submits again.
#[renox::test]
async fn only_the_area_manager_reopens_a_submitted_closing() {
    let (w, o) = boot_ops().await;
    let day = day_after(1);
    let closing = submitted(&w, &o, day, 2).await;
    let path = format!("/closings/{}/reopen", closing.id);
    w.post(&o.kitchen, &path, &[("reason", "Miscounted")])
        .await
        .assert_forbidden();
    w.post(&o.accountant, &path, &[("reason", "Miscounted")])
        .await
        .assert_forbidden();
    w.post(&o.area, &path, &[("reason", "")])
        .await
        .assert_status(422);
    w.post(&o.area, &path, &[("reason", "Miscounted the float")])
        .await
        .assert_status(303);
    let reopened = reload(&w, &closing).await;
    assert_eq!(reopened.status, ClosingStatus::Reopened);
    assert_eq!(
        reopened.reopen_reason.as_deref(),
        Some("Miscounted the float")
    );
    assert_eq!(reopened.reopened_by, Some(o.area.id));
    let till = till_desk(w.state())
        .find(w.state(), reopened.till_session_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(till.status, "open");
    // Submitted again with the right count.
    submit(&w, &o.kitchen, &o, &reopened, "100.00", true)
        .await
        .assert_status(303);
    // The accountant approves it.
    w.post(
        &o.accountant,
        &format!("/closings/{}/approve", closing.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    assert_eq!(reload(&w, &closing).await.status, ClosingStatus::Approved);
}

/// The roles: nobody without the permission sees or changes a closing; the
/// kitchen manager cannot approve; a cashier cannot submit.
#[renox::test]
async fn the_closing_screens_are_for_the_roles_that_hold_the_permissions() {
    let (w, o) = boot_ops().await;
    let closing = closing_of(&w, &o, day_after(1)).await;
    w.get(&w.nobody, "/closings").await.assert_forbidden();
    w.get(&w.nobody, &format!("/closings/{}", closing.id))
        .await
        .assert_forbidden();
    let list = w.get(&o.kitchen, "/closings").await;
    list.assert_ok();
    assert!(list.text().contains("Marina Kitchen"));
    w.post(
        &o.kitchen,
        &format!("/closings/{}/approve", closing.id),
        &[("x", "")],
    )
    .await
    .assert_forbidden();
    w.post(
        &o.till,
        "/closings",
        &[("outlet", &o.marina.id.to_string())],
    )
    .await
    .assert_forbidden();
    w.post(
        &o.kitchen,
        "/closings",
        &[("outlet", &o.marina.id.to_string())],
    )
    .await
    .assert_status(303);
    // A service call as the system runs the system twin.
    let ask = SubmitAsk {
        counted: 0,
        checklist_done: true,
        cashier: Some(o.till.id),
    };
    go_to(&w, dubai_at(3, "05:00")).await;
    let state = w.state().clone();
    let result = w
        .app
        .at_travelled_time(closings::submit(&state, None, closing.id, &ask))
        .await;
    assert_eq!(result.unwrap().status, ClosingStatus::Submitted);
}
