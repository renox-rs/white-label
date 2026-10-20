//! Making the company's books: the opening cash balances, the supplier bills
//! and their payments, the budget, the expense claims, the bank statements and
//! the VAT periods.
//!
//! Amounts are written as dirhams and fils (`2_300_00` is AED 2,300.00), which
//! is why the digit groups are uneven.
#![allow(clippy::inconsistent_digit_grouping)]

use super::real_today as today;
use renox::chrono::Duration;
use renox::prelude::*;
use wl_invoicing::SupplierBill;
use wl_invoicing::supplier_bill::SupplierBillStatus;
use wl_kernel::payments::{PaymentMethod, RecordPayment, payment_recorder};
use wl_kernel::{LegalEntity, PartyId, Subject};
use wl_payments::BankStatementImport;

use super::finance::{BILLS, BUDGET, BillFate, BillPlan, STATEMENTS, StatementPlan};
use super::people::login_of;
use super::roles::{ACCOUNTANT, ADMIN, FINANCE_MANAGER, OWNER};
use super::seed_leases::{text, upto};
use super::seed_money::aed;
use super::{Seed, day};

impl Seed {
    /// The books of the company.
    pub(crate) async fn finance(&mut self) -> Result {
        self.cash_balances().await?;
        for plan in BILLS {
            self.supplier_bill(plan).await?;
        }
        self.budgets().await?;
        self.expense_claims().await?;
        for plan in STATEMENTS {
            self.bank_statement(plan).await?;
        }
        self.vat_periods().await
    }

    async fn entity_id(&self, code: &str) -> Result<i64> {
        Ok(LegalEntity::query()
            .where_(LegalEntity::CODE.eq(code))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no legal entity {code}"))?
            .id)
    }

