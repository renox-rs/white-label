//! The money of a lease: the instalments invoiced on their due dates and
//! issued by receivables, the post-dated cheques received with the lease,
//! deposited on their dates by the accountant and cleared or bounced, and the
//! payments by transfer that settle what a bounced cheque left.

use property::{ChargeStatus, LeaseCharge};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::Invoice;
use wl_kernel::PartyId;
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus};
use wl_kernel::payments::{AllocationTarget, PaymentMethod, RecordPayment, payment_recorder};
use wl_payments::Cheque;

use super::Seed;
use super::leases::{Plan, Quirk};
use super::people::login_of;
use super::roles::{ACCOUNTANT, RECEIVABLES};
use super::seed_documents::party_number;
use super::seed_leases::text;

/// The banks the demo cheques are drawn on (fictional).
const BANKS: [&str; 4] = [
    "Gulf Demo Bank",
    "Emirates Demo Bank",
    "Al Wasl Demo Bank",
    "Creek Demo Bank",
];

/// The number of the cheque of instalment `index` (from 0) of lease `id`.
pub fn cheque_number(lease: i64, index: usize) -> String {
    format!("{:06}", 100_000 + lease * 100 + index as i64 + 1)
}

/// What the tenant pays for `charge`: the amount and its VAT.
pub fn gross_of(charge: &LeaseCharge) -> i64 {
    charge.amount + (charge.amount * charge.vat_category.rate_bp() + 5_000) / 10_000
}

/// `fils` as the dirhams a form takes: `6991.66`.
pub fn aed(fils: i64) -> String {
    format!("{}.{:02}", fils / 100, fils % 100)
}

/// The rent charges of a lease, in order.
fn rent_of(charges: Vec<LeaseCharge>) -> Vec<LeaseCharge> {
    let mut rent: Vec<LeaseCharge> = charges
        .into_iter()
        .filter(|c| c.charge_type == property::charge::RENT && c.status != ChargeStatus::Cancelled)
        .collect();
    rent.sort_by_key(|c| (c.due_date, c.id));
    rent
}

