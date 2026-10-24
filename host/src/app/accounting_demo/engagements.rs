//! The engagements of the firm, each through the onboarding of 04 §4.1 as the
//! people do it: the manager proposes it, the accountant records the tax
//! profile and prepares the CDD file, the MLRO approves it, the letter is
//! signed (in the portal by Marina Bay's owner, on paper for the others) and
//! the manager activates it, which lays out the compliance calendar and
//! starts the retainer. Then the ones that went another way: a CDD file still
//! waiting for the MLRO, a decline, a suspension and an end.

use accounting::{CddFile, Engagement};
use renox::prelude::*;

use super::{
    ACCOUNTANT_EMAIL, CLIENT_OWNER_EMAIL, MANAGER_EMAIL, MLRO_EMAIL, PARTNER_EMAIL, Seed, day,
};

/// One engagement: client key, services, fee basis, monthly retainer (AED),
/// VAT period, risk.
type EngagementRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

const ENGAGEMENTS: &[EngagementRow] = &[
    (
        "marina",
        "Bookkeeping, VAT and corporate tax",
        "retainer",
        "6500",
        "quarterly",
        "low",
    ),
    (
        "rose",
        "Bookkeeping and VAT",
        "retainer",
        "4200",
        "quarterly",
        "medium",
    ),
    (
        "oasis",
        "VAT returns and fractional CFO",
        "retainer",
        "8750",
        "monthly",
        "low",
    ),
    (
        "waha",
        "Bookkeeping, VAT and AML advisory",
        "retainer",
        "5900",
        "quarterly",
        "high",
    ),
    (
        "palm",
        "Bookkeeping and VAT",
        "retainer",
        "3300",
        "quarterly",
        "low",
    ),
    (
        "dune",
        "Bookkeeping and VAT",
        "retainer",
        "4800",
        "quarterly",
        "medium",
    ),
    ("coral", "VAT returns", "fixed", "2500", "quarterly", "low"),
    (
        "pearl",
        "Corporate tax registration and return",
        "fixed",
        "7500",
        "quarterly",
        "medium",
    ),
    (
        "horizon",
        "Bookkeeping, VAT and KPI reporting",
        "retainer",
        "7200",
        "quarterly",
        "low",
    ),
];

impl Seed {
    /// The engagement of client `key` (once made).
    pub(crate) fn engagement(&self, key: &str) -> i64 {
        self.engagements[key]
    }

    async fn propose(&mut self, key: &'static str, title: &str, basis: &str, fee: &str) -> Result {
        let client = self.clients[key].company;
        self.driver
            .post(
                MANAGER_EMAIL,
                "/accounting/engagements",
                &[
                    ("client", &client.0.to_string()),
                    ("title", title),
                    ("fee_basis", basis),
                    ("retainer_fee", fee),
                ],
            )
            .await?;
        let id = Engagement::of_client(self.db(), client)
            .await?
            .last()
            .map(|e| e.id)
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the engagement of {key}"))?;
        self.engagements.insert(key, id);
        Ok(())
    }

    async fn tax_profile(&self, key: &str, period: &str) -> Result {
        let client = self.clients[key].company;
        // The first VAT period ends at the end of the quarter (or month)
        // before last, so the calendar holds a return due soon.
        let first_end = if period == "monthly" {
            month_end(day(-75))
        } else {
            month_end(day(-200))
        };
        self.driver
            .post(
                ACCOUNTANT_EMAIL,
                &format!("/accounting/clients/{}/tax-profile", client.0),
                &[
                    ("trn", &format!("1000000000{:05}", 10_000 + client.0)),
                    ("vat_registered", "1"),
                    ("vat_period", period),
                    ("vat_first_period_end", &first_end.to_string()),
                    (
                        "financial_year_end",
                        &format!("{}-12-31", day(0).format("%Y")),
                    ),
                    ("ct_registration_no", &format!("CT-DEMO-{:06}", client.0)),
                    ("effective_date", &day(-240).to_string()),
                    ("note", "From the FTA certificate"),
                ],
            )
            .await?;
        Ok(())
    }

