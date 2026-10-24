//! The work of the firm on its clients: VAT returns in every state of the
//! machine (the accountant prepares, the tax specialist reviews, the client
//! approves in the portal, the filing is recorded, a filed return is
//! corrected), the AML concerns the MLRO handles, the KPI packs of the
//! fractional CFO service, a tax profile change waiting for the partner, the
//! retainer invoices (two of them overdue) and the re-screening hit.

use accounting::Return;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};

use super::engagements::month_end;
use super::{
    ACCOUNTANT_EMAIL, BILLING_EMAIL, CLIENT_OWNER_EMAIL, MANAGER_EMAIL, MLRO_EMAIL, PARTNER_EMAIL,
    ROSE_OWNER_EMAIL, Seed, TAX_EMAIL, day,
};

/// A trial balance: code, account, debit, credit (AED).
const TRIAL_BALANCE: &str = "code,account,debit,credit
4000,Sales - trading,0.00,1284500.00
4100,Sales - services,0.00,96300.00
5000,Cost of goods sold,812640.00,0.00
6000,Salaries and wages,214800.00,0.00
6100,Office rent,72000.00,0.00
6200,Utilities and telecom,9870.00,0.00
6300,Marketing,18450.00,0.00
7000,Bank charges,2315.00,0.00
1000,Cash at bank,388120.00,0.00
1100,Trade receivables,265400.00,0.00
2000,Trade payables,0.00,198760.00
3000,Share capital,0.00,300000.00
";

/// The categories of the trial balance's accounts.
const MAPPING: &[(&str, &str)] = &[
    ("4000", "revenue"),
    ("4100", "revenue"),
    ("5000", "cost_of_sales"),
    ("6000", "operating_expenses"),
    ("6100", "operating_expenses"),
    ("6200", "operating_expenses"),
    ("6300", "operating_expenses"),
    ("7000", "other_expenses"),
    ("1000", "assets"),
    ("1100", "assets"),
    ("2000", "liabilities"),
    ("3000", "equity"),
];

impl Seed {
    /// A VAT return of client `key` for the period ending `period_end`, due
    /// `due`, opened by the accountant.
    async fn open_return(
        &self,
        key: &str,
        period_end: renox::chrono::NaiveDate,
        due: renox::chrono::NaiveDate,
    ) -> Result<i64> {
        let engagement = self.engagement(key);
        self.driver
            .post(
                ACCOUNTANT_EMAIL,
                "/accounting/returns",
                &[
                    ("engagement", &engagement.to_string()),
                    ("return_type", "vat"),
                    ("period_end", &period_end.to_string()),
                    ("due_date", &due.to_string()),
                ],
            )
            .await?;
        Return::query()
            .where_(Return::ENGAGEMENT_ID.eq(engagement))
            .order_by_desc(Return::ID)
            .first(self.db())
            .await?
            .map(|r| r.id)
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the return of {key}").into())
    }

    async fn on_return(&self, who: &str, id: i64, action: &str, form: &[(&str, &str)]) -> Result {
        self.driver
            .post(who, &format!("/accounting/returns/{id}/{action}"), form)
            .await?;
        Ok(())
    }

    /// The accountant enters the figures (workpapers attached when
    /// `workpapers`) and, when `submit`, hands the return to the reviewer.
    async fn prepare(&self, id: i64, figures: &str, workpapers: bool, submit: bool) -> Result {
        self.on_return(
            ACCOUNTANT_EMAIL,
            id,
            "figures",
            &[
                ("figures", figures),
                ("workpapers", if workpapers { "on" } else { "" }),
            ],
        )
        .await?;
        if submit {
            self.on_return(ACCOUNTANT_EMAIL, id, "submit", &[]).await?;
        }
        Ok(())
    }

