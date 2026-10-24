//! BL-AUT-14 (#410): alert on missing export evidence and open the VAT
//! correction (`AUT.RUL.15`, compliance-critical, 08 §2 re-export, §4.1). A
//! zero-rated export sale registered with its supply date: the accountant is
//! told at 60 days without exit evidence (a customs exit certificate, a bill of
//! lading or an airway bill), at 90 days the sale is flagged "5% VAT due" and
//! the accountant gets a task linking the invoice; nothing is issued
//! automatically. Any one piece of evidence stops the rule; evidence after the
//! flag leaves the flag and tells the accountant. A run that fails or is
//! missing alerts the Owner / GM and the company admin.

mod support;

use std::time::Duration;

use auto::ExportSale;
use auto::export_rule::{self, NOTICE_TYPE, RULE, task_key};
use auto::export_sale::{AIRWAY_BILL, BILL_OF_LADING, EXIT_CERTIFICATE};
use auto::export_sales;
use renox::auth::permissions;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{Garage, boot, day};
use wl_kernel::automation::{RuleRun, transitions};
use wl_kernel::compliance::{AlertKind, ComplianceAlert, occurrence_key, schedule_subject};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::notify::{NotifyChannel, NotifyRecipient, NotifyRequest};
use wl_kernel::{COMPANY_ADMIN_ROLE, PartyId, Subject};

/// A user holding `role` (and the grants), registered once.
async fn holder(t: &Garage, name: &str, role: &str, grants: &[&str]) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    if let Some(found) = User::find_by_email(t.db(), &email).await.unwrap() {
        return found;
    }
    permissions::define_role(t.db(), role, grants)
        .await
        .unwrap();
    let user = User::register(t.db(), name, &email, "password123")
        .await
        .unwrap();
    user.assign_role(t.db(), role).await.unwrap();
    user
}

/// The accountant Adel: holds the role, registers export sales.
async fn accountant(t: &Garage) -> (User, AuthUser) {
    let user = holder(
        t,
        "Adel",
        auto::permissions::ACCOUNTANT_ROLE,
        &[
            auto::permissions::EXPORT_SALES_VIEW,
            auto::permissions::EXPORT_SALES_MANAGE,
        ],
    )
    .await;
    let auth = t.auth(&user).await;
    (user, auth)
}

/// The users a request is for.
fn users_of(request: &NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// An issued invoice of AED 25,000, zero-rated, for Gulf Movers Trading.
async fn zero_rated_invoice(t: &Garage) -> i64 {
    t.people.add(30, "Gulf Movers Trading");
    let books = invoicer(t.state());
    let draft =
        InvoiceDraft::new(PartyId(30), Subject::new("auto.export", 140)).line(DraftLine::new(
            "Brake pads, export to Mombasa",
            2_500_000,
            VatCategory::ZeroRated,
        ));
    let id = books
        .draft(t.db(), draft)
        .await
        .unwrap()
        .expect("an invoice");
    books.issue(t.state(), id, None).await.unwrap();
    id.0
}

/// Given INV zero-rated AED 25,000 supplied today with no evidence, the
/// accountant is told on day 60 (once), and on day 90 the sale is flagged and a
/// correction task linking the invoice exists; the credit note and the new
/// invoice are not issued.
#[renox::test]
async fn aut_14_the_accountant_is_told_at_60_days_and_the_sale_is_flagged_at_90() {
    let t = boot().await;
    let (adel_user, adel) = accountant(&t).await;
    let invoice = zero_rated_invoice(&t).await;
    let sale = export_sales::record(t.state(), &adel, invoice, today())
        .await
        .unwrap();
    assert_eq!(sale.gross_total, 2_500_000);
    assert_eq!(sale.party_id, 30);
    let supplied = sale.supply_date;

    let early = supplied + renox::chrono::Duration::days(59);
    export_rule::run_daily(t.state(), early).await.unwrap();
    assert!(
        t.messages.of_type(NOTICE_TYPE).is_empty(),
        "day 59 is early"
    );

    let sixty = supplied + renox::chrono::Duration::days(60);
    export_rule::run_daily(t.state(), sixty).await.unwrap();
    let told = t.messages.of_type(NOTICE_TYPE);
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), [adel_user.id]);
    let text = support::Messages::text(&told[0], NotifyChannel::Email);
    assert!(text.contains(&sale.number()), "{text}");
    assert!(text.contains("60 days"), "{text}");
    let after = ExportSale::find_or_404(t.db(), sale.id).await.unwrap();
    assert!(after.notified_at.is_some());
    assert!(after.vat_flagged_at.is_none());
    assert!(t.tickets.with_key("auto.export_vat").is_empty());

    // Each step once.
    export_rule::run_daily(t.state(), sixty).await.unwrap();
    export_rule::run_daily(t.state(), sixty + renox::chrono::Duration::days(1))
        .await
        .unwrap();
    assert_eq!(t.messages.of_type(NOTICE_TYPE).len(), 1);

    let eighty_nine = supplied + renox::chrono::Duration::days(89);
    export_rule::run_daily(t.state(), eighty_nine)
        .await
        .unwrap();
    assert!(t.tickets.with_key("auto.export_vat").is_empty());
    let ninety = supplied + renox::chrono::Duration::days(90);
    export_rule::run_daily(t.state(), ninety).await.unwrap();
    let flagged = ExportSale::find_or_404(t.db(), sale.id).await.unwrap();
    assert!(flagged.vat_flagged_at.is_some());
    let tasks = t.tickets.with_key(&task_key(sale.id));
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].ticket.subject,
        sale.invoice_subject(),
        "it links the invoice"
    );
    assert_eq!(tasks[0].assignee_user(), Some(adel_user.id));
    assert!(
        tasks[0].ticket.title.contains("5% VAT is due"),
        "{}",
        tasks[0].ticket.title
    );
    export_rule::run_daily(t.state(), ninety + renox::chrono::Duration::days(1))
        .await
        .unwrap();
    assert_eq!(t.tickets.with_key(&task_key(sale.id)).len(), 1);
    // Nothing was issued: the only invoice is the sale's own.
    assert_eq!(t.books.all().len(), 1);
}

