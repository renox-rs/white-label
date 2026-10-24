//! A package whose payment is reversed, with the real wl-invoicing (BL-SAL-16,
//! #135): the invoice is issued and paid by the real module, the package is
//! activated by its `InvoicePaid`, and the reopening comes from the real
//! `Invoicer::unallocate` of a reversed payment, not from a hand-built
//! `InvoiceReopened`.
//!
//! Seam: the payment itself is not a real wl-payments payment (only its
//! allocation to the invoice and the unallocation are called, as the
//! reversal listener of wl-payments does), and the seller's task and the
//! holder's message are recorded by fakes; the real tickets and notifications
//! need a vertical (BL-CLN-16).

mod support;

use std::sync::{Arc, Mutex};

use renox::auth::{Auth, Permissions, permissions};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::{Invoice, WlInvoicing, permissions as perm};
use wl_kernel::invoicing::{
    DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, ReopenReason, Unallocate, VatCategory,
    invoicer,
};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRequest};
use wl_kernel::parties::{BillingIdentity, PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::tickets::{OpenTicket, TaskComment, TaskCreator, TicketFuture, TicketId, TicketRef};
use wl_kernel::{ClientConfig, PartyId, Subject};
use wl_sales::notices::PACKAGE_BALANCE_DUE;
use wl_sales::packages::{self, NewPackage};
use wl_sales::{Package, PackageStatus, WlSales};

const HOLDER: PartyId = PartyId(7);

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "invoicing", "sales"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
trn = "100000000000003"
default = true
"#;

/// The one party that is billed.
struct Directory;

impl PartyDirectory for Directory {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            Ok((party == HOLDER).then(|| PartySummary {
                id: party,
                code: "P-00007".to_owned(),
                kind: wl_kernel::parties::PartyKind::Person,
                display_name: "Omar Hassan".to_owned(),
                status: PartyStatus::Active,
                display_block_reason: None,
            }))
        })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        _code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn status<'a>(&'a self, _db: &'a Db, _party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async { Ok(Some(PartyStatus::Active)) })
    }

    fn billing_identity<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<BillingIdentity>> {
        Box::pin(async move {
            Ok(Some(BillingIdentity {
                party,
                legal_name_en: "Omar Hassan".to_owned(),
                legal_name_ar: None,
                trn: None,
                billing_address: Some("Villa 4, Dubai".to_owned()),
                billing_email: None,
                einvoice_recipient_id: None,
            }))
        })
    }
}

/// The tasks opened, as ticketing would keep them.
#[derive(Clone, Default)]
struct Desk(Arc<Mutex<Vec<OpenTicket>>>);

impl TaskCreator for Desk {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut opened = self.0.lock().unwrap();
            opened.push(ticket);
            let n = opened.len() as i64;
            Ok(Some(TicketRef {
                id: TicketId(n),
                number: format!("TKT-{n}"),
            }))
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        _source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async { Ok(None) })
    }

    fn comment<'a>(
        &'a self,
        _state: &'a AppState,
        _ticket: TicketId,
        _comment: TaskComment,
    ) -> TicketFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

/// Every notification asked for.
#[derive(Clone, Default)]
struct Mail(Arc<Mutex<Vec<NotifyRequest>>>);

impl Notifier for Mail {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request);
            Ok(vec![MessageId(1)])
        })
    }
}

struct Real {
    app: TestApp,
    desk: Desk,
    mail: Mail,
    _views: tempfile::TempDir,
}

/// wl-invoicing and wl-sales in one app.
async fn boot() -> Real {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let desk = Desk::default();
    let mail = Mail::default();
    let directory: Arc<dyn PartyDirectory> = Arc::new(Directory);
    let creator: Arc<dyn TaskCreator> = Arc::new(desk.clone());
    let notifier: Arc<dyn Notifier> = Arc::new(mail.clone());
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(WlInvoicing::new())
        .module(WlSales::new())
        .provide(directory)
        .provide(creator)
        .provide(notifier);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    support::stand_in_parties(&app).await;
    Real {
        app,
        desk,
        mail,
        _views: views,
    }
}

