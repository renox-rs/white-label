//! Reminders and a hold on chasing (BL-PRP-13, #263): while a vertical holds
//! chasing for the source of an invoice (a legal case on a lease) no reminder
//! is sent; when the hold ends the steps whose day was reached meanwhile are
//! not sent, and the next one is.

mod support;

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_invoicing::reminders::{self, AFTER_DUE};
use wl_invoicing::{Invoice, ReminderLog};
use wl_kernel::Subject;
use wl_kernel::chase_holds::{ChaseHold, ChaseHolds, HoldFuture, register_chase_holds};
use wl_kernel::invoicing::VatCategory;

use support::{Booted, boot};

const LINE: (i64, VatCategory) = (100_000, VatCategory::Standard);

/// A provider the test switches.
#[derive(Clone)]
struct Switch(Arc<Mutex<ChaseHold>>);

impl ChaseHolds for Switch {
    fn hold<'a>(&'a self, _db: &'a Db, _source: &'a Subject) -> HoldFuture<'a, ChaseHold> {
        Box::pin(async move { Ok(*self.0.lock().unwrap()) })
    }
}

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

/// The invoices of the support are raised from `property.lease:9`.
#[renox::test]
async fn a_held_source_is_not_reminded_and_resumes_from_the_next_step() {
    let t = boot().await;
    let a = t.user_a().await;
    let switch = Switch(Arc::new(Mutex::new(ChaseHold::Held)));
    register_chase_holds("property.lease", switch.clone());
    // Due 10 days ago, never reminded.
    let invoice = t.issued(&[LINE], Some(t.day(-10)), &a).await;

    // Held: nothing is sent, whatever the day.
    for day in [-9, -3, 0, 3] {
        assert_eq!(reminders::run(t.state(), t.day(day)).await.unwrap(), 0);
    }
    assert!(steps(&t, &invoice).await.is_empty());
    assert!(t.notices.all().is_empty());

    // The hold ends today: the 7-day step was reached while it lasted and is
    // not sent; the 14-day step is, on its day.
    *switch.0.lock().unwrap() = ChaseHold::Resumed { on: t.day(0) };
    assert_eq!(reminders::run(t.state(), t.day(0)).await.unwrap(), 0);
    assert_eq!(reminders::run(t.state(), t.day(3)).await.unwrap(), 0);
    assert_eq!(reminders::run(t.state(), t.day(4)).await.unwrap(), 1);
    assert_eq!(steps(&t, &invoice).await, [14]);
    assert_eq!(t.notices.of_type(AFTER_DUE).len(), 1);

    // Free again: nothing changes for an invoice that was not held.
    *switch.0.lock().unwrap() = ChaseHold::Free;
    assert_eq!(reminders::run(t.state(), t.day(5)).await.unwrap(), 0);
}
