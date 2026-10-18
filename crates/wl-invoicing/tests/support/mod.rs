//! Booting wl-invoicing in a `TestApp`, as `crates/wl-parties/tests/support`
//! boots wl-parties: a party directory of its own (the crate does not depend
//! on wl-parties), a notifier that records what it is asked for, and the
//! kernel events recorded.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::auth::{Permissions, permissions};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_invoicing::vat_summary::{self, VatSummary, VatSummaryStatus};
use wl_invoicing::{
    AdjustmentKind, Invoice, InvoicingConfig, VatAdjustment, WlInvoicing, permissions as perm,
};
use wl_kernel::LegalEntity;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    BadDebtRecovered, CreditNoteReopened, DraftLine, InvoiceDisputeResolved, InvoiceDraft,
    InvoiceId, InvoiceIssued, InvoiceOverdue, InvoicePaid, InvoiceWrittenOff, VatCategory,
};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRequest};
use wl_kernel::parties::{BillingIdentity, PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::{ClientConfig, PartyId, Subject};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with the invoicing module (and the parties it needs) on: a
/// VAT-registered mainland company and a free-zone one without a TRN.
pub const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "invoicing"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
trn = "100000000000003"
default = true

[[legal_entities]]
code = "PGFZ"
name_en = "Palm Grove Facilities FZ-LLC"
"#;

/// Blue Dunes LLC: has a billing e-mail.
pub const BLUE_DUNES: PartyId = PartyId(1);
/// Blue Dunes' billing e-mail.
pub const BLUE_DUNES_EMAIL: &str = "ap@bluedunes.example";
/// Quiet Sands LLC: no billing e-mail.
pub const QUIET_SANDS: PartyId = PartyId(2);

/// The parties the tests bill.
#[derive(Debug, Clone, Copy, Default)]
pub struct TestDirectory;

impl PartyDirectory for TestDirectory {
    fn find<'a>(&'a self, _db: &'a Db, _party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
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
        let (name, email) = match party {
            BLUE_DUNES => ("Blue Dunes LLC", Some(BLUE_DUNES_EMAIL.to_owned())),
            QUIET_SANDS => ("Quiet Sands LLC", None),
            _ => return Box::pin(async { Ok(None) }),
        };
        Box::pin(async move {
            Ok(Some(BillingIdentity {
                party,
                legal_name_en: name.to_owned(),
                legal_name_ar: None,
                trn: Some("100000000000011".to_owned()),
                billing_address: Some("Office 12, Business Bay, Dubai".to_owned()),
                billing_email: email,
            }))
        })
    }
}

/// A test-only module with two probes of what a user is offered; a test
/// cannot build an `AuthUser`, so these read it from the request.
struct Probe;

impl Module for Probe {
    fn name(&self) -> &'static str {
        "invoicing-probe"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/probe/{id}/available", probe_available)
            .get("/probe/{id}/offered", probe_offered)
    }
}

async fn probe_available(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let invoice = Invoice::find_or_404(&state.db, id).await?;
    let names: Vec<String> = invoice
        .available(&user, &state.db)
        .await?
        .into_iter()
        .map(|action| action.name)
        .collect();
    Ok(names.join(","))
}

async fn probe_offered(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let invoice = Invoice::find_or_404(&state.db, id).await?;
    let config = state
        .provided::<wl_invoicing::InvoicingConfig>()
        .expect("the module provides its settings");
    let offered = wl_invoicing::write_off::offered(&config, &user, &invoice).await;
    Ok(if offered { "yes" } else { "no" }.to_owned())
}

/// A notifier that records every request and queues nothing.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

impl Recording {
    /// Every request so far, oldest first.
    pub fn all(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }

    /// The requests of one notification type, oldest first.
    pub fn of_type(&self, type_key: &str) -> Vec<NotifyRequest> {
        self.all()
            .into_iter()
            .filter(|r| r.type_key == type_key)
            .collect()
    }
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        self.0.lock().unwrap().push(request);
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Everything of one type `E` emitted so far.
#[derive(Clone)]
pub struct Recorded<E>(Arc<Mutex<Vec<E>>>);

impl<E: Clone> Recorded<E> {
    fn new() -> Self {
        Recorded(Arc::new(Mutex::new(Vec::new())))
    }

