//! A rejected or stuck e-invoice transmission (wl-invoicing §4 *E-invoice
//! transmission* `reject`, `resubmit`, `supersede`, §5.12 steps 3 to 5, §6
//! `FIN.RUL.16`): the acceptance checks of #73. Entity PGP is e-invoicing from
//! 2027-01-01 with a scripted ASP; the clock reads Wednesday 2027-01-05 06:00
//! UTC unless a test says otherwise.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::credit_notes::{self, By};
use wl_invoicing::einvoice_adapter::{AspAnswer, AspError, FakeAsp};
use wl_invoicing::einvoice_send;
use wl_invoicing::task::{InvoicingTask, TaskKind, TaskStatus};
use wl_invoicing::{
    EInvoiceTransmission, Invoice, TransmissionStatus, WlInvoicing, permissions as perm,
};
use wl_kernel::invoicing::{CreditLine, DraftLine, InvoiceDraft, InvoiceStatus, VatCategory};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{PartyId, Subject};

use support::{BLUE_DUNES, Booted, boot_with};

const PEPPOL: &str = "0235:100000000000011";
const FIXED: &str = "0235:100000000099999";

fn go_live() -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
}

/// A booted app with entity PGP e-invoicing through `asp`.
async fn booted(asp: &FakeAsp) -> Booted {
    let module = WlInvoicing::new()
        .einvoicing("PGP", go_live(), "asp")
        .asp_adapter("asp", asp.clone());
    let t = boot_with(module).await;
    t.directory.set_recipient(BLUE_DUNES, Some(PEPPOL));
    t.travel_to("2027-01-05", 6, 0);
    t
}

/// Moves the clock to `minutes` after 2027-01-05 06:00 UTC.
fn at(t: &Booted, minutes: u32) {
    let minutes = 6 * 60 + minutes;
    let day = 5 + minutes / (24 * 60);
    let rest = minutes % (24 * 60);
    t.travel_to(&format!("2027-01-{day:02}"), rest / 60, rest % 60);
}

async fn transmissions(t: &Booted) -> Vec<EInvoiceTransmission> {
    EInvoiceTransmission::query()
        .order_by(EInvoiceTransmission::ID)
        .get(t.db())
        .await
        .unwrap()
}

async fn draft_for(t: &Booted, party: PartyId) -> Invoice {
    t.draft_of(
        InvoiceDraft::new(party, Subject::new("property.lease", 9)).line(DraftLine::new(
            "Rent, January",
            100_000,
            VatCategory::Standard,
        )),
    )
    .await
}

async fn run(t: &Booted) -> einvoice_send::Report {
    t.app
        .at_travelled_time(einvoice_send::run_due(t.state()))
        .await
        .unwrap()
}

