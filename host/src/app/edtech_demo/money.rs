//! The money of the demo centre: the enrolment's invoice, issued by the
//! accountant on its page, and the payments the front desk and the accountant
//! record through wl-payments' recorder, allocated to the invoice (the first
//! payment activates the enrolment). No payment gateway adapter is installed
//! in the demo, so the one card payment that the cardholder disputes is
//! inserted as the gateway's webhook would leave it (`insert_in_state`).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_invoicing::Invoice;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus};
use wl_kernel::payments::{
    AllocationTarget, Direction, PaymentMethod, PaymentSource, PaymentStatus, RecordPayment,
    payment_recorder,
};
use wl_payments::Payment;

use super::super::demo_data::{at, day};
use super::{ACCOUNTANT_LOGIN, ADMIN_LOGIN, COUNSELLOR_LOGIN, Seed};

/// Dirhams as the forms read them: `2520` or `2520.50`.
pub(crate) fn aed(fils: i64) -> String {
    if fils % 100 == 0 {
        (fils / 100).to_string()
    } else {
        format!("{}.{:02}", fils / 100, fils % 100)
    }
}

impl Seed {
    /// The id of the enrolment of the story `key`.
    pub(crate) fn enrolment(&self, key: &str) -> i64 {
        *self
            .enrolments
            .get(key)
            .unwrap_or_else(|| panic!("the demo enrolment {key}"))
    }

    /// The enrolment as it is stored now.
    pub(crate) async fn enrolment_row(&self, key: &str) -> Result<edtech::Enrolment> {
        edtech::Enrolment::find_or_404(self.db(), self.enrolment(key)).await
    }

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

    /// Records `amount` fils (the whole balance when `None`) from the payer
    /// of invoice `id` on `on` and allocates it to the invoice.
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
        let who = if method == PaymentMethod::Cash {
            COUNSELLOR_LOGIN
        } else {
            ACCOUNTANT_LOGIN
        };
        let user = self.user(who).id;
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
        let when: NaiveDate = day(on);
        request = request.date(when);
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

    /// Pays the first invoice of the enrolment `key`.
    pub(crate) async fn pay_enrolment(
        &self,
        key: &str,
        method: PaymentMethod,
        amount: Option<i64>,
        on: i64,
    ) -> Result<i64> {
        let row = self.enrolment_row(key).await?;
        let invoice = row
            .first_invoice_id
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: {key} has no invoice"))?;
        self.pay_invoice(invoice, method, amount, on, "").await
    }

    /// A card payment that came through the online gateway for the first
    /// invoice of `key`, allocated to it. No gateway adapter is installed in
    /// the demo, so the payment is inserted received, as its webhook would
    /// leave it; the invoice and the enrolment follow from the allocation.
    pub(crate) async fn gateway_payment(&self, key: &str, on: i64, reference: &str) -> Result<i64> {
        let row = self.enrolment_row(key).await?;
        let invoice_id = row
            .first_invoice_id
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: {key} has no invoice"))?;
        let invoice = self.issue_invoice(invoice_id, on).await?;
        let mut payment = Payment {
            number: Some(format!("RCT-GW-{reference}")),
            direction: Direction::In,
            legal_entity_id: self.entity,
            party_id: invoice.party_id,
            method: PaymentMethod::Gateway,
            amount: invoice.balance,
            currency: "AED".to_owned(),
            payment_date: day(on),
            reference: Some(reference.to_owned()),
            source: PaymentSource::Gateway,
            gateway: Some("demo_gateway".to_owned()),
            gateway_transaction: Some(reference.to_owned()),
            status: PaymentStatus::Received,
            ..Payment::default()
        };
        let mut tx = self.db().begin().await?;
        payment
            .insert_in_state(&mut tx, PaymentStatus::Received, "demo seed")
            .await?;
        tx.commit().await?;
        let amount = payment.amount;
        at!(self, day(on), {
            payment_recorder(&self.state)
                .allocate(
                    &self.state,
                    wl_kernel::payments::PaymentId(payment.id),
                    AllocationTarget::Invoice(InvoiceId(invoice.id)),
                    amount,
                )
                .await
        })?;
        Ok(payment.id)
    }
}
