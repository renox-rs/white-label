//! A chargeback with the real wl-invoicing and wl-payments in one app
//! (wl-payments #97, #98): an online payment allocated to a real invoice is
//! charged back and the invoice reopens; a won dispute allocates it again; a
//! lost one leaves the invoice open; a won dispute whose invoice was written
//! off in the meantime stays as the party's credit and the accountant is told.
//!
//! The gateway is the module's `FakeGateway` and the dispute webhooks are the
//! service functions that the (later) webhook routes will call.

mod support;

use std::sync::Arc;

use renox::auth::{Permissions, permissions};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::{Invoice, WlInvoicing, permissions as perm};
use wl_kernel::invoicing::{
    DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};
use wl_kernel::payments::PaymentStatus;
use wl_kernel::{ClientConfig, LegalEntity, Subject};
use wl_payments::chargeback::{self, DisputeOpened};
use wl_payments::gateway::FakeGateway;
use wl_payments::payment_link::{self, NewLink};
use wl_payments::{Payment, WlPayments, chargeback_close, notices, online, permissions as pay};

use support::{BLUE_DUNES, QUIET_SANDS, Recording, TestDirectory};

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "invoicing", "payments"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
trn = "100000000000003"
default = true
"#;

/// AED 800.00, no VAT.
const AMOUNT: i64 = 80_000;

struct Real {
    app: TestApp,
    notices: Recording,
    _views: tempfile::TempDir,
}

/// wl-invoicing and wl-payments (with the fake gateway) in one app.
async fn boot() -> Real {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    // The public page of a payment link has a layout of its own.
    std::fs::write(views.path().join("layouts/public.html"), LAYOUT).unwrap();
    let directory: Arc<dyn wl_kernel::parties::PartyDirectory> = Arc::new(TestDirectory::default());
    let notices = Recording::default();
    let notifier: Arc<dyn wl_kernel::notify::Notifier> = Arc::new(notices.clone());
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(WlInvoicing::new())
        .module(
            WlPayments::new()
                .gateway(FakeGateway::new())
                .gateway_party(QUIET_SANDS),
        )
        .provide(directory)
        .provide(notifier);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    Real {
        app,
        notices,
        _views: views,
    }
}

/// The accountant: issues and writes off invoices, and the one the payment
/// notices go to.
async fn accountant(t: &Real) -> User {
    let grants = [pay::VIEW, pay::RECORD, perm::VIEW, perm::ISSUE, perm::WRITE_OFF];
    let user = User::register(t.app.db(), "Rita", "rita@example.com", "password123")
        .await
        .unwrap();
    permissions::define_role(t.app.db(), "accountant", &grants)
        .await
        .unwrap();
    user.assign_role(t.app.db(), "accountant").await.unwrap();
    user
}

async fn entity(t: &Real) -> i64 {
    LegalEntity::query()
        .where_(LegalEntity::CODE.eq("PGP"))
        .first(t.app.db())
        .await
        .unwrap()
        .unwrap()
        .id
}

async fn invoice_now(t: &Real, id: i64) -> Invoice {
    Invoice::find(t.app.db(), id).await.unwrap().unwrap()
}

async fn payment_now(t: &Real, id: i64) -> Payment {
    Payment::find(t.app.db(), id).await.unwrap().unwrap()
}

