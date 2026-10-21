//! What the payer bill tests (#79, #80, #81) share: the events of the payer
//! bills, a payer agreement, the users of the billing desk, a payer bill made
//! through the kernel contract as a vertical makes it, and the pages' posts.

use std::sync::atomic::{AtomicI64, Ordering};

use renox::auth::permissions;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::payer_bill::lines_of;
use wl_invoicing::permissions as perm;
use wl_invoicing::{CreditNote, Invoice, PayerAgreement, PayerBill, PayerBillLine, VatRecipient};
use wl_kernel::documents::today;
use wl_kernel::payer_bills::{
    NewPayerBill, NewPayerBillLine, PayerBillClosed, PayerBillId, PayerBillIssued,
    PayerBillSubmissionRejected, PayerRecoupmentRecorded, PayerRemittanceRecorded, payer_bills,
};
use wl_kernel::{PartyId, Subject};

use super::{BLUE_DUNES, Booted, OMAR, Recorded, record};

/// What the payer bills emitted.
pub struct PayerEvents {
    /// `PayerBillIssued`.
    pub issued: Recorded<PayerBillIssued>,
    /// `PayerBillSubmissionRejected`.
    pub rejected: Recorded<PayerBillSubmissionRejected>,
    /// `PayerRemittanceRecorded`.
    pub remitted: Recorded<PayerRemittanceRecorded>,
    /// `PayerRecoupmentRecorded`.
    pub recouped: Recorded<PayerRecoupmentRecorded>,
    /// `PayerBillClosed`.
    pub closed: Recorded<PayerBillClosed>,
}

impl PayerEvents {
    /// Nothing emitted yet.
    pub(super) fn new() -> Self {
        PayerEvents {
            issued: Recorded::new(),
            rejected: Recorded::new(),
            remitted: Recorded::new(),
            recouped: Recorded::new(),
            closed: Recorded::new(),
        }
    }

    /// Records every payer bill event of `app`.
    pub(super) fn record(&self, app: App) -> App {
        let app = record(app, &self.issued);
        let app = record(app, &self.rejected);
        let app = record(app, &self.remitted);
        let app = record(app, &self.recouped);
        record(app, &self.closed)
    }
}