/// The tasks of the e-invoice kinds, oldest first.
async fn einvoice_tasks(t: &Booted) -> Vec<InvoicingTask> {
    InvoicingTask::query()
        .where_(InvoicingTask::KIND.is_in([TaskKind::EinvoiceRejected, TaskKind::EinvoiceStuck]))
        .order_by(InvoicingTask::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// An invoice issued by clerk A and sent: its `submitted` transmission.
async fn submitted(t: &Booted) -> (Invoice, EInvoiceTransmission, User) {
    let clerk = t.user_a().await;
    let invoice = t.issue(&draft_for(t, BLUE_DUNES).await, &clerk).await;
    let sent = transmissions(t).await.remove(0);
    assert_eq!(sent.status, TransmissionStatus::Submitted);
    (invoice, sent, clerk)
}

/// Rejects `sent` the way the ASP does, with `code`.
async fn reject(t: &Booted, sent: &EInvoiceTransmission, code: &str) {
    let answered = einvoice_send::respond(
        t.state(),
        sent.id,
        sent.attempt,
        AspAnswer::Rejected(vec![AspError::new(code, "The totals do not add up.")]),
    )
    .await
    .unwrap();
    assert!(answered.is_some());
}

/// The reminder steps logged for `invoice`, oldest first.
async fn reminder_steps(t: &Booted, invoice: &Invoice) -> Vec<i64> {
    wl_invoicing::ReminderLog::query()
        .where_(wl_invoicing::ReminderLog::INVOICE_ID.eq(invoice.id))
        .order_by(wl_invoicing::ReminderLog::STEP)
        .get(t.db())
        .await
        .unwrap()
        .iter()
        .map(|log| log.step)
        .collect()
}

/// Given a `submitted` transmission, the ASP rejects it with BR-CO-15: it is
/// `rejected`, the codes are stored, the invoice owner and the accountant get a
/// task, `EInvoiceRejected` is emitted, and the invoice, its balance and its
/// reminders are unchanged. A second rejection updates the same task.
#[renox::test]
async fn a_rejection_stores_the_codes_and_raises_one_task_without_touching_the_invoice() {
    let asp = FakeAsp::new();
    let t = booted(&asp).await;
    let aya = t.accountant("Aya").await;
    let (invoice, sent, clerk) = submitted(&t).await;
    let before = t.reload(&invoice).await;
    let logs_before = reminder_steps(&t, &invoice).await;

    reject(&t, &sent, "BR-CO-15").await;
    let rejected = transmissions(&t).await.remove(0);
    assert_eq!(rejected.status, TransmissionStatus::Rejected);
    assert_eq!(rejected.error_codes.0, ["BR-CO-15"]);
    assert_eq!(rejected.error_messages.0, ["The totals do not add up."]);
    assert!(rejected.response_at.is_some());

    // One task, for the accountant, naming the codes; the owner and the
    // accountant are told.
    let open = einvoice_tasks(&t).await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].kind, TaskKind::EinvoiceRejected);
    assert_eq!(open[0].role, "accountant");
    assert_eq!(open[0].invoice_id, Some(invoice.id));
    assert_eq!(open[0].status, TaskStatus::Open);
    assert!(open[0].message.contains("BR-CO-15"), "{}", open[0].message);
    let told: Vec<i64> = t
        .notices
        .of_type("invoicing.einvoice_task")
        .iter()
        .flat_map(|r| r.recipients.clone())
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(id),
            _ => None,
        })
        .collect();
    assert!(
        told.contains(&aya.id) && told.contains(&clerk.id),
        "{told:?}"
    );

    // The event.
    let events = t.einvoice_rejected.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].error_codes, ["BR-CO-15"]);
    assert_eq!(events[0].document, invoice.subject());

    // The issued document is as it was.
    let after = t.reload(&invoice).await;
    assert_eq!(
        (
            after.status,
            after.balance,
            after.gross_total,
            after.disputed
        ),
        (
            before.status,
            before.balance,
            before.gross_total,
            before.disputed
        )
    );
    assert_eq!(after.due_date, before.due_date);

    // Its reminders are as they were: the log is the same, and the reminder
    // run still picks the invoice on the day its first step comes.
    assert_eq!(reminder_steps(&t, &invoice).await, logs_before);
    assert!(!after.disputed && after.dispute_grace_until.is_none());
    let first_day = after.due_date.unwrap() - renox::chrono::Duration::days(3);
    assert_eq!(
        wl_invoicing::reminders::run(t.state(), first_day)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        reminder_steps(&t, &invoice).await.len(),
        logs_before.len() + 1
    );

    // The task is on the task list and opens the board.
    let viewer = t
        .helper("Tasker", &[perm::VIEW, perm::RECEIVABLES_VIEW])
        .await;
    t.app.acting_as(&viewer);
    t.app
        .get("/invoices/tasks")
        .await
        .assert_ok()
        .assert_see("E-invoice rejected");
    t.app
        .get(&format!("/invoices/tasks/{}", open[0].id))
        .await
        .assert_redirect("/einvoices");
}