    /// The returns.
    async fn returns(&self) -> Result {
        let q3_end = last_quarter_end(day(0));
        let q2_end = quarter_end(month_end(q3_end - renox::chrono::Days::new(95)));

        // Marina Bay: last quarter approved by the owner in the portal and
        // filed by the client's own staff; this quarter waits for the owner.
        let filed = self
            .open_return("marina", q2_end, q2_end + renox::chrono::Days::new(28))
            .await?;
        self.prepare(
            filed,
            "Standard-rated sales AED 1,184,200, output VAT AED 59,210; input VAT AED 38,940; net payable AED 20,270",
            true,
            true,
        )
        .await?;
        self.on_return(TAX_EMAIL, filed, "review", &[]).await?;
        self.driver
            .post(
                CLIENT_OWNER_EMAIL,
                &format!("/accounting/portal/returns/{filed}/approve"),
                &[],
            )
            .await?;
        self.on_return(
            TAX_EMAIL,
            filed,
            "file",
            &[
                ("acknowledgement", "FTA-ACK-DEMO-204518"),
                (
                    "filed_on",
                    &(q2_end + renox::chrono::Days::new(25)).to_string(),
                ),
                ("by_client", "1"),
                ("client_user", "tariq.almarri"),
                ("tax_payable", "20270.00"),
            ],
        )
        .await?;
        let waiting = self.open_return("marina", q3_end, day(4)).await?;
        self.prepare(
            waiting,
            "Standard-rated sales AED 1,262,850, output VAT AED 63,142.50; input VAT AED 41,307.80; net payable AED 21,834.70",
            true,
            true,
        )
        .await?;
        self.on_return(TAX_EMAIL, waiting, "review", &[]).await?;

        // Desert Rose: this quarter with the reviewer; last quarter filed and
        // then corrected by a voluntary disclosure.
        let review = self.open_return("rose", q3_end, day(4)).await?;
        self.prepare(
            review,
            "Standard-rated supplies AED 486,300, output VAT AED 24,315; input VAT AED 17,920; net payable AED 6,395",
            true,
            true,
        )
        .await?;
        let old = self
            .open_return("rose", q2_end, q2_end + renox::chrono::Days::new(28))
            .await?;
        self.prepare(
            old,
            "Output VAT AED 21,480; input VAT AED 15,230; net payable AED 6,250",
            true,
            true,
        )
        .await?;
        self.on_return(TAX_EMAIL, old, "review", &[]).await?;
        self.driver
            .post(
                ROSE_OWNER_EMAIL,
                &format!("/accounting/portal/returns/{old}/approve"),
                &[],
            )
            .await?;
        self.on_return(
            TAX_EMAIL,
            old,
            "file",
            &[
                ("acknowledgement", "FTA-ACK-DEMO-198402"),
                (
                    "filed_on",
                    &(q2_end + renox::chrono::Days::new(26)).to_string(),
                ),
                ("by_client", "1"),
                ("client_user", "layla.haddad"),
                ("tax_payable", "6250.00"),
            ],
        )
        .await?;
        self.on_return(
            ACCOUNTANT_EMAIL,
            old,
            "correct",
            &[
                (
                    "reason",
                    "A supplier credit note of June was missed: input VAT over-claimed",
                ),
                ("error_amount", "12850.00"),
                ("found_on", &day(-6).to_string()),
                ("route", "voluntary_disclosure"),
            ],
        )
        .await?;

        // Oasis Fresh Foods (monthly): due in five days, the workpapers are
        // still missing.
        let missing = self
            .open_return("oasis", month_end(day(-35)), day(5))
            .await?;
        self.prepare(
            missing,
            "Draft: sales ledger imported; purchase invoices for the month still missing",
            false,
            false,
        )
        .await?;

        // Gulf Horizon: the return of the calendar's next period, just opened.
        self.open_return("horizon", month_end(day(7)), day(35))
            .await?;

        // Al Waha: a return cancelled, the client files this one itself.
        let cancelled = self.open_return("waha", q3_end, day(4)).await?;
        self.on_return(
            MANAGER_EMAIL,
            cancelled,
            "cancel",
            &[("reason", "client_files_itself")],
        )
        .await?;
        Ok(())
    }

