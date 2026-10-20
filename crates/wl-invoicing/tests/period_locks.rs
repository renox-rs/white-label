//! The kernel's `PeriodLocks` answered by wl-invoicing: a date is in a locked
//! period when the VAT summary of its tax period was filed.

mod support;

use renox::chrono::NaiveDate;
use wl_invoicing::vat_summary::VatSummaryStatus;
use wl_kernel::invoicing::period_locks;

use support::boot;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

#[renox::test]
async fn only_a_date_in_a_filed_period_is_locked() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    t.summary(&pgp, day(2020, 3, 15), VatSummaryStatus::Locked)
        .await;
    t.summary(&pgp, day(2020, 9, 15), VatSummaryStatus::Reviewed)
        .await;
    let locks = period_locks(t.state());

    assert!(
        locks
            .is_locked(t.db(), pgp.id, day(2020, 3, 15))
            .await
            .unwrap()
    );
    // Reviewed is not filed yet; a period with no summary is open.
    assert!(
        !locks
            .is_locked(t.db(), pgp.id, day(2020, 9, 15))
            .await
            .unwrap()
    );
    assert!(
        !locks
            .is_locked(t.db(), pgp.id, day(2019, 1, 1))
            .await
            .unwrap()
    );
    // An unknown entity has no locks.
    assert!(
        !locks
            .is_locked(t.db(), 9_999, day(2020, 3, 15))
            .await
            .unwrap()
    );
}
