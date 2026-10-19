use std::path::Path;

use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::parties::{NoParties, PartyDirectory};
use wl_kernel::{ClientConfig, LegalEntity, PartyId, Reset, Series};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A group of two companies: a mainland LLC (the default) and a free-zone one.
const GROUP: &str = r#"
[client]
name = "Acme Properties LLC"
app = "property"

[modules]
enabled = ["parties", "invoicing"]

[[legal_entities]]
code = "ACME"
name_en = "Acme Properties LLC"
name_ar = "أكمي للعقارات ش.ذ.م.م"
trn = "100123456700003"
default = true

[[legal_entities]]
code = "ACFZ"
name_en = "Acme FZ-LLC"
"#;

const INVOICE: Series = Series {
    key: "invoice",
    pattern: "{prefix}-{entity}-{yyyy}-{0000}",
    prefix: "INV",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// A views directory holding the layout and `files` (the app's own views).
fn views(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary views directory");
    for (name, text) in [("layouts/app.html", LAYOUT)].iter().chain(files) {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("a file path")).expect("a directory");
        std::fs::write(path, text).expect("a view file");
    }
    dir
}

/// The kernel for `GROUP` in an app with auth, in `views`, installed.
async fn boot(views: &Path) -> TestApp {
    let config = ClientConfig::parse(GROUP).expect("a valid client.toml");
    let app = wl_kernel::apply(App::new().module(Auth::new()).module(Permissions), config)
        .expect("the kernel applies");
    let app = TestApp::with_config(app, |c| c.views_path = views.to_path_buf()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app
}

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 18).unwrap()
}

#[renox::test]
async fn install_creates_the_entities_once_and_keeps_admin_changes() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    assert_eq!(LegalEntity::query().count(db).await.unwrap(), 2);
    let mut acme = LegalEntity::default_entity(db).await.unwrap();
    assert_eq!(acme.code, "ACME");
    assert!(acme.vat_registered);

    // A change made in the admin survives the next install.
    acme.name_en = "Acme Properties L.L.C.".into();
    acme.save(db).await.unwrap();
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    assert_eq!(LegalEntity::query().count(db).await.unwrap(), 2);
    let again = LegalEntity::default_entity(db).await.unwrap();
    assert_eq!(again.name_en, "Acme Properties L.L.C.");
}

#[renox::test]
async fn a_scope_issues_from_its_entity_else_the_default() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    let free_zone = LegalEntity::query()
        .where_(LegalEntity::CODE.eq("ACFZ"))
        .first(db)
        .await
        .unwrap()
        .unwrap();
    free_zone
        .link_scope(db, "property.building", 7)
        .await
        .unwrap();

    let linked = LegalEntity::for_scope(db, Some(("property.building", 7)))
        .await
        .unwrap();
    assert_eq!(linked.code, "ACFZ");
    let unlinked = LegalEntity::for_scope(db, Some(("property.building", 8)))
        .await
        .unwrap();
    assert_eq!(unlinked.code, "ACME");
    assert_eq!(LegalEntity::for_scope(db, None).await.unwrap().code, "ACME");
}

#[renox::test]
async fn the_snapshot_keeps_the_trn_only_when_vat_registered() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    let acme = LegalEntity::default_entity(db).await.unwrap();
    let snapshot = acme.snapshot();
    assert_eq!(snapshot.trn.as_deref(), Some("100123456700003"));
    assert_eq!(snapshot.name_ar.as_deref(), Some("أكمي للعقارات ش.ذ.م.م"));

    let free_zone = LegalEntity::query()
        .where_(LegalEntity::CODE.eq("ACFZ"))
        .first(db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(free_zone.snapshot().trn, None);
}

#[renox::test]
async fn gapless_numbers_count_per_entity_and_a_rollback_gives_one_back() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    let acme = LegalEntity::default_entity(db).await.unwrap();
    let free_zone = LegalEntity::query()
        .where_(LegalEntity::CODE.eq("ACFZ"))
        .first(db)
        .await
        .unwrap()
        .unwrap();

    let mut tx = db.begin().await.unwrap();
    let first = INVOICE
        .next(&mut tx, None, Some(&acme), day())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(first, "INV-ACME-2026-0001");

    // Taken, then rolled back: the next document gets the same number.
    let mut tx = db.begin().await.unwrap();
    let lost = INVOICE
        .next(&mut tx, None, Some(&acme), day())
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let mut tx = db.begin().await.unwrap();
    let second = INVOICE
        .next(&mut tx, Some("ACM-INV"), Some(&acme), day())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(lost, "INV-ACME-2026-0002");
    assert_eq!(second, "ACM-INV-ACME-2026-0002");

    let mut tx = db.begin().await.unwrap();
    let other = INVOICE
        .next(&mut tx, None, Some(&free_zone), day())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(other, "INV-ACFZ-2026-0001");

    // A new year starts again at 1.
    let next_year = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
    let mut tx = db.begin().await.unwrap();
    let january = INVOICE
        .next(&mut tx, None, Some(&acme), next_year)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(january, "INV-ACME-2027-0001");
}

#[renox::test]
async fn an_entity_series_needs_an_active_entity() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    let mut acme = LegalEntity::default_entity(db).await.unwrap();
    acme.status = "inactive".into();
    let mut tx = db.begin().await.unwrap();
    assert!(INVOICE.next(&mut tx, None, None, day()).await.is_err());
    assert!(
        INVOICE
            .next(&mut tx, None, Some(&acme), day())
            .await
            .is_err()
    );
}

#[renox::test]
async fn the_legal_entities_page_needs_the_permission() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    let user = User::register(db, "Mona", "mona@example.com", "password123")
        .await
        .unwrap();
    app.acting_as(&user);
    app.get("/legal-entities").await.assert_forbidden();

    permissions::define_role(
        db,
        "accountant",
        &[wl_kernel::permissions::LEGAL_ENTITIES_VIEW],
    )
    .await
    .unwrap();
    user.assign_role(db, "accountant").await.unwrap();
    app.get("/legal-entities")
        .await
        .assert_ok()
        .assert_view("wl-kernel/legal_entities.html")
        .assert_see("Acme FZ-LLC")
        .assert_see("100123456700003");
}

/// #34 AC1: without the parties module, the kernel's directory is
/// `NoParties` and finds nobody.
#[renox::test]
async fn without_parties_the_directory_finds_nobody() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let parties = wl_kernel::parties::directory(app.state());
    let db = app.db();
    assert_eq!(parties.find(db, PartyId(1)).await.unwrap(), None);
    assert_eq!(parties.find_by_code(db, "P-00001").await.unwrap(), None);
    assert_eq!(parties.status(db, PartyId(1)).await.unwrap(), None);
    assert_eq!(
        parties.display_name(db, PartyId(1), "en").await.unwrap(),
        None
    );
    assert_eq!(NoParties.find(db, PartyId(1)).await.unwrap(), None);
}

/// #57 AC7: without the parties module nobody is related to anybody, for
/// the kernel's directory as for `NoParties`; the chooser skips the parties
/// it is told to and takes the first (primary) one.
#[renox::test]
async fn without_parties_nobody_is_a_related_contact() {
    use wl_kernel::notify::ContactPurpose;
    use wl_kernel::parties::{RelatedContact, choose_contact};

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let parties = wl_kernel::parties::directory(app.state());
    let db = app.db();
    for purpose in [
        ContactPurpose::contact_of("finance"),
        ContactPurpose::guardian_of(),
    ] {
        assert!(
            parties
                .related_contacts(db, PartyId(1), &purpose)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            NoParties
                .related_contacts(db, PartyId(1), &purpose)
                .await
                .unwrap()
                .is_empty()
        );
    }
    let related = |party: i64, primary: bool| RelatedContact {
        party: PartyId(party),
        position: Some("finance".into()),
        primary,
    };
    let candidates = [related(11, true), related(12, false), related(13, false)];
    assert_eq!(choose_contact(&candidates, &[]).unwrap().party, PartyId(11));
    let tried = [PartyId(11), PartyId(12)];
    assert_eq!(
        choose_contact(&candidates, &tried).unwrap().party,
        PartyId(13)
    );
    assert!(choose_contact(&candidates, &[PartyId(11), PartyId(12), PartyId(13)]).is_none());
}

