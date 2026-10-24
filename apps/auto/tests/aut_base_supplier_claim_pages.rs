//! The supplier warranty claim pages (base for #407): the list with its tabs
//! and search, the form that raises a claim from a warranty ticket (job card
//! picked from a list, ticket by its number, supplier from the parties), the
//! claim with its actions while it is raised.

mod support;

use auto::supplier_claims::{self, NewSupplierClaim};
use auto::{JobCardStatus, SupplierClaim, SupplierClaimStatus, permissions};
use renox::prelude::*;
use support::{boot, day};
use wl_kernel::Subject;

async fn sign_in(t: &support::Garage, name: &str, grants: &[&str]) -> User {
    let user = t.user(name, grants).await;
    t.app.acting_as(&user);
    user
}

const PURCHASER: [&str; 2] = [
    permissions::SUPPLIER_CLAIMS_MANAGE,
    permissions::JOB_CARDS_VIEW,
];

/// The job card of the warranty ticket `TKT-2026-0012` (closed), and a claim
/// raised from it.
async fn raised(t: &support::Garage) -> (SupplierClaim, Subject) {
    t.people.add(40, "Gulf Auto Spares");
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::Delivered).await;
    let about = card.subject();
    t.tickets
        .add(&about, "warranty", 12, "TKT-2026-0012", "closed");
    let pia = t.user("Pia", &PURCHASER).await;
    let pia = t.auth(&pia).await;
    let claim = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            12,
            about.clone(),
            40,
            "Alternator",
            45_000,
            day(-170),
            12,
            day(-10),
        ),
    )
    .await
    .unwrap();
    (claim, about)
}

/// The list shows the claim with its ticket, supplier, part and status in
/// words; the tabs and the search narrow it; only the purchaser sees it.
#[renox::test]
async fn aut_base_the_supplier_claim_list_shows_filters_and_searches() {
    let t = boot().await;
    let (claim, _about) = raised(&t).await;
    sign_in(&t, "Pia", &PURCHASER).await;

    t.app
        .get("/auto/supplier-claims")
        .await
        .assert_ok()
        .assert_see(&claim.number())
        .assert_see("TKT-2026-0012")
        .assert_see("Gulf Auto Spares")
        .assert_see("Alternator")
        .assert_see("AED 450.00")
        .assert_see("Raised");
    let raised_tab = t.app.get("/auto/supplier-claims?in.status=raised").await;
    raised_tab.assert_ok().assert_see(&claim.number());
    let ended = t
        .app
        .get("/auto/supplier-claims?in.status=recovered&in.status=written_off")
        .await;
    assert!(!ended.text().contains(&claim.number()));
    t.app
        .get("/auto/supplier-claims?search=alternator")
        .await
        .assert_ok()
        .assert_see(&claim.number());
    let none = t.app.get("/auto/supplier-claims?search=nothing-here").await;
    assert!(!none.text().contains(&claim.number()));

    sign_in(&t, "Nadia", &[permissions::VEHICLES_VIEW]).await;
    t.app.get("/auto/supplier-claims").await.assert_status(403);
    t.app
        .get(&format!("/auto/supplier-claims/{}", claim.id))
        .await
        .assert_status(403);
}

/// The form raises a claim: the job card from the picker, the ticket by its
/// number, the supplier by its party code; a ticket that is not closed or a
/// failure out of terms is refused in place.
#[renox::test]
async fn aut_base_the_form_raises_a_claim_and_refuses_one_out_of_terms() {
    let t = boot().await;
    t.people.add(40, "Gulf Auto Spares");
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::Delivered).await;
    let about = card.subject();
    t.tickets
        .add(&about, "warranty", 12, "TKT-2026-0012", "closed");
    t.tickets
        .add(&about, "warranty", 13, "TKT-2026-0013", "in_progress");
    sign_in(&t, "Pia", &PURCHASER).await;

    t.app
        .get("/auto/supplier-claim-job-options?q=JOB")
        .await
        .assert_ok()
        .assert_see(&card.number());
    let job = card.id.to_string();
    let purchased = day(-170).to_string();
    let failed = day(-10).to_string();
    let form = |ticket: &str, months: &'static str| -> Vec<(String, String)> {
        vec![
            ("job_card".to_owned(), job.clone()),
            ("ticket".to_owned(), ticket.to_owned()),
            ("accepted".to_owned(), "on".to_owned()),
            ("supplier".to_owned(), "P-00040".to_owned()),
            ("part".to_owned(), "Alternator".to_owned()),
            ("cost".to_owned(), "450".to_owned()),
            ("purchase_date".to_owned(), purchased.clone()),
            ("warranty_months".to_owned(), months.to_owned()),
            ("failure_date".to_owned(), failed.clone()),
            ("kept".to_owned(), "on".to_owned()),
        ]
    };
    let post = |fields: Vec<(String, String)>| {
        let t = &t;
        async move {
            let borrowed: Vec<(&str, &str)> = fields
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            t.app.htmx().post("/auto/supplier-claims", &borrowed).await
        }
    };

    let open = post(form("TKT-2026-0013", "12")).await;
    open.assert_status(422);
    assert!(open.text().contains("not closed"));
    let out_of_terms = post(form("TKT-2026-0012", "3")).await;
    out_of_terms.assert_status(422);
    assert!(out_of_terms.text().contains("after the warranty period"));
    let unknown = post(form("TKT-2026-0099", "12")).await;
    unknown.assert_status(422);

    post(form("tkt-2026-0012", "12")).await.assert_ok();
    let claim = SupplierClaim::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("the claim");
    assert_eq!(
        (claim.status, claim.claim_amount, claim.ticket_id),
        (SupplierClaimStatus::Raised, 45_000, 12)
    );
    assert_eq!(claim.supplier_party_id, 40);
}