/// After the party's Peppol ID is fixed in wl-parties, user B (`einvoices.manage`)
/// resubmits: the transmission is `queued` again as attempt 2 and the rebuilt
/// payload carries the new Peppol ID; the issued invoice is unchanged. Without
/// the confirmation, or without the permission, nothing happens.
#[renox::test]
async fn a_rejection_with_a_cause_outside_the_document_is_resubmitted_with_the_fix() {
    let asp = FakeAsp::new();
    let t = booted(&asp).await;
    let (invoice, sent, _) = submitted(&t).await;
    reject(&t, &sent, "PEPPOL-EN16931-R010").await;
    let before = t.reload(&invoice).await;
    let manager = t
        .helper(
            "Bilal",
            &[perm::VIEW, perm::EINVOICES_VIEW, perm::EINVOICES_MANAGE],
        )
        .await;
    let reader = t.helper("Rima", &[perm::VIEW, perm::EINVOICES_VIEW]).await;
    let path = format!("/einvoices/{}/resubmit", sent.id);

    // The ASP is down when B resubmits, so it stays queued.
    asp.unreachable(true);
    t.directory.set_recipient(BLUE_DUNES, Some(FIXED));

    // Without the permission: refused.
    t.app.acting_as(&reader);
    t.app
        .htmx()
        .post(&path, &[("confirm", "on")])
        .await
        .assert_forbidden();
    // Without the confirmation: nothing happens.
    t.app.acting_as(&manager);
    t.app.htmx().post(&path, &[]).await.assert_status(303);
    assert_eq!(
        transmissions(&t).await[0].status,
        TransmissionStatus::Rejected
    );
    assert_eq!(transmissions(&t).await[0].attempt, 1);

    t.app
        .htmx()
        .post(&path, &[("confirm", "on")])
        .await
        .assert_redirect("/einvoices");
    let again = transmissions(&t).await.remove(0);
    assert_eq!(again.status, TransmissionStatus::Queued);
    assert_eq!(again.attempt, 2);
    assert_eq!(again.recipient_id.as_deref(), Some(FIXED));
    assert_eq!(again.payload.0["recipient"]["identifier"], FIXED);
    assert_eq!(
        again.payload_hash,
        wl_invoicing::einvoice_payload::hash_of(&again.payload.0)
    );
    assert!(again.error_codes.0.is_empty());

    // The issued invoice is unchanged.
    let after = t.reload(&invoice).await;
    assert_eq!(
        (
            after.status,
            after.balance,
            after.gross_total,
            after.number.clone()
        ),
        (
            before.status,
            before.balance,
            before.gross_total,
            before.number.clone()
        )
    );

    // The ASP is back: it is sent as attempt 2.
    asp.unreachable(false);
    // The first retry of the queued attempt is 5 minutes after it was queued.
    at(&t, 6);
    run(&t).await;
    let sent_again = transmissions(&t).await.remove(0);
    assert_eq!(sent_again.status, TransmissionStatus::Submitted);
    assert_eq!(asp.submitted().len(), 2);
    assert_eq!(asp.submitted()[1].key, format!("{}-2", sent.id));
    assert_eq!(asp.submitted()[1].recipient_id, FIXED);

    // Accepted this time: the task closes.
    asp.answers(AspAnswer::Accepted);
    run(&t).await;
    assert_eq!(
        transmissions(&t).await[0].status,
        TransmissionStatus::Accepted
    );
    assert!(
        einvoice_tasks(&t)
            .await
            .iter()
            .all(|task| task.status == TaskStatus::Done)
    );
}

