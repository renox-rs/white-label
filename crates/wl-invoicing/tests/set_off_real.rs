//! Setting off a party's invoice against its supplier bill (wl-payments #117,
//! `PTY.ACL.03`), with the real wl-invoicing and wl-payments in one app: one
//! payment of method `set_off`, no bank account, allocated at once to the
//! invoice (`allocate`) and the supplier bill (`pay`); `reverse` reopens both;
//! a blocked party waits for an approved exception; the method is off by
//! default.

mod support;

use std::sync::{Arc, Mutex};

use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::{TestApp, TestResponse};
use wl_invoicing::bill_machine::approve as approve_bill;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::supplier_bill::{self, BillDraft, SupplierBillStatus};
use wl_invoicing::{BillLineDraft, Invoice, SupplierBill, WlInvoicing, permissions as perm};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::block_guard::{ActionKind, BlockFuture, BlockGuard, BlockRefusal, ExceptionStep};
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceStatus, VatCategory, invoicer};
use wl_kernel::payments::{
    Direction, PaymentMethod, PaymentStatus, RecordPayment, payment_recorder,
};
use wl_kernel::{BLOCK_EXCEPTION_ACTION, ClientConfig, LegalEntity, PartyId, Subject};
use wl_payments::{Allocation, Payment, WlPayments, permissions as pay};

use support::{BLUE_DUNES, Recording, TestDirectory};

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

/// AED 100,000.00 plus 5% VAT: INV-0410, AED 105,000.00 open.
const INVOICE_NET: i64 = 10_000_000;
/// AED 5,000.00 plus 5% VAT: SB-77, AED 5,250.00.
const BILL_NET: i64 = 500_000;

/// The parties the test block guard refuses money out to.
#[derive(Clone, Default)]
struct Blocked(Arc<Mutex<Vec<i64>>>);

impl BlockGuard for Blocked {
    fn check_in<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        _viewer: Option<&'a User>,
        party: PartyId,
        kind: ActionKind,
    ) -> BlockFuture<'a, wl_kernel::block_guard::GuardOutcome> {
        Box::pin(async move {
            let blocked = kind.is_guarded() && self.0.lock().unwrap().contains(&party.0);
            Ok(if blocked {
                wl_kernel::block_guard::GuardOutcome::Blocked(BlockRefusal::neutral(
                    party,
                    kind,
                    "Compliance",
                ))
            } else {
                wl_kernel::block_guard::GuardOutcome::Allowed
            })
        })
    }

    fn exception_step(&self) -> ExceptionStep {
        ExceptionStep::new("compliance")
    }
}

struct Real {
    app: TestApp,
    _views: tempfile::TempDir,
    blocked: Blocked,
}

/// Both modules in one app; `module` is the payments module.
async fn boot_with(module: WlPayments) -> Real {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let directory: Arc<dyn wl_kernel::parties::PartyDirectory> =
        Arc::new(TestDirectory::default());
    let notifier: Arc<dyn wl_kernel::notify::Notifier> = Arc::new(Recording::default());
    let blocked = Blocked::default();
    let guard: Arc<dyn BlockGuard> = Arc::new(blocked.clone());
    // What wl-parties registers: an exception is decided by the compliance role.
    wl_kernel::PolicyDefault::new(BLOCK_EXCEPTION_ACTION)
        .with_step(
            approvals::Step::one_of(["compliance"]).requiring(BLOCK_EXCEPTION_ACTION),
        )
        .register();
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(WlInvoicing::new())
        .module(module)
        .provide(directory)
        .provide(notifier)
        .provide(guard);
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
        _views: views,
        blocked,
    }
}

/// The set-off method is on.
async fn boot() -> Real {
    boot_with(WlPayments::new().methods(&["cash", "transfer", "set_off"])).await
}

async fn user(t: &Real, name: &str, grants: &[&str]) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    if let Some(found) = User::find_by_email(t.app.db(), &email).await.unwrap() {
        return found;
    }
    let user = User::register(t.app.db(), name, &email, "password123")
        .await
        .unwrap();
    let role = format!("role_{}", name.to_lowercase());
    permissions::define_role(t.app.db(), &role, grants)
        .await
        .unwrap();
    user.assign_role(t.app.db(), &role).await.unwrap();
    user
}

/// The accountant: records and reverses payments, issues invoices.
async fn accountant(t: &Real) -> User {
    user(
        t,
        "Amal",
        &[
            pay::VIEW,
            pay::RECORD,
            pay::REVERSE,
            perm::VIEW,
            perm::ISSUE,
        ],
    )
    .await
}

