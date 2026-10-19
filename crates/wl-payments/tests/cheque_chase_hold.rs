//! `FIN.RUL.18` and a hold on chasing (BL-PRP-13, #263): while a vertical
//! holds chasing for the agreement a bounced cheque was received with (a legal
//! case on the lease), the party is not sent the reminder, and it is not sent
//! late when the case ends.

mod support;

use std::sync::{Arc, Mutex};

use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::chase_holds::{ChaseHold, ChaseHolds, HoldFuture, register_chase_holds};
use wl_kernel::cheques::RegisterCheque;
use wl_kernel::documents::today;
use wl_payments::cheque_chase;
use wl_payments::notices::CHEQUE_REMINDER;

use support::{BLUE_DUNES, boot};

/// A provider the test switches.
#[derive(Clone)]
struct Switch(Arc<Mutex<ChaseHold>>);

impl ChaseHolds for Switch {
    fn hold<'a>(&'a self, _db: &'a Db, _source: &'a Subject) -> HoldFuture<'a, ChaseHold> {
        Box::pin(async move { Ok(*self.0.lock().unwrap()) })
    }
}

/// A bounced cheque received with `property.lease:5`: held, the reminder of the
/// third day is skipped and not sent later; without a hold it is sent.
#[renox::test]
async fn a_held_agreement_gets_no_bounce_reminder() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let switch = Switch(Arc::new(Mutex::new(ChaseHold::Held)));
    register_chase_holds("property.lease", switch.clone());

    let entity = t.entity("PGP").await;
    let request = RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "5001", today(), 1_000_000)
        .agreement(Subject::new("property.lease", 5));
    let cheque = t.register_cheque(request).await;
    let cheque = t.deposit_cheque(&staff, &cheque).await;
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "CERT-1", "")
        .await
        .assert_ok();

    let day = today() + TimeDelta::days(3);
    let held = cheque_chase::run(t.state(), day).await.unwrap();
    assert_eq!(held.reminded, 0);
    assert!(t.notices.of_type(CHEQUE_REMINDER).is_empty());

    // The case ends: the skipped reminder is not sent late.
    *switch.0.lock().unwrap() = ChaseHold::Free;
    let after = cheque_chase::run(t.state(), day + TimeDelta::days(1)).await.unwrap();
    assert_eq!(after.reminded, 0);
    assert!(t.notices.of_type(CHEQUE_REMINDER).is_empty());
}