/// #55 AC10: without the notify module, the kernel's notifier is
/// `NoNotifier`: it queues nothing and returns no message id; a staff user
/// still gets a bell entry.
#[renox::test]
async fn without_notify_the_notifier_queues_nothing() {
    use wl_kernel::Subject;
    use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};

    notify::register_type(
        NotificationTypeDef::new("kernel_package.reminder", "invoicing").template(
            NotifyChannel::Email,
            "en",
            Some("Invoice {{ number }} is overdue"),
            "Please pay.",
        ),
    );
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let user = User::register(app.db(), "Mona", "mona@example.com", "password123")
        .await
        .unwrap();
    app.fake_notifications();
    let request = NotifyRequest::new(
        "kernel_package.reminder",
        Subject::new("invoicing.invoice", 7),
    )
    .to_party(PartyId(1))
    .to_user(user.id)
    .data(renox::serde_json::json!({ "number": "INV-0007" }));
    let state = app.state();
    let ids = notify::notifier(state)
        .notify(state, request)
        .await
        .unwrap();
    assert!(ids.is_empty());
    app.assert_notified(&user, notify::FALLBACK_KIND);
    assert_eq!(app.notifications().len(), 1);

    // An unregistered type is refused.
    let unknown = NotifyRequest::new(
        "kernel_package.missing",
        Subject::new("invoicing.invoice", 1),
    );
    assert!(
        notify::notifier(state)
            .notify(state, unknown)
            .await
            .is_err()
    );
}

/// #59 AC6: without the notify module, a statutory notice is never read as
/// delivered, and no undelivered notice is listed.
#[renox::test]
async fn without_notify_a_notice_is_not_delivered() {
    use wl_kernel::Subject;
    use wl_kernel::notify::{self, NoticeStatus};

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let subject = Subject::new("property.lease", 7);
    let notifier = notify::notifier(state);
    let status = notifier
        .notice_status(state, &subject, "lease.changed_terms_notice")
        .await
        .unwrap();
    assert_eq!(status, NoticeStatus::NotDelivered);
    assert!(
        notifier
            .undelivered_notices(state, &subject)
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC8 of #99: with payments off (the kernel alone), `RefundFailed` and
/// `RefundSource` still compile and serialise, so the listeners of other
/// modules (credit note, order, booking) build and can read the event.
#[renox::test]
async fn without_payments_the_refund_failed_event_still_serialises() {
    use wl_kernel::Subject;
    use wl_kernel::payments::{
        PaymentId, PaymentMethod, RefundAttemptRef, RefundFailed, RefundFailedFrom,
        RefundFailureKind, RefundId, RefundSource,
    };

    // The kernel boots without the payments module; the contract is data.
    let dir = views(&[]);
    let _app = boot(dir.path()).await;
    let source = RefundSource::CreditNote(Subject::new("invoicing.credit_note", 7));
    assert_eq!(source.kind(), "credit_note");
    assert_eq!(source.subject().to_string(), "invoicing.credit_note:7");
    let event = RefundFailed {
        refund: RefundId(5),
        party: PartyId(2),
        amount: 50_000,
        currency: "AED".to_owned(),
        source: source.clone(),
        method: PaymentMethod::Gateway,
        kind: RefundFailureKind::GatewayDeclined,
        reason: "card expired".to_owned(),
        attempt: RefundAttemptRef { id: 1, number: 1 },
        from: RefundFailedFrom::Approved,
        outgoing_payment: Some(PaymentId(9)),
    };
    let json = renox::serde_json::to_value(&event).expect("the event serialises");
    assert_eq!(json["source"]["kind"], "credit_note");
    assert_eq!(json["kind"], "gateway_declined");
    assert_eq!(json["outgoing_payment"], 9);
    let back: RefundFailed = renox::serde_json::from_value(json).expect("and reads back");
    assert_eq!(back, event);
    for kind in [
        "payment",
        "credit_note",
        "deposit",
        "party_credit",
        "payer_credit",
        "order",
        "booking",
    ] {
        let subject = Subject::new("x.y", 3);
        let source = RefundSource::from_kind(kind, &subject).expect(kind);
        assert_eq!(source.kind(), kind);
        let json = renox::serde_json::to_value(&source).expect("serialises");
        let back: RefundSource = renox::serde_json::from_value(json).expect("reads back");
        assert_eq!(back, source);
    }
}

/// AC6 of #38: the screening events are plain data other modules can listen
/// to, and the compliance-only one carries no candidate: party, screening and
/// status only.
#[renox::test]
async fn the_screening_events_serialise_without_any_candidate() {
    use wl_kernel::{ScreeningCompleted, ScreeningFlagged, ScreeningId, ScreeningStatus};

    let dir = views(&[]);
    let _app = boot(dir.path()).await;
    let flagged = ScreeningFlagged {
        party: PartyId(2),
        screening: ScreeningId(9),
        status: ScreeningStatus::PossibleMatch,
    };
    let json = renox::serde_json::to_value(&flagged).expect("the event serialises");
    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["party", "screening", "status"]);
    let back: ScreeningFlagged = renox::serde_json::from_value(json).expect("and reads back");
    assert_eq!(back, flagged);
    let done = ScreeningCompleted {
        party: PartyId(2),
        screening: ScreeningId(10),
        status: ScreeningStatus::Clear,
    };
    let json = renox::serde_json::to_value(&done).expect("the event serialises");
    let back: ScreeningCompleted = renox::serde_json::from_value(json).expect("and reads back");
    assert_eq!(back, done);
}

/// AC1 and AC5 of #39: the confirmed-match event carries party and screening
/// only (no list, candidate, note or reason), and `PartyRestored` only the
/// party.
#[renox::test]
async fn the_confirmed_match_and_restore_events_carry_ids_only() {
    use wl_kernel::{PartyRestored, ScreeningId, ScreeningMatchConfirmed};

    let dir = views(&[]);
    let _app = boot(dir.path()).await;
    let confirmed = ScreeningMatchConfirmed {
        party: PartyId(2),
        screening: ScreeningId(9),
    };
    let json = renox::serde_json::to_value(&confirmed).expect("the event serialises");
    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["party", "screening"]);
    let back: ScreeningMatchConfirmed = renox::serde_json::from_value(json).expect("reads back");
    assert_eq!(back, confirmed);

    let restored = PartyRestored { party: PartyId(2) };
    let json = renox::serde_json::to_value(&restored).expect("the event serialises");
    assert_eq!(json.as_object().unwrap().len(), 1);
    let back: PartyRestored = renox::serde_json::from_value(json).expect("reads back");
    assert_eq!(back, restored);
}

/// AC7 of #39: the held-funds registry asks every registered source for a
/// party, in source-key order, and tags each line with its source key.
#[renox::test]
async fn the_held_funds_registry_collects_every_source_for_a_party() {
    use wl_kernel::held_funds::{self, FundsFuture, FundsSource, HeldFund, HeldFundsSource};

    struct Lines(Vec<HeldFund>);

    impl FundsSource for Lines {
        fn held_for<'a>(
            &'a self,
            _state: &'a AppState,
            party: PartyId,
        ) -> FundsFuture<'a, Vec<HeldFund>> {
            Box::pin(async move {
                // Only party 4242 has money here.
                Ok(if party == PartyId(4242) {
                    self.0.clone()
                } else {
                    Vec::new()
                })
            })
        }
    }

    HeldFundsSource::new("pkg_b_credit")
        .owner("wl-test")
        .source(Lines(vec![HeldFund::new(
            "Credit C-1",
            "prepaid_credit",
            5_000,
            "AED",
        )]))
        .register();
    HeldFundsSource::new("pkg_a_deposits")
        .owner("wl-test")
        .source(Lines(vec![
            HeldFund::new("Deposit D-1", "deposit", 150_000, "AED"),
            HeldFund::new("Refund R-1", "refund", 25_050, "AED"),
        ]))
        .register();

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let funds = held_funds::held_for_all(app.state(), PartyId(4242))
        .await
        .expect("the sources answer");
    let ours: Vec<(&str, &str)> = funds
        .iter()
        .filter(|f| f.source.starts_with("pkg_"))
        .map(|f| (f.source.as_str(), f.label.as_str()))
        .collect();
    assert_eq!(
        ours,
        [
            ("pkg_a_deposits", "Deposit D-1"),
            ("pkg_a_deposits", "Refund R-1"),
            ("pkg_b_credit", "Credit C-1"),
        ]
    );
    let none = held_funds::held_for_all(app.state(), PartyId(1))
        .await
        .expect("the sources answer");
    assert!(none.iter().all(|f| !f.source.starts_with("pkg_")));
}

#[renox::test]
async fn the_models_match_the_schema() {
    let dir = views(&[]);
    boot(dir.path()).await.assert_models_match_schema().await;
}

#[renox::test]
async fn every_view_compiles() {
    let dir = views(&[]);
    boot(dir.path()).await.assert_views_compile();
}

#[test]
fn a_missing_module_dependency_stops_the_kernel() {
    let text = GROUP.replace(
        r#"enabled = ["parties", "invoicing"]"#,
        r#"enabled = ["invoicing"]"#,
    );
    let err = ClientConfig::parse(&text).unwrap_err();
    assert_eq!(
        err.to_string(),
        "client.toml: [modules] `invoicing` needs `parties`; enable it too"
    );
}