/// A day, from `YYYY-MM-DD`.
pub fn d(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// The next claim id: each bill is for a claim of its own.
fn next_claim() -> i64 {
    static CLAIM: AtomicI64 = AtomicI64::new(100);
    CLAIM.fetch_add(1, Ordering::SeqCst)
}

impl Booted {
    /// A payer agreement with `payer` under the default legal entity: the
    /// payer pays 30 days after the service at the latest for a first claim
    /// and 15 days after a remittance for a resubmission, twice at most; a
    /// write-off above AED 1,000.00 needs an approval; late claims and
    /// billing the customer are not allowed.
    pub async fn agreement(&self, payer: PartyId, recipient: VatRecipient) -> PayerAgreement {
        let entity = self.entity("PGP").await;
        let mut agreement = PayerAgreement {
            legal_entity_id: entity.id,
            payer_party_id: payer.0,
            name: "Daman Gold".to_owned(),
            active: true,
            vat_recipient: recipient,
            first_submission_days: 30,
            resubmission_days: 15,
            resubmission_limit: 2,
            write_off_limit: 100_000,
            ..Default::default()
        };
        agreement.insert(self.db()).await.expect("the agreement");
        agreement
    }

    /// Changes `agreement` with `change`, as stored.
    pub async fn change_agreement(
        &self,
        agreement: &PayerAgreement,
        change: impl FnOnce(&mut PayerAgreement),
    ) -> PayerAgreement {
        let mut changed = PayerAgreement::find(self.db(), agreement.id)
            .await
            .unwrap()
            .unwrap();
        change(&mut changed);
        changed
            .save(self.db())
            .await
            .expect("the agreement is saved");
        changed
    }

    /// The billing coordinator: sees payer bills, issues them and records a
    /// rejection; holds the `billing_coordinator` role the task goes to.
    pub async fn coordinator_cora(&self) -> User {
        let grants = [perm::PAYER_BILLS_VIEW, perm::PAYER_BILLS_ISSUE];
        let user = self.user("Cora", &grants).await;
        permissions::define_role(self.db(), "billing_coordinator", &grants)
            .await
            .unwrap();
        user.assign_role(self.db(), "billing_coordinator")
            .await
            .unwrap();
        user
    }

    /// A reconciler: records remittances and recoupments.
    pub async fn reconciler(&self, name: &str) -> User {
        self.user(name, &[perm::PAYER_BILLS_VIEW, perm::PAYER_BILLS_RECONCILE])
            .await
    }

    /// `by` records a real remittance (reference `reference`, 2026-12-10)
    /// paying `amount` on the first line of `bill`, through the page.
    pub async fn remit_first_line(
        &self,
        bill: &PayerBill,
        by: &User,
        reference: &str,
        amount: &str,
    ) -> renox::testing::TestResponse {
        let line = self.bill_lines(bill).await.remove(0);
        let field = format!("line_{}_paid", line.id);
        self.post_payer(
            bill,
            by,
            "remittance",
            &[
                ("reference", reference),
                ("date", "2026-12-10"),
                (field.as_str(), amount),
            ],
        )
        .await
    }

    /// A user who writes off or moves the open amount of a payer bill.
    pub async fn payer_writer(&self, name: &str) -> User {
        self.user(
            name,
            &[
                perm::PAYER_BILLS_VIEW,
                perm::PAYER_BILLS_ISSUE,
                perm::PAYER_BILLS_WRITE_OFF,
            ],
        )
        .await
    }

    /// A draft payer bill of `agreement` for a new claim of the patient
    /// Omar, for a service on `service`, with one line per `(description,
    /// claimed fils)`; made through the kernel contract.
    pub async fn payer_bill(
        &self,
        agreement: &PayerAgreement,
        service: NaiveDate,
        lines: &[(&str, i64)],
    ) -> PayerBill {
        self.payer_bill_for(agreement, OMAR, service, lines, None)
            .await
    }

    /// [`Booted::payer_bill`] for `customer`, carried by line positions of
    /// the customer's tax invoice `invoice` when given.
    pub async fn payer_bill_for(
        &self,
        agreement: &PayerAgreement,
        customer: PartyId,
        service: NaiveDate,
        lines: &[(&str, i64)],
        invoice: Option<&Invoice>,
    ) -> PayerBill {
        let mut request = NewPayerBill::new(
            agreement.id,
            customer,
            Subject::new("clinics.claim", next_claim()),
            service,
        );
        if let Some(invoice) = invoice {
            request = request.customer_invoice(wl_kernel::invoicing::InvoiceId(invoice.id));
        }
        for (position, (description, claimed)) in (1_i64..).zip(lines) {
            let mut line = NewPayerBillLine::new(description, *claimed);
            if invoice.is_some() {
                line = line.on_invoice_line(position);
            }
            request = request.line(line);
        }
        let id = payer_bills(self.state())
            .create(self.state(), request, None)
            .await
            .expect("the payer bill is made");
        self.payer_bill_now(id).await
    }

    /// Payer bill `id`, as stored now.
    pub async fn payer_bill_now(&self, id: PayerBillId) -> PayerBill {
        PayerBill::find(self.db(), id.0).await.unwrap().unwrap()
    }

    /// `bill`, as stored now.
    pub async fn reload_bill(&self, bill: &PayerBill) -> PayerBill {
        self.payer_bill_now(PayerBillId(bill.id)).await
    }

    /// The lines of `bill`, as stored now.
    pub async fn bill_lines(&self, bill: &PayerBill) -> Vec<PayerBillLine> {
        lines_of(self.db(), bill.id).await.unwrap()
    }

    /// `POST /payer-bills/{id}/{action}` as `by`, as the page's sheet sends
    /// it.
    pub async fn post_payer(
        &self,
        bill: &PayerBill,
        by: &User,
        action: &str,
        fields: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/payer-bills/{}/{action}", bill.id), fields)
            .await
    }

    /// Issues `bill` as `by` and returns it as stored.
    pub async fn issue_payer_bill(&self, bill: &PayerBill, by: &User) -> PayerBill {
        self.post_payer(bill, by, "issue", &[])
            .await
            .assert_redirect(&format!("/payer-bills/{}", bill.id));
        self.reload_bill(bill).await
    }

    /// The invoices that bill `bill` (the payer's tax invoices), oldest
    /// first.
    pub async fn payer_invoices(&self, bill: &PayerBill) -> Vec<Invoice> {
        Invoice::query()
            .where_(Invoice::SOURCE_TYPE.eq(bill.subject().subject_type.as_str()))
            .where_(Invoice::SOURCE_ID.eq(bill.id))
            .order_by(Invoice::ID)
            .get(self.db())
            .await
            .unwrap()
    }

    /// The credit notes against `invoice`, oldest first.
    pub async fn credit_notes_of(&self, invoice: &Invoice) -> Vec<CreditNote> {
        wl_invoicing::credit_notes::for_invoice(self.db(), invoice.id)
            .await
            .unwrap()
    }

    /// The default payer: Blue Dunes, whose tax invoices have a billing
    /// identity.
    pub fn payer(&self) -> PartyId {
        BLUE_DUNES
    }

    /// The service day of a bill whose first-submission window is open:
    /// today.
    pub fn service_today(&self) -> NaiveDate {
        today()
    }
}
