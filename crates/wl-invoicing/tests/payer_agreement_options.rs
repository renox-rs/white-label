//! The picker of payer agreements: by the agreement's or the payer's name as
//! the person types, by the value a form holds, and only for those who see
//! payer bills.

mod support;

use wl_invoicing::VatRecipient;
use wl_invoicing::permissions::PAYER_BILLS_VIEW;

use support::{BLUE_DUNES, boot};

#[renox::test]
async fn the_agreements_are_offered_by_name_to_those_who_see_payer_bills() {
    let t = boot().await;
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Customer).await;
    let viewer = t.user("Vera", &[PAYER_BILLS_VIEW]).await;
    let nobody = t.user("Nora", &[]).await;
    let name = agreement.name.clone();

    t.app.acting_as(&viewer);
    t.app
        .get("/payer-agreements/options")
        .await
        .assert_ok()
        .assert_see(&name);
    t.app
        .get(&format!(
            "/payer-agreements/options?values={}",
            agreement.id
        ))
        .await
        .assert_ok()
        .assert_see(&name);
    t.app
        .get("/payer-agreements/options?q=zzz")
        .await
        .assert_ok()
        .assert_dont_see(&name);
    t.app.acting_as(&nobody);
    t.app
        .get("/payer-agreements/options")
        .await
        .assert_forbidden();
}
