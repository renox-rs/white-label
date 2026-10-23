//! BL-MFG-14 (#395): the outcome proposed for the started goods of a
//! cancelled sales line follows the item's flag (07 §7.1 *Sales order*
//! `cancel`, rules `MFG.RUL.09`): scrap for an item made for one customer,
//! finish to stock for any other.

mod support;

use manufacturing::ItemFlag;
use renox::prelude::*;
use wl_kernel::sales::{LineOutcomeKind, SalesOrderId, outcome_proposals};

use support::{PANEL, boot};

async fn proposed(t: &support::Booted, code: &str) -> Option<LineOutcomeKind> {
    outcome_proposals(t.state())
        .proposed(t.state(), SalesOrderId(1), 1, code)
        .await
        .unwrap()
}

/// BL-MFG-14, Done-when "the proposed outcome follows the customer-specific
/// flag": an unflagged item is proposed *finish to stock*, a flagged one
/// *scrap*, an unknown code none.
#[renox::test]
async fn proposal_follows_customer_specific_flag() {
    let t = boot().await;
    let code = format!("I{PANEL}");
    assert_eq!(
        proposed(&t, &code).await,
        Some(LineOutcomeKind::FinishToStock)
    );
    ItemFlag::create(
        t.db(),
        ItemFlag {
            item_id: PANEL,
            customer_specific: true,
            ..ItemFlag::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(proposed(&t, &code).await, Some(LineOutcomeKind::Scrap));
    assert_eq!(proposed(&t, "nothing").await, None);
}