/// Resubmitting is refused while the party still has no identifier, and a
/// transmission that is not rejected is not resubmitted.
#[renox::test]
async fn resubmit_needs_an_identifier_and_a_rejected_transmission() {
    let asp = FakeAsp::new();
    let t = booted(&asp).await;
    let (_, sent, _) = submitted(&t).await;
    let manager = t
        .helper(
            "Bilal",
            &[perm::VIEW, perm::EINVOICES_VIEW, perm::EINVOICES_MANAGE],
        )
        .await;
    let path = format!("/einvoices/{}/resubmit", sent.id);
    t.app.acting_as(&manager);
    // Submitted, not rejected.
    t.app
        .htmx()
        .post(&path, &[("confirm", "on")])
        .await
        .assert_status(422);
    reject(&t, &sent, "BR-CO-15").await;
    t.directory.set_recipient(BLUE_DUNES, None);
    t.app
        .htmx()
        .post(&path, &[("confirm", "on")])
        .await
        .assert_status(303);
    assert_eq!(
        transmissions(&t).await[0].status,
        TransmissionStatus::Rejected
    );
    assert_eq!(transmissions(&t).await[0].attempt, 1);
}

/// When a credit note with reason *e-invoice correction* is issued against the
/// invoice, the rejected transmission is `superseded`, linked to the credit
/// note, its task is closed and the credit note has its own transmission. When
/// the new invoice naming the old one as replaced is issued, the superseded
/// transmission is also linked to it and it has its own `queued` transmission.
#[renox::test]
async fn a_correction_credit_note_supersedes_the_rejected_transmission() {
    let asp = FakeAsp::new();
    let t = booted(&asp).await;
    let credit = t
        .helper(
            "Cleo",
            &[perm::VIEW, perm::CREDIT, perm::CREATE, perm::ISSUE],
        )
        .await;
    let (invoice, sent, clerk) = submitted(&t).await;
    reject(&t, &sent, "BR-CO-15").await;
    assert_eq!(einvoice_tasks(&t).await.len(), 1);

    // A credit note with another reason supersedes nothing.
    let other = credit_notes::draft(
        t.db(),
        &invoice,
        &[CreditLine::new(1, 10_000)],
        "Price error",
        Some(credit.id),
    )
    .await
    .unwrap();
    t.app
        .at_travelled_time(credit_notes::issue(
            t.state(),
            By::System(Some(credit.id)),
            &other,
        ))
        .await
        .unwrap();
    assert_eq!(
        transmissions(&t).await[0].status,
        TransmissionStatus::Rejected
    );

    // The correction credits the invoice with the reason e-invoice correction.
    let draft = credit_notes::draft(
        t.db(),
        &invoice,
        &[CreditLine::new(1, 90_000)],
        "e-invoice correction",
        Some(credit.id),
    )
    .await
    .unwrap();
    let note = t
        .app
        .at_travelled_time(credit_notes::issue(
            t.state(),
            By::System(Some(credit.id)),
            &draft,
        ))
        .await
        .unwrap();
    let all = transmissions(&t).await;
    let old = all.iter().find(|row| row.id == sent.id).unwrap();
    assert_eq!(old.status, TransmissionStatus::Superseded);
    assert_eq!(old.replaced_by_credit_note_id, Some(note.id));
    assert!(
        einvoice_tasks(&t)
            .await
            .iter()
            .all(|task| task.status == TaskStatus::Done)
    );
    let own = all
        .iter()
        .find(|row| row.document_type == "invoicing.credit_note" && row.document_id == note.id)
        .expect("the credit note has its own transmission");
    assert_eq!(own.status, TransmissionStatus::Submitted);

    // The new invoice names the old one it replaces, and is issued.
    let new_draft = draft_for(&t, BLUE_DUNES).await;
    let name_it = |number: String| {
        let path = format!("/invoices/{}/replaces", new_draft.id);
        let user = clerk.clone();
        let t = &t;
        async move {
            t.app.acting_as(&user);
            t.app
                .htmx()
                .post(&path, &[("number", number.as_str())])
                .await
        }
    };
    name_it("INV-NOPE".to_owned()).await.assert_status(303);
    assert_eq!(t.reload(&new_draft).await.replaces_invoice_id, None);
    name_it(invoice.number.clone().unwrap())
        .await
        .assert_status(303);
    assert_eq!(
        t.reload(&new_draft).await.replaces_invoice_id,
        Some(invoice.id)
    );
    let new_invoice = t.issue(&new_draft, &clerk).await;
    let all = transmissions(&t).await;
    let old = all.iter().find(|row| row.id == sent.id).unwrap();
    assert_eq!(old.replaced_by_invoice_id, Some(new_invoice.id));
    let fresh = all
        .iter()
        .find(|row| row.document_type == "invoicing.invoice" && row.document_id == new_invoice.id)
        .expect("the new invoice has its own transmission");
    assert_ne!(fresh.status, TransmissionStatus::Superseded);
    assert_eq!(fresh.attempt, 1);
    // 10,000 + 90,000 net credit the whole line: with its VAT the credit notes
    // allocate the whole gross, so the balance is zero.
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Paid);
}