    /// The CDD file of client `key`, prepared and submitted by the
    /// accountant; approved by the MLRO when `approve`.
    async fn cdd(&self, key: &str, risk: &str, approve: bool) -> Result<i64> {
        let client = self.clients[key];
        let engagement = self.engagement(key);
        self.driver
            .post(
                ACCOUNTANT_EMAIL,
                &format!("/accounting/engagements/{engagement}/cdd"),
                &[],
            )
            .await?;
        let file = CddFile::current(self.db(), client.company)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the CDD file of {key}"))?;
        for (party, role) in [(client.ubo, "ubo"), (client.director, "director")] {
            self.driver
                .post(
                    ACCOUNTANT_EMAIL,
                    &format!("/accounting/cdd/{}/parties", file.id),
                    &[("party", &party.0.to_string()), ("role", role)],
                )
                .await?;
        }
        let (factors, edd) = match risk {
            "high" => (
                "Free zone trading company; cash-intensive re-export trade; UBO resident abroad",
                "Source of wealth evidenced by audited accounts 2024-2025 and bank reference; senior management approval obtained",
            ),
            "medium" => ("Cash sales above AED 55,000 a month", ""),
            _ => ("Mainland company, local bank account, known UBO", ""),
        };
        self.driver
            .post(
                ACCOUNTANT_EMAIL,
                &format!("/accounting/cdd/{}/assessment", file.id),
                &[
                    ("rating", risk),
                    ("risk_factors", factors),
                    ("edd_notes", edd),
                    ("documents", "on"),
                ],
            )
            .await?;
        self.driver
            .post(
                ACCOUNTANT_EMAIL,
                &format!("/accounting/cdd/{}/submit", file.id),
                &[],
            )
            .await?;
        if approve {
            self.driver
                .post(
                    MLRO_EMAIL,
                    &format!("/accounting/cdd/{}/approve", file.id),
                    &[],
                )
                .await?;
        }
        Ok(file.id)
    }

    /// The letter sent by the manager and signed: in the portal by Marina
    /// Bay's owner, on paper (recorded by the manager) for the others.
    async fn letter(&self, key: &str) -> Result {
        let engagement = self.engagement(key);
        self.driver
            .post(
                MANAGER_EMAIL,
                &format!("/accounting/engagements/{engagement}/letter/send"),
                &[],
            )
            .await?;
        if key == "marina" {
            self.driver
                .post(
                    CLIENT_OWNER_EMAIL,
                    &format!("/accounting/portal/engagements/{engagement}/letter/sign"),
                    &[],
                )
                .await?;
        } else {
            self.driver
                .post(
                    MANAGER_EMAIL,
                    &format!("/accounting/engagements/{engagement}/letter"),
                    &[("signed_on", &day(0).to_string())],
                )
                .await?;
        }
        Ok(())
    }

    async fn act(&self, who: &str, key: &str, action: &str, form: &[(&str, &str)]) -> Result {
        let engagement = self.engagement(key);
        self.driver
            .post(
                who,
                &format!("/accounting/engagements/{engagement}/{action}"),
                form,
            )
            .await?;
        Ok(())
    }

    /// The engagements.
    pub(crate) async fn engagements(&mut self) -> Result {
        for (key, title, basis, fee, period, risk) in ENGAGEMENTS {
            self.propose(key, title, basis, fee).await?;
            self.tax_profile(key, period).await?;
            match *key {
                // Onboarding: the CDD file waits for the MLRO, the letter is
                // out for signature.
                "palm" => {
                    self.cdd(key, risk, false).await?;
                    let engagement = self.engagement(key);
                    self.driver
                        .post(
                            MANAGER_EMAIL,
                            &format!("/accounting/engagements/{engagement}/letter/send"),
                            &[],
                        )
                        .await?;
                }
                // Declined: the client did not sign the letter.
                "pearl" => {
                    self.cdd(key, risk, true).await?;
                    self.act(
                        MANAGER_EMAIL,
                        key,
                        "decline",
                        &[("reason", "letter_not_signed")],
                    )
                    .await?;
                }
                _ => {
                    self.cdd(key, risk, true).await?;
                    self.letter(key).await?;
                    self.act(MANAGER_EMAIL, key, "activate", &[]).await?;
                }
            }
        }
        // Sand Dune stopped paying: suspended.
        self.act(
            MANAGER_EMAIL,
            "dune",
            "suspend",
            &[("reason", "non_payment")],
        )
        .await?;
        // Coral Reef sold its restaurants: the partner confirms there is
        // nothing left to bill and ends the engagement.
        self.act(PARTNER_EMAIL, "coral", "nothing-to-bill", &[])
            .await?;
        self.act(
            PARTNER_EMAIL,
            "coral",
            "end",
            &[("reason", "client_request")],
        )
        .await?;
        Ok(())
    }
}

/// The last day of the month of `date`.
pub(crate) fn month_end(date: renox::chrono::NaiveDate) -> renox::chrono::NaiveDate {
    use renox::chrono::Datelike;
    let (y, m) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    renox::chrono::NaiveDate::from_ymd_opt(y, m, 1)
        .and_then(|d| d.pred_opt())
        .unwrap_or(date)
}