/// A late run fires every step that is due, in order: the notice, then the flag.
#[renox::test]
async fn aut_14_a_missed_run_fires_the_due_steps_late() {
    let t = boot().await;
    let (_, adel) = accountant(&t).await;
    let invoice = zero_rated_invoice(&t).await;
    let sale = export_sales::record(t.state(), &adel, invoice, today())
        .await
        .unwrap();
    let late = sale.supply_date + renox::chrono::Duration::days(95);
    export_rule::run_daily(t.state(), late).await.unwrap();
    let after = ExportSale::find_or_404(t.db(), sale.id).await.unwrap();
    assert!(after.notified_at.is_some() && after.vat_flagged_at.is_some());
    assert_eq!(t.messages.of_type(NOTICE_TYPE).len(), 1);
    assert_eq!(t.tickets.with_key(&task_key(sale.id)).len(), 1);
}

/// Given a bill of lading attached on day 19, nothing fires; the same with an
/// airway bill and with a customs exit certificate.
#[renox::test]
async fn aut_14_any_one_piece_of_evidence_stops_the_rule() {
    let t = boot().await;
    let (_, adel) = accountant(&t).await;
    for (index, kind) in [BILL_OF_LADING, AIRWAY_BILL, EXIT_CERTIFICATE]
        .into_iter()
        .enumerate()
    {
        t.people.add(30, "Gulf Movers Trading");
        let books = invoicer(t.state());
        let draft = InvoiceDraft::new(PartyId(30), Subject::new("auto.export", 140 + index as i64))
            .line(DraftLine::new("Export", 1_000_000, VatCategory::ZeroRated));
        let id = books.draft(t.db(), draft).await.unwrap().unwrap();
        books.issue(t.state(), id, None).await.unwrap();
        let sale = export_sales::record(t.state(), &adel, id.0, today())
            .await
            .unwrap();
        export_sales::attach_evidence(t.state(), &adel, sale.id, kind, "EVD-1")
            .await
            .unwrap();
    }
    for days in [60, 90, 120] {
        export_rule::run_daily(t.state(), today() + renox::chrono::Duration::days(days))
            .await
            .unwrap();
    }
    assert!(t.messages.of_type(NOTICE_TYPE).is_empty());
    assert!(t.tickets.tasks().is_empty());
    for sale in export_sales::all(t.db()).await.unwrap() {
        assert!(sale.notified_at.is_none() && sale.vat_flagged_at.is_none());
    }
}

/// Evidence after the 90 days leaves the flag, tells the accountant once and
/// reverses nothing.
#[renox::test]
async fn aut_14_evidence_after_day_90_keeps_the_flag_and_tells_the_accountant() {
    let t = boot().await;
    let (adel_user, adel) = accountant(&t).await;
    let invoice = zero_rated_invoice(&t).await;
    let sale = export_sales::record(t.state(), &adel, invoice, today())
        .await
        .unwrap();
    export_rule::run_daily(
        t.state(),
        sale.supply_date + renox::chrono::Duration::days(91),
    )
    .await
    .unwrap();
    let before = t.messages.of_type(NOTICE_TYPE).len();

    export_sales::attach_evidence(t.state(), &adel, sale.id, BILL_OF_LADING, "BL-7788")
        .await
        .unwrap();
    export_sales::attach_evidence(t.state(), &adel, sale.id, AIRWAY_BILL, "AW-12")
        .await
        .unwrap();
    let kept = ExportSale::find_or_404(t.db(), sale.id).await.unwrap();
    assert!(kept.vat_flagged_at.is_some(), "the flag stays");
    assert!(kept.late_evidence_notified_at.is_some());
    let notices = t.messages.of_type(NOTICE_TYPE);
    assert_eq!(notices.len(), before + 1, "told once");
    let last = notices.last().unwrap();
    assert_eq!(users_of(last), [adel_user.id]);
    let text = support::Messages::text(last, NotifyChannel::Email);
    assert!(
        text.contains("BL-7788") || text.contains("Bill of lading"),
        "{text}"
    );
    assert_eq!(t.books.all().len(), 1, "nothing was reversed or issued");
    assert_eq!(kept.evidence(t.db()).await.unwrap().len(), 2);
}