/// The claim page shows the part, the terms, what is claimed and the actions
/// while it is raised; the purchaser records the document, the supplier's
/// answer and the recovery; an ended claim offers none.
#[renox::test]
async fn aut_base_the_claim_page_records_the_answer_and_the_recovery() {
    let t = boot().await;
    let (claim, _about) = raised(&t).await;
    sign_in(&t, "Pia", &PURCHASER).await;
    let path = format!("/auto/supplier-claims/{}", claim.id);

    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see(&claim.number())
        .assert_see("TKT-2026-0012")
        .assert_see("Gulf Auto Spares")
        .assert_see("Alternator")
        .assert_see("12 months")
        .assert_see("Sold over the counter")
        .assert_see("Waiting for the supplier")
        .assert_see("Write off");

    t.app
        .htmx()
        .post(
            &format!("{path}/document"),
            &[("document", "claim-pdf-1"), ("reference", "GAS-RMA-77")],
        )
        .await
        .assert_ok();
    let rejected = t
        .app
        .htmx()
        .post(&format!("{path}/answer"), &[("answer", "rejected")])
        .await;
    rejected.assert_status(422);
    t.app
        .htmx()
        .post(
            &format!("{path}/answer"),
            &[("answer", "accepted"), ("note", "Credit in 30 days")],
        )
        .await
        .assert_ok();
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("GAS-RMA-77")
        .assert_see("Accepted")
        .assert_see("claim-pdf-1");

    let none = t
        .app
        .htmx()
        .post(&format!("{path}/recover"), &[("reference", " ")])
        .await;
    none.assert_status(422);
    t.app
        .htmx()
        .post(&format!("{path}/recover"), &[("reference", "CN-2026-31")])
        .await
        .assert_ok();
    let recovered = SupplierClaim::find_or_404(t.db(), claim.id).await.unwrap();
    assert_eq!(recovered.status, SupplierClaimStatus::Recovered);
    let page = t.app.get(&path).await;
    page.assert_ok()
        .assert_see("Recovered")
        .assert_see("CN-2026-31");
    assert!(!page.text().contains("Write off"));
}

/// The purchaser writes a claim off with a reason from the page.
#[renox::test]
async fn aut_base_the_claim_page_writes_off_a_refused_claim() {
    let t = boot().await;
    let (claim, _about) = raised(&t).await;
    sign_in(&t, "Pia", &PURCHASER).await;
    let path = format!("/auto/supplier-claims/{}", claim.id);

    t.app
        .htmx()
        .post(&format!("{path}/write-off"), &[("reason", " ")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(&format!("{path}/write-off"), &[("reason", "Water damage")])
        .await
        .assert_ok();
    // The write-off waits for a branch manager other than the purchaser
    // (BL-AUT-11); on the manager's approval the page shows it written off.
    let basma = t
        .user(
            "Basma",
            &[
                permissions::SUPPLIER_CLAIMS_APPROVE,
                permissions::JOB_CARDS_VIEW,
            ],
        )
        .await;
    let waiting = wl_kernel::approvals::awaiting(t.db(), &basma)
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1);
    wl_kernel::approvals::approve(t.state(), waiting[0].id, &basma, "")
        .await
        .expect("the branch manager approves");
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Written off")
        .assert_see("Water damage");
    sign_in(&t, "Sami", &[permissions::JOB_CARDS_ADVISE]).await;
    t.app
        .htmx()
        .post(&format!("{path}/write-off"), &[("reason", "x")])
        .await
        .assert_status(403);
}