    /// The AML concerns: one on Al Waha the MLRO is reviewing, one on Oasis
    /// closed with no action; and the re-screening hit on Al Waha's UBO.
    async fn aml(&self) -> Result {
        self.driver
            .post(
                ACCOUNTANT_EMAIL,
                &format!(
                    "/accounting/engagements/{}/aml-concern",
                    self.engagement("waha")
                ),
                &[(
                    "facts",
                    "Three cash deposits of AED 54,000 each in one week, split across two branches, with no matching sales invoices",
                )],
            )
            .await?;
        self.driver
            .post(
                MANAGER_EMAIL,
                &format!(
                    "/accounting/engagements/{}/aml-concern",
                    self.engagement("oasis")
                ),
                &[(
                    "facts",
                    "A supplier paid from a personal account of the UBO; asked to confirm the source of funds",
                )],
            )
            .await?;
        let cases = accounting::AmlCase::query()
            .order_by(accounting::AmlCase::ID)
            .get(self.db())
            .await?;
        if let Some(case) = cases.first() {
            self.driver
                .post(
                    MLRO_EMAIL,
                    &format!("/accounting/aml-cases/{}/review", case.id),
                    &[],
                )
                .await?;
        }
        if let Some(case) = cases.get(1) {
            self.driver
                .post(
                    MLRO_EMAIL,
                    &format!("/accounting/aml-cases/{}/review", case.id),
                    &[],
                )
                .await?;
            self.driver
                .post(
                    MLRO_EMAIL,
                    &format!("/accounting/aml-cases/{}/close", case.id),
                    &[(
                        "reason",
                        "Loan from the shareholder evidenced by a signed agreement and bank statement; no suspicion",
                    )],
                )
                .await?;
        }
        self.rescreening_hit().await
    }

    /// The KPI packs: Marina Bay's last month published with the manager's
    /// commentary, Gulf Horizon's in draft.
    async fn kpi(&self) -> Result {
        for (key, publish) in [("marina", true), ("horizon", false)] {
            let client = self.clients[key].company;
            let period = month_end(day(-40));
            self.driver
                .post(
                    MANAGER_EMAIL,
                    "/accounting/kpi-packs/import",
                    &[
                        ("client", &client.0.to_string()),
                        ("period", &period.to_string()),
                        ("csv", TRIAL_BALANCE),
                    ],
                )
                .await?;
            let pack = accounting::KpiPack::query()
                .where_(accounting::KpiPack::CLIENT_PARTY_ID.eq(client.0))
                .order_by_desc(accounting::KpiPack::ID)
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the KPI pack of {key}"))?;
            for (code, category) in MAPPING {
                self.driver
                    .post(
                        MANAGER_EMAIL,
                        &format!("/accounting/kpi-packs/{}/map", pack.id),
                        &[("code", code), ("category", category)],
                    )
                    .await?;
            }
            if publish {
                self.driver
                    .post(
                        MANAGER_EMAIL,
                        &format!("/accounting/kpi-packs/{}/commentary", pack.id),
                        &[(
                            "commentary",
                            "Gross margin held at 41% despite the summer slowdown; receivables days rose to 71, chase the two largest distributors before year end.",
                        )],
                    )
                    .await?;
                self.driver
                    .post(
                        PARTNER_EMAIL,
                        &format!("/accounting/kpi-packs/{}/publish", pack.id),
                        &[],
                    )
                    .await?;
            }
        }
        Ok(())
    }