/// Given the 08:00 run ends `failed` (the notifier is down), the Owner / GM
/// role and the company admin are alerted once, after the fifth failure, and the
/// owner has a task; a successful rerun closes the alert.
#[renox::test]
async fn aut_14_a_failed_run_alerts_the_owner_and_a_successful_rerun_closes_it() {
    let t = boot().await;
    let owner = holder(&t, "Omar Owner", auto::permissions::OWNER_ROLE, &[]).await;
    let admin = holder(&t, "Aisha Admin", COMPANY_ADMIN_ROLE, &[]).await;
    let ada = holder(
        &t,
        "Ada Rerun",
        "automation_admin",
        &[wl_kernel::permissions::AUTOMATIONS_RETRY],
    )
    .await;
    let (_, adel) = accountant(&t).await;
    let invoice = zero_rated_invoice(&t).await;
    let sale = export_sales::record(t.state(), &adel, invoice, today())
        .await
        .unwrap();
    let on = sale.supply_date + renox::chrono::Duration::days(60);
    t.messages.fail(true);
    t.messages
        .always_works(wl_kernel::compliance::service::NOTIFICATION_TYPE);
    assert!(wl_kernel::compliance::is_critical(RULE));

    for wait in [60_u64, 300, 900, 3600] {
        t.app
            .at_travelled_time(export_rule::run_daily(t.state(), on))
            .await
            .unwrap();
        assert!(
            ComplianceAlert::open(t.db()).await.unwrap().is_empty(),
            "no alert before the fifth failure"
        );
        t.app.travel(Duration::from_secs(wait));
    }
    t.app
        .at_travelled_time(export_rule::run_daily(t.state(), on))
        .await
        .unwrap();

    let alerts: Vec<_> = ComplianceAlert::open(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == RULE)
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].kind, AlertKind::Failed);
    assert_eq!(alerts[0].owner_role, auto::permissions::OWNER_ROLE);
    let told = t
        .messages
        .of_type(wl_kernel::compliance::service::NOTIFICATION_TYPE);
    assert_eq!(told.len(), 1);
    let mut recipients = users_of(&told[0]);
    recipients.sort_unstable();
    let mut wanted = vec![owner.id, admin.id];
    wanted.sort_unstable();
    assert_eq!(recipients, wanted, "the owner and the company admin");
    let owner_task = t
        .tickets
        .tasks()
        .into_iter()
        .find(|task| task.assignee_user() == Some(owner.id))
        .expect("the owner has a task");
    assert!(owner_task.is_open());

    // The notifier works again; the admin runs the failed run again.
    t.messages.fail(false);
    let failed =
        RuleRun::find_occurrence(t.db(), RULE, &schedule_subject(), &occurrence_key(RULE, on))
            .await
            .unwrap()
            .expect("the run is kept");
    let rerun = t.auth(&ada).await;
    let _outcome = failed
        .transition(transitions::RERUN)
        .by(&rerun)
        .run(t.state())
        .await
        .unwrap();
    t.app
        .at_travelled_time(export_rule::run_daily(t.state(), on))
        .await
        .unwrap();
    assert!(
        ComplianceAlert::open(t.db())
            .await
            .unwrap()
            .into_iter()
            .all(|a| a.rule != RULE),
        "the rerun closed the alert"
    );
    let closed = t
        .tickets
        .tasks()
        .into_iter()
        .find(|task| task.assignee_user() == Some(owner.id))
        .unwrap();
    assert!(!closed.is_open(), "the owner's task is completed");
    assert_eq!(
        t.messages.of_type(NOTICE_TYPE).len(),
        1,
        "the notice went out"
    );
}

