//! The receivables ageing with its disputed column (wl-invoicing §5.10,
//! #67 part 7): the acceptance check AC3. The ageing is asked for on explicit
//! days; the page shows the same figures.

mod support;

use renox::prelude::*;
use wl_invoicing::ageing;
use wl_invoicing::dispute::{self, Dispute};
use wl_invoicing::permissions as perm;
use wl_kernel::invoicing::{DisputeBy, DisputeRequest, InvoiceId, VatCategory};

use support::{BLUE_DUNES, boot};

const EDGES: [u32; 3] = [30, 60, 90];

/// AC3: of an invoice of AED 5,000.00 with AED 1,200.00 disputed, the party's
/// Disputed column shows 1,200.00 and its bucket 3,800.00. After a payment of
/// AED 4,500.00 the column shows the balance, AED 500.00, and the stored
/// amount stays 1,200.00. The ageing page shows the same.
#[renox::test]
async fn the_disputed_column_holds_the_disputed_part_of_the_balance() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t
        .issued(&[(500_000, VatCategory::ZeroRated)], Some(t.day(-40)), &a)
        .await;
    let id = dispute::raise(
        t.state(),
        DisputeRequest::new(
            InvoiceId(invoice.id),
            "wrong hours",
            DisputeBy::User { id: a.id },
        )
        .amount(120_000),
    )
    .await
    .unwrap();

    let report = ageing::report(t.db(), &EDGES, None, t.day(0))
        .await
        .unwrap();
    assert_eq!(report.labels.len(), 4);
    assert_eq!(report.rows.len(), 1);
    let row = &report.rows[0];
    assert_eq!(row.party_id, BLUE_DUNES.0);
    assert_eq!(row.disputed, 120_000);
    assert_eq!(row.buckets, [0, 380_000, 0, 0], "40 days past due");
    assert_eq!(row.total, 500_000);

    let viewer = t.user("Rita", &[perm::RECEIVABLES_VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .get("/receivables/ageing")
        .await
        .assert_ok()
        .assert_see("AED 1,200.00")
        .assert_see("AED 3,800.00")
        .assert_see("AED 5,000.00");
    // The rows sort by a column and download as CSV (#525).
    let sorted = t.app.get("/receivables/ageing?sort=-bucket2").await;
    sorted.assert_ok().assert_see("AED 3,800.00");
    assert!(
        sorted
            .text()
            .contains("/receivables/ageing?sort=-bucket2&amp;export=csv")
    );
    let file = t.app.get("/receivables/ageing?export=csv").await;
    file.assert_ok();
    assert!(
        file.header("content-type")
            .is_some_and(|c| c.starts_with("text/csv"))
    );
    let csv = file.text();
    assert!(csv.contains("Party,Currency,0-30,31-60,61-90,90+,Disputed,Balance"));
    assert!(csv.contains(",AED,AED 0.00,\"AED 3,800.00\",AED 0.00,AED 0.00,\"AED 1,200.00\",\"AED 5,000.00\""));

    t.pay(&invoice, 1, 450_000).await;
    let report = ageing::report(t.db(), &EDGES, None, t.day(0))
        .await
        .unwrap();
    let row = &report.rows[0];
    assert_eq!(row.disputed, 50_000, "no more than the balance");
    assert_eq!(row.buckets, [0, 0, 0, 0]);
    assert_eq!(row.total, 50_000);
    let stored = Dispute::find(t.db(), id).await.unwrap().unwrap();
    assert_eq!(stored.amount, 120_000);

    t.app
        .get("/receivables/ageing")
        .await
        .assert_ok()
        .assert_see("AED 500.00");
}
