//! The money of the demo clinic: the patients' invoices paid by card and in
//! cash (one only in part), two packages sold, paid and activated, a payment
//! from the online gateway that the cardholder disputes, and the deposits
//! recorded with the bookings (one forfeited, one refunded).
//!
//! Payments are recorded through wl-payments' recorder as the front desk and
//! the accountant, allocated to the invoices, and the packages follow the
//! payment of their invoice. No payment gateway is installed in the demo, so
//! no payment link can be made; the chargeback is recorded on a payment that
//! came through the gateway.

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_invoicing::Invoice;
use wl_kernel::invoicing::{
    DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};
use wl_kernel::payments::{
    AllocationTarget, Direction, PaymentMethod, PaymentSource, PaymentStatus, RecordPayment,
    payment_recorder,
};
use wl_kernel::{PartyId, Subject};
use wl_payments::Payment;
use wl_sales::packages::{self, NewPackage};

use super::super::demo_data::day;
use super::Seed;
use super::people::{DANIEL, ELENA, SOFIA};

const ACCOUNTANT: &str = "accountant@palmgrove.example";
const ADMIN: &str = "admin@palmgrove.example";

impl Seed {
    /// Records `amount` fils from `party` and allocates it to `invoice`.
    async fn pay(
        &self,
        invoice: &Invoice,
        method: PaymentMethod,
        amount: i64,
        reference: &str,
        on: Option<renox::chrono::NaiveDate>,
    ) -> Result {
        let user = self.user(ACCOUNTANT).id;
        let mut request =
            RecordPayment::new(PartyId(invoice.party_id), self.entity, method, amount)
                .reference(reference)
                .collected_by(user);
        if let Some(on) = on {
            request = request.date(on);
        }
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
        Ok(())
    }

    /// Sells `credits` sessions as a package, bills and issues the invoice,
    /// and has it paid on `paid_on`; the payment activates the package.
    async fn package(
        &self,
        patient: usize,
        item: (&str, &str),
        credits: i64,
        price: i64,
        months: i64,
        paid_on: i64,
    ) -> Result {
        let party = self.patients[patient];
        let desk = self.user("reception.jumeirah@palmgrove.example").id;
        let package = packages::sell(
            &self.state,
            NewPackage::new(party, item.0, credits)
                .item_name(item.1)
                .price(price)
                .validity_months(months)
                .sold_by(desk),
        )
        .await?;
        let draft = InvoiceDraft::new(party, Subject::new("sales.package", package.id))
            .line(DraftLine::new(item.1, price, VatCategory::ZeroRated))
            .due_date(day(paid_on));
        let id = invoicer(&self.state)
            .draft(self.db(), draft)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("invoicing is off"))?;
        packages::link_invoice(&self.state, package.id, id).await?;
        self.driver
            .post(ADMIN, &format!("/invoices/{}/issue", id.0), &[])
            .await?;
        let invoice = Invoice::find_or_404(self.db(), id.0).await?;
        self.pay(
            &invoice,
            PaymentMethod::Cash,
            invoice.gross_total,
            "",
            Some(day(paid_on)),
        )
        .await
    }

    pub(crate) async fn money(&mut self) -> Result {
        // The invoices the visits left (the patient's share): the first paid
        // by card, the second in cash, the third half, the rest owed.
        let issued = Invoice::query()
            .where_(Invoice::STATUS.eq(InvoiceStatus::Issued))
            .order_by(Invoice::ID)
            .get(self.db())
            .await?;
        for (index, invoice) in issued.iter().enumerate() {
            match index {
                0 => {
                    self.pay(
                        invoice,
                        PaymentMethod::Card,
                        invoice.balance,
                        &format!("POS-{:05}", invoice.id),
                        None,
                    )
                    .await?;
                }
                1 => {
                    self.pay(invoice, PaymentMethod::Cash, invoice.balance, "", None)
                        .await?
                }
                2 => {
                    self.pay(invoice, PaymentMethod::Cash, invoice.balance / 2, "", None)
                        .await?;
                }
                _ => {}
            }
        }
        // Packages: six physiotherapy sessions, and one that ends this week.
        self.package(
            SOFIA,
            ("physio_session", "Physiotherapy, 6 sessions"),
            6,
            150_000,
            3,
            -12,
        )
        .await?;
        self.package(
            ELENA,
            ("iv_vitamin", "Vitamin drip, 4 sessions"),
            4,
            120_000,
            1,
            -26,
        )
        .await?;
        // A card payment through the gateway that the cardholder disputes.
        // No gateway adapter is installed in the demo (the method is not
        // enabled for the desk), so the payment the gateway would have made
        // is inserted received, as its webhook would leave it.
        let mut payment = Payment {
            number: Some("RCT-GW-0001".to_owned()),
            direction: Direction::In,
            legal_entity_id: self.entity,
            party_id: self.patients[DANIEL].0,
            method: PaymentMethod::Gateway,
            amount: 45_000,
            currency: "AED".to_owned(),
            payment_date: day(-6),
            reference: Some("GW-77120".to_owned()),
            source: PaymentSource::Gateway,
            gateway: Some("demo_gateway".to_owned()),
            gateway_transaction: Some("GW-77120".to_owned()),
            status: PaymentStatus::Received,
            ..Payment::default()
        };
        let mut tx = self.db().begin().await?;
        payment
            .insert_in_state(&mut tx, PaymentStatus::Received, "demo seed")
            .await?;
        tx.commit().await?;
        self.driver
            .post(
                ACCOUNTANT,
                &format!("/payments/{}/chargeback", payment.id),
                &[
                    ("reference", "CB-2026-0098"),
                    ("deadline", &day(10).to_string()),
                    ("fee", "50.00"),
                ],
            )
            .await?;
        Ok(())
    }
}
