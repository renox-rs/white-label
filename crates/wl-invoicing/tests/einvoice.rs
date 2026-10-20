//! Queueing and sending e-invoice transmissions (wl-invoicing §4 *E-invoice
//! transmission*, §5.12 steps 1 and 2, `FIN.INV.01`): the acceptance checks of
//! #72. Entity PGP goes live on 2027-01-01 with a scripted ASP; the clock reads
//! Wednesday 2027-01-05 06:00 UTC (10:00 in Dubai) unless a test says
//! otherwise.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::credit_notes::{self, By};
use wl_invoicing::einvoice::{CREDIT_NOTE, INVOICE};
use wl_invoicing::einvoice_adapter::{AspAnswer, FakeAsp};
use wl_invoicing::einvoice_payload::hash_of;
use wl_invoicing::einvoice_send;
use wl_invoicing::invoice::InvoiceKind;
use wl_invoicing::{EInvoiceTransmission, Invoice, TransmissionStatus, WlInvoicing};
use wl_kernel::PartyId;
use wl_kernel::Subject;
use wl_kernel::integrations::{ConnectionStatus, IntegrationConnection};
use wl_kernel::invoicing::{CreditLine, DraftLine, InvoiceDraft, InvoiceStatus, VatCategory};

use support::{BLUE_DUNES, Booted, OMAR, boot_with};

const PEPPOL: &str = "0235:100000000000011";

fn go_live() -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
}

/// A booted app whose entity PGP is e-invoicing from 2027-01-01 through the
/// scripted `asp`, with the clock at `date` `hour:00` UTC.
async fn booted(asp: &FakeAsp, date: &str, hour: u32) -> Booted {
    let module = WlInvoicing::new()
        .einvoicing("PGP", go_live(), "asp")
        .asp_adapter("asp", asp.clone());
    let t = boot_with(module).await;
    t.directory.set_recipient(BLUE_DUNES, Some(PEPPOL));
    t.travel_to(date, hour, 0);
    t
}

/// Moves the clock to `minutes` after 2027-01-05 06:00 UTC.
fn at(t: &Booted, minutes: u32) {
    let minutes = 6 * 60 + minutes;
    let day = 5 + minutes / (24 * 60);
    let rest = minutes % (24 * 60);
    t.travel_to(&format!("2027-01-{day:02}"), rest / 60, rest % 60);
}

