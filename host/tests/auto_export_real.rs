//! BL-AUT-14 against the REAL wl-invoicing, wl-notify and wl-ticketing of the
//! host: a zero-rated invoice of AED 25,000 registered as an export sale; the
//! accountant is told through the real notifier at 60 days, at 90 days the sale
//! is flagged and the accountant gets a real task about the invoice; no credit
//! note or new invoice is issued; evidence stops the rule. The app's own tests
//! (`apps/auto/tests`) show the same, and the failed run, against stand-ins.

mod support;

use auto::ExportSale;
use auto::export_rule::{self, NOTICE_TYPE, task_key};
use auto::export_sale::{AIRWAY_BILL, BILL_OF_LADING};
use auto::export_sales;
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::autofx::{boot, sent_of, shop};
use support::tenant;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};
use wl_kernel::{PartyId, Subject};

/// The tasks of the ticketing module whose source key starts with `prefix`.
async fn tasks(w: &support::autofx::World, prefix: &str) -> Vec<wl_ticketing::Ticket> {
    wl_ticketing::Ticket::query()
        .get(w.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|t| {
            t.source_key
                .as_deref()
                .is_some_and(|k| k.starts_with(prefix))
        })
        .collect()
}

/// An issued invoice of AED 25,000, zero-rated, for `customer`.
async fn export_invoice(
    w: &support::autofx::World,
    accountant: &AuthUser,
    customer: PartyId,
    reference: i64,
) -> InvoiceId {
    let books = invoicer(w.state());
    let draft =
        InvoiceDraft::new(customer, Subject::new("auto.export", reference)).line(DraftLine::new(
            "Brake pads, export to Mombasa",
            2_500_000,
            VatCategory::ZeroRated,
        ));
    let id = books
        .draft(w.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    books
        .issue(w.state(), id, Some(accountant))
        .await
        .expect("the accountant issues it");
    id
}

/// INV zero-rated AED 25,000 supplied today with no evidence: told at 60 days
/// (the real message to the accountant), flagged at 90 with a real task that
/// links the invoice, once each; nothing is credited or issued; a sale with a
/// bill of lading is left alone.
#[renox::test]
async fn aut_14_real_the_accountant_is_told_and_gets_the_correction_task() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let customer = tenant(&w.app, "Gulf Movers Trading").await;
    let slow = export_invoice(&w, &s.aisha, customer, 140).await;
    let quick = export_invoice(&w, &s.aisha, customer, 141).await;
    let a = export_sales::record(w.state(), &s.aisha, slow.0, today())
        .await
        .unwrap();
    let b = export_sales::record(w.state(), &s.aisha, quick.0, today())
        .await
        .unwrap();
    assert_eq!(a.gross_total, 2_500_000, "zero-rated: no VAT");
    export_sales::attach_evidence(w.state(), &s.aisha, b.id, BILL_OF_LADING, "BL-7788")
        .await
        .unwrap();

    export_rule::run_daily(w.state(), today() + TimeDelta::days(59))
        .await
        .unwrap();
    assert!(sent_of(&w, NOTICE_TYPE).await.is_empty());
    export_rule::run_daily(w.state(), today() + TimeDelta::days(60))
        .await
        .unwrap();
    let mails = sent_of(&w, NOTICE_TYPE).await;
    assert_eq!(mails.len(), 1, "only the sale without evidence");
    assert_eq!(mails[0].user_id, Some(s.aisha.id));
    assert!(tasks(&w, "auto.export_vat").await.is_empty());

    export_rule::run_daily(w.state(), today() + TimeDelta::days(90))
        .await
        .unwrap();
    let flagged = ExportSale::find_or_404(w.db(), a.id).await.unwrap();
    assert!(flagged.vat_flagged_at.is_some());
    let waiting = tasks(&w, &task_key(a.id)).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].subject_type, "invoicing.invoice");
    assert_eq!(waiting[0].subject_id, slow.0, "it links the invoice");
    assert_eq!(waiting[0].assignee_user_id, Some(s.aisha.id));
    assert!(tasks(&w, &task_key(b.id)).await.is_empty());
    export_rule::run_daily(w.state(), today() + TimeDelta::days(91))
        .await
        .unwrap();
    assert_eq!(tasks(&w, "auto.export_vat").await.len(), 1);

    // No document was issued by the rule.
    assert!(
        invoicer(w.state())
            .credit_notes_for(w.db(), slow)
            .await
            .unwrap()
            .is_empty()
    );
    let invoice = invoicer(w.state())
        .find(w.db(), slow)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(invoice.status, InvoiceStatus::Credited);
    assert_eq!(invoice.gross_total, 2_500_000);

    // Evidence after the flag: the flag stays, the accountant is told.
    export_sales::attach_evidence(w.state(), &s.aisha, a.id, AIRWAY_BILL, "AW-12")
        .await
        .unwrap();
    let kept = ExportSale::find_or_404(w.db(), a.id).await.unwrap();
    assert!(kept.vat_flagged_at.is_some() && kept.late_evidence_notified_at.is_some());
    assert_eq!(sent_of(&w, NOTICE_TYPE).await.len(), 2);
}

