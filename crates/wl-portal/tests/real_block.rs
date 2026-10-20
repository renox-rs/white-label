//! BL-POR-01 (#44) through the real `wl-parties` block guard (not the fake of
//! `support::boot`): a party blocked for `sanctions` and one blocked for
//! `non_payment` get the very same neutral text for every refused action; the
//! lease, the invoices, the statements and the requests stay visible with the
//! status their module stored (a request submitted before the block still
//! reads *In review*); a signed payment link keeps working and a signed link
//! whose action is new business shows the neutral text and creates nothing.

mod support;

use std::time::Duration;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_parties::{Party, block_by_system};
use wl_portal::{
    LINK_USER, PortalAction, PortalSection, RefusalLog, SectionFuture, SectionRow, SignedLinkUsed,
    SignedLinks, register_section,
};

use support::{Booted, boot_real};

const NEUTRAL_EN: &str = "This request cannot be completed online. Please contact us.";
const NEUTRAL_AR: &str = "هذا الطلب لا يمكن إتمامه عبر الإنترنت. يرجى التواصل معنا.";

/// What a block must never show on a portal page or in a refusal.
const BLOCK_WORDS: [&str; 7] = [
    "On hold",
    "on_hold",
    "Sanctions",
    "sanctions",
    "non_payment",
    "Non-payment",
    "Blocked",
];