/// An invoice that no transmission has been superseded for cannot be named as
/// replaced.
#[renox::test]
async fn only_an_invoice_with_a_superseded_transmission_can_be_replaced() {
    let asp = FakeAsp::new();
    let t = booted(&asp).await;
    let (invoice, _, _) = submitted(&t).await;
    let draft = draft_for(&t, BLUE_DUNES).await;
    let refused = einvoice_send::set_replaces(t.db(), &draft, invoice.id).await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    // Not a draft either.
    let issued = t.reload(&invoice).await;
    assert!(
        einvoice_send::set_replaces(t.db(), &issued, invoice.id)
            .await
            .is_err()
    );
}

/// A transmission still `queued` 24 hours after issue has a task, once; at 9
/// hours (the retries at +5 min, +30 min, +2 h and +6 h failed) there is none
/// yet, and the next retry is at +12 h.
#[renox::test]
async fn a_transmission_not_sent_after_24_hours_raises_one_task() {
    let asp = FakeAsp::new();
    asp.unreachable(true);
    let t = booted(&asp).await;
    let clerk = t.user_a().await;
    t.issue(&draft_for(&t, BLUE_DUNES).await, &clerk).await;
    for minute in [6, 31, 121, 361] {
        at(&t, minute);
        run(&t).await;
    }
    at(&t, 9 * 60);
    run(&t).await;
    let row = transmissions(&t).await.remove(0);
    assert_eq!(row.status, TransmissionStatus::Queued);
    assert_eq!(
        (row.next_try_at.unwrap() - row.queued_at).num_minutes(),
        12 * 60
    );
    assert!(einvoice_tasks(&t).await.is_empty(), "no task at 9 hours");

    at(&t, 24 * 60 + 1);
    run(&t).await;
    let open = einvoice_tasks(&t).await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].kind, TaskKind::EinvoiceStuck);
    assert!(open[0].message.contains("24 hours"), "{}", open[0].message);
    at(&t, 24 * 60 + 5);
    run(&t).await;
    assert_eq!(einvoice_tasks(&t).await.len(), 1, "once");
}

/// When every transmission of entity E is queued for 24 hours the company admin is
/// told, once per outage; an ASP that answers anything stops the outage.
#[renox::test]
async fn an_outage_tells_the_company_admin_once() {
    let asp = FakeAsp::new();
    asp.unreachable(true);
    let t = booted(&asp).await;
    let admin = t.helper("Boss", &[perm::VIEW]).await;
    admin.assign_role(t.db(), "admin").await.unwrap();
    let clerk = t.user_a().await;
    t.issue(&draft_for(&t, BLUE_DUNES).await, &clerk).await;

    at(&t, 12 * 60);
    run(&t).await;
    assert!(
        t.notices.of_type("invoicing.einvoice_outage").is_empty(),
        "not yet"
    );
    at(&t, 24 * 60 + 1);
    run(&t).await;
    let notices = t.notices.of_type("invoicing.einvoice_outage");
    assert_eq!(notices.len(), 1);
    assert!(notices[0].recipients.iter().any(|r| matches!(
        r,
        NotifyRecipient::User { id } if *id == admin.id
    )));
    at(&t, 24 * 60 + 30);
    run(&t).await;
    assert_eq!(
        t.notices.of_type("invoicing.einvoice_outage").len(),
        1,
        "once per outage"
    );
}
