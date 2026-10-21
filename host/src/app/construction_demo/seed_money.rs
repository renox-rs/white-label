//! The company's money besides the IPA invoices: the budget of the year with
//! the construction categories (the materials cost code is at 92% of this
//! month's line), the bank balances of the cash forecast, supplier bills, the
//! client's cheques (one bounced, one post-dated), the set-off of a client's
//! back-charge against its invoice, credit notes with refunds (requested,
//! approved, rejected), the payment of the retention invoice, a client
//! blocked by compliance, and the daily rules of the modules run for today.

use construction::{Ipa, Retention};
use renox::chrono::{Datelike, NaiveDate};
use renox::prelude::*;
use wl_invoicing::{BudgetCategory, Invoice, SupplierBill};
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::cheques::{RegisterCheque, cheques};
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::payments::{PaymentMethod, RecordPayment, payment_recorder};
use wl_kernel::{PartyId, Subject};
use wl_payments::{Cheque, Refund};

use super::at;
use super::people::{self, email_of};
use super::roles::{ACCOUNTANT, ADMIN, COMPLIANCE, RECEIVABLES};
use super::seed_ipas::after;
use super::{Seed, aed, day, text, upto};

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
    /// The entity of the company, as the forms take it.
    async fn entity_text(&self) -> Result<String> {
        Ok(wl_kernel::LegalEntity::default_entity(self.db())
            .await?
            .id
            .to_string())
    }

    async fn category_id(&self, code: &str) -> Result<i64> {
        Ok(BudgetCategory::query()
            .where_(BudgetCategory::CODE.eq(code))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no budget category {code}"))?
            .id)
    }

    /// The money of the company, after the projects' story.
    pub(crate) async fn money(&mut self) -> Result {
        self.client_blocked().await?;
        self.budget().await?;
        self.balances().await?;
        self.client_cheques().await?;
        self.set_off().await?;
        self.credit_notes().await?;
        self.retention_paid().await?;
        self.supplier_bills().await?;
        Ok(())
    }

    /// Compliance blocks the healthcare client, after its cheque bounced: no
    /// new project is started for it, the running one goes on.
    async fn client_blocked(&mut self) -> Result {
        let compliance = email_of(COMPLIANCE);
        let client = self.party(people::AL_NOOR).id;
        at!(self, day(-15), {
            self.driver
                .post(
                    compliance,
                    &format!("/parties/{client}/block"),
                    &[
                        ("reason_code", "non_payment"),
                        ("note", "A cheque bounced and the second invoice is unpaid"),
                    ],
                )
                .await
        })?;
        Ok(())
    }

    /// The budget of the year: the categories of a contractor, the lines of
    /// every month, submitted by the accountant and approved by the MD.
    async fn budget(&mut self) -> Result {
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
    async fn balances(&mut self) -> Result {
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
        let due = text(entered.checked_add_days(renox::chrono::Days::new(30)).unwrap_or(entered));
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
                    &[("reason", "Checked against the purchase order and the receipt")],
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
    async fn supplier_bills(&mut self) -> Result {
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

    /// The clinic's client paid its first invoice by cheque, which bounced;
    /// a post-dated cheque for the second invoice is held.
    async fn client_cheques(&mut self) -> Result {
        let accountant = email_of(ACCOUNTANT);
        let entity = wl_kernel::LegalEntity::default_entity(self.db()).await?.id;
        let client = self.party(people::AL_NOOR).id;
        let by = self.user_id(accountant).await?;
        let first = self.ipa_of("CLN-03", 1).await?;
        let second = self.ipa_of("CLN-03", 2).await?;
        for (ipa, number, dated, registered, bounced) in [
            (first, "100231", -61, -62, true),
            (second, "100287", 14, -2, false),
        ] {
            let Some(invoice_id) = ipa.invoice_id else {
                continue;
            };
            let invoice = Invoice::find_or_404(self.db(), invoice_id).await?;
            let mut request = RegisterCheque::new(
                PartyId(client),
                entity,
                "Gulf Demo Bank",
                number,
                day(dated),
                invoice.balance,
            );
            request.linked = vec![Subject::new(INVOICE_SUBJECT_TYPE, invoice_id)];
            request.collected_by = Some(by);
            let summary = at!(self, upto(day(registered)), {
                cheques(&self.state)
                    .register(&self.state, request, Some(by))
                    .await
            })?;
            self.report.add("cheques", 1);
            if bounced {
                let cheque = Cheque::find_or_404(self.db(), summary.id.0).await?;
                let path = format!("/cheques/{}", cheque.id);
                let deposit = after(day(dated), 1);
                at!(self, deposit, {
                    self.driver
                        .post(accountant, &format!("{path}/deposit"), &[])
                        .await
                })?;
                at!(self, after(deposit, 3), {
                    self.driver
                        .post(
                            accountant,
                            &format!("{path}/bounce"),
                            &[
                                ("reason", "insufficient_funds"),
                                ("certificate", "RTN-DEMO-00231"),
                                ("honoured", ""),
                            ],
                        )
                        .await
                })?;
            }
        }
        Ok(())
    }

    /// The client of the tower deducted a back-charge, which became a supplier
    /// bill from the client: the accountant approves it and sets it off
    /// against the client's invoice.
    async fn set_off(&mut self) -> Result {
        let accountant = email_of(ACCOUNTANT);
        let ipa = self.ipa_of("TWR-01", 2).await?;
        let (Some(invoice), Some(bill)) = (ipa.invoice_id, ipa.supplier_bill_id) else {
            return Ok(());
        };
        let client = self.party(people::GULF_HORIZON).clone();
        let entity = self.entity_text().await?;
        let when = upto(day(-60));
        at!(self, when, {
            self.driver
                .post(
                    accountant,
                    &format!("/supplier-bills/{bill}/transitions/approve"),
                    &[("reason", "The back-charge is on the client's certificate")],
                )
                .await
        })?;
        let balance = SupplierBill::find_or_404(self.db(), bill).await?.balance;
        let amount = aed(balance);
        let invoice_text = invoice.to_string();
        let bill_text = bill.to_string();
        at!(self, after(when, 1), {
            self.driver
                .post(
                    accountant,
                    "/payments",
                    &[
                        ("party_code", client.code.as_str()),
                        ("legal_entity_id", entity.as_str()),
                        ("method", "set_off"),
                        ("amount", amount.as_str()),
                        ("reference", "Set-off BC-TWR-017"),
                        ("set_off_invoice_id", invoice_text.as_str()),
                        ("set_off_bill_id", bill_text.as_str()),
                    ],
                )
                .await
        })?;
        self.report.add("set-offs", 1);
        Ok(())
    }

    /// Credit notes on paid invoices (a measurement adjustment after payment)
    /// and the refunds they ask for: one waiting, one approved, one rejected.
    async fn credit_notes(&mut self) -> Result {
        let receivables = email_of(RECEIVABLES);
        let admin = email_of(ADMIN);
        let plans = [
            ("OFC-06", 2, 5_000, -75, "wait"),
            ("MALL-02", 1, 3_000, -60, "approve"),
            ("MALL-02", 2, 2_000, -45, "reject"),
        ];
        for (code, number, net, offset, fate) in plans {
            let ipa = self.ipa_of(code, number).await?;
            let Some(invoice) = ipa.invoice_id else {
                continue;
            };
            let net_text = format!("{net}.00");
            let on = upto(day(offset));
            at!(self, on, {
                self.driver
                    .post(
                        receivables,
                        &format!("/invoices/{invoice}/credit-notes"),
                        &[
                            ("net_1", net_text.as_str()),
                            ("reason", "Measurement adjustment agreed after payment"),
                        ],
                    )
                    .await
            })?;
            self.report.add("credit notes", 1);
            let note = wl_invoicing::CreditNote::query()
                .where_(wl_invoicing::CreditNote::INVOICE_ID.eq(invoice))
                .order_by_desc(wl_invoicing::CreditNote::ID)
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no credit note"))?;
            let gross = aed(note.remaining);
            at!(self, after(on, 2), {
                self.driver
                    .post(
                        receivables,
                        &format!("/credit-notes/{}/refund", note.id),
                        &[
                            ("amount", gross.as_str()),
                            ("method", "transfer"),
                            ("reason", "The client paid in full before the adjustment"),
                        ],
                    )
                    .await
            })?;
            self.report.add("refunds", 1);
            if fate == "wait" {
                continue;
            }
            let refund = Refund::query()
                .order_by_desc(Refund::ID)
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no refund"))?;
            // Approving a refund is its own transition (`FIN.ACL.04`): the
            // MD decides, never the user who asked.
            let verb = if fate == "approve" { "approve" } else { "reject" };
            at!(self, after(on, 4), {
                self.driver
                    .post(
                        admin,
                        &format!("/refunds/{}/transitions/{verb}", refund.id),
                        &[("reason", "Checked against the certificate")],
                    )
                    .await
            })?;
        }
        Ok(())
    }

    /// The client paid the retention invoice of the handed-over project.
    async fn retention_paid(&mut self) -> Result {
        let project = self.project("OFC-06").await?;
        let part = Retention::query()
            .where_(Retention::PROJECT_ID.eq(project.id))
            .where_(Retention::PART.eq(construction::retention::RetentionPart::First))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no retention part"))?;
        if let Some(invoice) = part.invoice_id {
            self.pay_invoice(
                people::GULF_HORIZON,
                invoice,
                upto(day(-95)),
                100,
                "TRF-OFC-06-RET1",
            )
            .await?;
        }
        let _ = Ipa::TABLE;
        Ok(())
    }

    /// The daily rules of the modules, run for today: overdue invoices and
    /// their reminders, cheques due, documents about to expire, the unanswered
    /// variations, the guarantees about to expire, the retention that falls
    /// due, the notice deadlines.
    pub(crate) async fn daily_rules(&mut self) -> Result {
        let today = crate::app::demo_data::real_today();
        at!(self, today, {
            wl_invoicing::overdue::run(&self.state).await?;
            wl_invoicing::reminders::run(&self.state, today).await?;
            wl_payments::cheque_deposit::flag_due_run(&self.state, today).await?;
            wl_payments::cheque_chase::run(&self.state, today).await?;
            wl_documents::expiry::run(&self.state, today).await?;
            construction::chase::daily(&self.state).await?;
            construction::notice_deadline::daily(&self.state).await?;
            construction::expiry::daily(&self.state).await?;
            construction::release::daily(&self.state).await
        })
    }
}
