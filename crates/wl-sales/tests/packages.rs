//! The package lifecycle base (wl-sales §4.4, base for BL-SAL-16): a sold
//! package activates when its invoice is paid, credits are consumed and given
//! back by their sources, a package expires after its end date, and the
//! kernel's `PackageLedger` does the same for the verticals.

mod support;

use std::sync::{Arc, Mutex};

use renox::chrono::{Months, NaiveDate};
use renox::prelude::*;
use wl_kernel::invoicing::{InvoiceId, InvoicePaid};
use wl_kernel::sales::{
    ConsumePackage, PackageActivated, PackageConsumed, PackageConsumption, PackageExpired,
    package_ledger,
};
use wl_kernel::{PartyId, Subject};
use wl_sales::packages::{self, NewPackage};
use wl_sales::{MovementKind, Package, PackageStatus};

use support::{Booted, boot_with};

const HOLDER: PartyId = PartyId(7);
const ITEM: &str = "physio_10";

/// Records the package events a test saw.
#[derive(Clone, Default)]
struct Seen {
    activated: Arc<Mutex<Vec<PackageActivated>>>,
    consumed: Arc<Mutex<Vec<PackageConsumed>>>,
    expired: Arc<Mutex<Vec<PackageExpired>>>,
}

impl Module for Seen {
    fn name(&self) -> &'static str {
        "package_events"
    }

    fn register(&self, app: &mut Registry) {
        let log = self.activated.clone();
        app.listen(move |event: PackageActivated, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
        let log = self.consumed.clone();
        app.listen(move |event: PackageConsumed, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
        let log = self.expired.clone();
        app.listen(move |event: PackageExpired, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
    }
}

async fn boot() -> (Booted, Seen) {
    let seen = Seen::default();
    let t = boot_with(seen.clone()).await;
    (t, seen)
}

fn today() -> NaiveDate {
    wl_kernel::documents::today()
}

/// A package of 10 credits sold for invoice 55.
async fn sold(t: &Booted) -> Package {
    let new = NewPackage::new(HOLDER, ITEM, 10)
        .price(150_000)
        .invoice(InvoiceId(55));
    packages::sell(t.state(), new).await.unwrap()
}

/// `package` after its invoice was paid, as `wl-invoicing` announces it.
async fn pay(t: &Booted, package: &Package) -> Package {
    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(55),
            party: HOLDER,
            source: package.subject(),
        })
        .await
        .unwrap();
    Package::find_or_404(t.db(), package.id).await.unwrap()
}

/// A package sold for invoice 55, and paid.
async fn paid(t: &Booted) -> Package {
    let package = sold(t).await;
    pay(t, &package).await
}

async fn reload(t: &Booted, package: &Package) -> Package {
    Package::find_or_404(t.db(), package.id).await.unwrap()
}

fn booking(n: i64) -> Subject {
    Subject::new("scheduling.booking", n)
}

#[renox::test]
async fn a_package_waits_for_payment_and_activates_when_its_invoice_is_paid() {
    let (t, seen) = boot().await;
    let package = sold(&t).await;
    assert_eq!(package.status, PackageStatus::PendingPayment);
    assert_eq!(package.valid_to, None);

    let package = pay(&t, &package).await;
    assert_eq!(package.status, PackageStatus::Active);
    assert_eq!(package.valid_from, Some(today()));
    assert_eq!(
        package.valid_to,
        today().checked_add_months(Months::new(6))
    );
    let events = seen.activated.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].credits, 10);
    assert_eq!(events[0].party, HOLDER);
}

