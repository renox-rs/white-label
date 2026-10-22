//! The shop floor of the loyalty tests: the users, the programme, a member,
//! and counter sales and orders rung up through the pages.

#![allow(dead_code)]

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_sales::permissions as perm;
use wl_sales::{CounterSale, LoyaltyAccount, LoyaltyProgramme, PointsKind, PointsMovement};

use super::Booted;

/// The two users of the floor.
pub struct Floor {
    /// A cashier: sells, enrols, attaches members, handles returns and
    /// orders.
    pub cora: User,
    /// A manager of the programme.
    pub mia: User,
}

impl Floor {
    /// Creates the users.
    pub async fn new(t: &Booted) -> Floor {
        let cora = t
            .user(
                "Cora",
                &[
                    perm::POS_SELL,
                    perm::LOYALTY_VIEW,
                    perm::LOYALTY_ENROL,
                    perm::LOYALTY_REDEEM,
                    perm::RETURNS_REQUEST,
                    perm::RETURNS_HANDLE,
                    perm::SALES_ORDERS_VIEW,
                    perm::SALES_ORDERS_CREATE,
                    perm::SALES_ORDERS_ACCEPT,
                    perm::SALES_ORDERS_CANCEL,
                ],
            )
            .await;
        let mia = t
            .user("Mia", &[perm::LOYALTY_VIEW, perm::LOYALTY_MANAGE])
            .await;
        t.app.acting_as(&cora);
        Floor { cora, mia }
    }

    /// Turns the programme on (as the manager) with `extra` fields over a
    /// per-AED default, then goes back to the cashier.
    pub async fn turn_on(&self, t: &Booted, extra: &[(&str, &str)]) -> LoyaltyProgramme {
        let mut fields: Vec<(&str, &str)> = vec![("name", "Friends"), ("terms_en", "Terms")];
        fields.extend_from_slice(extra);
        t.app.acting_as(&self.mia);
        t.app
            .post("/loyalty/programme", &fields)
            .await
            .assert_status(303);
        t.app.acting_as(&self.cora);
        LoyaltyProgramme::first_active(t.db())
            .await
            .unwrap()
            .expect("the programme was created")
    }

    /// Enrols the party `party` with `mobile` at the counter.
    pub async fn enrol(&self, t: &Booted, party: i64, mobile: &str) -> LoyaltyAccount {
        t.app.acting_as(&self.cora);
        t.app
            .post(
                "/loyalty/accounts",
                &[
                    ("party_id", &party.to_string()),
                    ("mobile", mobile),
                    ("terms", "1"),
                ],
            )
            .await
            .assert_status(303);
        LoyaltyAccount::of_party(t.db(), PartyId(party))
            .await
            .unwrap()
            .expect("the member was enrolled")
    }

    /// Gives the member `m` that many points: one earning sale of
    /// `points`.00 AED before VAT (one point per AED).
    pub async fn earn(&self, t: &Booted, m: &LoyaltyAccount, points: i64) {
        let price = format!("{points}.00");
        let fils = points * 105;
        let paid = format!("{}.{:02}", fils / 100, fils % 100);
        self.ring_up(
            t,
            &[("gift", price.as_str(), "1")],
            Some(&m.code),
            &[("cash", paid.as_str())],
        )
        .await;
    }

    /// Opens a counter sale with `lines` (item code, price in AED before VAT,
    /// quantity) and attaches `member` (a code) when given; it stays open.
    pub async fn open_cart(
        &self,
        t: &Booted,
        lines: &[(&str, &str, &str)],
        member: Option<&str>,
    ) -> CounterSale {
        t.app.acting_as(&self.cora);
        t.app
            .post("/counter-sales", &[("x", "")])
            .await
            .assert_status(303);
        let sale = CounterSale::query()
            .order_by_desc(CounterSale::ID)
            .first(t.db())
            .await
            .unwrap()
            .expect("the cart was opened");
        for (code, price, quantity) in lines {
            t.app
                .post(
                    &format!("/counter-sales/{}/lines", sale.id),
                    &[
                        ("item_code", *code),
                        ("unit_price", *price),
                        ("quantity", *quantity),
                    ],
                )
                .await
                .assert_status(303);
        }
        if let Some(code) = member {
            t.app
                .post(
                    &format!("/counter-sales/{}/member", sale.id),
                    &[("code", code)],
                )
                .await
                .assert_status(303);
        }
        CounterSale::find_or_404(t.db(), sale.id).await.unwrap()
    }

    /// Rings up a counter sale of `lines` (item code, price in AED before
    /// VAT, quantity), attaches `member` (a code) when given, and completes
    /// it with `tenders` (method, amount in AED, payment id or empty).
    pub async fn ring_up(
        &self,
        t: &Booted,
        lines: &[(&str, &str, &str)],
        member: Option<&str>,
        tenders: &[(&str, &str)],
    ) -> CounterSale {
        let sale = self.open_cart(t, lines, member).await;
        t.app
            .post(&format!("/counter-sales/{}/complete", sale.id), tenders)
            .await
            .assert_status(303);
        CounterSale::find_or_404(t.db(), sale.id).await.unwrap()
    }
}

/// The account as it is now.
pub async fn account(t: &Booted, account: &LoyaltyAccount) -> LoyaltyAccount {
    LoyaltyAccount::find_or_404(t.db(), account.id)
        .await
        .unwrap()
}

/// The movements of the account of one kind, oldest first.
pub async fn movements(
    t: &Booted,
    account: &LoyaltyAccount,
    kind: PointsKind,
) -> Vec<PointsMovement> {
    PointsMovement::of_account(t.db(), account.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.kind == kind)
        .collect()
}
