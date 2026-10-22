//! A claim of an insured visit billed through the routes, and the routes of
//! the billing coordinator that drive it, for the claim tests.

#![allow(dead_code)]

use renox::chrono::NaiveDate;
use renox::db::ToDbValue;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_kernel::payer_bills::PayerBillStatus;

use clinics::{Claim, ClaimLine, ClaimStatus};

use super::billing::Bill;
use super::desk::{Desk, PAYER, desk};

/// The clinic's day with one claim: a consultation of AED 450.00 under a
/// payer contract with a copay of 20%, so the claim is AED 360.00.
pub struct Billed {
    /// The clinic's day.
    pub d: Desk,
    /// The billing coordinator.
    pub bea: User,
    /// The draft claim.
    pub claim: Claim,
}

/// Bills an insured visit through the routes and returns its claim.
pub async fn billed() -> Billed {
    let d = desk().await;
    d.contract(PAYER, 2_000, 11).await;
    let policy = d.policy("Gold", PAYER).await;
    let booking = d.insured(&d.consultation, 9, &policy).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    d.deliver(&visit, "consultation", 1).await.assert_ok();
    d.complete(&booking).await;
    let visit = d.visit(visit.id).await;
    let claim = Claim::of_visit(d.t.db(), visit.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("billing made the claim");
    let bea = d.t.coordinator().await;
    Billed { d, bea, claim }
}

impl Billed {
    /// The database.
    pub fn db(&self) -> &Db {
        self.d.t.db()
    }

    /// The claim, read again.
    pub async fn claim(&self) -> Claim {
        Claim::find_or_404(self.db(), self.claim.id).await.unwrap()
    }

    /// The lines of the claim.
    pub async fn lines(&self) -> Vec<ClaimLine> {
        ClaimLine::of_claim(self.db(), self.claim.id).await.unwrap()
    }

    /// The payer bill of the claim, as the stand-in keeps it.
    pub fn bill(&self) -> Bill {
        self.d.t.claims.bill(self.claim.payer_bill_id)
    }

    /// `/clinics/claims/{id}/{action}`.
    pub fn path(&self, action: &str) -> String {
        format!("/clinics/claims/{}/{action}", self.claim.id)
    }

    /// Posts `fields` to `action` of the claim as the coordinator.
    pub async fn act(&self, action: &str, fields: &[(&str, &str)]) -> TestResponse {
        self.d.post(&self.bea, &self.path(action), fields).await
    }

    /// Validates the claim through the route.
    pub async fn validate(&self) {
        self.act("validate", &[]).await.assert_ok();
    }

    /// Submits the claim through the route with `reference`.
    pub async fn submit(&self, reference: &str) {
        self.act("submit", &[("reference", reference)])
            .await
            .assert_ok();
    }

    /// Validates and submits the claim.
    pub async fn send(&self) {
        self.validate().await;
        self.submit("CH-1").await;
    }

    /// Sets columns of the claim other than the status.
    pub async fn set(&self, columns: &[(&str, &(dyn ToDbValue + Sync))]) {
        Claim::query()
            .where_(Claim::ID.eq(self.claim.id))
            .update(self.db(), columns)
            .await
            .unwrap();
    }

    /// Puts the claim and its payer bill where a remittance dated `on` left
    /// them, as the remittance itself leaves them: `paid` or `denied` (all
    /// lines), or `partially_paid` (half of every line paid).
    pub async fn answered(&self, status: ClaimStatus, on: NaiveDate) {
        let claimed = self.claim.claimed;
        let (paid, denied, bill_status) = match status {
            ClaimStatus::Paid => (claimed, 0, PayerBillStatus::Paid),
            ClaimStatus::PartiallyPaid => (
                claimed / 2,
                claimed - claimed / 2,
                PayerBillStatus::PartiallyPaid,
            ),
            _ => (0, claimed, PayerBillStatus::Denied),
        };
        let shown = status.as_str().to_owned();
        self.set(&[
            ("status", &shown),
            ("has_remittance", &true),
            ("last_remittance_on", &Some(on)),
            ("last_remittance_reference", &Some("ADV-0".to_owned())),
            ("paid", &paid),
            ("denied", &denied),
        ])
        .await;
        for line in self.lines().await {
            let (line_paid, line_denied) = match status {
                ClaimStatus::Paid => (line.claimed, 0),
                ClaimStatus::PartiallyPaid => (line.claimed / 2, line.claimed - line.claimed / 2),
                _ => (0, line.claimed),
            };
            ClaimLine::query()
                .where_(ClaimLine::ID.eq(line.id))
                .update(
                    self.db(),
                    &[
                        ("paid", &line_paid),
                        ("denied", &line_denied),
                        ("denial_code", &Some("R12".to_owned())),
                    ],
                )
                .await
                .unwrap();
        }
        self.d.t.claims.force(self.claim.payer_bill_id, |bill| {
            bill.status = bill_status;
            bill.has_remittance = true;
            bill.last_remittance_on = Some(on);
            for line in &mut bill.lines {
                match status {
                    ClaimStatus::Paid => line.paid = line.claimed,
                    ClaimStatus::PartiallyPaid => {
                        line.paid = line.claimed / 2;
                        line.denied = line.claimed - line.claimed / 2;
                    }
                    _ => line.denied = line.claimed,
                }
            }
        });
    }
}
