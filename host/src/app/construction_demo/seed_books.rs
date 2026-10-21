//! The company's books: the budget of the year with the construction
//! categories (the materials cost code is at 92% of this month's line), the
//! bank balances of the cash forecast and the supplier bills.

use renox::chrono::{Datelike, NaiveDate};
use renox::prelude::*;
use wl_invoicing::SupplierBill;
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::payments::{PaymentMethod, RecordPayment, payment_recorder};
use wl_kernel::{PartyId, Subject};

use super::at;
use super::people::{self, email_of};
use super::roles::{ACCOUNTANT, ADMIN};
use super::seed_ipas::after;
use super::{Seed, day, text, upto};

/// The materials a month: the line of the budget, in dirhams.
const MATERIALS_LINE: i64 = 200_000;

/// The bills approved this month for materials (net, dirhams); together 92% of
/// the month's line.
const MATERIALS_BILLS: [(&str, &str, i64); 3] = [
    (
        people::GYPSUM,
        "Boards, studs, plywood and tiles for the fit-outs",
        78_000,
    ),
    (
        people::ELECTRICAL,
        "Cables, breakers and LED panels for the fit-outs",
        71_000,
    ),
    (
        people::PAINTS,
        "Interior emulsion and primers for the fit-outs",
        35_000,
    ),
];

impl Seed {
    /// The budget of the year: the categories of a contractor, the lines of
    /// every month, submitted by the accountant and approved by the MD.
    pub(crate) async fn budget(&mut self) -> Result {
        let accountant = email_of(ACCOUNTANT);
        let admin = email_of(ADMIN);
        let entity = self.entity_text().await?;
        let year = crate::app::demo_data::real_today().year();
        let year_text = year.to_string();
        // The categories the seeded ones lack: what is paid to subcontractors.
        at!(self, day(-330), {
            self.driver
                .post(
                    accountant,
                    "/budget-categories",
                    &[
                        ("code", "subcontractors"),
                        ("name_en", "Subcontractors"),
                        ("name_ar", "مقاولو الباطن"),
                        ("kind", "expense"),
                        ("owner_role", "accountant"),
                        ("mappings", "cost_code: subcontract\ncost_code: mep_package"),
                        ("active", "on"),
                    ],
                )
                .await
        })?;
        let reply = at!(self, day(-60), {
            self.driver
                .post(
                    accountant,
                    "/budgets",
                    &[
                        ("legal_entity_id", entity.as_str()),
                        ("fiscal_year", year_text.as_str()),
                        ("name", "Company budget"),
                        ("start", "empty"),
                        ("revision_reason", ""),
                    ],
                )
                .await
        })?;
        let draft = reply
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no budget id"))?;
        for (code, amount) in [
            ("sales_services", "900000"),
            ("cost_of_sales", "200000"),
            ("subcontractors", "120000"),
            ("salaries_staff", "180000"),
            ("rent_utilities", "45000"),
            ("transport_fuel", "12000"),
            ("professional_fees", "8000"),
            ("other_expense", "10000"),
        ] {
            let category = self.category_id(code).await?.to_string();
            at!(self, day(-60), {
                self.driver
                    .post(
                        accountant,
                        &format!("/budgets/{draft}/lines"),
                        &[
                            ("category_id", category.as_str()),
                            ("from", format!("{year}-01").as_str()),
                            ("to", format!("{year}-12").as_str()),
                            ("amount", amount),
                        ],
                    )
                    .await
            })?;
        }
        at!(self, day(-59), {
            self.driver
                .post(accountant, &format!("/budgets/{draft}/submit"), &[])
                .await
        })?;
        let request = ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_ID.eq(draft))
            .where_(
                ApprovalRequest::SUBJECT_TYPE.eq(wl_kernel::budgets::COMPANY_BUDGET_SUBJECT_TYPE),
            )
            .order_by_desc(ApprovalRequest::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no budget approval"))?;
        at!(self, day(-58), {
            self.driver
                .post(admin, &format!("/approvals/{}/approve", request.id), &[])
                .await
        })?;
        self.report.add("budgets", 1);
        Ok(())
    }

    /// What the bank accounts held, for the cash forecast.
    pub(crate) async fn balances(&mut self) -> Result {
        let accountant = email_of(ACCOUNTANT);
        let entity = self.entity_text().await?;
        let today = crate::app::demo_data::real_today();
        for (account, amount) in [
            ("Gulf Demo Bank AE00 0000 0000 0000 0001 234", "1186400.00"),
            ("Creek Demo Bank AE00 0000 0000 0000 0005 678", "242750.00"),
        ] {
            at!(self, today, {
                self.driver
                    .post(
                        accountant,
                        "/cash-balances",
                        &[
                            ("legal_entity_id", entity.as_str()),
                            ("bank_account", account),
                            ("amount", amount),
                            ("as_of", text(today).as_str()),
                            ("note", "Balance from the bank statement"),
                        ],
                    )
                    .await
            })?;
        }
        Ok(())
    }

