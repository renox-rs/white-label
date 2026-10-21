//! BL-ATT-09 (#186) with the real wl-invoicing in place of the fake `Invoicer`:
//! a dispute raised on an invoice built from a locked timesheet T, an
//! adjustment J opened for it, J approved (`TimesheetApproved` reaches
//! wl-invoicing, which credits the hours), the dispute resolved `credited`
//! naming J, and the link between J and the credit note both ways. The same
//! disputes end `upheld` for the other half of the point (wl-attendance §4
//! *Correcting a locked timesheet*, §5.7 point 4; `HRM.ACL.08`).

mod support;

use renox::prelude::*;
use wl_attendance::timesheet::{Affects, ApproverKind};
use wl_attendance::{Timesheet, TimesheetStatus};
use wl_invoicing::dispute::{self, Dispute};
use wl_invoicing::dispute_resolve::{self, ResolveInput};
use wl_invoicing::usage::{self, UsageDraft};
use wl_invoicing::{Invoice, credit_notes};
use wl_kernel::invoicing::{DisputeBy, DisputeOutcome, DisputeRequest, InvoiceId, InvoiceStatus};

use support::{
    BLUE_DUNES, Booted, boot_with_invoicing, locked_eight_hours, minus_eight_hours, post_to,
    project,
};

const UPHELD_NOTICE: &str = "attendance.adjustment_dispute_upheld";

/// The people of the case: Amal opens adjustments, Bilal approves, Tariq
/// submits, Cleo is the receivables clerk who issues and disputes.
struct Cast {
    adjuster: User,
    approver: User,
    keeper: User,
    clerk: User,
}

async fn cast(t: &Booted) -> Cast {
    Cast {
        adjuster: t.user_with("Amal", "adjuster").await,
        approver: t.user_with("Bilal", "approver").await,
        keeper: t.user_with("Tariq", "timekeeper").await,
        clerk: t.user_with("Cleo", "receivables").await,
    }
}

/// INV-1 of 160 h at AED 40.00 for Blue Dunes, built from the locked
/// timesheet T and issued, and the dispute on its first line.
async fn disputed_invoice(t: &Booted, who: &Cast) -> (Timesheet, Invoice, i64) {
    let original =
        locked_eight_hours(t, t.employee(1, &[]), &project(5), ApproverKind::Internal).await;
    usage::record(
        t.db(),
        UsageDraft::hours(BLUE_DUNES, original.subject(), "normal", 160_000, 4_000)
            .description("Normal hours, week 44"),
    )
    .await
    .unwrap();
    let drafts = usage::bill(t.db(), BLUE_DUNES).await.unwrap();
    assert_eq!(drafts.len(), 1);
    t.app.acting_as(&who.clerk);
    t.app
        .htmx()
        .post(&format!("/invoices/{}/issue", drafts[0].id), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", drafts[0].id));
    let invoice = Invoice::find_or_404(t.db(), drafts[0].id).await.unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Issued);
    let id = dispute::raise(
        t.state(),
        DisputeRequest::new(
            InvoiceId(invoice.id),
            "8 hours not worked",
            DisputeBy::User { id: who.clerk.id },
        )
        .lines([1]),
    )
    .await
    .unwrap();
    (original, invoice, id)
}

/// Opens J on `original` as the adjuster, linked to dispute `dispute`.
async fn open_j(t: &Booted, who: &Cast, original: &Timesheet, dispute: i64) -> Timesheet {
    let entries = minus_eight_hours();
    let link = format!("invoicing.dispute:{dispute}");
    post_to(
        t,
        &who.adjuster,
        original,
        "adjustments",
        &[
            ("reason", "client and business agreed to cut 8 h"),
            ("origin", "client_dispute"),
            ("linked", link.as_str()),
            ("entries", entries.as_str()),
        ],
    )
    .await
    .assert_ok();
    original.adjusted_by(t.db()).await.unwrap().pop().unwrap()
}

