//! The money of the demo company: invoices issued by the accountant on their
//! page, and the payments recorded through wl-payments' recorder and
//! allocated to the invoice (a paid invoice earns a loyalty member their
//! points). Some invoices stay open, so the receivables have something to age.

use renox::prelude::*;
use wl_invoicing::Invoice;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus};
use wl_kernel::payments::{AllocationTarget, PaymentMethod, RecordPayment, payment_recorder};

use super::super::demo_data::{at, day};
use super::{ADMIN_LOGIN, AISHA, Seed};

impl Seed {
    /// Issues the invoice `id` (the accountant does, on its page) when it is
    /// still a draft, on `on` (days from today).
    pub(crate) async fn issue_invoice(&self, id: i64, on: i64) -> Result<Invoice> {
        let invoice = Invoice::find_or_404(self.db(), id).await?;
        if invoice.status == InvoiceStatus::Draft {
            at!(self, day(on), {
                self.driver
                    .post(ADMIN_LOGIN, &format!("/invoices/{id}/issue"), &[])
                    .await
            })?;
        }
        Invoice::find_or_404(self.db(), id).await
    }

    /// Records `amount` fils (the whole balance when `None`) from the payer of
    /// invoice `id` on `on` and allocates it to the invoice.
    pub(crate) async fn pay_invoice(
        &self,
        id: i64,
        method: PaymentMethod,
        amount: Option<i64>,
        on: i64,
        reference: &str,
    ) -> Result<i64> {
        let invoice = self.issue_invoice(id, on).await?;
        let amount = amount.unwrap_or(invoice.balance);
        let user = self.user(AISHA).id;
        let mut request = RecordPayment::new(
            wl_kernel::PartyId(invoice.party_id),
            self.entity,
            method,
            amount,
        )
        .collected_by(user);
        let reference = match (reference.is_empty(), method) {
            (false, _) => reference.to_owned(),
            (true, PaymentMethod::Card) => format!("POS-{:05}", invoice.id),
            (true, PaymentMethod::Transfer) => format!("TRF-{:05}", invoice.id),
            (true, _) => String::new(),
        };
        if !reference.is_empty() {
            request = request.reference(&reference);
        }
        request = request.date(day(on));
        let payment = at!(self, day(on), {
            let recorder = payment_recorder(&self.state);
            let payment = recorder.record(&self.state, request, Some(user)).await?;
            recorder
                .allocate(
                    &self.state,
                    payment.id,
                    AllocationTarget::Invoice(InvoiceId(invoice.id)),
                    amount,
                )
                .await?;
            Ok::<_, renox::Error>(payment)
        })?;
        Ok(payment.id.0)
    }

    /// A part of the cancelled job card's bill is paid by transfer; the rest
    /// stays open.
    pub(crate) async fn money(&mut self) -> Result {
        let card = auto::JobCard::find_or_404(self.db(), self.cards["cancelled"]).await?;
        if let Some(invoice) = card.invoice_id {
            let open = self.issue_invoice(invoice, -16).await?.balance;
            self.pay_invoice(invoice, PaymentMethod::Transfer, Some(open / 2), -14, "")
                .await?;
        }
        Ok(())
    }
}
