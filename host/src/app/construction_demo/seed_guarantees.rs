//! The bank guarantees: registered and issued by the accountant. The advance
//! guarantees and performance bonds of the projects in progress, a
//! performance bond that expires in 25 days (the extension task is due), a
//! bond the client called in part and the bank paid, the performance bond of
//! the completed project released, and a retention bond linked to the final
//! retention part.

use construction::Guarantee;
use construction::Retention;
use construction::retention::RetentionPart;
use renox::prelude::*;

use super::at;
use super::people::email_of;
use super::roles::{ACCOUNTANT, PROJECT_MANAGER};
use super::seed_ipas::after;
use super::seed_projects::plan_of;
use super::{Seed, day, text};

/// What happens to a guarantee after it is issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Then {
    /// It stays active.
    Stay,
    /// The bank is asked to extend it to this day (days from today), this many
    /// days after it was issued.
    Extend(i64, i64),
    /// The beneficiary releases it this many days after it was issued.
    Release(i64),
    /// The client calls it for this many dirhams, this many days after it was
    /// issued; the bank pays that amount three days later.
    Call(i64, i64),
}

/// A guarantee of the demo.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    /// The project.
    pub code: &'static str,
    /// `advance`, `performance` or `retention_bond`.
    pub kind: &'static str,
    /// The bank (fictional).
    pub bank: &'static str,
    /// The amount, in dirhams.
    pub amount: i64,
    /// The day it is issued, days from today.
    pub issued: i64,
    /// The day it expires, days from today.
    pub expires: i64,
    /// What happens after.
    pub then: Then,
}

/// The guarantees, in the order they are registered.
pub const PLANS: &[Plan] = &[
    Plan {
        code: "OFC-06",
        kind: "performance",
        bank: "Gulf Demo Bank",
        amount: 330_000,
        issued: -330,
        expires: -100,
        then: Then::Release(210),
    },
    Plan {
        code: "OFC-06",
        kind: "retention_bond",
        bank: "Gulf Demo Bank",
        amount: 165_000,
        issued: -120,
        expires: 250,
        then: Then::Stay,
    },
    Plan {
        code: "TWR-01",
        kind: "advance",
        bank: "Emirates Demo Bank",
        amount: 640_000,
        issued: -146,
        expires: 190,
        then: Then::Stay,
    },
    // Expires in 25 days: the accountant has the extension task.
    Plan {
        code: "TWR-01",
        kind: "performance",
        bank: "Emirates Demo Bank",
        amount: 640_000,
        issued: -146,
        expires: 25,
        then: Then::Stay,
    },
    Plan {
        code: "MALL-02",
        kind: "advance",
        bank: "Creek Demo Bank",
        amount: 285_000,
        issued: -132,
        expires: 210,
        then: Then::Stay,
    },
    Plan {
        code: "MALL-02",
        kind: "performance",
        bank: "Creek Demo Bank",
        amount: 285_000,
        issued: -132,
        expires: 120,
        then: Then::Extend(300, 60),
    },
    // The client called the advance guarantee for the delay and the bank paid.
    Plan {
        code: "CLN-03",
        kind: "advance",
        bank: "Gulf Demo Bank",
        amount: 410_000,
        issued: -108,
        expires: 170,
        then: Then::Call(50_000, 70),
    },
    Plan {
        code: "RST-04",
        kind: "performance",
        bank: "Creek Demo Bank",
        amount: 190_000,
        issued: -72,
        expires: 150,
        then: Then::Stay,
    },
];

impl Seed {
    /// All the guarantees of the demo.
    pub(crate) async fn guarantees(&mut self) -> Result {
        for plan in PLANS {
            self.guarantee(plan).await?;
        }
        // The retention bond of the completed project replaces its final
        // retention part: it is linked, and the part is made due by it.
        self.link_retention_bond("OFC-06").await?;
        Ok(())
    }