/// The transmissions, oldest first.
async fn transmissions(t: &Booted) -> Vec<EInvoiceTransmission> {
    EInvoiceTransmission::query()
        .order_by(EInvoiceTransmission::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// A draft to `party` with lines of AED 600.00 and AED 400.00, standard
/// rated: AED 1,000.00 and AED 50.00 VAT.
async fn draft_for(t: &Booted, party: PartyId) -> Invoice {
    t.draft_of(
        InvoiceDraft::new(party, Subject::new("property.lease", 9))
            .line(DraftLine::new(
                "Rent, January",
                60_000,
                VatCategory::Standard,
            ))
            .line(DraftLine::new(
                "Service charge",
                40_000,
                VatCategory::Standard,
            )),
    )
    .await
}

/// Runs the scheduled run now (the clock the test set).
async fn run(t: &Booted) -> einvoice_send::Report {
    t.app
        .at_travelled_time(einvoice_send::run_due(t.state()))
        .await
        .unwrap()
}

/// Given entity E with go-live 2027-01-01 and a B2B party with a Peppol ID, an
/// invoice issued on 2027-01-05 is `queued`, then `submitted` with an ASP
/// reference; its payload carries the issuer TRN, the party's TRN and Peppol
/// ID, the number, the issue date, both lines with their VAT category and VAT
/// amount, the totals per category and the AED VAT total 50.00, and the stored
/// hash matches a recomputation.
#[renox::test]
async fn an_invoice_after_go_live_is_queued_and_submitted_with_its_payload() {
    let asp = FakeAsp::new();
    let t = booted(&asp, "2027-01-05", 6).await;
    let clerk = t.user_a().await;
    let draft = draft_for(&t, BLUE_DUNES).await;
    let invoice = t.issue(&draft, &clerk).await;

    let all = transmissions(&t).await;
    assert_eq!(all.len(), 1);
    let sent = &all[0];
    assert_eq!(sent.status, TransmissionStatus::Submitted);
    assert_eq!(sent.document(), invoice.subject());
    assert_eq!(sent.attempt, 1);
    assert_eq!(sent.recipient_id.as_deref(), Some(PEPPOL));
    assert_eq!(
        sent.asp_reference.as_deref(),
        Some(format!("ASP-{}-1", sent.id).as_str())
    );
    assert_eq!(asp.submitted().len(), 1);
    assert_eq!(asp.submitted()[0].recipient_id, PEPPOL);

    let payload = &sent.payload.0;
    assert_eq!(payload["issuer"]["trn"], "100000000000003");
    assert_eq!(payload["recipient"]["trn"], "100000000000011");
    assert_eq!(payload["recipient"]["identifier"], PEPPOL);
    assert_eq!(
        payload["document"]["number"],
        invoice.number.clone().unwrap()
    );
    assert_eq!(payload["document"]["issue_date"], "2027-01-05");
    assert_eq!(payload["document"]["kind"], "tax_invoice");
    let lines = payload["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["vat_category"], "standard");
    assert_eq!(lines[0]["vat_amount"], 3_000);
    assert_eq!(lines[1]["vat_amount"], 2_000);
    assert_eq!(payload["totals"]["per_category"][0]["category"], "standard");
    assert_eq!(payload["totals"]["per_category"][0]["net"], 100_000);
    assert_eq!(payload["totals"]["vat_aed"], 5_000);
    assert_eq!(sent.payload_hash, hash_of(payload));

    // The issued invoice is as it was.
    let after = t.reload(&invoice).await;
    assert_eq!(
        (after.status, after.gross_total, after.balance),
        (InvoiceStatus::Issued, 105_000, 105_000)
    );
    // The recipient identifier is not on the document.
    assert!(after.party_snapshot.is_some());
}

/// When the ASP accepts, the transmission is `accepted`, the document's e-invoice
/// status is read from it and `EInvoiceAccepted` is emitted.
#[renox::test]
async fn the_asps_acceptance_is_recorded_and_announced() {
    let asp = FakeAsp::new();
    let t = booted(&asp, "2027-01-05", 6).await;
    let clerk = t.user_a().await;
    let invoice = t.issue(&draft_for(&t, BLUE_DUNES).await, &clerk).await;
    let sent = transmissions(&t).await.remove(0);

    // Pending: nothing changes.
    asp.answers(AspAnswer::Pending);
    assert_eq!(run(&t).await.answered, 0);
    assert_eq!(
        transmissions(&t).await[0].status,
        TransmissionStatus::Submitted
    );

    // An answer to an older attempt changes nothing.
    assert!(
        einvoice_send::respond(t.state(), sent.id, 0, AspAnswer::Accepted)
            .await
            .unwrap()
            .is_none()
    );

    asp.answers(AspAnswer::Accepted);
    assert_eq!(run(&t).await.answered, 1);
    let done = transmissions(&t).await.remove(0);
    assert_eq!(done.status, TransmissionStatus::Accepted);
    assert!(done.response_at.is_some());
    assert_eq!(
        EInvoiceTransmission::status_of(t.db(), &invoice.subject())
            .await
            .unwrap(),
        Some(TransmissionStatus::Accepted)
    );
    let events = t.einvoice_accepted.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].document, invoice.subject());
    assert_eq!(events[0].asp_reference, done.asp_reference);
}

/// A B2C party gets no transmission; a B2B party without an identifier cannot
/// be invoiced; a simplified tax invoice is refused once e-invoicing applies.
#[renox::test]
async fn issue_refuses_what_the_mandate_forbids_and_skips_b2c() {
    let asp = FakeAsp::new();
    let t = booted(&asp, "2027-01-05", 6).await;
    let clerk = t.user_a().await;

    // B2C: issued, no transmission.
    let b2c = t.issue(&draft_for(&t, OMAR).await, &clerk).await;
    assert_eq!(b2c.status, InvoiceStatus::Issued);
    assert!(transmissions(&t).await.is_empty());

    // B2B without a recipient identifier: refused with a clear message.
    t.directory.set_recipient(BLUE_DUNES, None);
    let draft = draft_for(&t, BLUE_DUNES).await;
    t.post_issue(&draft, &clerk)
        .await
        .assert_status(422)
        .assert_see("recipient identifier");
    assert_eq!(t.reload(&draft).await.status, InvoiceStatus::Draft);
    t.directory.set_recipient(BLUE_DUNES, Some(PEPPOL));

    // A simplified tax invoice is refused after go-live, a tax invoice is not.
    Invoice::query()
        .where_(Invoice::ID.eq(draft.id))
        .update(t.db(), &[("kind", &InvoiceKind::SimplifiedTaxInvoice)])
        .await
        .unwrap();
    t.post_issue(&draft, &clerk)
        .await
        .assert_status(422)
        .assert_see("simplified");
    Invoice::query()
        .where_(Invoice::ID.eq(draft.id))
        .update(t.db(), &[("kind", &InvoiceKind::TaxInvoice)])
        .await
        .unwrap();
    t.issue(&draft, &clerk).await;
    assert_eq!(transmissions(&t).await.len(), 1);
}

/// An invoice issued on 2026-12-31 (23:30 in Dubai) is not queued, and neither
/// is one issued before go-live.
#[renox::test]
async fn an_invoice_before_go_live_is_not_queued() {
    let asp = FakeAsp::new();
    let t = booted(&asp, "2026-12-31", 19).await;
    let clerk = t.user_a().await;
    // 19:30 UTC is 23:30 on 2026-12-31 in Dubai.
    t.travel_to("2026-12-31", 19, 30);
    let invoice = t.issue(&draft_for(&t, BLUE_DUNES).await, &clerk).await;
    assert_eq!(
        invoice.issue_date,
        Some(NaiveDate::from_ymd_opt(2026, 12, 31).unwrap())
    );
    assert!(transmissions(&t).await.is_empty());
    assert!(run(&t).await.queued == 0);
    // A simplified invoice may still be issued before go-live.
    let simplified = draft_for(&t, BLUE_DUNES).await;
    Invoice::query()
        .where_(Invoice::ID.eq(simplified.id))
        .update(t.db(), &[("kind", &InvoiceKind::SimplifiedTaxInvoice)])
        .await
        .unwrap();
    assert_eq!(
        t.issue(&simplified, &clerk).await.status,
        InvoiceStatus::Issued
    );
}

/// A credit note issued on 2027-01-10 against an invoice issued on 2026-12-20
/// to a B2B party gets a `queued` transmission of its own, from its own
/// issue date; the invoice, issued before go-live, has none.
#[renox::test]
async fn a_credit_note_after_go_live_is_queued_for_an_invoice_before_it() {
    let asp = FakeAsp::new();
    let t = booted(&asp, "2026-12-20", 6).await;
    let clerk = t.user_a().await;
    let credit = t
        .helper(
            "Cleo",
            &[
                wl_invoicing::permissions::VIEW,
                wl_invoicing::permissions::CREDIT,
            ],
        )
        .await;
    let invoice = t.issue(&draft_for(&t, BLUE_DUNES).await, &clerk).await;
    assert!(transmissions(&t).await.is_empty());

    t.travel_to("2027-01-10", 6, 0);
    let draft = t
        .app
        .at_travelled_time(credit_notes::draft(
            t.db(),
            &invoice,
            &[CreditLine::new(1, 20_000)],
            "Damaged goods",
            Some(credit.id),
        ))
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
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].document_type, CREDIT_NOTE);
    assert_eq!(all[0].document_id, note.id);
    assert_eq!(all[0].status, TransmissionStatus::Submitted);
    assert_eq!(all[0].payload.0["document"]["type"], "credit_note");
    assert_eq!(
        all[0].payload.0["document"]["credits_invoice"],
        invoice.number.clone().unwrap()
    );
    assert_ne!(all[0].document_type, INVOICE);
}