    /// A supplier bill: entered and submitted by the accountant, approved by
    /// the MD. `paid` is the share paid by transfer, in percent. Returns its
    /// id.
    #[allow(clippy::too_many_arguments)]
    async fn bill(
        &mut self,
        supplier: &str,
        number: &str,
        description: &str,
        net: i64,
        cost_code: &str,
        entered: NaiveDate,
        paid: i64,
    ) -> Result<i64> {
        let accountant = email_of(ACCOUNTANT);
        let admin = email_of(ADMIN);
        let entity = self.entity_text().await?;
        let party = self.party(supplier).clone();
        let price = format!("{net}.00");
        let date = text(entered);
        let due = text(
            entered
                .checked_add_days(renox::chrono::Days::new(30))
                .unwrap_or(entered),
        );
        let reply = at!(self, entered, {
            self.driver
                .post(
                    accountant,
                    "/supplier-bills",
                    &[
                        ("supplier", party.code.as_str()),
                        ("supplier_invoice_number", number),
                        ("bill_date", date.as_str()),
                        ("due_date", due.as_str()),
                        ("legal_entity_id", entity.as_str()),
                        ("line_1_description", description),
                        ("line_1_quantity", "1"),
                        ("line_1_unit_price", price.as_str()),
                        ("line_1_vat", "standard"),
                        ("line_1_cost_code", cost_code),
                    ],
                )
                .await
        })?;
        let id = reply
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no bill id"))?;
        self.report.add("supplier bills", 1);
        let path = format!("/supplier-bills/{id}");
        at!(self, entered, {
            self.driver
                .post(accountant, &format!("{path}/submit"), &[])
                .await?;
            self.driver
                .post(
                    admin,
                    &format!("{path}/transitions/approve"),
                    &[(
                        "reason",
                        "Checked against the purchase order and the receipt",
                    )],
                )
                .await
        })?;
        if paid > 0 {
            let bill = SupplierBill::find_or_404(self.db(), id).await?;
            let amount = bill.balance * paid / 100;
            let by = self.user_id(accountant).await?;
            let paid_on = after(entered, 20);
            let request = RecordPayment::new(
                PartyId(party.id),
                bill.legal_entity_id.unwrap_or_default(),
                PaymentMethod::Transfer,
                amount,
            )
            .outgoing()
            .date(paid_on)
            .reference(&format!("PAY-{number}"));
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
        }
        Ok(id)
    }

    /// The supplier bills: three for materials this month (92% of the line),
    /// a subcontractor's bill paid in part, and a draft.
    pub(crate) async fn supplier_bills(&mut self) -> Result {
        let today = crate::app::demo_data::real_today();
        let _ = MATERIALS_LINE;
        for (index, (supplier, description, net)) in MATERIALS_BILLS.iter().enumerate() {
            self.bill(
                supplier,
                &format!("INV-AW-{}", 4100 + index),
                description,
                *net,
                "materials",
                today,
                0,
            )
            .await?;
        }
        self.bill(
            people::MEP,
            "MEP-PKG-0112",
            "MEP first fix package, Festival Plaza unit L2-14",
            46_000,
            "subcontract",
            upto(day(-28)),
            60,
        )
        .await?;
        // A bill from the glass subcontractor is entered and left in draft.
        let accountant = email_of(ACCOUNTANT);
        let entity = self.entity_text().await?;
        let glass = self.party(people::GLASS).code.clone();
        at!(self, day(-2), {
            self.driver
                .post(
                    accountant,
                    "/supplier-bills",
                    &[
                        ("supplier", glass.as_str()),
                        ("supplier_invoice_number", "AQG-2291"),
                        ("bill_date", text(day(-2)).as_str()),
                        ("due_date", text(day(28)).as_str()),
                        ("legal_entity_id", entity.as_str()),
                        ("line_1_description", "Aluminium frames, deposit"),
                        ("line_1_unit_price", "28000.00"),
                        ("line_1_vat", "standard"),
                        ("line_1_cost_code", "subcontract"),
                    ],
                )
                .await
        })?;
        self.report.add("supplier bills", 1);
        Ok(())
    }
}