/// INV-9: an issued invoice of Blue Dunes for AED 800.00, paid in full by an
/// online payment through a payment link, then charged back by the dispute
/// `dp_1` (evidence due in ten days). The invoice is reopened.
async fn charged_back(t: &Real, by: &User) -> (Payment, Invoice) {
    let draft = InvoiceDraft::new(BLUE_DUNES, Subject::new("property.lease", 9)).line(
        DraftLine::new("Consultation", AMOUNT, VatCategory::ZeroRated),
    )
    .due_date(wl_kernel::documents::today() - Days::new(5));
    let id = invoicer(t.app.state())
        .draft(t.app.db(), draft)
        .await
        .unwrap()
        .unwrap();
    t.app.acting_as(by);
    t.app
        .htmx()
        .post(&format!("/invoices/{}/issue", id.0), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", id.0));

    let link = payment_link::create(
        t.app.state(),
        NewLink::new(BLUE_DUNES, entity(t).await, AMOUNT, "AED")
            .target(&InvoiceId(id.0).subject(), AMOUNT)
            .created_by(by.id),
    )
    .await
    .expect("the link is made");
    t.app.logout();
    t.app
        .htmx()
        .post(&format!("{}/start", payment_link::path(t.app.state(), link.id)), &[])
        .await;
    let started = Payment::query()
        .where_(Payment::PAYMENT_LINK_ID.eq(link.id))
        .first(t.app.db())
        .await
        .unwrap()
        .expect("a payment on the link");
    assert_eq!(started.status, PaymentStatus::Pending);
    let txn = started.gateway_transaction.clone().unwrap();
    online::gateway_succeeded(t.app.state(), &txn, AMOUNT)
        .await
        .unwrap()
        .expect("confirmed");
    assert_eq!(invoice_now(t, id.0).await.status, InvoiceStatus::Paid);

    let due: NaiveDate = wl_kernel::documents::today() + Days::new(10);
    let charged = chargeback::gateway_dispute_opened(
        t.app.state(),
        &txn,
        &DisputeOpened::new("dp_1").evidence_due(due),
    )
    .await
    .unwrap()
    .expect("charged back");
    (charged, invoice_now(t, id.0).await)
}

/// Done when (#97) 1: INV-9 is `paid` by the allocated payment; the dispute
/// charges the payment back and the real invoice reopens (AED 800.00 open again;
/// `overdue`, its due date having passed).
#[renox::test]
async fn a_chargeback_reopens_the_real_invoice() {
    let t = boot().await;
    let rita = accountant(&t).await;
    let (payment, invoice) = charged_back(&t, &rita).await;
    assert_eq!(payment.status, PaymentStatus::ChargedBack);
    assert_eq!(
        invoice.status,
        InvoiceStatus::Overdue,
        "open again, and past its due date"
    );
    assert_eq!(invoice.balance, AMOUNT);
}

/// Done when (#98) 1: a won dispute puts the payment back to `received` and
/// allocates it to the invoice again, which is `paid`.
#[renox::test]
async fn a_won_dispute_pays_the_real_invoice_again() {
    let t = boot().await;
    let rita = accountant(&t).await;
    let (payment, invoice) = charged_back(&t, &rita).await;

    chargeback_close::gateway_dispute_won(t.app.state(), "dp_1")
        .await
        .unwrap()
        .expect("won");
    assert_eq!(
        payment_now(&t, payment.id).await.status,
        PaymentStatus::Received
    );
    let after = invoice_now(&t, invoice.id).await;
    assert_eq!(after.status, InvoiceStatus::Paid);
    assert_eq!(after.balance, 0);
}

/// Done when (#98) 3: a lost dispute reverses the payment and the invoice
/// stays open.
#[renox::test]
async fn a_lost_dispute_leaves_the_real_invoice_open() {
    let t = boot().await;
    let rita = accountant(&t).await;
    let (payment, invoice) = charged_back(&t, &rita).await;

    chargeback_close::gateway_dispute_lost(t.app.state(), "dp_1")
        .await
        .unwrap()
        .expect("lost");
    assert_eq!(
        payment_now(&t, payment.id).await.status,
        PaymentStatus::Reversed
    );
    let after = invoice_now(&t, invoice.id).await;
    assert_eq!(after.status, InvoiceStatus::Overdue);
    assert_eq!(after.balance, AMOUNT);
}

/// Edge case (#98): the invoice was written off while the dispute ran. The
/// won dispute cannot be applied to it: the payment is `received` again, its
/// AED 800.00 stays unallocated as the party's credit, the invoice stays
/// written off and the accountant is told.
#[renox::test]
async fn a_won_dispute_whose_invoice_was_written_off_stays_as_credit() {
    let t = boot().await;
    let rita = accountant(&t).await;
    let (payment, invoice) = charged_back(&t, &rita).await;
    // Overdue since the reopening; written off through a second person's
    // approval (`FIN.ACL.15`).
    t.app.acting_as(&rita);
    t.app
        .htmx()
        .post(
            &format!("/invoices/{}/write-off", invoice.id),
            &[("reason_code", "insolvent"), ("reason_text", "")],
        )
        .await
        .assert_ok();
    let wade = User::register(t.app.db(), "Wade", "wade@example.com", "password123")
        .await
        .unwrap();
    // The write-off approvers are the holders of the `receivables` role.
    permissions::define_role(t.app.db(), "receivables", &[perm::VIEW, perm::WRITE_OFF])
        .await
        .unwrap();
    wade.assign_role(t.app.db(), "receivables").await.unwrap();
    let request = wl_kernel::approvals::pending_for(t.app.db(), &invoice.subject())
        .await
        .unwrap()
        .remove(0);
    wl_kernel::approvals::approve(t.app.state(), request.id, &wade, "")
        .await
        .unwrap()
        .expect("the approval runs");
    assert_eq!(
        invoice_now(&t, invoice.id).await.status,
        InvoiceStatus::WrittenOff
    );
    let before = t.notices.of_type(notices::ONLINE_TASK).len();

    chargeback_close::gateway_dispute_won(t.app.state(), "dp_1")
        .await
        .unwrap()
        .expect("won");
    let received = payment_now(&t, payment.id).await;
    assert_eq!(received.status, PaymentStatus::Received);
    let after = invoice_now(&t, invoice.id).await;
    assert_eq!(after.status, InvoiceStatus::WrittenOff);
    assert_eq!(
        wl_payments::credit::unallocated(t.app.db(), &received)
            .await
            .unwrap(),
        AMOUNT,
        "the money stays as the party's credit"
    );
    assert!(t.notices.of_type(notices::ONLINE_TASK).len() > before);
}