/// #410 point 2 with the real notifier and ticketing: an airway bill or an exit
/// certificate stops the rule like a bill of lading: no notice at 60 days, no
/// task at 90.
#[renox::test]
async fn aut_14_real_any_one_piece_of_evidence_stops_the_rule() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let customer = tenant(&w.app, "Gulf Movers Trading").await;
    for (reference, kind, number) in [
        (142, AIRWAY_BILL, "AW-1"),
        (143, auto::export_sale::EXIT_CERTIFICATE, "EX-1"),
    ] {
        let invoice = export_invoice(&w, &s.aisha, customer, reference).await;
        let sale = export_sales::record(w.state(), &s.aisha, invoice.0, today())
            .await
            .unwrap();
        export_sales::attach_evidence(w.state(), &s.aisha, sale.id, kind, number)
            .await
            .unwrap();
    }
    for days in [60, 90, 91] {
        export_rule::run_daily(w.state(), today() + TimeDelta::days(days))
            .await
            .unwrap();
    }
    assert!(sent_of(&w, NOTICE_TYPE).await.is_empty());
    assert!(tasks(&w, "auto.export_vat").await.is_empty());
}

/// #410 point 3 on the real modules: the 08:00 run fails (the notice cannot be
/// stored); after the fifth attempt the Owner / GM and the company admin are
/// told through the real notifier and the owner has a real task; the admin
/// runs it again once the cause is gone, the alert closes, the owner's task is
/// completed and the notice goes out.
#[renox::test]
async fn aut_14_real_a_failed_run_alerts_the_owner_and_the_admin_and_a_rerun_closes_it() {
    use host::roles::{ADMIN, OWNER};
    use renox::state_machine::StateMachine;
    use wl_kernel::automation::{MAX_ATTEMPTS, RuleRun, transitions};
    use wl_kernel::compliance::{AlertKind, ComplianceAlert, occurrence_key, schedule_subject};

    let w = boot().await;
    let s = shop(&w, None).await;
    let (owner_user, _) = w.staff("Omar", OWNER).await;
    let (admin_user, admin) = w.staff("Ada", ADMIN).await;
    let customer = tenant(&w.app, "Gulf Movers Trading").await;
    let invoice = export_invoice(&w, &s.aisha, customer, 150).await;
    let sale = export_sales::record(w.state(), &s.aisha, invoice.0, today())
        .await
        .unwrap();
    let on = today() + TimeDelta::days(60);

    renox::db::sql(
        "CREATE TRIGGER refuse_notice BEFORE INSERT ON wl_notify_messages WHEN NEW.type_key = 'auto.export_notice' \
         BEGIN SELECT RAISE(ABORT, 'refused for the test'); END",
    )
    .execute(w.db())
    .await
    .unwrap();
    for attempt in 0..MAX_ATTEMPTS {
        export_rule::run_daily(w.state(), on).await.unwrap();
        if attempt + 1 < MAX_ATTEMPTS {
            assert!(
                ComplianceAlert::open(w.db())
                    .await
                    .unwrap()
                    .iter()
                    .all(|a| a.rule != export_rule::RULE),
                "no alert before the fifth failure"
            );
        }
        RuleRun::query()
            .update(
                w.db(),
                &[(
                    "next_attempt_at",
                    &Some(renox::db::now() - TimeDelta::hours(1)),
                )],
            )
            .await
            .unwrap();
    }
    let alerts: Vec<_> = ComplianceAlert::open(w.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == export_rule::RULE)
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].kind, AlertKind::Failed);
    let told = sent_of(&w, wl_kernel::compliance::service::NOTIFICATION_TYPE).await;
    let recipients: Vec<_> = told.iter().filter_map(|m| m.user_id).collect();
    assert!(
        recipients.contains(&owner_user.id) && recipients.contains(&admin_user.id),
        "the owner and the company admin: {recipients:?}"
    );
    assert!(
        !recipients.contains(&s.aisha.id),
        "not the accountant: {recipients:?}"
    );
    let owner_task = || async {
        wl_ticketing::Ticket::query()
            .get(w.db())
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.assignee_user_id == Some(owner_user.id))
            .expect("the owner has a task")
    };
    assert!(!matches!(
        owner_task().await.status,
        wl_ticketing::TicketStatus::Closed | wl_ticketing::TicketStatus::Resolved
    ));
    assert!(sent_of(&w, NOTICE_TYPE).await.is_empty());

    renox::db::sql("DROP TRIGGER refuse_notice")
        .execute(w.db())
        .await
        .unwrap();
    let failed = RuleRun::find_occurrence(
        w.db(),
        export_rule::RULE,
        &schedule_subject(),
        &occurrence_key(export_rule::RULE, on),
    )
    .await
    .unwrap()
    .expect("the run is kept");
    let _outcome = failed
        .transition(transitions::RERUN)
        .by(&admin)
        .run(w.state())
        .await
        .unwrap();
    export_rule::run_daily(w.state(), on).await.unwrap();
    assert!(
        ComplianceAlert::open(w.db())
            .await
            .unwrap()
            .into_iter()
            .all(|a| a.rule != export_rule::RULE),
        "the rerun closed the alert"
    );
    assert!(matches!(
        owner_task().await.status,
        wl_ticketing::TicketStatus::Closed | wl_ticketing::TicketStatus::Resolved
    ));
    assert_eq!(
        sent_of(&w, NOTICE_TYPE).await.len(),
        1,
        "the notice went out"
    );
    let _ = sale;
}
