//! The company's money besides the IPA invoices: the budget of the year with
//! the construction categories (the materials cost code is at 92% of this
//! month's line), the bank balances of the cash forecast, supplier bills, the
//! client's cheques (one bounced, one post-dated), the set-off of a client's
//! back-charge against its invoice, credit notes with refunds (requested,
//! approved, rejected), the payment of the retention invoice, a client
//! blocked by compliance, and the daily rules of the modules run for today.

use construction::{Ipa, Retention};
use renox::prelude::*;
use wl_invoicing::{BudgetCategory, Invoice, SupplierBill};
use wl_kernel::cheques::{RegisterCheque, cheques};
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::{PartyId, Subject};
use wl_payments::{Cheque, Refund};

use super::at;
use super::people::{self, email_of};
use super::roles::{ACCOUNTANT, ADMIN, COMPLIANCE, RECEIVABLES};
use super::seed_ipas::after;
use super::{Seed, aed, day, upto};

impl Seed {
    /// The entity of the company, as the forms take it.
    pub(crate) async fn entity_text(&self) -> Result<String> {
        Ok(wl_kernel::LegalEntity::default_entity(self.db())
            .await?
            .id
            .to_string())
    }

    pub(crate) async fn category_id(&self, code: &str) -> Result<i64> {
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
            let verb = if fate == "approve" {
                "approve"
            } else {
                "reject"
            };
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