/// AC1 and AC2: the dispute of INV-1 is real, J (-8 h) is linked to it and
/// defaults to `billing`; nothing was opened when the dispute was raised; J is
/// approved and wl-invoicing raises the credit note that names J; the dispute
/// is resolved `credited` naming J; J carries the credit note's id and the
/// credit note names J, and a second resolution event changes nothing.
#[renox::test]
async fn att09_a_credited_dispute_links_the_adjustment_and_the_credit_note_both_ways() {
    let t = boot_with_invoicing().await;
    let who = cast(&t).await;
    let (original, invoice, dispute_id) = disputed_invoice(&t, &who).await;
    assert!(
        original.adjusted_by(t.db()).await.unwrap().is_empty(),
        "nothing is opened when the dispute is raised"
    );

    let j = open_j(&t, &who, &original, dispute_id).await;
    assert_eq!(j.status, TimesheetStatus::Open);
    assert_eq!(j.affects, Some(Affects::Billing));
    assert_eq!(j.linked().unwrap().subject_id, dispute_id);

    post_to(&t, &who.keeper, &j, "submit", &[("x", "y")])
        .await
        .assert_ok();
    post_to(&t, &who.approver, &j, "approve", &[("x", "y")])
        .await
        .assert_ok();
    assert_eq!(t.reload_sheet(&j).await.status, TimesheetStatus::Approved);

    // wl-invoicing credited the hours as soon as TimesheetApproved arrived.
    let notes = credit_notes::for_invoice(t.db(), invoice.id).await.unwrap();
    assert_eq!(notes.len(), 1, "one credit note against INV-1");
    assert_eq!(
        (notes[0].net_total, notes[0].gross_total),
        (32_000, 33_600),
        "8 h at AED 40.00 plus VAT"
    );
    assert!(
        notes[0].reason.contains(&j.subject().to_string()),
        "the credit note names J: {}",
        notes[0].reason
    );
    assert_eq!(t.reload_sheet(&j).await.credit_note_id, None, "not yet");

    let resolution = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput {
            outcome: DisputeOutcome::Credited,
            note: "Client's 8 hours credited".into(),
            agreed_amount: None,
            adjustment_timesheet: Some(j.subject()),
        },
        DisputeBy::User { id: who.clerk.id },
    )
    .await
    .unwrap();
    assert!(resolution.is_resolved());
    assert_eq!(resolution.credit_note, Some(notes[0].id));

    // J -> credit note, from wl-attendance's listener of the real event.
    assert_eq!(t.reload_sheet(&j).await.credit_note_id, Some(notes[0].id));
    // Credit note -> J, and the dispute -> credit note, on wl-invoicing's side.
    let stored = Dispute::find(t.db(), dispute_id).await.unwrap().unwrap();
    assert_eq!(stored.outcome, Some(DisputeOutcome::Credited));
    assert_eq!(stored.credit_note_id, Some(notes[0].id));
    let notes = credit_notes::for_invoice(t.db(), invoice.id).await.unwrap();
    assert_eq!(notes.len(), 1, "no second credit note");
    assert!(notes[0].reason.contains(&j.subject().to_string()));

    // The dispute is over: a new adjustment cannot be linked to it.
    let entries = minus_eight_hours();
    let link = format!("invoicing.dispute:{dispute_id}");
    post_to(
        &t,
        &who.adjuster,
        &original,
        "adjustments",
        &[
            ("reason", "again"),
            ("origin", "client_dispute"),
            ("linked", link.as_str()),
            ("entries", entries.as_str()),
        ],
    )
    .await
    .assert_status(422);
}

/// AC3 with the real invoicer: J was opened and submitted, then the dispute is
/// resolved `upheld` by wl-invoicing; J can no longer be approved, A is told
/// once, and no credit note exists.
#[renox::test]
async fn att09_an_upheld_dispute_stops_the_adjustment_and_creates_no_credit_note() {
    let t = boot_with_invoicing().await;
    let who = cast(&t).await;
    let (original, invoice, dispute_id) = disputed_invoice(&t, &who).await;
    let j = open_j(&t, &who, &original, dispute_id).await;
    post_to(&t, &who.keeper, &j, "submit", &[("x", "y")])
        .await
        .assert_ok();

    let resolution = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::upheld("The hours were worked"),
        DisputeBy::User { id: who.clerk.id },
    )
    .await
    .unwrap();
    assert!(resolution.is_resolved());

    let stored = t.reload_sheet(&j).await;
    assert!(stored.dispute_upheld_at.is_some());
    assert_eq!(stored.status, TimesheetStatus::Submitted);
    post_to(&t, &who.approver, &j, "approve", &[("x", "y")])
        .await
        .assert_status(422);
    assert_eq!(t.reload_sheet(&j).await.status, TimesheetStatus::Submitted);
    assert!(
        credit_notes::for_invoice(t.db(), invoice.id)
            .await
            .unwrap()
            .is_empty()
    );
    let told = t.notices.of_type(UPHELD_NOTICE);
    assert_eq!(told.len(), 1);
    assert_eq!(support::users_of(&told[0]), vec![who.adjuster.id]);
}