    /// What each entity had in its bank accounts at the start of the cash
    /// forecast.
    async fn cash_balances(&mut self) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        for (code, account, amount) in [
            (
                "PGP",
                "Gulf Demo Bank AE00 0000 0000 0000 0001 234",
                "1452880.00",
            ),
            (
                "PGFZ",
                "Emirates Demo Bank AE00 0000 0000 0000 0005 678",
                "318450.00",
            ),
        ] {
            let entity = self.entity_id(code).await?.to_string();
            at!(self, today(), {
                self.driver
                    .post(
                        accountant,
                        "/cash-balances",
                        &[
                            ("legal_entity_id", entity.as_str()),
                            ("bank_account", account),
                            ("amount", amount),
                            ("as_of", text(today()).as_str()),
                            ("note", "Balance from the bank statement"),
                        ],
                    )
                    .await
            })?;
        }
        Ok(())
    }

    async fn supplier_bill(&mut self, plan: &BillPlan) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        let admin = login_of(ADMIN).email;
        let supplier = self.party(plan.supplier).clone();
        let entity = self.entity_id(plan.entity).await?.to_string();
        let date = text(day(-plan.age));
        let due = text(day(-plan.age + 30));
        let mut form: Vec<(String, String)> = vec![
            ("supplier".into(), supplier.code.clone()),
            ("supplier_invoice_number".into(), plan.number.into()),
            ("bill_date".into(), date),
            ("due_date".into(), due),
            ("legal_entity_id".into(), entity),
        ];
        for (index, (description, price, vat)) in plan.lines.iter().enumerate() {
            let n = index + 1;
            form.push((format!("line_{n}_description"), (*description).into()));
            form.push((format!("line_{n}_quantity"), "1".into()));
            form.push((format!("line_{n}_unit_price"), format!("{price}.00")));
            form.push((format!("line_{n}_vat"), (*vat).into()));
        }
        let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let entered = day(-plan.age);
        let reply = at!(self, entered, {
            self.driver
                .post(accountant, "/supplier-bills", &pairs)
                .await
        })?;
        let id = reply
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no bill id"))?;
        self.report.add("supplier bills", 1);
        let path = format!("/supplier-bills/{id}");
        let later = |days: i64| upto(entered + Duration::days(days));
        match plan.fate {
            BillFate::Draft => return Ok(()),
            BillFate::Voided => {
                at!(self, later(1), {
                    self.driver
                        .post(
                            accountant,
                            &format!("{path}/transitions/void"),
                            &[("reason", "Entered against the wrong building")],
                        )
                        .await
                })?;
                return Ok(());
            }
            _ => {}
        }
        at!(self, later(1), {
            self.driver
                .post(accountant, &format!("{path}/submit"), &[])
                .await
        })?;
        if matches!(plan.fate, BillFate::Matched | BillFate::Duplicate) {
            return Ok(());
        }
        at!(self, later(4), {
            self.driver
                .post(
                    admin,
                    &format!("{path}/transitions/approve"),
                    &[(
                        "reason",
                        "Checked against the contract and the visit report",
                    )],
                )
                .await
        })?;
        if matches!(plan.fate, BillFate::Approved) {
            return Ok(());
        }
        let bill = SupplierBill::find_or_404(self.db(), id).await?;
        let amount = if plan.fate == BillFate::Paid {
            bill.balance
        } else {
            bill.balance / 2
        };
        let by = self.user_id(accountant).await?;
        let paid_on = later(25);
        let request = RecordPayment::new(
            PartyId(supplier.id),
            bill.legal_entity_id.unwrap_or_default(),
            PaymentMethod::Transfer,
            amount,
        )
        .outgoing()
        .date(paid_on)
        .reference(&format!("PAY-{}", plan.number.trim()));
        at!(self, paid_on, {
            let payment = payment_recorder(&self.state)
                .record(&self.state, request, Some(by))
                .await
                .map_err(|e| renox::anyhow::anyhow!("demo seed: the bill payment: {e:?}"))?;
            wl_invoicing::bill_pay::pay(
                &self.state,
                &bill,
                &Subject::new("payments.payment", payment.id.0),
                amount,
                Some(by),
            )
            .await
            .map_err(renox::state_machine::TransitionError::into_error)
        })?;
        let _ = SupplierBillStatus::Paid;
        Ok(())
    }

    /// The budget of the year: one approved, then revised (the first is
    /// superseded), one for the next year in the making, one waiting for the
    /// finance manager, one given up.
    async fn budgets(&mut self) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        let finance = login_of(FINANCE_MANAGER).email;
        let year = today()
            .format("%Y")
            .to_string()
            .parse::<i32>()
            .unwrap_or(2026);
        let pgp = self.entity_id("PGP").await?.to_string();
        let pgfz = self.entity_id("PGFZ").await?.to_string();
        // The company budget of this year, drawn up in December and approved
        // by the finance manager; then revised in the autumn.
        let first = at!(self, day(-320), {
            self.budget(accountant, &pgp, year, "Company budget", "empty", true)
                .await
        })?;
        at!(self, day(-318), {
            self.submit_and_approve(accountant, finance, first).await
        })?;
        let marketing = self.category_id("marketing").await?.to_string();
        let revision = at!(self, day(-40), {
            let revision = self
                .budget(
                    accountant,
                    &pgp,
                    year,
                    "Company budget, revised",
                    "last_budget",
                    false,
                )
                .await?;
            self.driver
                .post(
                    accountant,
                    &format!("/budgets/{revision}/lines"),
                    &[
                        ("category_id", marketing.as_str()),
                        ("from", format!("{year}-10").as_str()),
                        ("to", format!("{year}-12").as_str()),
                        ("amount", "16500"),
                        ("note", "Q4 launch campaign"),
                    ],
                )
                .await?;
            Ok::<i64, renox::Error>(revision)
        })?;
        at!(self, day(-38), {
            self.submit_and_approve(accountant, finance, revision).await
        })?;
        // The facilities company: waiting for approval.
        at!(self, day(-20), {
            let waiting = self
                .budget(accountant, &pgfz, year, "Facilities budget", "empty", true)
                .await?;
            self.driver
                .post(accountant, &format!("/budgets/{waiting}/submit"), &[])
                .await
        })?;
        // Next year, in the making, and one given up.
        at!(self, day(-10), {
            self.budget(
                accountant,
                &pgp,
                year + 1,
                "Company budget, next year",
                "last_budget",
                false,
            )
            .await
        })?;
        let given_up = at!(self, day(-60), {
            self.budget(
                accountant,
                &pgfz,
                year + 1,
                "Facilities budget, first idea",
                "empty",
                true,
            )
            .await
        })?;
        at!(self, day(-55), {
            self.driver
                .post(
                    accountant,
                    &format!("/budgets/{given_up}/discard"),
                    &[("reason", "Replaced by the company plan for the year")],
                )
                .await
        })?;
        Ok(())
    }

    async fn category_id(&self, key: &str) -> Result<i64> {
        Ok(wl_invoicing::BudgetCategory::query()
            .where_(wl_invoicing::BudgetCategory::CODE.eq(key))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no budget category {key}"))?
            .id)
    }

    /// A draft budget, with the monthly amounts of [`BUDGET`] when `fill`.
    async fn budget(
        &mut self,
        who: &str,
        entity: &str,
        year: i32,
        name: &str,
        start: &str,
        fill: bool,
    ) -> Result<i64> {
        let year_text = year.to_string();
        let reply = self
            .driver
            .post(
                who,
                "/budgets",
                &[
                    ("legal_entity_id", entity),
                    ("fiscal_year", year_text.as_str()),
                    ("name", name),
                    ("start", start),
                    ("revision_reason", "Marketing raised for the Q4 launch"),
                ],
            )
            .await?;
        let id = reply
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no budget id"))?;
        self.report.add("budgets", 1);
        if fill {
            for (key, amount) in BUDGET {
                let category = self.category_id(key).await?.to_string();
                // Facilities carries a fifth of the company's figures.
                let scaled = if entity == self.entity_id("PGFZ").await?.to_string() {
                    amount / 5
                } else {
                    *amount
                };
                self.driver
                    .post(
                        who,
                        &format!("/budgets/{id}/lines"),
                        &[
                            ("category_id", category.as_str()),
                            ("from", format!("{year}-01").as_str()),
                            ("to", format!("{year}-12").as_str()),
                            ("amount", scaled.to_string().as_str()),
                        ],
                    )
                    .await?;
            }
        }
        Ok(id)
    }

    async fn submit_and_approve(&mut self, who: &str, approver: &str, id: i64) -> Result {
        let reply = self
            .driver
            .post(who, &format!("/budgets/{id}/submit"), &[])
            .await?;
        let request = wl_kernel::approvals::ApprovalRequest::query()
            .where_(wl_kernel::approvals::ApprovalRequest::SUBJECT_ID.eq(id))
            .where_(
                wl_kernel::approvals::ApprovalRequest::SUBJECT_TYPE
                    .eq(wl_kernel::budgets::COMPANY_BUDGET_SUBJECT_TYPE),
            )
            .order_by_desc(wl_kernel::approvals::ApprovalRequest::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| {
                renox::anyhow::anyhow!(
                    "demo seed: no approval request for budget {id}: {:?} {}",
                    reply.headers,
                    reply.message()
                )
            })?;
        self.driver
            .post(approver, &format!("/approvals/{}/approve", request.id), &[])
            .await?;
        Ok(())
    }

    /// Expense claims of the finance team: approved, rejected, waiting, draft.
    async fn expense_claims(&mut self) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        let finance = login_of(FINANCE_MANAGER).email;
        let pgp = self.entity_id("PGP").await?.to_string();
        let claims: [(&str, &str, i64, &str, &str); 8] = [
            (
                "Site visit fuel and parking, July",
                "fuel",
                112,
                "187.50",
                "approve",
            ),
            (
                "Tools for move-out repairs",
                "tools_materials",
                74,
                "642.00",
                "reject",
            ),
            (
                "Parking at the Land Department, Ejari filings",
                "parking_tolls",
                51,
                "96.00",
                "approve",
            ),
            (
                "Lunch with the AC contractor",
                "meals",
                33,
                "310.00",
                "reject",
            ),
            (
                "Fuel for the weekend inspections",
                "fuel",
                20,
                "144.00",
                "wait",
            ),
            (
                "Taxi to the tenant meeting, Sharjah",
                "travel",
                16,
                "78.00",
                "wait",
            ),
            (
                "Printer cartridges for the leasing office",
                "tools_materials",
                5,
                "265.00",
                "draft",
            ),
            (
                "Courier for the cheque deposits",
                "other",
                3,
                "55.00",
                "draft",
            ),
        ];
        for (title, category, age, amount, fate) in claims {
            let date = text(day(-age));
            at!(self, day(-age), {
                self.driver
                    .post(
                        accountant,
                        "/expense-claims",
                        &[
                            ("legal_entity_id", pgp.as_str()),
                            ("title", title),
                            ("line_1_category", category),
                            ("line_1_date", date.as_str()),
                            ("line_1_amount", amount),
                            ("line_1_merchant", "Demo vendor"),
                            ("line_1_description", title),
                        ],
                    )
                    .await
            })?;
            self.report.add("expense claims", 1);
            let claim = wl_payments::expense_claim::ExpenseClaim::query()
                .order_by_desc(wl_payments::expense_claim::ExpenseClaim::ID)
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no claim"))?;
            let path = format!("/expense-claims/{}", claim.id);
            if fate == "draft" {
                continue;
            }
            at!(self, upto(day(-age + 1)), {
                self.driver
                    .post(accountant, &format!("{path}/submit"), &[])
                    .await
            })?;
            let decided = upto(day(-age + 4));
            match fate {
                "approve" => {
                    at!(self, decided, {
                        self.driver
                            .post(finance, &format!("{path}/approve"), &[])
                            .await
                    })?;
                }
                "reject" => {
                    at!(self, decided, {
                        self.driver
                            .post(
                                finance,
                                &format!("{path}/reject"),
                                &[("reason", "No receipt attached")],
                            )
                            .await
                    })?;
                }
                _ => {}
            }
        }
        let _ = OWNER;
        Ok(())
    }

    /// One bank statement, taken as far as its plan says.
    async fn bank_statement(&mut self, plan: &StatementPlan) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        let entity = self.entity_id(plan.entity).await?.to_string();
        let (from, to) = (day(plan.period.0), day(plan.period.1));
        let content = if plan.format == "csv" {
            self.statement_csv(plan).await?
        } else {
            "binary".to_owned()
        };
        let closing = format!("{}.00", plan.closing);
        let uploaded = upto(to + Duration::days(1));
        at!(self, uploaded, {
            self.driver
                .post(
                    accountant,
                    "/bank-statements",
                    &[
                        ("legal_entity_id", entity.as_str()),
                        ("bank_account", plan.account),
                        ("file_name", plan.file),
                        ("format", plan.format),
                        ("period_start", text(from).as_str()),
                        ("period_end", text(to).as_str()),
                        ("closing_balance", closing.as_str()),
                        ("content", content.as_str()),
                    ],
                )
                .await
        })?;
        let import = BankStatementImport::query()
            .order_by_desc(BankStatementImport::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no statement import"))?;
        self.report.add("bank statements", 1);
        let path = format!("/bank-statements/{}", import.id);
        at!(self, uploaded, {
            self.driver
                .try_post(accountant, &format!("{path}/parse"), &[])
                .await
        })?;
        let worked = upto(uploaded + Duration::days(2));
        match plan.end {
            "completed" | "reverted" => {
                let lines = wl_payments::bank_statement_line::BankStatementLine::query()
                    .where_(
                        wl_payments::bank_statement_line::BankStatementLine::IMPORT_ID
                            .eq(import.id),
                    )
                    .get(self.db())
                    .await?;
                if plan.end == "completed" {
                    at!(self, worked, {
                        for line in lines {
                            if line.status
                                == wl_payments::bank_statement_line::LineStatus::Unmatched
                            {
                                self.driver
                                    .post(
                                        accountant,
                                        &format!("/bank-statement-lines/{}/ignore", line.id),
                                        &[(
                                            "reason",
                                            "Bank charge or transfer between own accounts",
                                        )],
                                    )
                                    .await?;
                            }
                        }
                        self.driver
                            .post(accountant, &format!("{path}/complete"), &[])
                            .await
                    })?;
                } else {
                    at!(self, worked, {
                        self.driver
                            .post(accountant, &format!("{path}/revert"), &[])
                            .await
                    })?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The CSV of a statement: rent transfers in, supplier payments and bank
    /// charges out.
    async fn statement_csv(&self, plan: &StatementPlan) -> Result<String> {
        let (from, to) = (plan.period.0, plan.period.1);
        let span = (to - from).max(1);
        let mut rows = vec!["date,description,amount,reference,counterparty".to_owned()];
        let rent = [
            ("Khalid Saeed Al Mansoori", 2_300_00_i64),
            ("Priya Raghunathan", 6_991_66),
            ("Fatuma Ali Hassan", 3_500_00),
            ("Camille Laurent", 3_900_00),
            ("Mahmoud Adel Farouk", 5_200_00),
        ];
        let credits = if plan.entity == "PGP" { 4 } else { 1 };
        for (i, (name, amount)) in rent.iter().enumerate().take(credits) {
            let when = day(from + span * (i as i64 + 1) / 6);
            rows.push(format!(
                "{},TRF {} RENT,{},{}-{},{}",
                text(when),
                name.to_uppercase(),
                aed(*amount),
                "TRF",
                (900 + i as i64) * 7 + from.abs(),
                name
            ));
        }
        let payee = if plan.entity == "PGP" {
            "SKYHIGH LIFTS"
        } else {
            "BRIGHTCLEAN FACILITY"
        };
        rows.push(format!(
            "{},PAYMENT {payee},-{},PAY-{},{payee}",
            text(day(from + span / 2)),
            aed(13_230_00),
            from.abs() + 40
        ));
        rows.push(format!(
            "{},BANK CHARGES AND VAT,-{},,Gulf Demo Bank",
            text(day(to - 1)),
            aed(185_25)
        ));
        let _ = Duration::days(0);
        Ok(rows.join("\n"))
    }

    /// The VAT summaries of the past quarters, taken through the review: each
    /// summary opens with its period, is prepared once the period is over,
    /// reviewed and locked.
    async fn vat_periods(&mut self) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        let admin = login_of(ADMIN).email;
        for ago in [420, 330, 240, 150, 60, 0] {
            at!(self, day(-ago), {
                wl_invoicing::vat_summary::run(&self.state, day(-ago)).await
            })?;
        }
        let summaries = wl_invoicing::vat_summary::VatSummary::query()
            .order_by(wl_invoicing::vat_summary::VatSummary::PERIOD_START)
            .get(self.db())
            .await?;
        let past: Vec<_> = summaries
            .into_iter()
            .filter(|summary| summary.period_end < today())
            .collect();
        let count = past.len();
        for (index, summary) in past.into_iter().enumerate() {
            let path = format!("/vat-summaries/{}/transitions", summary.id);
            let prepared = upto(summary.period_end + Duration::days(8));
            at!(self, prepared, {
                self.driver
                    .post(accountant, &format!("{path}/prepare"), &[])
                    .await
            })?;
            if index + 1 == count {
                continue;
            }
            at!(self, upto(prepared + Duration::days(2)), {
                self.driver
                    .post(admin, &format!("{path}/review"), &[])
                    .await
            })?;
            if index + 2 == count {
                continue;
            }
            at!(self, upto(prepared + Duration::days(5)), {
                self.driver
                    .post(accountant, &format!("{path}/lock"), &[])
                    .await
            })?;
        }
        Ok(())
    }
}