/// #64 AC1: without the invoicing module, the kernel's invoicer is
/// `NoInvoicer`: nothing is drafted or found, and an allocation is refused.
#[renox::test]
async fn without_invoicing_the_invoicer_drafts_nothing() {
    use wl_kernel::Subject;
    use wl_kernel::invoicing::{
        DraftLine, InvoiceDraft, InvoiceId, Invoicer, NoInvoicer, VatCategory, invoicer,
    };

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let draft = InvoiceDraft::new(PartyId(1), Subject::new("property.lease", 1))
        .line(DraftLine::new("Rent", 100_000, VatCategory::Standard));
    let provided = invoicer(state);
    let fallback: &dyn Invoicer = &NoInvoicer;
    for invoicer in [&*provided, fallback] {
        assert_eq!(invoicer.draft(app.db(), draft.clone()).await.unwrap(), None);
        assert_eq!(invoicer.find(app.db(), InvoiceId(1)).await.unwrap(), None);
        let refused = invoicer
            .allocate(
                state,
                InvoiceId(1),
                Subject::new("payments.payment", 1),
                100,
            )
            .await;
        assert!(matches!(
            refused,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
    }
}

/// #70 AC10: without the invoicing module, `unallocate` and
/// `unallocate_all` are refused (422).
#[renox::test]
async fn without_invoicing_no_bill_takes_money() {
    use wl_kernel::Subject;
    use wl_kernel::invoicing::{
        BillReopenReason, PayBill, SupplierBillId, UnallocateBill, supplier_bills,
    };

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let bills = supplier_bills(state);
    let source = Subject::new("payments.payment", 1);
    let pay = PayBill::new(
        SupplierBillId(1),
        source.clone(),
        100,
        "payments.allocation:1",
    );
    assert!(matches!(
        bills.pay(state, pay).await,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    let undo = UnallocateBill::new(
        SupplierBillId(1),
        source,
        100,
        "payments.allocation:1",
        BillReopenReason::PaymentReversed,
    );
    assert!(matches!(
        bills.unallocate(state, undo).await,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
}

#[renox::test]
async fn without_invoicing_nothing_can_be_unallocated() {
    use wl_kernel::Subject;
    use wl_kernel::invoicing::{
        InvoiceId, Invoicer, NoInvoicer, ReopenReason, Unallocate, invoicer,
    };

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let provided = invoicer(state);
    let fallback: &dyn Invoicer = &NoInvoicer;
    for invoicer in [&*provided, fallback] {
        let request = Unallocate::new(
            InvoiceId(1),
            Subject::new("payments.payment", 1),
            100,
            "payments.allocation:1",
            ReopenReason::PaymentReversed,
        );
        assert!(matches!(
            invoicer.unallocate(state, request).await,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
        assert!(matches!(
            invoicer
                .unallocate_all(state, InvoiceId(1), ReopenReason::AllocationMoved)
                .await,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
    }
}

/// #71 AC1: without the invoicing module, crediting is refused (422), no
/// credit note is found, an invoice has none, the party has no open credit,
/// and a refund cannot take one. A credit note's subject is
/// `invoicing.credit_note:<id>`, and the three events serialise with plain
/// ids.
#[renox::test]
async fn without_invoicing_no_credit_note_can_be_made() {
    use wl_kernel::Subject;
    use wl_kernel::invoicing::{
        CreditLine, CreditNoteId, CreditNoteIssued, CreditNoteReopened, InvoiceCredited, InvoiceId,
        Invoicer, NoInvoicer, invoicer,
    };

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let provided = invoicer(state);
    let fallback: &dyn Invoicer = &NoInvoicer;
    for invoicer in [&*provided, fallback] {
        let full = invoicer
            .credit_in_full(state, InvoiceId(1), "Billed twice".into(), None)
            .await;
        assert!(matches!(
            full,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
        let partly = invoicer
            .credit_partly(
                state,
                InvoiceId(1),
                vec![CreditLine::new(1, 100)],
                "Short".into(),
                None,
            )
            .await;
        assert!(matches!(
            partly,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
        assert_eq!(
            invoicer
                .find_credit_note(app.db(), CreditNoteId(1))
                .await
                .unwrap(),
            None
        );
        assert!(
            invoicer
                .credit_notes_for(app.db(), InvoiceId(1))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            invoicer
                .open_credit(app.db(), PartyId(1), "AED")
                .await
                .unwrap(),
            0
        );
        let taken = invoicer
            .take_credit_for_refund(
                state,
                CreditNoteId(1),
                Subject::new("payments.refund", 1),
                2,
                100,
            )
            .await;
        assert!(matches!(
            taken,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
    }

    assert_eq!(
        CreditNoteId(7).subject().to_string(),
        "invoicing.credit_note:7"
    );
    let issued = renox::serde_json::to_value(CreditNoteIssued {
        credit_note: CreditNoteId(7),
        invoice: InvoiceId(3),
        party: PartyId(2),
        amount: 21_000,
        net_amount: 20_000,
        reason: "Damaged".into(),
    })
    .unwrap();
    assert_eq!(issued["credit_note"], 7);
    assert_eq!(issued["invoice"], 3);
    assert_eq!(issued["party"], 2);
    let credited = renox::serde_json::to_value(InvoiceCredited {
        invoice: InvoiceId(3),
        credit_note: CreditNoteId(7),
        amount: 105_000,
        reason: "Billed twice".into(),
        released_origins: vec![Subject::new("property.lease", 4)],
    })
    .unwrap();
    assert_eq!(credited["invoice"], 3);
    assert_eq!(credited["credit_note"], 7);
    let reopened = renox::serde_json::to_value(CreditNoteReopened {
        credit_note: CreditNoteId(7),
        party: PartyId(2),
        amount_restored: 50_000,
        cause: Subject::new("payments.refund", 9),
    })
    .unwrap();
    assert_eq!(reopened["credit_note"], 7);
    assert_eq!(reopened["amount_restored"], 50_000);
}

/// #146 AC1: without the ticketing module, the kernel's task creator is
/// `NoTickets`: no ticket is opened and the caller carries on.
#[renox::test]
async fn without_ticketing_the_task_creator_opens_nothing() {
    use wl_kernel::Subject;
    use wl_kernel::tickets::{
        NoTickets, OpenTicket, TaskComment, TaskCreator, TicketId, task_creator,
    };

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let request = OpenTicket::new(Subject::new("property.unit", 12), "general", "Leaking tap");
    let provided = task_creator(state);
    let fallback: &dyn TaskCreator = &NoTickets;
    for creator in [&*provided, fallback] {
        assert_eq!(creator.open(state, request.clone()).await.unwrap(), None);
        assert_eq!(creator.find_open(state, "any").await.unwrap(), None);
        creator
            .comment(state, TicketId(1), TaskComment::internal("x"))
            .await
            .unwrap();
    }
}

/// #146 AC2: the business calendar (Asia/Dubai, Monday to Friday, 09:00 to
/// 18:00) counts SLA time in working minutes; the client's holidays are
/// skipped, and `working_minutes_between` is the inverse of `add_working`.
#[test]
fn the_business_calendar_counts_working_time() {
    use renox::chrono::NaiveDateTime;
    use wl_kernel::BusinessCalendar;

    let text = format!("{GROUP}\n[calendar]\nholidays = [\"2026-11-04\"]\n");
    let config = ClientConfig::parse(&text).expect("a valid client.toml");
    let holiday = config.business_calendar();
    let plain = BusinessCalendar::default();
    let at = |date: &str, time: &str| {
        let local: NaiveDateTime = format!("{date}T{time}:00").parse().expect("a time");
        plain.from_local(local)
    };

    // Tuesday 09:00 plus 8 working hours is 17:00.
    assert_eq!(
        plain.add_working(at("2026-11-03", "09:00"), 8 * 60),
        at("2026-11-03", "17:00")
    );
    // Friday 16:00 plus 8 hours is Monday 15:00 (the weekend is skipped).
    assert_eq!(
        plain.add_working(at("2026-11-06", "16:00"), 8 * 60),
        at("2026-11-09", "15:00")
    );
    // A start after closing counts from the next opening.
    assert_eq!(
        plain.add_working(at("2026-11-03", "20:00"), 60),
        at("2026-11-04", "10:00")
    );
    // A listed holiday is skipped: Tuesday 16:00 plus 3 hours is Thursday 10:00.
    assert_eq!(
        holiday.add_working(at("2026-11-03", "16:00"), 3 * 60),
        at("2026-11-05", "10:00")
    );
    assert!(!holiday.is_working_day("2026-11-04".parse().unwrap()));

    for calendar in [&plain, &holiday] {
        for (from, minutes) in [
            (at("2026-11-03", "09:00"), 8 * 60),
            (at("2026-11-06", "16:00"), 8 * 60),
            (at("2026-11-03", "10:30"), 27 * 60),
            (at("2026-11-02", "09:00"), 126 * 60),
        ] {
            let due = calendar.add_working(from, minutes);
            assert_eq!(calendar.working_minutes_between(from, due), minutes);
        }
    }
}

/// #36 AC1 and AC7 (kernel side): with no module registered, the registry
/// lists the 12 kinds of wl-parties §5.6 with their defaults, and a kind
/// nobody registered is reviewed with no decider. This binary registers
/// nothing, so the defaults are what is read.
#[renox::test]
async fn the_block_effects_registry_lists_the_twelve_kinds_and_reviews_the_unknown() {
    use wl_kernel::block_effects::{self, BlockEffect, RuleSource};

    let dir = views(&[]);
    let _app = boot(dir.path()).await;
    let kinds = block_effects::kinds();
    let listed: Vec<(&str, BlockEffect, Option<&str>)> = kinds
        .iter()
        .map(|k| (k.key.as_str(), k.effect, k.decider_role.as_deref()))
        .collect();
    assert_eq!(
        listed,
        [
            ("engagement", BlockEffect::Pause, Some("compliance")),
            (
                "subscription",
                BlockEffect::Pause,
                Some("subscriptions_manager")
            ),
            ("enrolment", BlockEffect::Pause, Some("centre_manager")),
            ("lease", BlockEffect::Continue, Some("property_manager")),
            (
                "deployment",
                BlockEffect::Continue,
                Some("managing_director")
            ),
            ("project", BlockEffect::Continue, Some("managing_director")),
            ("sales_order", BlockEffect::Pause, Some("sales_manager")),
            ("booking", BlockEffect::Review, Some("branch_manager")),
            ("purchase_order", BlockEffect::Continue, Some("procurement")),
            (
                "service_contract",
                BlockEffect::Review,
                Some("facility_manager")
            ),
            ("money_out", BlockEffect::Pause, Some("compliance")),
            ("money_in", BlockEffect::Continue, Some("compliance")),
        ]
    );
    assert!(
        kinds
            .iter()
            .all(|k| k.source == RuleSource::Default && !k.has_source)
    );

    let scope = renox::auth::permissions::Scope::new("branches", 1);
    let unknown = block_effects::effect_for("unregistered_kind", &scope);
    assert_eq!(unknown.effect, BlockEffect::Review);
    assert_eq!(unknown.decider_role, None);
    assert_eq!(unknown.source, RuleSource::Default);
    assert_eq!(unknown.scope, scope);
    assert!(block_effects::source_for("booking").is_none());
}

/// #56 AC7: `scope_of` gives the scope of a subject's machine first, then a
/// registered resolver's, then the global scope; a machine wins over a
/// resolver for the same type.
#[renox::test]
async fn the_scope_of_a_subject_is_its_machine_then_a_resolver_then_global() {
    use renox::auth::permissions::Scope;
    use renox::db::{Migration, sql};
    use renox::state_machine::{Machine, StateMachine, Transition};
    use serde::{Deserialize, Serialize};
    use wl_kernel::{Subject, SubjectMachine, scopes};

    #[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    enum ItemStatus {
        #[default]
        New,
        Done,
    }

    /// A record in a building.
    #[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
    #[model(table = "test_scoped_items", state_machine)]
    struct Item {
        id: i64,
        status: ItemStatus,
        building_id: i64,
    }

    impl StateMachine for Item {
        type State = ItemStatus;
        const COLUMN: renox::db::Col<Item, ItemStatus> = Item::STATUS;

        fn machine() -> Machine<Self> {
            Machine::new()
                .initial([ItemStatus::New])
                .state(ItemStatus::New)
                .state(ItemStatus::Done)
                .terminal()
                .transition(
                    Transition::new("finish")
                        .from([ItemStatus::New])
                        .to(ItemStatus::Done),
                )
        }
    }

    const MIGRATIONS: &[Migration] = &[Migration::new(
        "20261018130098_create_test_scoped_items",
        r#"CREATE TABLE "test_scoped_items" (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            status TEXT NOT NULL DEFAULT 'new',
            building_id INTEGER NOT NULL DEFAULT 0
        );"#,
        Some(r#"DROP TABLE "test_scoped_items";"#),
    )];

    struct Items;

    impl Module for Items {
        fn name(&self) -> &'static str {
            "scoped-items"
        }

        fn migrations(&self) -> &'static [Migration] {
            MIGRATIONS
        }

        fn register(&self, app: &mut Registry) {
            app.model::<Item>();
        }
    }

    let app = TestApp::new(App::new().module(Items)).await;
    let db = app.db();
    sql(r#"INSERT INTO "test_scoped_items" (building_id) VALUES (7)"#)
        .execute(db)
        .await
        .unwrap();
    let tower_a = Scope::new("buildings", 7);
    let tower_b = Scope::new("buildings", 8);
    let subject = Subject::new("scopes_it.item", 1);

    // Nothing registered: global.
    assert_eq!(
        scopes::scope_of(db, &subject).await.unwrap(),
        Scope::global()
    );

    // A resolver gives its scope.
    scopes::register("scopes_it.item", |_db, _id| {
        Box::pin(async { Ok(Scope::new("buildings", 8)) })
    });
    assert_eq!(scopes::scope_of(db, &subject).await.unwrap(), tower_b);

    // A machine with a scope wins over the resolver.
    SubjectMachine::of::<Item>("scopes_it.item")
        .scope(|item: &Item| Scope::new("buildings", item.building_id))
        .register();
    assert_eq!(scopes::scope_of(db, &subject).await.unwrap(), tower_a);
}

/// #56 AC2: working days skip the weekend and the holidays of `client.toml`
/// and keep the Asia/Dubai clock time: Thursday to Friday, Friday to
/// Monday, and the day before a holiday to the day after it.
#[test]
fn a_due_time_is_the_next_working_day_at_the_same_clock_time() {
    use renox::chrono::NaiveDateTime;

    let text = format!("{GROUP}\n[calendar]\nholidays = [\"2026-11-10\"]\n");
    let calendar = ClientConfig::parse(&text)
        .expect("a valid client.toml")
        .business_calendar();
    let local = |text: &str| -> NaiveDateTime { text.parse().expect("a time") };
    let due = |from: &str| {
        let at = calendar.from_local(local(from));
        calendar
            .local(calendar.add_working_days(at, 1))
            .naive_local()
    };

    assert_eq!(due("2026-11-05T10:00:00"), local("2026-11-06T10:00:00"));
    assert_eq!(due("2026-11-06T16:00:00"), local("2026-11-09T16:00:00"));
    assert_eq!(due("2026-11-07T10:00:00"), local("2026-11-09T10:00:00"));
    // Monday the 9th is the day before the holiday: due Wednesday the 11th.
    assert_eq!(due("2026-11-09T10:00:00"), local("2026-11-11T10:00:00"));
}

/// #94 AC1: without the payments module, the kernel's payment recorder is
/// `NoPayments`: nothing is found, the credit balance is 0, and recording,
/// allocating and reversing are refused.
#[renox::test]
async fn without_payments_the_recorder_finds_nothing_and_refuses_to_record() {
    use wl_kernel::Subject;
    use wl_kernel::invoicing::InvoiceId;
    use wl_kernel::payments::{
        AllocationTarget, NoPayments, PaymentId, PaymentMethod, PaymentRecorder, RecordPayment,
        ReversalReason, payment_recorder,
    };

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let provided = payment_recorder(state);
    let fallback: &dyn PaymentRecorder = &NoPayments;
    for recorder in [&*provided, fallback] {
        assert_eq!(recorder.find(app.db(), PaymentId(1)).await.unwrap(), None);
        assert_eq!(
            recorder
                .credit_balance(app.db(), PartyId(1), "AED")
                .await
                .unwrap(),
            0
        );
        assert!(
            recorder
                .payments_for(app.db(), &Subject::new("property.deposit", 1))
                .await
                .unwrap()
                .is_empty()
        );
        let record = recorder
            .record(
                state,
                RecordPayment::new(PartyId(1), 1, PaymentMethod::Cash, 100),
                None,
            )
            .await;
        assert!(matches!(
            record,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
        let allocate = recorder
            .allocate(
                state,
                PaymentId(1),
                AllocationTarget::Invoice(InvoiceId(1)),
                100,
            )
            .await;
        assert!(matches!(
            allocate,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
        let reverse = recorder
            .reverse(
                state,
                PaymentId(1),
                ReversalReason::EnteredInError,
                None,
                None,
            )
            .await;
        assert!(matches!(
            reverse,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
    }
}

/// #58 AC7: without the parties module the directory marks no contact point
/// invalid (`None`, for the kernel's directory as for `NoParties`), so a
/// give-up run still succeeds.
#[renox::test]
async fn without_parties_no_contact_point_is_marked_invalid() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let parties = wl_kernel::parties::directory(app.state());
    let since = renox::db::now();
    assert_eq!(
        parties
            .invalidate_contact(app.db(), 11, since, "hard_bounce")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        NoParties
            .invalidate_contact(app.db(), 11, since, "hard_bounce")
            .await
            .unwrap(),
        None
    );
}

/// #58 AC6: a module that registers a non-marketing type with
/// `mark_contact_invalid` alone fails at boot; a marketing type, or one with
/// another reaction beside it, registers.
#[test]
fn a_type_with_only_mark_contact_invalid_is_refused_unless_marketing() {
    use wl_kernel::notify::{
        self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, check_give_up,
    };

    let def = |key: &str, category: Category, reactions: &[GiveUpReaction]| {
        NotificationTypeDef::new(key, "invoicing")
            .category(category)
            .give_up(reactions)
            .template(NotifyChannel::Email, "en", Some("Hello"), "Hello.")
    };
    let mark = GiveUpReaction::MarkContactInvalid;
    let refused = std::panic::catch_unwind(|| {
        notify::register_type(def("kernel_package.alone", Category::Reminder, &[mark]));
    });
    assert!(refused.is_err());
    assert!(notify::notification_type("kernel_package.alone").is_none());
    assert!(check_give_up(Category::Reminder, &[mark]).is_err());

    notify::register_type(def(
        "kernel_package.marketing",
        Category::Marketing,
        &[mark],
    ));
    notify::register_type(def(
        "kernel_package.paired",
        Category::Reminder,
        &[mark, GiveUpReaction::StaffTask],
    ));
    assert!(notify::notification_type("kernel_package.marketing").is_some());
    assert!(notify::notification_type("kernel_package.paired").is_some());
}

/// #60 AC1/AC7: a type registered without a list gets the default of its
/// group (and the group's role); an own list is kept.
#[test]
fn a_type_registered_without_reactions_gets_the_group_default() {
    use wl_kernel::notify::{
        self, GROUP_INVOICES, GiveUpReaction, NotificationTypeDef, default_give_up,
    };

    let def = NotificationTypeDef::new("kernel_package.seeded", "invoicing").group(GROUP_INVOICES);
    let expected = default_give_up(&def);
    notify::register_type(def);
    let stored = notify::notification_type("kernel_package.seeded").unwrap();
    assert_eq!(stored.give_up, expected.reactions);
    assert_eq!(stored.staff_role.as_deref(), Some("receivables"));

    notify::register_type(
        NotificationTypeDef::new("kernel_package.own", "invoicing")
            .group(GROUP_INVOICES)
            .give_up(&[GiveUpReaction::None]),
    );
    let own = notify::notification_type("kernel_package.own").unwrap();
    assert_eq!(own.give_up, [GiveUpReaction::None]);
}

/// #60 AC2/AC3/AC4/AC7: statutory, OTP and ungrouped types, and the seeded
/// booking default, through the public registry.
#[test]
fn the_registry_seeds_statutory_otp_ungrouped_and_booking_types() {
    use wl_kernel::notify::{
        self, GROUP_BOOKINGS, GROUP_OTP, GiveUpReaction as G, NotificationTypeDef, Urgency,
    };

    let def = |key: &str| NotificationTypeDef::new(key, "kernel_package");
    notify::register_type(
        def("kernel_package.changed_terms_notice")
            .statutory(true)
            .urgency(Urgency::Urgent),
    );
    notify::register_type(
        def("kernel_package.code")
            .group(GROUP_OTP)
            .urgency(Urgency::Urgent),
    );
    notify::register_type(def("kernel_package.plain"));
    notify::register_type(def("kernel_package.booking").group(GROUP_BOOKINGS));
    let reactions = |key: &str| {
        notify::notification_type(key)
            .expect("registered")
            .give_up
            .clone()
    };
    assert_eq!(
        reactions("kernel_package.changed_terms_notice"),
        [G::MarkContactInvalid, G::StatutoryNotice]
    );
    assert_eq!(reactions("kernel_package.code"), [G::None]);
    assert_eq!(reactions("kernel_package.plain"), [G::StaffTask]);
    assert_eq!(reactions("kernel_package.booking"), [G::MarkContactInvalid]);
}

/// #76 AC1: without the invoicing module, debit notes refuse or are empty
/// and the cost poster is a no-op.
#[renox::test]
async fn without_invoicing_debit_notes_are_off_and_cost_is_a_no_op() {
    use wl_kernel::invoicing::{
        ReverseCost, SupplierDebitNoteId, cost_poster, supplier_debit_notes,
    };
    use wl_kernel::{PartyId, Subject};

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let notes = supplier_debit_notes(state);
    assert_eq!(
        SupplierDebitNoteId(7).subject().to_string(),
        "invoicing.supplier_debit_note:7"
    );
    assert_eq!(
        notes.find(&state.db, SupplierDebitNoteId(7)).await.unwrap(),
        None
    );
    assert_eq!(
        notes
            .debit_balance(&state.db, PartyId(1), 1, "AED")
            .await
            .unwrap(),
        0
    );
    let request = ReverseCost::new(SupplierDebitNoteId(7).subject(), "k")
        .line(Subject::new("sales.campaign", 1), 100);
    cost_poster(state)
        .reverse(state, request.clone())
        .await
        .unwrap();
    cost_poster(state).restore(state, request).await.unwrap();
    // #77 AC1: settling from a payment and cancelling refuse (422).
    assert!(matches!(
        notes
            .settle_from_payment(
                state,
                SupplierDebitNoteId(7),
                Subject::new("payments.payment", 1),
                100,
                "payments.allocation:1"
            )
            .await,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    assert!(matches!(
        notes
            .cancel(state, SupplierDebitNoteId(7), "issued in error", None)
            .await,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
}

/// #28: the late-entry contract. Thirty minutes is not late and thirty
/// minutes and a second is; a closed period needs approval, a locked one is
/// refused; a module opts an entity in with `LateEntryDef`.
#[test]
fn the_late_entry_check_and_the_opt_in_registry() {
    use renox::db::DateTime;
    use wl_kernel::late_entries::{self, assess};
    use wl_kernel::{ApprovalWhy, Assessment, LateEntryDef, Limits, Period};

    let at = |text: &str| text.parse::<DateTime>().expect("an instant");
    let entered = at("2026-11-03T06:00:00Z");
    let limits = Limits::default();
    assert_eq!(limits.flag_after_minutes, 30);
    assert_eq!(limits.approval_after_hours, 24);
    assert_eq!(
        assess(limits, at("2026-11-03T05:30:00Z"), entered, &Period::Open),
        Assessment::OnTime
    );
    assert!(
        assess(limits, at("2026-11-03T05:29:59Z"), entered, &Period::Open).needs_reason()
    );
    assert!(matches!(
        assess(limits, at("2026-11-03T04:00:00Z"), entered, &Period::Closed),
        Assessment::NeedsApproval {
            why: ApprovalWhy::ClosedPeriod,
            ..
        }
    ));
    assert!(matches!(
        assess(
            limits,
            at("2026-11-03T04:00:00Z"),
            entered,
            &Period::Locked("locked".into())
        ),
        Assessment::Refused(_)
    ));

    LateEntryDef::of("bookings.check_in", |_tx, _new| Box::pin(async { Ok(1) }))
        .business_time("checked_in_at")
        .parent("booking")
        .register();
    let def = late_entries::get("bookings.check_in").expect("registered");
    assert_eq!(def.business_time_field(), Some("checked_in_at"));
    assert_eq!(def.parent_field(), Some("booking"));
    assert_eq!(
        wl_kernel::permissions::LATE_ENTRIES_APPROVE,
        "late_entries.approve"
    );
}

/// #29: the integration contract. A module registers a connection with its
/// screens and manual fallback; the retry wait is 1, 2, 4 and 8 minutes and
/// then 15 minutes for ever.
#[test]
fn the_integration_registry_and_the_retry_wait() {
    use wl_kernel::integrations::{self, retry_wait};
    use wl_kernel::{Delivery, IntegrationDef};

    IntegrationDef::of("asp.acme", "invoicing", |_state, _call| {
        Box::pin(async { Ok(Delivery::Sent) })
    })
    .owner_role("accountant")
    .screens(["invoicing.invoice_list", "invoicing.invoice_detail"])
    .fallback("File the invoice on the ASP portal by hand.", None)
    .register();
    let def = integrations::get("asp.acme").expect("registered");
    assert_eq!(def.owner_role_name(), "accountant");
    assert_eq!(def.down_after(), 15);
    assert_eq!(def.screen_keys().len(), 2);

    let waits: Vec<i64> = (1..=6).map(|n| retry_wait(n).num_minutes()).collect();
    assert_eq!(waits, [1, 2, 4, 8, 15, 15]);
    assert_eq!(wl_kernel::permissions::INTEGRATIONS_VIEW, "integrations.view");
    assert_eq!(
        wl_kernel::permissions::INTEGRATIONS_MANAGE,
        "integrations.manage"
    );
}

/// #30: a connection takes inbound data that arrives late when its module
/// gives a matcher (by natural key) and the record to enter otherwise.
#[test]
fn a_connection_can_take_late_inbound_data() {
    use wl_kernel::integrations;
    use wl_kernel::late_entries::EntrySpec;
    use wl_kernel::{Delivery, Inbound, IntegrationDef, Match, Subject};

    IntegrationDef::of("gateway.acme", "payments", |_state, _call| {
        Box::pin(async { Ok(Delivery::Sent) })
    })
    .inbound(
        |_state, _item| {
            Box::pin(async { Ok(Match::Same(Subject::new("payments.payment", 4))) })
        },
        |item| EntrySpec::new("payments.payment", item.user_id, item.occurred_at),
    )
    .register();
    let def = integrations::get("gateway.acme").expect("registered");
    assert!(def.takes_inbound());
    let at = "2026-11-03T06:20:00Z".parse().expect("an instant");
    let item = Inbound::new("gateway.acme", "TX-889", at, renox::serde_json::Value::Null, 7);
    let spec = def.entry_for(&item).expect("an entry");
    assert_eq!(spec.subject_type, "payments.payment");
    assert_eq!(spec.occurred_at, at);
}

/// #32: a module marks a rule compliance-critical; a nightly rule is watched
/// for a missed run, which counts from 60 minutes after its planned time.
#[test]
fn a_critical_rule_is_registered_and_its_nightly_occurrences_are_planned() {
    use renox::db::DateTime;
    use wl_kernel::CriticalRule;
    use wl_kernel::compliance::{self, planned_daily};

    CriticalRule::new("DOC.RUL.03")
        .owner_role("md")
        .daily_at("01:00")
        .register();
    assert!(compliance::is_critical("DOC.RUL.03"));
    assert!(!compliance::is_critical("DOC.RUL.99"));
    let rule = compliance::rule("DOC.RUL.03").expect("registered");
    assert_eq!(rule.owner_role_name(), Some("md"));

    let at = |text: &str| text.parse::<DateTime>().expect("an instant");
    let planned = planned_daily(
        "DOC.RUL.03",
        "01:00",
        at("2026-11-02T00:00:00Z"),
        at("2026-11-03T00:00:00Z"),
    );
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].planned_at, at("2026-11-02T21:00:00Z"));
    assert_eq!(planned[0].occurrence_key, "DOC.RUL.03:2026-11-03");
    assert!(planned[0].planned_at <= compliance::missed_before(at("2026-11-02T22:00:00Z")));
}

/// #33: the digest of a date covers 07:00 the day before to 07:00 that day in
/// Asia/Dubai, and a confidential record shows only its type and number.
#[test]
fn the_digest_period_and_a_confidential_line() {
    use renox::db::DateTime;
    use wl_kernel::Subject;
    use wl_kernel::digest::{self, Item, ItemKind, Section};

    let date = "2026-11-04".parse().expect("a date");
    let (from, until) = digest::period_for(date);
    let at = |text: &str| text.parse::<DateTime>().expect("an instant");
    assert_eq!(from, at("2026-11-03T03:00:00Z"));
    assert_eq!(until, at("2026-11-04T03:00:00Z"));

    let secret = Item {
        kind: ItemKind::Late,
        record: Subject::new("bookings.check_in", 7),
        number: Some("CI-9".into()),
        by: "Anna".into(),
        reason: "router down".into(),
        at: at("2026-11-03T06:30:00Z"),
        occurred_at: Some(at("2026-11-03T05:05:00Z")),
        fired: Vec::new(),
        confidential: true,
    };
    let text = digest::render(
        from,
        until,
        &[Section {
            scope: "Tower A".into(),
            items: vec![secret],
        }],
    );
    assert!(text.contains("- check in CI-9"), "{text}");
    assert!(!text.contains("Anna") && !text.contains("router down"), "{text}");
    assert_eq!(digest::TASK_AT, "07:00");
}

/// #43: without an ASP adapter no lookup is made and the identifier is
/// entered by hand; a change of a party names its fields only.
#[renox::test]
async fn without_an_asp_adapter_the_recipient_lookup_is_unavailable() {
    use wl_kernel::einvoice::{self, Lookup, NoRecipientLookup, RecipientLookup};
    use wl_kernel::parties::PartyUpdated;

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let entity = LegalEntity::default_entity(app.db()).await.unwrap();
    assert_eq!(NoRecipientLookup.connection(&entity), None);
    assert_eq!(
        einvoice::lookup(app.state(), &entity, "100123456700003")
            .await
            .unwrap(),
        Lookup::Unavailable
    );
    assert!(
        einvoice::available_connection(app.state(), &entity)
            .await
            .unwrap()
            .is_none()
    );
    let event = PartyUpdated {
        party: PartyId(1),
        changed_fields: vec!["einvoice_recipient_id".into()],
    };
    let json = renox::serde_json::to_value(&event).unwrap();
    assert_eq!(json["changed_fields"][0], "einvoice_recipient_id");
}

/// #148: a planned visit carries its planned due date, a chargeable ticket
/// its billed party, and the compliance flag events name the subject, the
/// category and the reason; without ticketing nothing is opened.
#[renox::test]
async fn a_planned_visit_and_the_compliance_flag_events_are_part_of_the_ticket_contract() {
    use wl_kernel::tickets::{
        ComplianceFlagCleared, ComplianceFlagRaised, ComplianceFlagReason, OpenTicket,
        TaskCreator, task_creator,
    };
    use wl_kernel::Subject;

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let visit = OpenTicket::new(Subject::new("property.system", 4), "fire_safety", "Fire alarm")
        .planned_due(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap())
        .bill_to(PartyId(9));
    assert_eq!(visit.planned_due.unwrap().to_string(), "2026-11-01");
    assert_eq!(visit.bill_party, Some(PartyId(9)));
    let creator = task_creator(app.state());
    assert_eq!(creator.open(app.state(), visit).await.unwrap(), None);

    let reason = ComplianceFlagReason::VisitMissed.as_str().to_owned();
    let raised = ComplianceFlagRaised {
        subject: Subject::new("property.system", 4),
        category: "fire_safety".into(),
        reason: reason.clone(),
        contract: None,
    };
    let cleared = ComplianceFlagCleared {
        subject: raised.subject.clone(),
        category: raised.category.clone(),
        reason,
        contract: Some(3),
    };
    assert_eq!(renox::serde_json::to_value(&raised).unwrap()["reason"], "visit_missed");
    assert_eq!(renox::serde_json::to_value(&cleared).unwrap()["contract"], 3);
    let _: &dyn TaskCreator = &*creator;
}

/// #149, #150: a planned visit names its service contract, and the contract
/// and no-show events name the contract, the visits and the vendor; the
/// contract number is its own yearly series.
#[renox::test]
async fn a_service_contract_and_its_events_are_part_of_the_ticket_contract() {
    use wl_kernel::tickets::{
        OpenTicket, SERVICE_CONTRACT_SERIES, ServiceContractActivated, ServiceContractEnded,
        ServiceContractId, TicketId, VendorNoShow, task_creator,
    };
    use wl_kernel::Subject;

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let contract = ServiceContractId(7);
    assert_eq!(contract.subject().to_string(), "ticketing.service_contract:7");
    let visit = OpenTicket::new(Subject::new("property.system", 4), "fire_safety", "Fire alarm")
        .planned_due(NaiveDate::from_ymd_opt(2026, 12, 20).unwrap())
        .under_contract(contract);
    assert_eq!(visit.contract, Some(contract));
    assert_eq!(
        task_creator(app.state()).open(app.state(), visit).await.unwrap(),
        None,
        "without ticketing nothing is opened"
    );

    let number = SERVICE_CONTRACT_SERIES
        .next_allowing_gaps(app.db(), None, None, day())
        .await
        .unwrap();
    assert_eq!(number, "AMC-2026-0001");

    let activated = ServiceContractActivated {
        contract,
        vendor: PartyId(5),
        subjects: vec![Subject::new("property.system", 4)],
        start: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        end: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
    };
    assert_eq!(renox::serde_json::to_value(&activated).unwrap()["end"], "2026-12-31");
    let ended = ServiceContractEnded {
        contract,
        outcome: "expired".into(),
        new_version: None,
        visits_cancelled: vec![TicketId(3), TicketId(4)],
        visits_left: vec![TicketId(1), TicketId(2)],
    };
    let json = renox::serde_json::to_value(&ended).unwrap();
    assert_eq!(json["visits_cancelled"].as_array().unwrap().len(), 2);
    assert_eq!(json["visits_left"].as_array().unwrap().len(), 2);
    let no_show = VendorNoShow {
        ticket: TicketId(42),
        vendor: PartyId(5),
        contract: Some(contract),
        scheduled_at: "2026-11-03T06:00:00Z".parse().unwrap(),
    };
    assert_eq!(renox::serde_json::to_value(&no_show).unwrap()["contract"], 7);
}

/// #102: without the `payments` module the cheque contract finds nothing,
/// owes nothing and refuses a registration; a bounce fee adds no charge; the
/// events name the cheque, the party and the amounts.
#[renox::test]
async fn the_cheque_contract_falls_back_when_payments_is_off() {
    use wl_kernel::Subject;
    use wl_kernel::cheques::{
        BounceFeeCharge, ChequeBounceResolved, ChequeBounced, ChequeId, ChequeOutcome,
        ChequeSchedule, RegisterCheque, ScheduledCheque, bounce_fees, cheques,
    };
    use wl_kernel::invoicing::VatCategory;

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let contract = cheques(state);
    assert_eq!(ChequeId(4).subject().to_string(), "payments.cheque:4");
    assert!(contract.find(app.db(), ChequeId(4)).await.unwrap().is_none());
    let lease = Subject::new("property.lease", 9);
    assert!(contract.of_agreement(app.db(), &lease).await.unwrap().is_empty());
    assert_eq!(
        contract.open_bounced_amount(app.db(), PartyId(7), "AED").await.unwrap(),
        0
    );
    let cheque = RegisterCheque::new(PartyId(7), 1, "ENBD", "1042", day(), 2_000_000)
        .link(Subject::new("invoicing.invoice", 5));
    assert!(contract.register(state, cheque, None).await.is_err());
    let schedule = ChequeSchedule::new(lease.clone(), PartyId(7), 1, "ENBD")
        .cheque(ScheduledCheque::new("1", day(), 1_000_000))
        .cheque(ScheduledCheque::new("2", day(), 1_000_000));
    assert_eq!(schedule.total(), 2_000_000);
    assert!(contract.register_schedule(state, schedule, None).await.is_err());

    let charge = BounceFeeCharge {
        cheque: ChequeId(4),
        party: PartyId(7),
        legal_entity_id: 1,
        agreement: Some(lease),
        charge_type: "bounce_fee".into(),
        amount: 50_000,
        currency: "AED".into(),
        vat_category: VatCategory::Standard,
        due: day(),
    };
    assert_eq!(bounce_fees(state).add(state, charge).await.unwrap(), None);

    let bounced = ChequeBounced {
        cheque: ChequeId(4),
        party: PartyId(7),
        credit_group: None,
        amount: 2_000_000,
        honoured_amount: 600_000,
        open_amount: 1_400_000,
        currency: "AED".into(),
        reason: "insufficient funds".into(),
        presentation: 1,
        linked: Vec::new(),
    };
    assert_eq!(renox::serde_json::to_value(&bounced).unwrap()["open_amount"], 1_400_000);
    let resolved = ChequeBounceResolved {
        cheque: ChequeId(4),
        party: PartyId(7),
        outcome: ChequeOutcome::Cleared,
        others_open: Vec::new(),
    };
    assert_eq!(renox::serde_json::to_value(&resolved).unwrap()["outcome"], "cleared");
}

/// #106: without the module that owns the charges nothing is invoiced, so a
/// cheque write-off asks for the charge to be invoiced first; the answer of an
/// invoice write-off names the approval request that waits.
#[renox::test]
async fn a_charge_is_not_invoiced_when_nothing_provides_it() {
    use wl_kernel::Subject;
    use wl_kernel::cheques::{CHARGE_SUBJECT_TYPE, charge_invoicer};
    use wl_kernel::invoicing::WriteOffAnswer;

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let charge = Subject::new(CHARGE_SUBJECT_TYPE, 12);
    assert_eq!(charge.to_string(), "invoicing.charge:12");
    assert_eq!(charge_invoicer(state).invoice(state, &charge).await.unwrap(), None);
    assert_eq!(
        WriteOffAnswer::Pending { request: 3 },
        WriteOffAnswer::Pending { request: 3 }
    );
    assert_ne!(WriteOffAnswer::WrittenOff, WriteOffAnswer::Pending { request: 3 });
}

/// #108: without the `payments` module the portal sees no published
/// deductions and a dispute of them is refused; the events name the deposit.
#[renox::test]
async fn the_deposit_deductions_fall_back_when_payments_is_off() {
    use wl_kernel::Subject;
    use wl_kernel::deposits::{
        DepositDeductionsDisputed, DepositOutcome, DepositSettled, DisputeDeductions,
        deposit_deductions,
    };
    use wl_kernel::payments::DepositId;

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let contract = deposit_deductions(state);
    assert!(contract.published_for(state, PartyId(7)).await.unwrap().is_empty());
    let request = DisputeDeductions {
        deposit: DepositId(3),
        version: 1,
        lines: vec![4],
        comment: "The wall was painted before".into(),
    };
    assert!(contract.dispute(state, PartyId(7), request).await.is_err());

    let disputed = DepositDeductionsDisputed {
        deposit: DepositId(3),
        source: Subject::new("property.lease", 9),
        party: PartyId(7),
        version: 1,
        lines: vec![4],
        raised_on: day(),
    };
    assert_eq!(renox::serde_json::to_value(&disputed).unwrap()["lines"][0], 4);
    let settled = DepositSettled {
        deposit: DepositId(3),
        source: Subject::new("property.lease", 9),
        party: PartyId(7),
        outcome: DepositOutcome::Forfeited,
        deductions: 250_000,
        refund_amount: 0,
        new_holder: None,
    };
    assert_eq!(renox::serde_json::to_value(&settled).unwrap()["outcome"], "forfeited");
}

/// #115: without the `payments` module a deposit cannot be adjusted and no
/// top-up charge is made; the event names the amounts and the charge.
#[renox::test]
async fn the_deposit_adjustment_falls_back_when_payments_is_off() {
    use wl_kernel::Subject;
    use wl_kernel::deposits::{
        AdjustDeposit, DepositAdjusted, TopUpCharge, deposit_top_ups, deposits,
    };
    use wl_kernel::payments::{DepositId, RefundId};

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let state = app.state();
    let request = AdjustDeposit {
        source: Subject::new("property.lease", 9),
        new_amount: 600_000,
        source_version: 2,
        effective_on: day(),
    };
    assert!(deposits(state).adjust(state, request, None).await.is_err());

    let charge = TopUpCharge {
        deposit: DepositId(3),
        party: PartyId(7),
        legal_entity_id: 1,
        source: Subject::new("property.lease", 9),
        source_version: 2,
        amount: 100_000,
        currency: "AED".into(),
        due: day(),
    };
    assert_eq!(deposit_top_ups(state).request(state, charge).await.unwrap(), None);

    let adjusted = DepositAdjusted {
        deposit: DepositId(3),
        source: Subject::new("property.lease", 9),
        party: PartyId(7),
        source_version: 2,
        old_amount: 500_000,
        new_amount: 600_000,
        effective_on: day(),
        top_up_charge: Some(Subject::new("invoicing.charge", 12)),
        part_refund: Some(RefundId(4)),
    };
    let json = renox::serde_json::to_value(&adjusted).unwrap();
    assert_eq!(json["new_amount"], 600_000);
    assert_eq!(json["part_refund"], 4);
}

/// #88: without the `invoicing` module there is no approved budget and no
/// category is owner drawings; the event names the version it replaced.
#[renox::test]
async fn the_company_budgets_fall_back_when_invoicing_is_off() {
    use wl_kernel::budgets::{
        COMPANY_BUDGET_SUBJECT_TYPE, CompanyBudgetApproved, CompanyBudgetId, company_budgets,
    };

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let budgets = company_budgets(app.state());
    assert_eq!(
        budgets
            .approved_amount(app.db(), 1, "marketing", day())
            .await
            .unwrap(),
        None
    );
    assert!(!budgets.is_owner_drawings(app.db(), "owner_drawings").await.unwrap());

    assert_eq!(COMPANY_BUDGET_SUBJECT_TYPE, "invoicing.company_budget");
    let approved = CompanyBudgetApproved {
        budget: CompanyBudgetId(2),
        legal_entity_id: 1,
        fiscal_year: 2027,
        version: 2,
        superseded: Some(CompanyBudgetId(1)),
    };
    let json = renox::serde_json::to_value(&approved).unwrap();
    assert_eq!(json["superseded"], 1);
    assert_eq!(json["fiscal_year"], 2027);
}

/// #89: a draft line names its charge type for budget vs actual, an older
/// stored line without one still reads, and without the `invoicing` module the
/// actual of a category is 0.
#[renox::test]
async fn a_draft_line_names_its_charge_type_and_the_actual_falls_back() {
    use wl_kernel::budgets::{CompanyBudgetVarianceReached, company_budgets};
    use wl_kernel::invoicing::{DraftLine, VatCategory};

    let line = DraftLine::new("Rent, March", 100_000, VatCategory::Exempt).charge_type(" rent ");
    assert_eq!(line.charge_type.as_deref(), Some("rent"));
    assert_eq!(DraftLine::new("x", 1, VatCategory::Exempt).charge_type(" ").charge_type, None);
    let mut json = renox::serde_json::to_value(&line).unwrap();
    json.as_object_mut().unwrap().remove("charge_type");
    let older: DraftLine = renox::serde_json::from_value(json).unwrap();
    assert_eq!(older.charge_type, None);

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let actual = company_budgets(app.state())
        .actual_amount(app.db(), 1, "marketing", day())
        .await
        .unwrap();
    assert_eq!(actual, 0);
    let reached = CompanyBudgetVarianceReached {
        legal_entity_id: 1,
        category: "marketing".into(),
        month: day(),
        budget: 1_000_000,
        actual: 1_050_000,
        percent_used: Some(105),
    };
    assert_eq!(renox::serde_json::to_value(&reached).unwrap()["percent_used"], 105);
}

/// #90: posting cost goes through `post_cost`, which a cost code that is not
/// owner drawings passes to the poster (a no-op without one).
#[renox::test]
async fn posting_cost_without_a_poster_is_a_no_op() {
    use wl_kernel::Subject;
    use wl_kernel::invoicing::{PostCost, post_cost};

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let request = PostCost::new(
        Subject::new("invoicing.supplier_bill", 4),
        "invoicing.supplier_bill_line:9",
        Subject::new("property.unit", 2),
        105_000,
    )
    .cost_code(" Repairs ");
    assert_eq!(request.cost_code.as_deref(), Some("repairs"));
    post_cost(app.state(), request.clone()).await.expect("a no-op");
    post_cost(app.state(), PostCost { cost_code: None, ..request })
        .await
        .expect("a no-op");
}

/// #93: the event of a projected balance below the threshold carries the
/// entity, the first week below, the balance and the threshold.
#[test]
fn the_below_threshold_event_names_the_first_week() {
    use wl_kernel::cash_forecast::CashForecastBelowThreshold;

    let event = CashForecastBelowThreshold {
        legal_entity_id: 1,
        week: 6,
        week_start: NaiveDate::from_ymd_opt(2027, 2, 15).unwrap(),
        projected_balance: 1_500_000,
        threshold: 2_000_000,
    };
    let json = renox::serde_json::to_value(&event).unwrap();
    assert_eq!(json["week"], 6);
    assert_eq!(json["week_start"], "2027-02-15");
    assert_eq!(json["projected_balance"], 1_500_000);
    let back: CashForecastBelowThreshold = renox::serde_json::from_value(json).unwrap();
    assert_eq!(back, event);
}

/// #119: without the `invoicing` module there is no drawings category and a
/// drawing request is refused; the cheque of a drawing names its request.
#[renox::test]
async fn owner_drawings_without_invoicing_are_refused() {
    use wl_kernel::Subject;
    use wl_kernel::cheques::RegisterCheque;
    use wl_kernel::drawings::{
        DRAWING_REQUEST_SUBJECT_TYPE, DrawingRequest, owner_drawings,
    };

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let drawings = owner_drawings(app.state());
    assert!(drawings
        .category(app.db(), "owner_drawings")
        .await
        .unwrap()
        .is_none());
    let refused = drawings
        .request(
            app.state(),
            DrawingRequest {
                legal_entity_id: 1,
                category: "owner_drawings".into(),
                source: Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, 1),
                amount: 1_500_000,
                requested_by: 1,
            },
        )
        .await;
    assert!(refused.is_err());
    let cheque = RegisterCheque::new(PartyId(1), 1, "ENBD", "1042", day(), 400_000)
        .issued()
        .drawing(7);
    assert_eq!(cheque.drawing, Some(7));
}

/// #72: the events of an e-invoice transmission name it, its document, the
/// entity and the ASP's error codes.
#[test]
fn the_einvoice_events_name_the_transmission_and_the_document() {
    use wl_kernel::Subject;
    use wl_kernel::einvoice::{EInvoiceAccepted, EInvoiceRejected};

    let rejected = EInvoiceRejected {
        transmission: Subject::new("invoicing.einvoice_transmission", 4),
        document: Subject::new("invoicing.invoice", 9),
        legal_entity_id: 2,
        asp_reference: Some("ASP-77".into()),
        error_codes: vec!["BR-CO-15".into(), "BR-S-08".into()],
    };
    let json = renox::serde_json::to_value(&rejected).unwrap();
    assert_eq!(json["error_codes"].as_array().map(Vec::len), Some(2));
    let back: EInvoiceRejected = renox::serde_json::from_value(json).unwrap();
    assert_eq!(back, rejected);
    let accepted = EInvoiceAccepted {
        transmission: rejected.transmission.clone(),
        document: rejected.document.clone(),
        legal_entity_id: 2,
        asp_reference: Some("ASP-77".into()),
    };
    assert_eq!(accepted.document.to_string(), "invoicing.invoice:9");
}

/// #87: without the `ticketing` module no service contract is known, so the
/// coverage of a day is unknown.
#[renox::test]
async fn without_ticketing_a_service_contract_covers_nothing_known() {
    use wl_kernel::tickets::{ContractCover, ServiceContractId, service_contract_coverage};

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let cover = service_contract_coverage(app.state())
        .covers(app.db(), ServiceContractId(12), day())
        .await
        .unwrap();
    assert_eq!(cover, ContractCover::Unknown);
    let after = ContractCover::AfterEnd {
        ends_on: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
    };
    assert_eq!(renox::serde_json::to_value(after).unwrap()["AfterEnd"]["ends_on"], "2026-09-30");
}

/// #139: without the `sales` module attribution and lead conversion do
/// nothing and report it.
#[renox::test]
async fn without_sales_attribution_and_lead_conversion_do_nothing() {
    use wl_kernel::sales::{
        Attribute, FirstSale, LeadId, campaign_attribution, lead_conversion,
    };
    use wl_kernel::Subject;

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let attribution = campaign_attribution(app.state());
    assert!(!attribution
        .attribute(app.state(), Attribute::party(PartyId(3), "referral"))
        .await
        .unwrap());
    let sale = FirstSale::new(PartyId(3), Subject::new("invoicing.invoice", 9), 200_000, day());
    assert!(!attribution.record_first_sale(app.state(), sale).await.unwrap());
    assert!(!lead_conversion(app.state())
        .win(app.state(), LeadId(1), Subject::new("property.lease", 4))
        .await
        .unwrap());
    let merged = wl_kernel::PartiesMerged {
        survivor: PartyId(1),
        merged: PartyId(2),
    };
    assert_eq!(renox::serde_json::to_value(merged).unwrap()["merged"], 2);
}

/// #216, #217: without the `hr`, `ticketing` and `documents` modules nobody
/// works anywhere, no checklist is started and no SOP is pending.
#[renox::test]
async fn without_hr_the_workforce_checklists_and_sop_acknowledgements_are_empty() {
    use wl_kernel::documents::{SopStaff, sop_acknowledgements};
    use wl_kernel::tickets::{ChecklistOutcome, checklists};
    use wl_kernel::workforce::{EmployeeId, workforce};
    use wl_kernel::Subject;

    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let branch = Subject::new("property.site", 2);
    assert!(workforce(app.state())
        .employees_in(app.db(), "driver", &branch)
        .await
        .unwrap()
        .is_empty());
    // #53: nobody is on the staff list, no profile and no account either.
    assert!(workforce(app.state()).all_staff(app.db()).await.unwrap().is_empty());
    assert_eq!(
        workforce(app.state())
            .staff(app.state(), EmployeeId(7))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        workforce(app.state())
            .employee_of_user(app.db(), 3)
            .await
            .unwrap(),
        None
    );
    let employee = EmployeeId(7).subject();
    assert_eq!(
        checklists(app.state())
            .start(app.state(), "ppe", employee.clone())
            .await
            .unwrap(),
        None
    );
    let who = SopStaff {
        employee,
        role: "driver".into(),
        scopes: vec![branch],
    };
    assert!(sop_acknowledgements(app.state())
        .pending_for(app.state(), &who)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(renox::serde_json::to_value(ChecklistOutcome::Passed).unwrap(), "passed");
}