/// A section with fixed rows, as a module would register.
struct Rows(&'static str, Vec<SectionRow>);

impl PortalSection for Rows {
    fn title(&self) -> String {
        self.0.to_owned()
    }

    fn rows<'a>(
        &'a self,
        _state: &'a AppState,
        _party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>> {
        let rows = self.1.clone();
        Box::pin(async move { Ok(rows) })
    }
}

fn register_sections() {
    register_section(
        "lease",
        Rows(
            "Lease",
            vec![SectionRow::new("Lease L-204, Tower A", "Active")],
        ),
    );
    register_section(
        "invoices",
        Rows(
            "Invoices",
            vec![
                SectionRow::new("INV-2026-0031", "Overdue"),
                SectionRow::new("INV-2026-0012", "Paid"),
            ],
        ),
    );
    register_section(
        "statements",
        Rows(
            "Statements",
            vec![SectionRow::new("Statement October 2026", "Issued")],
        ),
    );
    register_section(
        "requests",
        Rows(
            "Requests",
            vec![SectionRow::new("Refund request RR-12", "In review")],
        ),
    );
}

async fn party(t: &Booted, name: &str) -> Party {
    Party::query()
        .where_(Party::LEGAL_NAME_EN.eq(name))
        .first(t.db())
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no party `{name}`"))
}

/// Blocks `name` with the reason `code` through the real block.
async fn block(t: &Booted, name: &str, code: &str) -> Party {
    let party = party(t, name).await;
    block_by_system(t.state(), &party, code, "reviewed", None)
        .await
        .expect("the party is blocked");
    party
}

fn assert_no_block_word(what: &str, text: &str) {
    for word in BLOCK_WORDS {
        assert!(!text.contains(word), "{what} shows `{word}`: {text}");
    }
}

/// AC3: a sanctions-blocked tenant and a non-payment-blocked tenant get the
/// same status and the same text for a new booking and a deposit release, in
/// English and in Arabic; nothing names the block or its reason; paying an
/// invoice works for both.
#[renox::test]
async fn por01_a_sanctions_block_and_a_non_payment_block_are_refused_with_the_same_text() {
    let t = boot_real().await;
    let sanctions = block(&t, "Khalid Saeed Al Mansoori", "sanctions").await;
    let unpaid = block(&t, "Priya Raghunathan", "non_payment").await;
    let (khalid, _) = t.portal_user("Khalid", 1, sanctions.id).await;
    let (priya, _) = t.portal_user("Priya", 2, unpaid.id).await;

    for key in ["new_booking", "deposit_release_request"] {
        for (lang, expected) in [("en", NEUTRAL_EN), ("ar", NEUTRAL_AR)] {
            let a = t.act(&khalid, key, sanctions.id, lang).await;
            let b = t.act(&priya, key, unpaid.id, lang).await;
            a.assert_status(422);
            b.assert_status(422);
            assert!(a.text().contains(expected), "{key}/{lang}: {}", a.text());
            assert_eq!(
                a.text(),
                b.text(),
                "{key}/{lang}: the reason changes the text"
            );
            assert_no_block_word(key, &a.text());
        }
    }
    for (user, party) in [(&khalid, sanctions.id), (&priya, unpaid.id)] {
        t.act(user, "pay_invoice", party, "en").await.assert_ok();
        t.act(user, "maintenance_request", party, "en")
            .await
            .assert_ok();
    }
    assert_eq!(RefusalLog::latest(t.db(), 100).await.unwrap().len(), 8);
    assert_eq!(t.done.all().len(), 4);
}

/// AC1 and AC8: while blocked the tenant still opens the lease, the invoices,
/// the statements and the requests; a refund request submitted before the
/// block still reads *In review*; no page shows a block status, badge or
/// reason. Nothing is logged by reading.
#[renox::test]
async fn por01_a_blocked_tenant_still_sees_the_lease_the_invoices_the_statements_and_the_requests()
{
    register_sections();
    let t = boot_real().await;
    let sanctions = block(&t, "Khalid Saeed Al Mansoori", "sanctions").await;
    let (khalid, _) = t.portal_user("Khalid", 1, sanctions.id).await;
    t.app.acting_as(&khalid);

    for (key, expected) in [
        ("lease", "Lease L-204, Tower A"),
        ("invoices", "INV-2026-0031"),
        ("statements", "Statement October 2026"),
        ("requests", "In review"),
    ] {
        let page = t.app.get(&format!("/portal/{}/{key}", sanctions.id)).await;
        page.assert_ok();
        assert!(page.text().contains(expected), "{key}: {}", page.text());
        assert_no_block_word(key, &page.text());
    }
    assert!(RefusalLog::latest(t.db(), 10).await.unwrap().is_empty());

    // Another party's section is not theirs.
    let other = party(&t, "Priya Raghunathan").await;
    t.app
        .get(&format!("/portal/{}/requests", other.id))
        .await
        .assert_forbidden();
    // An unknown section is not found.
    t.app
        .get(&format!("/portal/{}/unknown", sanctions.id))
        .await
        .assert_not_found();
}

/// AC9: a signed payment link keeps working while the party is blocked; a
/// signed link whose action is new business shows the neutral text, creates
/// nothing and is logged for compliance; an altered link is refused.
#[renox::test]
async fn por01_a_signed_payment_link_works_and_a_new_business_link_shows_the_neutral_text() {
    let t = boot_real().await;
    t.app.fake_events();
    let sanctions = block(&t, "Khalid Saeed Al Mansoori", "sanctions").await;
    let party = PartyId(sanctions.id);
    let ttl = Duration::from_secs(3600);

    let pay = SignedLinks::issue(t.state(), party, PortalAction::PayInvoice, ttl).unwrap();
    let path = pay.trim_start_matches(&t.state().config.url).to_owned();
    t.app.get(&path).await.assert_ok();
    t.app
        .assert_emitted::<SignedLinkUsed>(|e| e.party == party && e.action == "pay_invoice");
    assert!(RefusalLog::latest(t.db(), 10).await.unwrap().is_empty());

    for action in [
        PortalAction::NewOrder,
        PortalAction::SignAgreement,
        PortalAction::RefundRequest,
    ] {
        let link = SignedLinks::issue(t.state(), party, action, ttl).unwrap();
        let path = link.trim_start_matches(&t.state().config.url).to_owned();
        let page = t.app.get(&path).await;
        page.assert_status(422);
        assert!(page.text().contains(NEUTRAL_EN), "{}", page.text());
        assert_no_block_word(action.key(), &page.text());
    }
    assert_eq!(
        t.app.emitted::<SignedLinkUsed>().len(),
        1,
        "only the payment link was used"
    );
    assert!(t.done.all().is_empty(), "nothing was created");
    let log = RefusalLog::latest(t.db(), 10).await.unwrap();
    assert_eq!(log.len(), 3);
    assert!(
        log.iter()
            .all(|e| e.user_id == LINK_USER && e.party_id == sanctions.id)
    );

    // Altered or unsigned: refused before the guard.
    t.app
        .get(&path.replace(&sanctions.id.to_string(), "9999"))
        .await
        .assert_forbidden();
    t.app
        .get(&format!("/portal/link/{}/pay_invoice", sanctions.id))
        .await
        .assert_forbidden();
}
