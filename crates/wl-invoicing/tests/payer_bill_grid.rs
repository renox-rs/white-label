//! The payer bills list is a standard grid (#525): it searches by the payer's
//! name as well as the number, sorts by the payer, pages, shows the source
//! record in words and fills the page (the grid is not wrapped).

mod support;

use wl_invoicing::VatRecipient;
use wl_kernel::documents::today;

use support::{BLUE_DUNES, QUIET_SANDS, boot};

/// Whether the grid is the page's own block: no stack, card or pane between
/// the header and the grid.
fn fills_the_page(html: &str) -> bool {
    let Some(header) = html.find("class=\"rx-page-header\"") else {
        return false;
    };
    let Some(grid) = html.find("<form class=\"rx-grid") else {
        return false;
    };
    let between = &html[header..grid];
    header < grid
        && !between.contains("rx-stack")
        && !between.contains("rx-card")
        && !between.contains("rx-pane")
}

#[renox::test]
async fn the_payer_bills_grid_searches_by_payer_sorts_and_pages() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let blue = t.agreement(BLUE_DUNES, VatRecipient::Customer).await;
    let quiet = t.agreement(QUIET_SANDS, VatRecipient::Customer).await;
    for n in 0..12 {
        t.payer_bill(&blue, today(), &[("Consultation", 10_000 + n)])
            .await;
    }
    t.payer_bill(&quiet, today(), &[("Physiotherapy", 50_000)])
        .await;
    t.app.acting_as(&cora);

    let first = t.app.get("/payer-bills?per_page=10").await;
    first.assert_ok().assert_see("1–10 of 13");
    let html = first.text();
    assert!(fills_the_page(&html), "the grid fills the page");
    assert!(html.contains("Blue Dunes LLC") && html.contains("Omar Hassan"));
    assert!(html.contains("Claim #"), "the source in words");
    assert!(!html.contains("clinics.claim:"));
    t.app
        .get("/payer-bills?per_page=10&page=2")
        .await
        .assert_ok()
        .assert_see("11–13 of 13");

    let quiet_only = t.app.get("/payer-bills?search=quiet").await;
    quiet_only.assert_ok().assert_see("1–1 of 1");
    assert!(!quiet_only.text().contains("Blue Dunes LLC"));
    t.app
        .get("/payer-bills?search=blue%20dunes&per_page=25")
        .await
        .assert_ok()
        .assert_see("1–12 of 12");

    let at = |html: &str, text: &str| html.find(text).unwrap_or(usize::MAX);
    let desc = t
        .app
        .get("/payer-bills?sort=-payer&per_page=25")
        .await
        .text();
    assert!(at(&desc, "Quiet Sands LLC") < at(&desc, "Blue Dunes LLC"));
    let asc = t
        .app
        .get("/payer-bills?sort=payer&per_page=25")
        .await
        .text();
    assert!(at(&asc, "Blue Dunes LLC") < at(&asc, "Quiet Sands LLC"));
}