/// The ASP unreachable and an invoice issued at 10:00: the transmission stays
/// `queued` and is tried again 5 min, 30 min, 2 h, 6 h after `issue`, then
/// every 6 h until it is sent; the banner shows the queue count; the ASP
/// being back sends it, in the end, and the connection recovers.
#[renox::test]
async fn an_unreachable_asp_is_retried_on_the_schedule_until_it_is_sent() {
    let asp = FakeAsp::new();
    asp.unreachable(true);
    let t = booted(&asp, "2027-01-05", 6).await;
    let clerk = t.user_a().await;
    let invoice = t.issue(&draft_for(&t, BLUE_DUNES).await, &clerk).await;

    let first = transmissions(&t).await.remove(0);
    assert_eq!(first.status, TransmissionStatus::Queued);
    assert_eq!(first.tries, 1);
    assert!(
        first
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("does not answer"))
    );
    let after = |t: &EInvoiceTransmission| (t.next_try_at.unwrap() - t.queued_at).num_minutes();
    assert_eq!(after(&first), 5);
    let health = IntegrationConnection::by_key(t.db(), "asp.pgp")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(health.status, ConnectionStatus::Degraded);

    // The banner counts the queue.
    let viewer = t
        .helper(
            "Viewer",
            &[
                wl_invoicing::permissions::VIEW,
                wl_invoicing::permissions::EINVOICES_VIEW,
            ],
        )
        .await;
    t.app.acting_as(&viewer);
    t.app
        .get("/invoices")
        .await
        .assert_ok()
        .assert_see("1 e-invoice transmissions are waiting");

    // Before the first retry nothing is tried; each retry then sets the next.
    at(&t, 4);
    run(&t).await;
    assert_eq!(transmissions(&t).await[0].tries, 1);
    // (minutes after issue at which the run happens, the wait it sets in minutes)
    let steps = [
        (6, 30),
        (31, 120),
        (121, 360),
        (361, 720),
        (721, 1_080),
        (1_081, 1_440),
    ];
    for (minute, next) in steps {
        at(&t, minute);
        run(&t).await;
        let row = transmissions(&t).await.remove(0);
        assert_eq!(row.status, TransmissionStatus::Queued, "at +{minute} min");
        assert_eq!(after(&row), next, "at +{minute} min");
    }
    assert_eq!(asp.submitted().len(), 0);

    // The ASP is back: the next run sends it, in the end, and the connection
    // is up again.
    asp.unreachable(false);
    at(&t, 1_441);
    run(&t).await;
    let sent = transmissions(&t).await.remove(0);
    assert_eq!(sent.status, TransmissionStatus::Submitted);
    assert_eq!(sent.document(), invoice.subject());
    assert_eq!(asp.submitted().len(), 1);
    let health = IntegrationConnection::by_key(t.db(), "asp.pgp")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(health.status, ConnectionStatus::Up);
}