/// A user of the compliance role, who may decide a block exception.
async fn compliance(t: &Real) -> User {
    permissions::define_role(t.app.db(), "compliance", &[BLOCK_EXCEPTION_ACTION])
        .await
        .unwrap();
    let user = User::register(t.app.db(), "Comply", "comply@example.com", "password123")
        .await
        .unwrap();
    user.assign_role(t.app.db(), "compliance").await.unwrap();
    user
}

async fn entity(t: &Real) -> LegalEntity {
    LegalEntity::query()
        .where_(LegalEntity::CODE.eq("PGP"))
        .first(t.app.db())
        .await
        .unwrap()
        .unwrap()
}

/// INV-0410: an issued invoice of Blue Dunes, AED 105,000.00.
async fn issued_invoice(t: &Real, by: &User) -> Invoice {
    let draft = InvoiceDraft::new(BLUE_DUNES, Subject::new("property.lease", 9)).line(
        DraftLine::new("IPA 14", INVOICE_NET, VatCategory::Standard),
    );
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
    Invoice::find(t.app.db(), id.0).await.unwrap().unwrap()
}

/// SB-77: a supplier bill from Blue Dunes, AED 5,250.00, submitted and, when
/// `approved`, approved.
async fn bill(t: &Real, approved: bool) -> SupplierBill {
    let entity = entity(t).await;
    let mut draft = BillDraft::new(
        BLUE_DUNES,
        "SB-77",
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(),
    )
    .line(BillLineDraft::new(
        "Back-charge",
        BILL_NET,
        VatCategory::Standard,
    ));
    draft.legal_entity_id = Some(entity.id);
    let bill = supplier_bill::create(t.app.db(), &draft, Some(7))
        .await
        .unwrap();
    submit(t.app.state(), By::System(Some(7)), bill.id)
        .await
        .unwrap();
    if approved {
        approve_bill(t.app.state(), By::System(Some(8)), bill.id, "")
            .await
            .unwrap()
    } else {
        SupplierBill::find_or_404(t.app.db(), bill.id).await.unwrap()
    }
}

/// `POST /payments` as `by`: a set-off of `amount` dirhams.
async fn post_set_off(
    t: &Real,
    by: &User,
    invoice: &Invoice,
    bill: &SupplierBill,
    amount: &str,
) -> TestResponse {
    let entity = entity(t).await.id.to_string();
    let invoice_id = invoice.id.to_string();
    let bill_id = bill.id.to_string();
    t.app.acting_as(by);
    t.app
        .htmx()
        .post(
            "/payments",
            &[
                ("legal_entity_id", entity.as_str()),
                ("party_code", "P-00001"),
                ("method", "set_off"),
                ("amount", amount),
                ("set_off_invoice_id", invoice_id.as_str()),
                ("set_off_bill_id", bill_id.as_str()),
            ],
        )
        .await
}

async fn reload_invoice(t: &Real, invoice: &Invoice) -> Invoice {
    Invoice::find(t.app.db(), invoice.id).await.unwrap().unwrap()
}

async fn reload_bill(t: &Real, bill: &SupplierBill) -> SupplierBill {
    SupplierBill::find_or_404(t.app.db(), bill.id).await.unwrap()
}

async fn payments(t: &Real) -> Vec<Payment> {
    Payment::query().order_by(Payment::ID).get(t.app.db()).await.unwrap()
}