    /// The events, oldest first.
    pub fn all(&self) -> Vec<E> {
        self.0.lock().unwrap().clone()
    }
}

fn record<E: Event + Clone>(app: App, into: &Recorded<E>) -> App {
    let into = into.clone();
    app.listen(move |event: E, _state: AppState| {
        let into = into.clone();
        async move {
            into.0.lock().unwrap().push(event);
            Ok(())
        }
    })
}

/// The app under test with what it emitted.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// The reminders (and other notices) asked of the kernel's notifier.
    pub notices: Recording,
    /// `InvoiceIssued`.
    pub issued: Recorded<InvoiceIssued>,
    /// `InvoiceOverdue`.
    pub overdue: Recorded<InvoiceOverdue>,
    /// `InvoicePaid`.
    pub paid: Recorded<InvoicePaid>,
    /// `InvoiceWrittenOff`.
    pub written_off: Recorded<InvoiceWrittenOff>,
    /// `CreditNoteReopened`.
    pub reopened: Recorded<CreditNoteReopened>,
    /// `BadDebtRecovered`.
    pub recovered: Recorded<BadDebtRecovered>,
    /// `InvoiceDisputeResolved`.
    pub resolved: Recorded<InvoiceDisputeResolved>,
}

impl Booted {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// A user holding exactly `grants` (through a role of their own).
    pub async fn user(&self, name: &str, grants: &[&str]) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        let role = format!("role_{}", name.to_lowercase());
        permissions::define_role(self.db(), &role, grants)
            .await
            .unwrap();
        user.assign_role(self.db(), &role).await.unwrap();
        user
    }

    /// A user holding the roles of the host: `receivables` (and `admin`
    /// too when `admin`).
    async fn staff(&self, name: &str, admin: bool) -> User {
        let user = User::register(
            self.db(),
            name,
            &format!("{}@example.com", name.to_lowercase()),
            "password123",
        )
        .await
        .unwrap();
        user.assign_role(self.db(), "receivables").await.unwrap();
        if admin {
            user.assign_role(self.db(), "admin").await.unwrap();
        }
        user
    }

    /// User A: a receivables clerk.
    pub async fn user_a(&self) -> User {
        self.staff("Amal", false).await
    }

    /// User A, who is also an admin.
    pub async fn admin_a(&self) -> User {
        self.staff("Amal", true).await
    }

    /// User B: another receivables clerk.
    pub async fn user_b(&self) -> User {
        self.staff("Bilal", false).await
    }

    /// A user who may only see invoices.
    pub async fn viewer(&self) -> User {
        self.user("Vera", &[perm::VIEW]).await
    }

    /// A user of the host's `accountant` role (the ones the relief notice
    /// goes to).
    pub async fn accountant(&self, name: &str) -> User {
        let user = User::register(
            self.db(),
            name,
            &format!("{}@example.com", name.to_lowercase()),
            "password123",
        )
        .await
        .unwrap();
        user.assign_role(self.db(), "accountant").await.unwrap();
        user
    }

    /// The user `name` with `grants` (through a role of their own), made
    /// once: asked again, the same user.
    pub async fn helper(&self, name: &str, grants: &[&str]) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        match User::find_by_email(self.db(), &email).await.unwrap() {
            Some(user) => user,
            None => self.user(name, grants).await,
        }
    }

    /// A receivables clerk `name`, made once: asked again, the same user.
    pub async fn clerk(&self, name: &str) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        match User::find_by_email(self.db(), &email).await.unwrap() {
            Some(user) => user,
            None => self.staff(name, false).await,
        }
    }

    /// The module's settings, as the app has them.
    pub fn config(&self) -> std::sync::Arc<InvoicingConfig> {
        self.state()
            .provided::<InvoicingConfig>()
            .expect("the module provides its settings")
    }

    /// The legal entity `code`.
    pub async fn entity(&self, code: &str) -> LegalEntity {
        LegalEntity::query()
            .where_(LegalEntity::CODE.eq(code))
            .first(self.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("no legal entity {code}"))
    }

    /// A new active, VAT-registered legal entity.
    pub async fn new_entity(&self, code: &str) -> LegalEntity {
        let mut entity = LegalEntity {
            code: code.to_owned(),
            name_en: format!("{code} LLC"),
            trn: Some("100000000000029".to_owned()),
            vat_registered: true,
            status: "active".to_owned(),
            ..Default::default()
        };
        entity.insert(self.db()).await.unwrap();
        entity
    }

    /// The summary of `entity` for the period that contains `date`, taken
    /// to `status` through its transitions (so the period must be over for
    /// anything past `open`).
    pub async fn summary(
        &self,
        entity: &LegalEntity,
        date: NaiveDate,
        status: VatSummaryStatus,
    ) -> VatSummary {
        let config = self.config();
        vat_summary::ensure_open(self.db(), &config, entity, date)
            .await
            .unwrap();
        let summary = VatSummary::containing(self.db(), &config, entity, date)
            .await
            .unwrap()
            .expect("the summary exists");
        let preparer = self
            .helper("Prep", &[perm::VAT_VIEW, perm::VAT_PREPARE])
            .await;
        let reviewer = self
            .helper("Rev", &[perm::VAT_VIEW, perm::VAT_REVIEW, perm::VAT_LOCK])
            .await;
        let steps = [
            ("prepare", VatSummaryStatus::Prepared, &preparer),
            ("review", VatSummaryStatus::Reviewed, &reviewer),
            ("lock", VatSummaryStatus::Locked, &reviewer),
        ];
        for (name, reaches, by) in steps {
            if summary_rank(reaches) > summary_rank(status) {
                break;
            }
            self.post_transition(&summary, name, by, "")
                .await
                .assert_status(303);
        }
        self.summary_now(summary.id).await
    }

    /// Summary `id`, as stored now.
    pub async fn summary_now(&self, id: i64) -> VatSummary {
        VatSummary::find(self.db(), id).await.unwrap().unwrap()
    }

    /// `POST /vat-summaries/{id}/transitions/{name}` as `by`, as the page's
    /// actions send it.
    pub async fn post_transition(
        &self,
        summary: &VatSummary,
        name: &str,
        by: &User,
        reason: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(
                &format!("/vat-summaries/{}/transitions/{name}", summary.id),
                &[("reason", reason)],
            )
            .await
    }

    /// An invoice of `entity` (the default one when `None`) for AED
    /// 1,000.00 plus 5% VAT, written off for its whole balance through the
    /// approval, with the date of supply `supply` and the write-off notice
    /// date `notice_on`. Returns it with its pending bad-debt relief of
    /// AED 50.00.
    pub async fn written_off(
        &self,
        entity: Option<&LegalEntity>,
        supply: NaiveDate,
        notice_on: Option<NaiveDate>,
    ) -> (Invoice, VatAdjustment) {
        let clerk = self.clerk("Wendy").await;
        let approver = self.clerk("Wade").await;
        let mut draft = InvoiceDraft::new(BLUE_DUNES, Subject::new("property.lease", 9))
            .line(DraftLine::new("Line 1", 100_000, VatCategory::Standard))
            .due_date(self.day(-5));
        if let Some(entity) = entity {
            draft = draft.legal_entity(entity.id);
        }
        let draft = self.draft_of(draft).await;
        self.issue(&draft, &clerk).await;
        wl_invoicing::overdue::run(self.state())
            .await
            .expect("the overdue run");
        let invoice = self.reload(&draft).await;
        self.post_write_off(&invoice, &clerk, "insolvent", "")
            .await
            .assert_ok();
        let request = self.pending(&invoice).await.remove(0);
        approvals::approve(self.state(), request.id, &approver, "")
            .await
            .unwrap()
            .expect("the approval runs");
        Invoice::query()
            .where_(Invoice::ID.eq(invoice.id))
            .update(
                self.db(),
                &[
                    ("date_of_supply", &Some(supply)),
                    ("write_off_notice_on", &notice_on),
                ],
            )
            .await
            .unwrap();
        let invoice = self.reload(&invoice).await;
        let relief = VatAdjustment::relief_of(self.db(), &invoice)
            .await
            .unwrap()
            .expect("the write-off made the relief");
        assert_eq!(relief.kind, AdjustmentKind::BadDebtRelief);
        (invoice, relief)
    }

    /// The adjustment `id`, as stored now.
    pub async fn adjustment(&self, id: i64) -> VatAdjustment {
        VatAdjustment::find(self.db(), id).await.unwrap().unwrap()
    }

    /// `POST` of the include or exclude sheet (`action`) of `adjustment`.
    pub async fn post_decision(
        &self,
        summary: &VatSummary,
        adjustment: &VatAdjustment,
        action: &str,
        by: &User,
        reason: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(
                &format!(
                    "/vat-summaries/{}/adjustments/{}/{action}",
                    summary.id, adjustment.id
                ),
                &[("reason", reason)],
            )
            .await
    }

    /// `n` days from today (negative: before).
    pub fn day(&self, n: i64) -> NaiveDate {
        let days = Days::new(n.unsigned_abs());
        if n >= 0 {
            today().checked_add_days(days).unwrap()
        } else {
            today().checked_sub_days(days).unwrap()
        }
    }

    /// A draft to Blue Dunes with lines of `(unit price in fils, category)`,
    /// through the kernel contract.
    pub async fn draft(&self, lines: &[(i64, VatCategory)], due: Option<NaiveDate>) -> Invoice {
        let mut draft = InvoiceDraft::new(BLUE_DUNES, Subject::new("property.lease", 9));
        for (i, (price, category)) in lines.iter().enumerate() {
            draft = draft.line(DraftLine::new(
                &format!("Line {}", i + 1),
                *price,
                *category,
            ));
        }
        if let Some(due) = due {
            draft = draft.due_date(due);
        }
        self.draft_of(draft).await
    }

    /// `draft` through the kernel contract, as stored.
    pub async fn draft_of(&self, draft: InvoiceDraft) -> Invoice {
        let id = wl_kernel::invoicing::invoicer(self.state())
            .draft(self.db(), draft)
            .await
            .expect("the draft is made")
            .expect("invoicing is on");
        self.invoice(id.0).await
    }

    /// Invoice `id`, as stored now.
    pub async fn invoice(&self, id: i64) -> Invoice {
        Invoice::find(self.db(), id).await.unwrap().unwrap()
    }

    /// `invoice`, as stored now.
    pub async fn reload(&self, invoice: &Invoice) -> Invoice {
        self.invoice(invoice.id).await
    }

    /// `POST /invoices/{id}/issue` as `by`, as the page sends it.
    pub async fn post_issue(&self, draft: &Invoice, by: &User) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/invoices/{}/issue", draft.id), &[])
            .await
    }

    /// `POST /invoices/{id}/write-off` as `by`, as the write-off sheet
    /// sends it.
    pub async fn post_write_off(
        &self,
        invoice: &Invoice,
        by: &User,
        code: &str,
        text: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(
                &format!("/invoices/{}/write-off", invoice.id),
                &[("reason_code", code), ("reason_text", text)],
            )
            .await
    }

    /// Issues `draft` as `by`.
    pub async fn issue(&self, draft: &Invoice, by: &User) -> Invoice {
        // Issued: back to the invoice's page.
        self.post_issue(draft, by)
            .await
            .assert_redirect(&format!("/invoices/{}", draft.id));
        self.reload(draft).await
    }

    /// The names of the transitions `by` is offered on `invoice`
    /// (`available()`), comma separated.
    pub async fn available(&self, invoice: &Invoice, by: &User) -> String {
        self.app.acting_as(by);
        self.app
            .get(&format!("/probe/{}/available", invoice.id))
            .await
            .text()
    }

    /// Whether the write-off is offered to `by` now (the page's button).
    pub async fn write_off_offered(&self, invoice: &Invoice, by: &User) -> bool {
        self.app.acting_as(by);
        self.app
            .get(&format!("/probe/{}/offered", invoice.id))
            .await
            .text()
            == "yes"
    }

    /// An issued invoice to Blue Dunes with `lines`, issued by user A.
    pub async fn issued(
        &self,
        lines: &[(i64, VatCategory)],
        due: Option<NaiveDate>,
        by: &User,
    ) -> Invoice {
        let draft = self.draft(lines, due).await;
        self.issue(&draft, by).await
    }

    /// An issued invoice of AED 1,000.00 plus 5% VAT (AED 1,050.00) that
    /// fell due 5 days ago and was marked `overdue`.
    pub async fn overdue_invoice(&self, by: &User) -> Invoice {
        let invoice = self
            .issued(&[(100_000, VatCategory::Standard)], Some(self.day(-5)), by)
            .await;
        wl_invoicing::overdue::run(self.state())
            .await
            .expect("the overdue run");
        self.reload(&invoice).await
    }

    /// Allocates `amount` fils of payment `n` to `invoice`.
    pub async fn pay(&self, invoice: &Invoice, n: i64, amount: i64) {
        wl_kernel::invoicing::invoicer(self.state())
            .allocate(
                self.state(),
                InvoiceId(invoice.id),
                Subject::new("payments.payment", n),
                amount,
            )
            .await
            .expect("the payment is allocated");
    }

    /// The approval requests about `invoice`, oldest first.
    pub async fn requests(&self, invoice: &Invoice) -> Vec<ApprovalRequest> {
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq(invoice.subject().subject_type.as_str()))
            .where_(ApprovalRequest::SUBJECT_ID.eq(invoice.id))
            .order_by(ApprovalRequest::ID)
            .get(self.db())
            .await
            .unwrap()
    }

    /// The pending approval requests about `invoice`.
    pub async fn pending(&self, invoice: &Invoice) -> Vec<ApprovalRequest> {
        approvals::pending_for(self.db(), &invoice.subject())
            .await
            .unwrap()
    }

    /// The names of the transitions `invoice` went through, in order.
    pub async fn history(&self, invoice: &Invoice) -> Vec<String> {
        renox::state_machine::history(self.db(), Invoice::TABLE, invoice.id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.transition)
            .collect()
    }
}