/// A package of 10 credits sold on a real invoice of AED 1,500.00 (no VAT),
/// the invoice issued and paid in full by payment 42, which activates the
/// package; 4 credits used.
async fn paid_package(t: &Real) -> (Package, Invoice) {
    let user = User::register(t.app.db(), "Vera", "vera@example.com", "password123")
        .await
        .unwrap();
    permissions::define_role(
        t.app.db(),
        "role_vera",
        &[perm::VIEW, perm::CREATE, perm::ISSUE],
    )
    .await
    .unwrap();
    user.assign_role(t.app.db(), "role_vera").await.unwrap();

    let draft = InvoiceDraft::new(HOLDER, Subject::new("sales.package", 1)).line(DraftLine::new(
        "Physio, 10 sessions",
        150_000,
        VatCategory::ZeroRated,
    ));
    let id = invoicer(t.app.state())
        .draft(t.app.db(), draft)
        .await
        .expect("the draft is made")
        .expect("invoicing is on");
    t.app.acting_as(&user);
    t.app
        .htmx()
        .post(&format!("/invoices/{}/issue", id.0), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", id.0));

    let package = packages::sell(
        t.app.state(),
        NewPackage::new(HOLDER, "physio_10", 10)
            .item_name("Physio, 10 sessions")
            .price(150_000)
            .invoice(id),
    )
    .await
    .unwrap();
    assert_eq!(package.id, 1, "the invoice names package 1 as its source");

    invoicer(t.app.state())
        .allocate(
            t.app.state(),
            id,
            Subject::new("payments.payment", 42),
            150_000,
        )
        .await
        .expect("the payment is allocated");
    let paid = Invoice::find_or_404(t.app.db(), id.0).await.unwrap();
    assert_eq!(paid.status, InvoiceStatus::Paid);
    let active = Package::find_or_404(t.app.db(), package.id).await.unwrap();
    assert_eq!(
        active.status,
        PackageStatus::Active,
        "the paid invoice activates it"
    );
    packages::consume(
        t.app.state(),
        package.id,
        4,
        Subject::new("scheduling.booking", 1),
    )
    .await
    .unwrap();
    (
        Package::find_or_404(t.app.db(), package.id).await.unwrap(),
        paid,
    )
}

/// The reversal of payment 42 reopens the invoice (what wl-payments asks for).
async fn reverse(t: &Real, invoice: &Invoice) {
    invoicer(t.app.state())
        .unallocate(
            t.app.state(),
            Unallocate::new(
                InvoiceId(invoice.id),
                Subject::new("payments.payment", 42),
                150_000,
                "payments.allocation:11",
                ReopenReason::PaymentReversed,
            ),
        )
        .await
        .expect("the invoice is reopened");
}

/// Done when 1, with the real reopen: the reversed payment reopens the real
/// invoice, the package stays `active` with 6 credits, the seller's task is
/// opened once and the holder is told; paying the invoice again clears the
/// balance and a consume works.
#[renox::test]
async fn a_real_reversal_reopens_the_invoice_and_the_package_asks_for_the_balance() {
    let t = boot().await;
    let (package, invoice) = paid_package(&t).await;
    assert_eq!(package.remaining(), 6);

    reverse(&t, &invoice).await;

    let reopened = Invoice::find_or_404(t.app.db(), invoice.id).await.unwrap();
    assert_eq!(reopened.status, InvoiceStatus::Issued);
    assert_eq!(reopened.balance, 150_000);
    let after = Package::find_or_404(t.app.db(), package.id).await.unwrap();
    assert_eq!(after.status, PackageStatus::Active);
    assert_eq!((after.credits_used, after.remaining()), (4, 6));
    assert_eq!(after.reopened_balance, 150_000);
    assert_eq!(t.desk.0.lock().unwrap().len(), 1);
    let notices: Vec<_> = t
        .mail
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.type_key == PACKAGE_BALANCE_DUE)
        .cloned()
        .collect();
    assert_eq!(notices.len(), 1);

    // The same unallocation again changes nothing.
    reverse(&t, &invoice).await;
    assert_eq!(t.desk.0.lock().unwrap().len(), 1);

    // Paid again: the balance is cleared and credits can be used.
    invoicer(t.app.state())
        .allocate(
            t.app.state(),
            InvoiceId(invoice.id),
            Subject::new("payments.payment", 43),
            150_000,
        )
        .await
        .expect("paid again");
    let cleared = Package::find_or_404(t.app.db(), package.id).await.unwrap();
    assert_eq!(cleared.reopened_balance, 0);
    packages::consume(
        t.app.state(),
        package.id,
        1,
        Subject::new("scheduling.booking", 2),
    )
    .await
    .expect("consume works again");
}