#[renox::test]
async fn consuming_records_a_movement_and_the_last_credits_complete_the_package() {
    let (t, seen) = boot().await;
    let package = paid(&t).await;

    let outcome = packages::consume(t.state(), package.id, 4, booking(1))
        .await
        .unwrap();
    assert_eq!(
        outcome,
        PackageConsumption::Consumed {
            package: package.package_id(),
            remaining: 6
        }
    );
    let after = reload(&t, &package).await;
    assert_eq!((after.status, after.credits_used), (PackageStatus::Active, 4));
    let movements = after.movements(t.db()).await.unwrap();
    assert_eq!(movements.len(), 1);
    assert_eq!(movements[0].kind, MovementKind::Consume);
    assert_eq!(movements[0].source(), Some(booking(1)));

    // The same source again changes nothing.
    let again = packages::consume(t.state(), package.id, 4, booking(1))
        .await
        .unwrap();
    assert!(matches!(again, PackageConsumption::Already { .. }));
    assert_eq!(reload(&t, &package).await.credits_used, 4);

    // More than is left is refused; the exact rest completes the package.
    let too_many = packages::consume(t.state(), package.id, 7, booking(2)).await;
    assert!(too_many.is_err());
    packages::consume(t.state(), package.id, 6, booking(3))
        .await
        .unwrap();
    let done = reload(&t, &package).await;
    assert_eq!((done.status, done.remaining()), (PackageStatus::Completed, 0));
    let events = seen.consumed.lock().unwrap().clone();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].remaining, 0);
    assert_eq!(events[1].source, booking(3));

    // The cancelled source gives its credits back and the package is active.
    let back = package_ledger(t.state()).reverse(t.state(), booking(3)).await;
    assert!(back.unwrap());
    let reversed = reload(&t, &package).await;
    assert_eq!((reversed.status, reversed.remaining()), (PackageStatus::Active, 6));
}

#[renox::test]
async fn a_package_past_its_end_date_takes_no_credits_and_expires_in_the_daily_run() {
    let (t, seen) = boot().await;
    let package = paid(&t).await;
    let yesterday = today().pred_opt().unwrap();
    Package::query()
        .where_(Package::ID.eq(package.id))
        .update(t.db(), &[("valid_to", &Some(yesterday))])
        .await
        .unwrap();

    assert!(
        packages::consume(t.state(), package.id, 1, booking(1))
            .await
            .is_err()
    );
    assert_eq!(packages::run_expiry(t.state(), today()).await.unwrap(), 1);
    assert_eq!(reload(&t, &package).await.status, PackageStatus::Expired);
    assert_eq!(seen.expired.lock().unwrap().len(), 1);
    // A second run finds nothing.
    assert_eq!(packages::run_expiry(t.state(), today()).await.unwrap(), 0);
}

#[renox::test]
async fn the_kernel_ledger_takes_credits_of_the_package_that_ends_first() {
    let (t, _) = boot().await;
    let first = paid(&t).await;
    // A second package for the same item, valid longer.
    let second = packages::sell(
        t.state(),
        NewPackage::new(HOLDER, ITEM, 5).validity_months(12),
    )
    .await
    .unwrap();
    packages::link_invoice(t.state(), second.id, InvoiceId(56))
        .await
        .unwrap();
    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(56),
            party: HOLDER,
            source: second.subject(),
        })
        .await
        .unwrap();

    let ledger = package_ledger(t.state());
    assert_eq!(ledger.balance(t.db(), HOLDER, ITEM).await.unwrap(), 15);
    let request = ConsumePackage::new(HOLDER, ITEM, 2, booking(9));
    let outcome = ledger.consume(t.state(), request).await.unwrap();
    assert_eq!(
        outcome,
        PackageConsumption::Consumed {
            package: first.package_id(),
            remaining: 8
        }
    );
    let none = ConsumePackage::new(HOLDER, "other_item", 1, booking(10));
    assert_eq!(
        ledger.consume(t.state(), none).await.unwrap(),
        PackageConsumption::NoCredit
    );
}

#[renox::test]
async fn a_package_needs_credits_and_a_valid_price() {
    let (t, _) = boot().await;
    assert!(
        packages::sell(t.state(), NewPackage::new(HOLDER, ITEM, 0))
            .await
            .is_err()
    );
    assert!(
        packages::sell(t.state(), NewPackage::new(HOLDER, ITEM, 3).price(-1))
            .await
            .is_err()
    );
    t.app.assert_models_match_schema().await;
}