/// The position of a summary status in its lifecycle.
fn summary_rank(status: VatSummaryStatus) -> usize {
    match status {
        VatSummaryStatus::Open => 0,
        VatSummaryStatus::Prepared => 1,
        VatSummaryStatus::Reviewed => 2,
        VatSummaryStatus::Locked => 3,
    }
}

/// wl-invoicing with the §7 defaults, booted.
pub async fn boot() -> Booted {
    boot_with(WlInvoicing::new()).await
}

/// `module` booted in an app with auth and the kernel, the roles of the
/// host (`receivables`, `admin`) defined.
pub async fn boot_with(module: WlInvoicing) -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();

    let issued = Recorded::new();
    let overdue = Recorded::new();
    let paid = Recorded::new();
    let written_off = Recorded::new();
    let reopened = Recorded::new();
    let recovered = Recorded::new();
    let resolved = Recorded::new();
    let notices = Recording::default();
    let directory: Arc<dyn PartyDirectory> = Arc::new(TestDirectory);
    let notifier: Arc<dyn Notifier> = Arc::new(notices.clone());
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(module)
        .module(Probe)
        .provide(directory)
        .provide(notifier);
    let app = record(app, &issued);
    let app = record(app, &overdue);
    let app = record(app, &paid);
    let app = record(app, &written_off);
    let app = record(app, &reopened);
    let app = record(app, &recovered);
    let app = record(app, &resolved);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    let all = [
        perm::VIEW,
        perm::CREATE,
        perm::ISSUE,
        perm::CREDIT,
        perm::WRITE_OFF,
        perm::DISPUTE,
        perm::RECEIVABLES_VIEW,
        perm::VAT_VIEW,
        perm::VAT_PREPARE,
        perm::VAT_REVIEW,
        perm::VAT_LOCK,
    ];
    permissions::define_role(app.db(), "admin", &all)
        .await
        .unwrap();
    // The host's `accountant` role: the VAT permissions and the invoices.
    permissions::define_role(
        app.db(),
        "accountant",
        &[
            perm::VIEW,
            perm::VAT_VIEW,
            perm::VAT_PREPARE,
            perm::VAT_REVIEW,
            perm::VAT_LOCK,
        ],
    )
    .await
    .unwrap();
    permissions::define_role(
        app.db(),
        "receivables",
        &[
            perm::VIEW,
            perm::CREATE,
            perm::ISSUE,
            perm::WRITE_OFF,
            perm::DISPUTE,
            perm::RECEIVABLES_VIEW,
        ],
    )
    .await
    .unwrap();
    Booted {
        app,
        _views: views,
        notices,
        issued,
        overdue,
        paid,
        written_off,
        reopened,
        recovered,
        resolved,
    }
}