/// After the first 24 hours the retry schedule is every 6 hours: +30 h, +36 h,
/// +42 h, +48 h, each measured from the issue; nothing is tried between them.
#[renox::test]
async fn after_the_first_day_an_unreachable_asp_is_retried_every_six_hours() {
    let asp = FakeAsp::new();
    asp.unreachable(true);
    let t = booted(&asp, "2027-01-05", 6).await;
    let clerk = t.user_a().await;
    t.issue(&draft_for(&t, BLUE_DUNES).await, &clerk).await;
    let after = |t: &EInvoiceTransmission| (t.next_try_at.unwrap() - t.queued_at).num_minutes();

    // The first day: +5 m, +30 m, +2 h, +6 h, +12 h, +18 h, +24 h.
    for minute in [6, 31, 121, 361, 721, 1_081, 1_441] {
        at(&t, minute);
        run(&t).await;
    }
    let row = transmissions(&t).await.remove(0);
    assert_eq!(row.tries, 8);
    assert_eq!(after(&row), 1_800, "next try at +30 h");

    // Then every 6 hours: tried at each step, not in between.
    let mut tries = row.tries;
    for hours in [30_u32, 36, 42, 48] {
        let step = hours * 60;
        at(&t, step - 30);
        run(&t).await;
        assert_eq!(transmissions(&t).await[0].tries, tries, "before +{hours} h");
        at(&t, step + 1);
        run(&t).await;
        let row = transmissions(&t).await.remove(0);
        tries += 1;
        assert_eq!(row.tries, tries, "at +{hours} h");
        assert_eq!(row.status, TransmissionStatus::Queued);
        assert_eq!(after(&row), i64::from(step + 360), "after +{hours} h");
        at(&t, step + 120);
        run(&t).await;
        assert_eq!(transmissions(&t).await[0].tries, tries, "between steps");
    }
    assert_eq!(asp.submitted().len(), 0);
}

/// An entity with a go-live date and no adapter: the invoice is issued, its
/// transmission waits `queued`, and nothing is lost.
#[renox::test]
async fn an_entity_without_an_adapter_keeps_the_transmission_queued() {
    let module = WlInvoicing::new().einvoicing("PGP", go_live(), "missing");
    let t = boot_with(module).await;
    t.directory.set_recipient(BLUE_DUNES, Some(PEPPOL));
    t.travel_to("2027-01-05", 6, 0);
    let clerk = t.user_a().await;
    let invoice = t.issue(&draft_for(&t, BLUE_DUNES).await, &clerk).await;
    assert_eq!(invoice.status, InvoiceStatus::Issued);
    let row = transmissions(&t).await.remove(0);
    assert_eq!(row.status, TransmissionStatus::Queued);
    assert!(
        row.last_error
            .as_deref()
            .is_some_and(|e| e.contains("adapter"))
    );
}