impl Seed<'_> {
    /// Drafts the invoices of the charges due through `day` and has receivables
    /// issue the drafts of leases.
    pub(crate) async fn invoice_through(&mut self, day: NaiveDate) -> Result {
        let report = property::invoicing::run(&self.state, day).await?;
        self.report.add("lease invoices drafted", report.invoiced);
        self.issue_drafts().await
    }

    /// Issues the draft invoices of leases, as receivables does from the page.
    pub(crate) async fn issue_drafts(&mut self) -> Result {
        let drafts = Invoice::query()
            .where_(Invoice::STATUS.eq(InvoiceStatus::Draft))
            .where_(Invoice::SOURCE_TYPE.eq("property.lease"))
            .order_by(Invoice::ID)
            .get(self.db())
            .await?;
        for draft in drafts {
            if self.held_back.contains(&draft.id) {
                continue;
            }
            self.driver
                .post(
                    login_of(RECEIVABLES).email,
                    &format!("/invoices/{}/issue", draft.id),
                    &[],
                )
                .await?;
            self.report.add("lease invoices issued", 1);
        }
        Ok(())
    }

    /// The rent charges of lease `id`, in order of their due dates.
    pub(crate) async fn rent_charges(&self, id: i64) -> Result<Vec<LeaseCharge>> {
        Ok(rent_of(LeaseCharge::of_lease(self.db(), id).await?))
    }

    /// The user id of the demo login `email`.
    pub(crate) async fn user_id(&self, email: &str) -> Result<i64> {
        Ok(User::find_by_email(self.db(), email)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("the demo login {email}"))?
            .id)
    }

    /// Records a payment from the tenant of `plan` on `date`, not allocated;
    /// its id.
    pub(crate) async fn record_payment(
        &mut self,
        plan: &Plan,
        amount: i64,
        date: NaiveDate,
        method: PaymentMethod,
        reference: String,
    ) -> Result<i64> {
        let party = PartyId(self.party(plan.tenant).id);
        let entity = self.entity_of(plan.unit.0).await?;
        let by = self.user_id(login_of(ACCOUNTANT).email).await?;
        let mut request = RecordPayment::new(party, entity, method, amount);
        request.date = Some(date);
        request.reference = Some(reference);
        let payment = payment_recorder(&self.state)
            .record(&self.state, request, Some(by))
            .await
            .map_err(|e| renox::anyhow::anyhow!("demo seed: the payment: {e:?}"))?;
        self.report.add("payments recorded", 1);
        Ok(payment.id.0)
    }

    /// Records a payment by transfer from the tenant of `plan` on `date` and
    /// allocates `amount` fils of it to invoice `invoice`; the payment's id.
    pub(crate) async fn pay_invoice(
        &mut self,
        plan: &Plan,
        invoice: i64,
        amount: i64,
        date: NaiveDate,
        method: PaymentMethod,
    ) -> Result<i64> {
        let reference = format!(
            "{}-{}-{}",
            if method == PaymentMethod::Cash {
                "CASH"
            } else {
                "TRF"
            },
            party_number(&self.party(plan.tenant).code),
            invoice
        );
        let payment = self
            .record_payment(plan, amount, date, method, reference)
            .await?;
        payment_recorder(&self.state)
            .allocate(
                &self.state,
                wl_kernel::payments::PaymentId(payment),
                AllocationTarget::Invoice(InvoiceId(invoice)),
                amount,
            )
            .await
            .map_err(|e| renox::anyhow::anyhow!("demo seed: the allocation: {e:?}"))?;
        Ok(payment)
    }

    /// Registers the cheques of the lease `id` of `plan` with the property
    /// manager of its building: one for each rent instalment but the ones paid
    /// another way.
    pub(crate) async fn cheques_of(&mut self, plan: &Plan, id: i64) -> Result {
        let rent = self.rent_charges(id).await?;
        let by_transfer = |index: usize| {
            matches!(plan.quirk, Quirk::FirstByTransfer | Quirk::PartPaid) && index == 0
                || matches!(plan.quirk, Quirk::PartPaid) && index == 1
        };
        let past = rent.iter().filter(|c| c.due_date <= today()).count();
        let unpaid = match plan.quirk {
            Quirk::Unpaid(n) => n,
            _ => 0,
        };
        let party_n = party_number(&self.party(plan.tenant).code);
        let bank = BANKS[party_n as usize % BANKS.len()];
        let mut lines = Vec::new();
        for (index, charge) in rent.iter().enumerate() {
            let unpaid_one = index < past && index >= past.saturating_sub(unpaid) && unpaid > 0;
            // The invoice a quirk acts on is not paid by cheque.
            let acted_on = matches!(
                plan.quirk,
                Quirk::Disputed(k) | Quirk::Credited(k) | Quirk::WrittenOff(k) if k == index + 1
            );
            if by_transfer(index) || unpaid_one || acted_on {
                continue;
            }
            lines.push(format!(
                "{} | {} | {} | {}",
                cheque_number(id, index),
                text(charge.due_date),
                aed(gross_of(charge)),
                index + 1
            ));
        }
        if lines.is_empty() {
            return Ok(());
        }
        let manager = super::people::manager_of(plan.unit.0);
        self.driver
            .post(
                manager,
                &format!("/property/leases/{id}/cheques"),
                &[("bank", bank), ("cheques", &lines.join("\n"))],
            )
            .await?;
        self.report.add("cheques received", lines.len());
        Ok(())
    }

    /// The bank: transfers on their dates, and the cheques dated by today
    /// deposited on their dates by the accountant, then cleared, or bounced as
    /// the quirk of the lease says.
    pub(crate) async fn bank_run(&mut self, plan: &Plan, id: i64) -> Result {
        let rent = self.rent_charges(id).await?;
        let accountant = login_of(ACCOUNTANT).email;
        for (index, charge) in rent.iter().enumerate() {
            let k = index + 1;
            if charge.due_date > today() {
                continue;
            }
            let invoice = charge.invoice_id;
            match (plan.quirk, index, invoice) {
                (Quirk::FirstByTransfer | Quirk::PartPaid, 0, Some(invoice)) => {
                    self.pay_invoice(
                        plan,
                        invoice,
                        gross_of(charge),
                        charge.due_date,
                        PaymentMethod::Transfer,
                    )
                    .await?;
                    continue;
                }
                (Quirk::PartPaid, 1, Some(invoice)) => {
                    self.pay_invoice(
                        plan,
                        invoice,
                        gross_of(charge) / 2,
                        charge.due_date,
                        PaymentMethod::Cash,
                    )
                    .await?;
                    continue;
                }
                _ => {}
            }
            let Some(cheque) = Cheque::query()
                .where_(Cheque::NUMBER.eq(cheque_number(id, index).as_str()))
                .first(self.db())
                .await?
            else {
                continue;
            };
            let path = format!("/cheques/{}", cheque.id);
            self.driver
                .post(
                    accountant,
                    &format!("{path}/deposit"),
                    &[("deposit_date", &text(charge.due_date))],
                )
                .await?;
            let certificate = format!("RTN-DEMO-{:05}", cheque.id * 17);
            match plan.quirk {
                Quirk::Bounced(n) if n == k => {
                    self.bounce(accountant, &path, &certificate).await?;
                }
                Quirk::BouncedReplaced(n) if n == k => {
                    self.bounce(accountant, &path, &certificate).await?;
                    let payment = self
                        .record_payment(
                            plan,
                            cheque.amount,
                            today(),
                            PaymentMethod::Transfer,
                            format!("TRF-REPL-{}", cheque.id),
                        )
                        .await?;
                    self.driver
                        .post(
                            accountant,
                            &format!("{path}/replacements"),
                            &[("kind", "payment"), ("ref_id", &payment.to_string())],
                        )
                        .await?;
                    self.driver
                        .post(accountant, &format!("{path}/replace"), &[])
                        .await?;
                }
                Quirk::BouncedCleared(n) if n == k => {
                    self.bounce(accountant, &path, &certificate).await?;
                    self.driver
                        .post(
                            accountant,
                            &format!("{path}/represent"),
                            &[
                                ("date", &text(today())),
                                ("funds_confirmed", "true"),
                                ("note", "Tenant confirmed funds by phone, call logged"),
                            ],
                        )
                        .await?;
                    self.driver
                        .post(accountant, &format!("{path}/clear"), &[])
                        .await?;
                }
                Quirk::BouncedLegal(n) if n == k => {
                    self.bounce(accountant, &path, &certificate).await?;
                    self.driver
                        .post(
                            accountant,
                            &format!("{path}/refer"),
                            &[("case_type", "civil_execution")],
                        )
                        .await?;
                }
                _ => {
                    self.driver
                        .post(accountant, &format!("{path}/clear"), &[])
                        .await?;
                }
            }
        }
        Ok(())
    }

    /// What happens to one invoice of the lease: disputed by the tenant,
    /// credited in full, or written off with a property manager's approval.
    pub(crate) async fn invoice_quirk(&mut self, plan: &Plan, id: i64) -> Result {
        let (k, kind) = match plan.quirk {
            Quirk::Disputed(k) => (k, "dispute"),
            Quirk::Credited(k) => (k, "credit"),
            Quirk::WrittenOff(k) => (k, "write_off"),
            _ => return Ok(()),
        };
        let rent = self.rent_charges(id).await?;
        let Some(charge) = rent.get(k - 1) else {
            return Ok(());
        };
        let Some(invoice) = charge.invoice_id else {
            return Ok(());
        };
        let receivables = login_of(RECEIVABLES).email;
        match kind {
            "dispute" => {
                self.driver
                    .post(
                        receivables,
                        &format!("/invoices/{invoice}/dispute"),
                        &[
                            (
                                "reason",
                                "The tenant says the instalment was paid in cash to the building office",
                            ),
                            ("amount", aed(gross_of(charge)).as_str()),
                        ],
                    )
                    .await?;
            }
            "credit" => {
                self.driver
                    .post(
                        receivables,
                        &format!("/invoices/{invoice}/credit"),
                        &[("reason", "Issued twice for the same instalment")],
                    )
                    .await?;
            }
            _ => {
                // The page asks for `invoices.write_off` and `leases.view`; no
                // other role than the company admin holds both.
                let admin = login_of(super::roles::ADMIN).email;
                self.driver
                    .post(
                        admin,
                        &format!("/property/leases/{id}/write-off"),
                        &[
                            ("invoice", invoice.to_string().as_str()),
                            ("reason_code", "insolvent"),
                        ],
                    )
                    .await?;
                let request = ApprovalRequest::query()
                    .where_(ApprovalRequest::SUBJECT_TYPE.eq("invoicing.invoice"))
                    .where_(ApprovalRequest::SUBJECT_ID.eq(invoice))
                    .first(self.db())
                    .await?;
                if let Some(request) = request {
                    self.driver
                        .post(
                            super::people::manager_of(plan.unit.0),
                            &format!("/approvals/{}/approve", request.id),
                            &[],
                        )
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn bounce(&mut self, who: &str, path: &str, certificate: &str) -> Result {
        self.driver
            .post(
                who,
                &format!("{path}/bounce"),
                &[
                    ("reason", "insufficient_funds"),
                    ("certificate", certificate),
                    ("honoured", ""),
                ],
            )
            .await?;
        self.report.add("cheques bounced", 1);
        Ok(())
    }

    /// The cheque with this number.
    pub(crate) async fn cheque(&self, number: &str) -> Result<Cheque> {
        Cheque::query()
            .where_(Cheque::NUMBER.eq(number))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no cheque {number}").into())
    }
}