/// Done when 1: INV-0410 (AED 105,000 open) and SB-77 (AED 5,250), a set-off
/// of AED 5,250 leaves INV-0410 `partially_paid` with AED 99,750 open and
/// SB-77 `paid`; no bank account is set.
#[renox::test]
async fn a_set_off_settles_the_bill_and_part_of_the_invoice() {
    let t = boot().await;
    let amal = accountant(&t).await;
    let invoice = issued_invoice(&t, &amal).await;
    assert_eq!(invoice.balance, 10_500_000);
    let sb = bill(&t, true).await;
    assert_eq!(sb.balance, 525_000);
    assert_eq!(sb.status, SupplierBillStatus::Approved);

    post_set_off(&t, &amal, &invoice, &sb, "5250.00")
        .await
        .assert_status(303);

    let after = reload_invoice(&t, &invoice).await;
    assert_eq!(after.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(after.balance, 9_975_000);
    let paid = reload_bill(&t, &sb).await;
    assert_eq!(paid.status, SupplierBillStatus::Paid);
    assert_eq!(paid.balance, 0);

    let recorded = payments(&t).await;
    assert_eq!(recorded.len(), 1);
    let payment = &recorded[0];
    assert_eq!(payment.method, PaymentMethod::SetOff);
    assert_eq!(payment.direction, Direction::In);
    assert_eq!(payment.status, PaymentStatus::Received);
    assert_eq!(payment.bank_account, None);
    assert_eq!(payment.amount, 525_000);
    assert!(payment.number.is_some());
    // One record, allocated to both documents.
    let rows = Allocation::all_of(t.app.db(), payment.id).await.unwrap();
    let mut targets: Vec<(String, i64)> = rows
        .iter()
        .filter(|row| row.is_active())
        .map(|row| (row.target().to_string(), row.amount))
        .collect();
    targets.sort();
    assert_eq!(
        targets,
        [
            (format!("invoicing.invoice:{}", invoice.id), 525_000),
            (format!("invoicing.supplier_bill:{}", sb.id), 525_000),
        ]
    );
}

/// The rest of either stays open: a smaller set-off leaves the bill partly paid.
#[renox::test]
async fn a_smaller_set_off_leaves_the_rest_of_both_open() {
    let t = boot().await;
    let amal = accountant(&t).await;
    let invoice = issued_invoice(&t, &amal).await;
    let sb = bill(&t, true).await;
    post_set_off(&t, &amal, &invoice, &sb, "2000.00")
        .await
        .assert_status(303);
    let after = reload_invoice(&t, &invoice).await;
    assert_eq!(after.balance, 10_300_000);
    let part = reload_bill(&t, &sb).await;
    assert_eq!(part.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!(part.balance, 325_000);
}

/// Done when 2: a set-off of AED 6,000 against SB-77 is refused, and nothing
/// is recorded. So is a bill that is not approved.
#[renox::test]
async fn an_amount_above_the_bill_and_an_unapproved_bill_are_refused() {
    let t = boot().await;
    let amal = accountant(&t).await;
    let invoice = issued_invoice(&t, &amal).await;
    let sb = bill(&t, true).await;
    post_set_off(&t, &amal, &invoice, &sb, "6000.00")
        .await
        .assert_status(422);
    post_set_off(&t, &amal, &invoice, &sb, "0")
        .await
        .assert_status(422);
    assert!(payments(&t).await.is_empty());
    assert_eq!(reload_invoice(&t, &invoice).await.balance, 10_500_000);
    assert_eq!(reload_bill(&t, &sb).await.balance, 525_000);

    // A bill that was submitted but not approved is refused too.
    let t = boot().await;
    let amal = accountant(&t).await;
    let invoice = issued_invoice(&t, &amal).await;
    let held = bill(&t, false).await;
    post_set_off(&t, &amal, &invoice, &held, "5250.00")
        .await
        .assert_status(422);
    assert!(payments(&t).await.is_empty());
}

/// A set-off names both documents; the form without them is refused.
#[renox::test]
async fn a_set_off_without_its_documents_is_refused() {
    let t = boot().await;
    let amal = accountant(&t).await;
    let entity = entity(&t).await.id.to_string();
    t.app.acting_as(&amal);
    t.app
        .htmx()
        .post(
            "/payments",
            &[
                ("legal_entity_id", entity.as_str()),
                ("party_code", "P-00001"),
                ("method", "set_off"),
                ("amount", "5250.00"),
            ],
        )
        .await
        .assert_status(422);
    // Nor does the kernel's recorder make one without them.
    let refused = payment_recorder(t.app.state())
        .record(
            t.app.state(),
            RecordPayment::new(BLUE_DUNES, entity.parse().unwrap(), PaymentMethod::SetOff, 500),
            None,
        )
        .await;
    assert!(refused.is_err());
    assert!(payments(&t).await.is_empty());
}

/// Done when 3: `reverse` of the set-off with reason *entered in error*
/// reopens both documents and removes both allocations.
#[renox::test]
async fn reversing_a_set_off_reopens_both_documents() {
    let t = boot().await;
    let amal = accountant(&t).await;
    let invoice = issued_invoice(&t, &amal).await;
    let sb = bill(&t, true).await;
    post_set_off(&t, &amal, &invoice, &sb, "5250.00")
        .await
        .assert_status(303);
    let payment = payments(&t).await.remove(0);

    t.app.acting_as(&amal);
    t.app
        .htmx()
        .post(
            &format!("/payments/{}/reverse", payment.id),
            &[("reason", "entered_in_error"), ("note", "Wrong bill")],
        )
        .await
        .assert_ok();

    let reversed = Payment::find_or_404(t.app.db(), payment.id).await.unwrap();
    assert_eq!(reversed.status, PaymentStatus::Reversed);
    let invoice_after = reload_invoice(&t, &invoice).await;
    assert_eq!(invoice_after.status, InvoiceStatus::Issued);
    assert_eq!(invoice_after.balance, 10_500_000);
    let bill_after = reload_bill(&t, &sb).await;
    assert_eq!(bill_after.status, SupplierBillStatus::Approved);
    assert_eq!(bill_after.balance, 525_000);
    let rows = Allocation::all_of(t.app.db(), payment.id).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| !row.is_active()));
}