/// A document the issue could not queue is queued by the scheduled run.
#[renox::test]
async fn the_scheduled_run_queues_what_issue_missed() {
    let asp = FakeAsp::new();
    let t = booted(&asp, "2027-01-05", 6).await;
    let clerk = t.user_a().await;
    let invoice = t.issue(&draft_for(&t, BLUE_DUNES).await, &clerk).await;
    EInvoiceTransmission::query()
        .where_(EInvoiceTransmission::DOCUMENT_ID.eq(invoice.id))
        .delete(t.db())
        .await
        .unwrap();
    assert!(transmissions(&t).await.is_empty());
    let report = run(&t).await;
    assert_eq!(report.queued, 1);
    assert_eq!(transmissions(&t).await.len(), 1);
    // Once is enough.
    assert_eq!(run(&t).await.queued, 0);
}

/// The board lists the transmissions with the filters, and needs
/// `einvoices.view`.
#[renox::test]
async fn the_board_lists_the_transmissions_for_those_who_may_see_them() {
    let asp = FakeAsp::new();
    let t = booted(&asp, "2027-01-05", 6).await;
    let clerk = t.user_a().await;
    let invoice = t.issue(&draft_for(&t, BLUE_DUNES).await, &clerk).await;
    let viewer = t
        .helper(
            "Viewer",
            &[
                wl_invoicing::permissions::VIEW,
                wl_invoicing::permissions::EINVOICES_VIEW,
            ],
        )
        .await;
    t.app.acting_as(&viewer);
    t.app
        .get("/einvoices")
        .await
        .assert_ok()
        .assert_see(&invoice.number.clone().unwrap())
        .assert_see("Submitted");
    t.app
        .get("/einvoices?status=rejected")
        .await
        .assert_ok()
        .assert_dont_see(&invoice.number.clone().unwrap());
    let nobody = t.helper("Nobody", &[wl_invoicing::permissions::VIEW]).await;
    t.app.acting_as(&nobody);
    t.app.get("/einvoices").await.assert_forbidden();
    assert_eq!(
        EInvoiceTransmission::status_of(t.db(), &invoice.subject())
            .await
            .unwrap(),
        Some(TransmissionStatus::Submitted)
    );
}

/// BL-PTY-10 (#43) with the real lookup: wl-invoicing answers the kernel
/// `RecipientLookup` through the ASP adapter set for the entity. With the
/// connection `asp.pgp` up, a TRN the ASP knows gives its identifier, an
/// unknown one "not found"; an ASP that does not answer is a failed call
/// reported to the connection's health.
#[renox::test]
async fn the_lookup_of_a_recipient_identifier_asks_the_entitys_asp() {
    use wl_kernel::einvoice::{Lookup, lookup, recipient_lookup};
    let asp = FakeAsp::new();
    let t = booted(&asp, "2027-01-05", 6).await;
    asp.knows("100123456700003", "0235:100123456700003");
    let entity = wl_kernel::LegalEntity::query()
        .where_(wl_kernel::LegalEntity::CODE.eq("PGP"))
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        recipient_lookup(t.state()).connection(&entity).as_deref(),
        Some("asp.pgp")
    );
    wl_kernel::integrations::report_success(t.state(), "asp.pgp")
        .await
        .unwrap();

    let found = t
        .app
        .at_travelled_time(lookup(t.state(), &entity, "100123456700003"))
        .await
        .unwrap();
    assert_eq!(found, Lookup::Found("0235:100123456700003".into()));
    let unknown = t
        .app
        .at_travelled_time(lookup(t.state(), &entity, "100999999900003"))
        .await
        .unwrap();
    assert_eq!(unknown, Lookup::NotFound);

    asp.unreachable(true);
    let failed = t
        .app
        .at_travelled_time(lookup(t.state(), &entity, "100123456700003"))
        .await
        .unwrap();
    assert!(matches!(failed, Lookup::Failed(_)), "{failed:?}");
}