    async fn guarantee(&mut self, plan: &Plan) -> Result {
        let accountant = email_of(ACCOUNTANT);
        let project = self.project(plan.code).await?;
        let client = self.party(plan_of(plan.code).client).legal_name_en.clone();
        let amount = (plan.amount * 100).to_string();
        let expires = text(day(plan.expires));
        let issued = day(plan.issued);
        at!(self, issued, {
            self.driver
                .post(
                    accountant,
                    &format!("/construction/projects/{}/guarantees", project.id),
                    &[
                        ("kind", plan.kind),
                        ("bank", plan.bank),
                        ("beneficiary", client.as_str()),
                        ("amount", amount.as_str()),
                        ("expires_on", expires.as_str()),
                    ],
                )
                .await
        })?;
        let made = Guarantee::query()
            .where_(Guarantee::PROJECT_ID.eq(project.id))
            .order_by_desc(Guarantee::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no guarantee"))?;
        let document = format!("{}-{}-{}.pdf", plan.code, plan.kind, made.id);
        at!(self, after(issued, 2), {
            self.driver
                .post(
                    accountant,
                    &format!("/construction/guarantees/{}/issue", made.id),
                    &[("document", document.as_str())],
                )
                .await
        })?;
        self.report.add("guarantees", 1);
        match plan.then {
            Then::Stay => {}
            Then::Extend(to, days) => {
                let to = text(day(to));
                at!(self, after(issued, days), {
                    self.driver
                        .post(
                            accountant,
                            &format!("/construction/guarantees/{}/extend", made.id),
                            &[
                                ("expires_on", to.as_str()),
                                ("document", "BANK-AMENDMENT-1.pdf"),
                            ],
                        )
                        .await
                })?;
            }
            Then::Release(days) => {
                at!(self, after(issued, days), {
                    self.driver
                        .post(
                            accountant,
                            &format!("/construction/guarantees/{}/release", made.id),
                            &[("document", "BANK-CANCELLATION-LETTER.pdf")],
                        )
                        .await
                })?;
            }
            Then::Call(called, days) => {
                let called = (called * 100).to_string();
                let on = after(issued, days);
                at!(self, on, {
                    self.driver
                        .post(
                            accountant,
                            &format!("/construction/guarantees/{}/call", made.id),
                            &[
                                ("amount", called.as_str()),
                                ("document", "DEMAND-NOTICE-CLN-03.pdf"),
                            ],
                        )
                        .await
                })?;
                let call = construction::GuaranteeCall::query()
                    .where_(construction::GuaranteeCall::GUARANTEE_ID.eq(made.id))
                    .first(self.db())
                    .await?
                    .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no call"))?;
                at!(self, after(on, 3), {
                    self.driver
                        .post(
                            accountant,
                            &format!("/construction/guarantees/{}/settle-call", made.id),
                            &[
                                ("amount", called.as_str()),
                                ("document", "BANK-DEBIT-ADVICE-CLN-03.pdf"),
                            ],
                        )
                        .await
                })?;
                // The project manager disputes the paid call: it is a claim in
                // the final account.
                at!(self, after(on, 8), {
                    self.driver
                        .post(
                            email_of(PROJECT_MANAGER),
                            &format!("/construction/guarantee-calls/{}/dispute", call.id),
                            &[],
                        )
                        .await
                })?;
            }
        }
        Ok(())
    }

    /// Links the active retention bond of project `code` to its final
    /// retention part (the accountant).
    async fn link_retention_bond(&mut self, code: &str) -> Result {
        let project = self.project(code).await?;
        let bond = Guarantee::query()
            .where_(Guarantee::PROJECT_ID.eq(project.id))
            .where_(Guarantee::KIND.eq(construction::GuaranteeKind::RetentionBond))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no retention bond"))?;
        let part = Retention::query()
            .where_(Retention::PROJECT_ID.eq(project.id))
            .where_(Retention::PART.eq(RetentionPart::Final))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no final retention part"))?;
        let bond_id = bond.id.to_string();
        at!(self, day(-110), {
            self.driver
                .post(
                    email_of(ACCOUNTANT),
                    &format!("/construction/retentions/{}/bond", part.id),
                    &[("guarantee_id", bond_id.as_str())],
                )
                .await
        })?;
        Ok(())
    }
}