/// A day with no run: from 09:00 the kernel's check raises the alert for that
/// day's occurrence of the rule.
#[renox::test]
async fn aut_14_a_day_with_no_run_is_alerted_from_09_00() {
    let t = boot().await;
    holder(&t, "Omar Owner", auto::permissions::OWNER_ROLE, &[]).await;
    let tomorrow = day(1);
    let at = |hour, minute| support::at(tomorrow, hour, minute);
    let go_to = |to: renox::db::DateTime| {
        t.app.travel_back();
        let ahead = (to - renox::db::now()).to_std().expect("ahead");
        t.app.travel(Duration::from_secs(ahead.as_secs() + 1));
    };
    let key = occurrence_key(RULE, tomorrow);
    let raised =
        |alerts: &[ComplianceAlert]| alerts.iter().filter(|a| a.occurrence_key == key).count();

    go_to(at(8, 30));
    t.app
        .at_travelled_time(wl_kernel::compliance::check_missed(t.state()))
        .await
        .unwrap();
    assert_eq!(raised(&ComplianceAlert::open(t.db()).await.unwrap()), 0);
    go_to(at(9, 0));
    t.app
        .at_travelled_time(wl_kernel::compliance::check_missed(t.state()))
        .await
        .unwrap();
    let alerts = ComplianceAlert::open(t.db()).await.unwrap();
    assert_eq!(raised(&alerts), 1, "{alerts:?}");
    assert!(
        alerts
            .iter()
            .any(|a| a.kind == AlertKind::Missed && a.rule == RULE)
    );
}

/// The sale is registered by the accountant for an invoice that exists, once,
/// with a supply date that is not in the future; the evidence is one of the
/// three kinds with a reference.
#[renox::test]
async fn aut_14_the_sale_and_the_evidence_are_checked() {
    let t = boot().await;
    let (_, adel) = accountant(&t).await;
    let invoice = zero_rated_invoice(&t).await;
    let nobody = t
        .user("Nadia", &[auto::permissions::EXPORT_SALES_VIEW])
        .await;
    let nadia = t.auth(&nobody).await;
    assert!(matches!(
        export_sales::record(t.state(), &nadia, invoice, today()).await,
        Err(Error::Forbidden)
    ));
    let future = export_sales::record(t.state(), &adel, invoice, day(2))
        .await
        .unwrap_err();
    assert!(future.to_string().contains("future"), "{future}");
    let unknown = export_sales::record(t.state(), &adel, 9_999, today())
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("does not exist"), "{unknown}");
    let sale = export_sales::record(t.state(), &adel, invoice, today())
        .await
        .unwrap();
    let again = export_sales::record(t.state(), &adel, invoice, today())
        .await
        .unwrap_err();
    assert!(again.to_string().contains("already"), "{again}");

    let bad = export_sales::attach_evidence(t.state(), &adel, sale.id, "invoice_copy", "X")
        .await
        .unwrap_err();
    assert!(bad.to_string().contains("exit certificate"), "{bad}");
    let blank = export_sales::attach_evidence(t.state(), &adel, sale.id, AIRWAY_BILL, " ")
        .await
        .unwrap_err();
    assert!(blank.to_string().contains("file name"), "{blank}");
    assert!(matches!(
        export_sales::attach_evidence(t.state(), &nadia, sale.id, AIRWAY_BILL, "AW-1").await,
        Err(Error::Forbidden)
    ));
}

/// The page lists the sale with where it stands in words, and the accountant
/// registers a sale and attaches evidence from it.
#[renox::test]
async fn aut_14_the_page_registers_a_sale_and_attaches_evidence() {
    let t = boot().await;
    let (adel_user, _) = accountant(&t).await;
    let invoice = zero_rated_invoice(&t).await;
    t.app.acting_as(&adel_user);
    t.app
        .get("/auto/export-sales")
        .await
        .assert_ok()
        .assert_see("Export sales");
    t.app
        .htmx()
        .post(
            "/auto/export-sales",
            &[
                ("invoice", &invoice.to_string()),
                ("supply_date", &today().to_string()),
            ],
        )
        .await
        .assert_ok();
    t.app
        .get("/auto/export-sales")
        .await
        .assert_ok()
        .assert_see("Gulf Movers Trading")
        .assert_see("AED 25,000.00")
        .assert_see("Waiting for exit evidence");
    let sale = export_sales::all(t.db()).await.unwrap().remove(0);
    t.app
        .htmx()
        .post(
            &format!("/auto/export-sales/{}/evidence", sale.id),
            &[("kind", "bill_of_lading"), ("reference", "BL-7788")],
        )
        .await
        .assert_ok();
    t.app
        .get("/auto/export-sales")
        .await
        .assert_ok()
        .assert_see("Exit evidence attached")
        .assert_see("Bill of lading BL-7788");
    let nobody = t.user("Nadia", &[auto::permissions::VEHICLES_VIEW]).await;
    t.app.acting_as(&nobody);
    t.app.get("/auto/export-sales").await.assert_status(403);
}