    /// Gulf Horizon moves to monthly VAT periods: the accountant asks, the
    /// partner decides.
    async fn tax_profile_change(&self) -> Result {
        let client = self.clients["horizon"].company;
        self.driver
            .post(
                ACCOUNTANT_EMAIL,
                &format!("/accounting/clients/{}/tax-profile/change", client.0),
                &[
                    ("trn", &format!("1000000000{:05}", 10_000 + client.0)),
                    ("vat_registered", "1"),
                    ("vat_period", "monthly"),
                    ("vat_first_period_end", &month_end(day(40)).to_string()),
                    (
                        "financial_year_end",
                        &format!("{}-12-31", day(0).format("%Y")),
                    ),
                    ("ct_registration_no", &format!("CT-DEMO-{:06}", client.0)),
                    (
                        "effective_date",
                        &month_end(day(10)).succ_opt().unwrap_or(day(10)).to_string(),
                    ),
                    (
                        "note",
                        "Turnover passed AED 150 million: the FTA assigned monthly periods",
                    ),
                ],
            )
            .await?;
        Ok(())
    }

    /// The retainer invoices of this month, and two older ones overdue
    /// (Oasis Fresh Foods and Sand Dune, which is why Sand Dune is
    /// suspended).
    async fn money(&self) -> Result {
        wl_invoicing::subscriptions::run(&self.state, day(0)).await?;
        for (key, what, amount, due) in [
            (
                "oasis",
                "Retainer: VAT returns and fractional CFO, August",
                918_750,
                -38,
            ),
            ("dune", "Retainer: bookkeeping and VAT, July", 504_000, -66),
            (
                "dune",
                "Retainer: bookkeeping and VAT, August",
                504_000,
                -35,
            ),
        ] {
            let source = Subject::new(
                accounting::engagement::ENGAGEMENT_SUBJECT_TYPE,
                self.engagement(key),
            );
            let draft = InvoiceDraft::new(self.clients[key].company, source.clone())
                .line(DraftLine::new(what, amount, VatCategory::Standard).origin(source))
                .due_date(day(due));
            let id = invoicer(&self.state)
                .draft(self.db(), draft)
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("invoicing is off"))?;
            self.driver
                .post(BILLING_EMAIL, &format!("/invoices/{}/issue", id.0), &[])
                .await?;
        }
        wl_invoicing::overdue::run(&self.state).await?;
        Ok(())
    }

    /// The returns, AML cases, KPI packs and the rest.
    pub(crate) async fn work(&mut self) -> Result {
        self.returns().await?;
        self.aml().await?;
        self.kpi().await?;
        self.tax_profile_change().await?;
        self.money().await?;
        self.requests_and_reviews().await?;
        Ok(())
    }

    /// The document request lists of last month (Marina Bay sent its
    /// documents, the others not yet; Sand Dune's period skipped while
    /// suspended), and the CDD files of Desert Rose and Al Waha due for their
    /// periodic review.
    async fn requests_and_reviews(&self) -> Result {
        let made =
            accounting::request_lists::period_end_now(&self.state, month_end(day(-30))).await?;
        if let Some(first) = made.created.first() {
            accounting::request_lists::mark_received(&self.state, *first, day(-2)).await?;
        }
        for key in ["rose", "waha"] {
            if let Some(file) =
                accounting::CddFile::current(self.db(), self.clients[key].company).await?
            {
                accounting::cdd_files::flag_review(&self.state, file.id, day(0)).await?;
            }
        }
        Ok(())
    }
}

/// The last day of the calendar quarter before the one of `date`.
fn last_quarter_end(date: renox::chrono::NaiveDate) -> renox::chrono::NaiveDate {
    use renox::chrono::Datelike;
    let first = ((date.month() - 1) / 3) * 3 + 1;
    renox::chrono::NaiveDate::from_ymd_opt(date.year(), first, 1)
        .and_then(|d| d.pred_opt())
        .unwrap_or(date)
}

/// The last day of the calendar quarter of `date`.
fn quarter_end(date: renox::chrono::NaiveDate) -> renox::chrono::NaiveDate {
    use renox::chrono::Datelike;
    let month = ((date.month() - 1) / 3) * 3 + 3;
    month_end(renox::chrono::NaiveDate::from_ymd_opt(date.year(), month, 1).unwrap_or(date))
}
