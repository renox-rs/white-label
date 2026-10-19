//! Payment reminders (wl-invoicing §5.3, `FIN.RUL.01`): the acceptance check
//! AC15 of #64.

mod support;

use renox::prelude::*;
use wl_invoicing::reminders::{self, AFTER_DUE, BEFORE_DUE};
use wl_invoicing::{Invoice, ReminderLog};
use wl_kernel::approvals;
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};
use wl_kernel::notify::NotifyRecipient;

use support::{BLUE_DUNES, Booted, boot};

const LINE: (i64, VatCategory) = (100_000, VatCategory::Standard);

/// The steps logged for `invoice`, oldest first.
async fn steps(t: &Booted, invoice: &Invoice) -> Vec<i64> {
    ReminderLog::query()
        .where_(ReminderLog::INVOICE_ID.eq(invoice.id))
        .order_by(ReminderLog::STEP)
        .get(t.db())
        .await
        .unwrap()
        .iter()
        .map(|log| log.step)
        .collect()
}

/// AC15: an issued invoice gets the "3 days before" reminder once, and the
/// "1 day after" reminder on its day.
#[renox::test]
async fn an_issued_invoice_gets_each_reminder_once() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&[LINE], Some(t.day(10)), &a).await;
    let number = invoice.number.clone().unwrap();

    // Earlier than 3 days before: nothing.
    assert_eq!(reminders::run(t.state(), t.day(6)).await.unwrap(), 0);
    assert!(t.notices.all().is_empty());

    // 3 days before the due date.
    assert_eq!(reminders::run(t.state(), t.day(7)).await.unwrap(), 1);
    let sent = t.notices.all();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].type_key, BEFORE_DUE);
    assert_eq!(sent[0].subject, invoice.subject());
    assert!(matches!(
        sent[0].recipients[..],
        [NotifyRecipient::Party { id }] if id == BLUE_DUNES
    ));
    assert_eq!(sent[0].data["number"], number.as_str());
    assert_eq!(sent[0].data["balance"], "AED 1,050.00");
    assert_eq!(sent[0].data["days"], 3);

    // The same day again, and the days up to the due date: not twice.
    for day in [7, 7, 8, 9, 10] {
        assert_eq!(reminders::run(t.state(), t.day(day)).await.unwrap(), 0);
    }
    assert_eq!(t.notices.all().len(), 1);

    // 1 day after.
    assert_eq!(reminders::run(t.state(), t.day(11)).await.unwrap(), 1);
    let sent = t.notices.all();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].type_key, AFTER_DUE);
    assert_eq!(sent[1].data["days"], 1);
    assert_eq!(reminders::run(t.state(), t.day(11)).await.unwrap(), 0);
    assert_eq!(steps(&t, &invoice).await, [-3, 1]);
}

/// AC15 (§5.3 step 2): at most one step per invoice per run, the latest that
/// has come; the reminder types are registered with the kernel.
#[renox::test]
async fn a_run_sends_only_the_latest_step_that_has_come() {
    let t = boot().await;
    let a = t.user_a().await;
    // Due 10 days ago and never reminded: the 7-day step, not 1 and 7.
    let late = t.issued(&[LINE], Some(t.day(-10)), &a).await;
    assert_eq!(reminders::run(t.state(), t.day(0)).await.unwrap(), 1);
    assert_eq!(steps(&t, &late).await, [7]);
    assert_eq!(t.notices.of_type(AFTER_DUE).len(), 1);
    // Four days later the 14-day step comes.
    assert_eq!(reminders::run(t.state(), t.day(4)).await.unwrap(), 1);
    assert_eq!(steps(&t, &late).await, [7, 14]);

    for key in [BEFORE_DUE, AFTER_DUE] {
        let def = wl_kernel::notify::notification_type(key).expect("a registered type");
        assert_eq!(def.module, "invoicing");
    }
}

/// AC15: a written-off invoice gets no reminder, whatever the date.
#[renox::test]
async fn a_written_off_invoice_gets_no_reminder() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let invoice = t.overdue_invoice(&a).await;
    t.post_write_off(&invoice, &a, "insolvent", "")
        .await
        .assert_ok();
    let request = t.pending(&invoice).await.remove(0);
    approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::WrittenOff);

    // A control that is still open is reminded on the same days.
    let open = t.issued(&[LINE], Some(t.day(-5)), &a).await;
    for day in [-10, -5, -2, 0, 3, 30, 400] {
        reminders::run(t.state(), t.day(day)).await.unwrap();
    }
    assert!(steps(&t, &invoice).await.is_empty());
    assert!(!steps(&t, &open).await.is_empty());
    let for_invoice = invoice.subject();
    assert!(t.notices.all().iter().all(|r| r.subject != for_invoice));
}

/// AC15: a disputed invoice gets none; neither do a draft or a paid one.
#[renox::test]
async fn disputed_draft_and_paid_invoices_get_no_reminder() {
    let t = boot().await;
    let a = t.user_a().await;
    let disputed = t.issued(&[LINE], Some(t.day(-5)), &a).await;
    Invoice::query()
        .where_(Invoice::ID.eq(disputed.id))
        .update(t.db(), &[("disputed", &true)])
        .await
        .unwrap();
    let draft = t.draft(&[LINE], Some(t.day(-5))).await;
    let paid = t.issued(&[LINE], Some(t.day(-5)), &a).await;
    t.pay(&paid, 1, 105_000).await;

    for day in [-5, 0, 3, 30, 400] {
        assert_eq!(reminders::run(t.state(), t.day(day)).await.unwrap(), 0);
    }
    for invoice in [&disputed, &draft, &paid] {
        assert!(steps(&t, invoice).await.is_empty());
    }
    assert!(t.notices.all().is_empty());
}