/// Done when 4: with the party blocked, the set-off is refused until the
/// compliance role approves an exception; the requester cannot approve it.
#[renox::test]
async fn a_blocked_party_waits_for_an_approved_exception() {
    let t = boot().await;
    let amal = accountant(&t).await;
    let invoice = issued_invoice(&t, &amal).await;
    let sb = bill(&t, true).await;
    t.blocked.0.lock().unwrap().push(BLUE_DUNES.0);

    // Asked: nothing is recorded or allocated, an exception is requested.
    post_set_off(&t, &amal, &invoice, &sb, "5250.00")
        .await
        .assert_status(303);
    let held = payments(&t).await;
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].status, PaymentStatus::Pending);
    assert_eq!(held[0].number, None);
    assert_eq!(reload_invoice(&t, &invoice).await.balance, 10_500_000);
    assert_eq!(reload_bill(&t, &sb).await.balance, 525_000);
    let requests = ApprovalRequest::pending_about(t.app.db(), &held[0].subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, BLOCK_EXCEPTION_ACTION);
    assert_eq!(requests[0].transition, "record_set_off");

    // The requester is not the approver.
    let refused = approvals::approve(t.app.state(), requests[0].id, &amal, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(
        Payment::find_or_404(t.app.db(), held[0].id).await.unwrap().status,
        PaymentStatus::Pending
    );

    let officer = compliance(&t).await;
    let decided = approvals::approve(t.app.state(), requests[0].id, &officer, "approved")
        .await
        .unwrap();
    assert!(decided.is_some());

    let done = Payment::find_or_404(t.app.db(), held[0].id).await.unwrap();
    assert_eq!(done.status, PaymentStatus::Received);
    assert!(done.number.is_some());
    assert_eq!(done.bank_account, None);
    let invoice_after = reload_invoice(&t, &invoice).await;
    assert_eq!(invoice_after.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(invoice_after.balance, 9_975_000);
    assert_eq!(reload_bill(&t, &sb).await.status, SupplierBillStatus::Paid);
    let active = Allocation::all_of(t.app.db(), done.id)
        .await
        .unwrap()
        .into_iter()
        .filter(Allocation::is_active)
        .count();
    assert_eq!(active, 2);
}

/// A party that is no longer blocked is not asked for an exception: the
/// set-off runs at once, and only that one is received.
#[renox::test]
async fn an_exception_is_asked_only_for_a_blocked_party() {
    let t = boot().await;
    let amal = accountant(&t).await;
    let invoice = issued_invoice(&t, &amal).await;
    let sb = bill(&t, true).await;
    t.blocked.0.lock().unwrap().push(BLUE_DUNES.0);
    post_set_off(&t, &amal, &invoice, &sb, "1000.00")
        .await
        .assert_status(303);
    // Free again: the set-off runs at once.
    t.blocked.0.lock().unwrap().clear();
    post_set_off(&t, &amal, &invoice, &sb, "1000.00")
        .await
        .assert_status(303);
    let all = payments(&t).await;
    assert_eq!(
        all.iter()
            .filter(|p| p.status == PaymentStatus::Received)
            .count(),
        1
    );
    assert_eq!(reload_invoice(&t, &invoice).await.balance, 10_400_000);
}

/// Done when 5: with the setting off, method *set-off* is not offered, and
/// not recorded; turned on, it is.
#[renox::test]
async fn the_method_is_offered_only_when_the_setting_is_on() {
    let off = boot_with(WlPayments::new()).await;
    let amal = accountant(&off).await;
    off.app.acting_as(&amal);
    let page = off.app.get("/payments/new").await;
    page.assert_ok();
    assert!(!page.text().contains(r#"value="set_off""#), "{}", page.text());
    let invoice = issued_invoice(&off, &amal).await;
    let sb = bill(&off, true).await;
    post_set_off(&off, &amal, &invoice, &sb, "5250.00")
        .await
        .assert_status(422);
    assert!(payments(&off).await.is_empty());

    let on = boot().await;
    let amal = accountant(&on).await;
    on.app.acting_as(&amal);
    let page = on.app.get("/payments/new").await;
    page.assert_ok();
    assert!(page.text().contains(r#"value="set_off""#), "{}", page.text());
}
