//! Hours corrected after billing (wl-invoicing §5.2 step 5, #86): an approved
//! adjustment timesheet credits fewer hours against the original invoice,
//! bills extra hours on the next usage invoice or on one of their own, is
//! billed with the original when that is not yet invoiced, and a dispute
//! resolved `credited` links to its credit note.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::dispute::{self, Dispute};
use wl_invoicing::dispute_resolve::{self, ResolveInput};
use wl_invoicing::invoicer::lines_of;
use wl_invoicing::permissions as perm;
use wl_invoicing::task::{InvoicingTask, TaskKind, TaskStatus};
use wl_invoicing::usage::{self, UsageDraft, UsageRecord};
use wl_invoicing::usage_adjustment::{self, AdjustmentOutcome};
use wl_invoicing::{AdjustmentPlus, Invoice, WlInvoicing, credit_notes};
use wl_kernel::attendance::{CategoryMinutes, TimesheetApproved};
use wl_kernel::invoicing::{
    CreditLine, DisputeBy, DisputeOutcome, DisputeRequest, InvoiceId, InvoiceStatus,
};
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{PartyId, Subject};

use support::{BLUE_DUNES, Booted, boot, boot_with};

/// The timesheet the 160 hours were billed from.
fn original() -> Subject {
    Subject::new("attendance.timesheet", 2)
}

/// The adjustment timesheet.
fn adjustment() -> Subject {
    Subject::new("attendance.timesheet", 9)
}

/// `TimesheetApproved` of an adjustment of the original, with signed
/// `minutes` of normal hours.
fn approved(minutes: i64, affects: &str) -> TimesheetApproved {
    TimesheetApproved {
        timesheet: adjustment(),
        owner: EmployeeId(1),
        period_from: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
        period_to: NaiveDate::from_ymd_opt(2026, 11, 8).unwrap(),
        context: None,
        totals: vec![CategoryMinutes {
            category: "normal".into(),
            minutes,
        }],
        approver_kind: "external".into(),
        approval_basis: "approved".into(),
        kind: "adjustment".into(),
        adjusts: Some(original()),
        origin: Some("client_dispute".into()),
        affects: Some(affects.to_owned()),
    }
}

/// 160 h at AED 40.00 recorded for Blue Dunes, unbilled.
async fn recorded(t: &Booted) -> UsageRecord {
    usage::record(
        t.db(),
        UsageDraft::hours(BLUE_DUNES, original(), "normal", 160_000, 4_000)
            .description("Normal hours, week 44")
            .rate_date(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()),
    )
    .await
    .unwrap()
}

/// INV-100: the 160 hours billed and issued.
async fn invoiced(t: &Booted) -> (UsageRecord, Invoice) {
    let record = recorded(t).await;
    let drafts = usage::bill(t.db(), BLUE_DUNES).await.unwrap();
    assert_eq!(drafts.len(), 1);
    let clerk = t.user_a().await;
    let invoice = t.issue(&drafts[0], &clerk).await;
    assert_eq!(invoice.status, InvoiceStatus::Issued);
    (reload(t, &record).await, invoice)
}

async fn reload(t: &Booted, record: &UsageRecord) -> UsageRecord {
    UsageRecord::find(t.db(), record.id).await.unwrap().unwrap()
}

/// Emits the event as wl-attendance does after the commit.
async fn arrives(t: &Booted, event: TimesheetApproved) {
    t.state().emit(event).await.unwrap();
}

/// A user who may dispute and credit.
async fn crediter(t: &Booted) -> User {
    t.helper(
        "Cleo",
        &[perm::VIEW, perm::ISSUE, perm::DISPUTE, perm::CREDIT],
    )
    .await
}

/// Done when 1: INV-100 has 160 h x AED 40.00; an approved adjustment of -8 h
/// (origin `client_dispute`) gives a credit note of AED 320.00 + VAT against
/// INV-100 that names the adjustment; the invoice is not edited.
#[renox::test]
async fn fewer_hours_give_a_credit_note_against_the_invoice() {
    let t = boot().await;
    let (_, invoice) = invoiced(&t).await;
    assert_eq!(invoice.net_total, 640_000);
    arrives(&t, approved(-480, "billing")).await;
    let notes = credit_notes::for_invoice(t.db(), invoice.id).await.unwrap();
    assert_eq!(notes.len(), 1);
    let note = &notes[0];
    assert_eq!(
        (note.net_total, note.vat_total, note.gross_total),
        (32_000, 1_600, 33_600)
    );
    assert!(note.reason.contains("attendance.timesheet:9"), "{}", note.reason);
    assert!(note.reason.contains("client_dispute"), "{}", note.reason);
    assert!(note.reason.contains("8 h"), "{}", note.reason);
    // The credit is used on the invoice; its lines and totals are as issued.
    let after = t.reload(&invoice).await;
    assert_eq!(after.net_total, 640_000);
    assert_eq!(lines_of(t.db(), invoice.id).await.unwrap()[0].quantity_milli, 160_000);
    assert_eq!(after.balance, invoice.gross_total - 33_600);
    let rows = usage_adjustment::of_adjustment(t.db(), &adjustment())
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, AdjustmentOutcome::CreditNote);
    assert_eq!(rows[0].credit_note_id, Some(note.id));
}

/// Event received twice: one credit note per adjustment.
#[renox::test]
async fn an_event_received_twice_credits_once() {
    let t = boot().await;
    let (_, invoice) = invoiced(&t).await;
    arrives(&t, approved(-480, "billing")).await;
    arrives(&t, approved(-480, "billing")).await;
    assert_eq!(
        credit_notes::for_invoice(t.db(), invoice.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// Done when 2: +6 h with the setting at its default is an extra line of
/// 6 h x AED 40.00 = AED 240.00 on the next usage invoice, naming the
/// adjustment, at the original record's price.
#[renox::test]
async fn extra_hours_are_an_extra_line_on_the_next_invoice() {
    let t = boot().await;
    let (_, invoice) = invoiced(&t).await;
    assert_eq!(t.config().adjustment_plus, AdjustmentPlus::ExtraLine);
    arrives(&t, approved(360, "both")).await;
    // Nothing is credited and the original invoice is as it was.
    assert!(
        credit_notes::for_invoice(t.db(), invoice.id)
            .await
            .unwrap()
            .is_empty()
    );
    let waiting = usage::unbilled(t.db(), BLUE_DUNES).await.unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].adjustment(), Some(adjustment()));
    // The next usage invoice carries the line.
    let drafts = usage::bill(t.db(), BLUE_DUNES).await.unwrap();
    assert_eq!(drafts.len(), 1);
    let lines = lines_of(t.db(), drafts[0].id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(
        (lines[0].quantity_milli, lines[0].unit_price, lines[0].net_amount),
        (6_000, 4_000, 24_000)
    );
    assert!(
        lines[0].description.contains("attendance.timesheet:9"),
        "{}",
        lines[0].description
    );
    assert_eq!(lines[0].origin(), Some(adjustment()));
    let rows = usage_adjustment::of_adjustment(t.db(), &adjustment())
        .await
        .unwrap();
    assert_eq!(rows[0].outcome, AdjustmentOutcome::ExtraLine);
}

/// Done when 3: +6 h with the setting `separate invoice` gives a separate
/// draft invoice with one line of AED 240.00 naming the adjustment; the next
/// invoice has no such line.
#[renox::test]
async fn extra_hours_can_be_a_separate_invoice() {
    let t = boot_with(WlInvoicing::new().adjustment_plus(AdjustmentPlus::SeparateInvoice)).await;
    let (_, invoice) = invoiced(&t).await;
    arrives(&t, approved(360, "billing")).await;
    let rows = usage_adjustment::of_adjustment(t.db(), &adjustment())
        .await
        .unwrap();
    assert_eq!(rows[0].outcome, AdjustmentOutcome::ExtraInvoice);
    let separate = t.invoice(rows[0].extra_invoice_id.unwrap()).await;
    assert_eq!(separate.status, InvoiceStatus::Draft);
    assert_eq!(separate.party_id, BLUE_DUNES.0);
    assert_ne!(separate.id, invoice.id);
    let lines = lines_of(t.db(), separate.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].net_amount, 24_000);
    assert!(lines[0].description.contains("attendance.timesheet:9"));
    // The next usage invoice has no such line: nothing is left to bill.
    assert!(usage::unbilled(t.db(), BLUE_DUNES).await.unwrap().is_empty());
    assert!(usage::bill(t.db(), BLUE_DUNES).await.unwrap().is_empty());
    // And later usage is billed without it.
    let later = usage::record(
        t.db(),
        UsageDraft::hours(
            BLUE_DUNES,
            Subject::new("attendance.timesheet", 5),
            "normal",
            8_000,
            4_000,
        ),
    )
    .await
    .unwrap();
    let next = usage::bill(t.db(), BLUE_DUNES).await.unwrap();
    assert_eq!(next.len(), 1);
    let ids: Vec<i64> = usage::of_invoice(t.db(), next[0].id)
        .await
        .unwrap()
        .iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, [later.id]);
}

/// Done when 4: the original is not yet invoiced, so the draft bills 152 h.
#[renox::test]
async fn an_uninvoiced_original_is_billed_with_the_change() {
    let t = boot().await;
    let record = recorded(&t).await;
    arrives(&t, approved(-480, "billing")).await;
    assert_eq!(reload(&t, &record).await.quantity_milli, 152_000);
    let drafts = usage::bill(t.db(), BLUE_DUNES).await.unwrap();
    let lines = lines_of(t.db(), drafts[0].id).await.unwrap();
    assert_eq!(lines[0].quantity_milli, 152_000);
    assert_eq!(drafts[0].net_total, 608_000);
    // No credit note: nothing was invoiced.
    assert!(
        credit_notes::for_invoice(t.db(), drafts[0].id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// A draft that already holds the line is corrected, totals included.
#[renox::test]
async fn a_draft_invoice_line_is_corrected() {
    let t = boot().await;
    let record = recorded(&t).await;
    let drafts = usage::bill(t.db(), BLUE_DUNES).await.unwrap();
    arrives(&t, approved(-480, "billing")).await;
    let draft = t.reload(&drafts[0]).await;
    assert_eq!(draft.status, InvoiceStatus::Draft);
    assert_eq!(
        (draft.net_total, draft.vat_total, draft.gross_total),
        (608_000, 30_400, 638_400)
    );
    assert_eq!(draft.balance, 638_400);
    assert_eq!(
        lines_of(t.db(), draft.id).await.unwrap()[0].quantity_milli,
        152_000
    );
    assert_eq!(reload(&t, &record).await.quantity_milli, 152_000);
}

/// An adjustment that affects payroll only is ignored, and so is a regular
/// timesheet or one that corrects nothing billed.
#[renox::test]
async fn payroll_only_and_unbilled_timesheets_are_ignored() {
    let t = boot().await;
    let (record, invoice) = invoiced(&t).await;
    arrives(&t, approved(-480, "payroll")).await;
    let regular = TimesheetApproved {
        kind: "regular".into(),
        adjusts: None,
        origin: None,
        affects: None,
        ..approved(-480, "billing")
    };
    arrives(&t, regular).await;
    let other = TimesheetApproved {
        adjusts: Some(Subject::new("attendance.timesheet", 77)),
        ..approved(-480, "billing")
    };
    arrives(&t, other).await;
    assert!(
        credit_notes::for_invoice(t.db(), invoice.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(usage_adjustment::of_adjustment(t.db(), &adjustment())
        .await
        .unwrap()
        .is_empty());
    assert_eq!(reload(&t, &record).await.quantity_milli, 160_000);
}

/// Edge case: a credit above what the invoice can still be credited by is
/// refused, and the coordinator gets a task (once).
#[renox::test]
async fn a_credit_above_the_remaining_credit_gives_the_coordinator_a_task() {
    let t = boot().await;
    let (_, invoice) = invoiced(&t).await;
    // The whole line is credited already.
    credit_notes::credit_partly(
        t.state(),
        invoice.id,
        &[CreditLine::new(1, 640_000)],
        "Goodwill",
        credit_notes::By::System(None),
    )
    .await
    .unwrap();
    arrives(&t, approved(-480, "billing")).await;
    arrives(&t, approved(-480, "billing")).await;
    let notes = credit_notes::for_invoice(t.db(), invoice.id).await.unwrap();
    assert_eq!(notes.len(), 1, "the refused credit is not made");
    let tasks = InvoicingTask::query()
        .where_(InvoicingTask::KIND.eq(TaskKind::AdjustmentCreditRefused))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.role, "coordinator");
    assert_eq!(task.status, TaskStatus::Open);
    assert_eq!(task.invoice_id, Some(invoice.id));
    assert_eq!(task.amount, 32_000);
    assert!(task.message.contains("attendance.timesheet:9"), "{}", task.message);
    let rows = usage_adjustment::of_adjustment(t.db(), &adjustment())
        .await
        .unwrap();
    assert_eq!(rows[0].outcome, AdjustmentOutcome::Refused);
    assert_eq!(rows[0].credit_note_id, None);
}

/// Done when 5: a disputed INV-100 resolved `credited` links to the credit
/// note of the adjustment, without a second one.
#[renox::test]
async fn a_dispute_resolved_credited_links_to_the_adjustment_credit_note() {
    let t = boot().await;
    let (_, invoice) = invoiced(&t).await;
    let cleo = crediter(&t).await;
    let id = dispute::raise(
        t.state(),
        DisputeRequest::new(
            InvoiceId(invoice.id),
            "8 hours not worked",
            DisputeBy::User { id: cleo.id },
        )
        .lines([1]),
    )
    .await
    .unwrap();
    arrives(&t, approved(-480, "billing")).await;
    let notes = credit_notes::for_invoice(t.db(), invoice.id).await.unwrap();
    assert_eq!(notes.len(), 1);
    // Resolving without the adjustment is still refused for a timesheet line.
    let refused = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("Agreed", 33_600),
        DisputeBy::User { id: cleo.id },
    )
    .await;
    assert!(refused.is_err());
    let resolution = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput {
            outcome: DisputeOutcome::Credited,
            note: "Client's 8 hours credited".into(),
            agreed_amount: None,
            adjustment_timesheet: Some(adjustment()),
        },
        DisputeBy::User { id: cleo.id },
    )
    .await
    .unwrap();
    assert!(resolution.is_resolved());
    assert_eq!(resolution.credit_note, Some(notes[0].id));
    let dispute = Dispute::find(t.db(), id).await.unwrap().unwrap();
    assert_eq!(dispute.outcome, Some(DisputeOutcome::Credited));
    assert_eq!(dispute.credit_note_id, Some(notes[0].id));
    assert_eq!(dispute.agreed_amount, Some(33_600));
    assert!(!t.reload(&invoice).await.disputed);
    assert_eq!(
        credit_notes::for_invoice(t.db(), invoice.id)
            .await
            .unwrap()
            .len(),
        1,
        "no second credit note"
    );
    assert_eq!(t.resolved.all().len(), 1);
}

/// An agreed amount that is not the adjustment's credit note is refused.
#[renox::test]
async fn the_agreed_amount_must_be_the_credit_notes() {
    let t = boot().await;
    let (_, invoice) = invoiced(&t).await;
    let cleo = crediter(&t).await;
    dispute::raise(
        t.state(),
        DisputeRequest::new(
            InvoiceId(invoice.id),
            "8 hours not worked",
            DisputeBy::User { id: cleo.id },
        )
        .lines([1]),
    )
    .await
    .unwrap();
    arrives(&t, approved(-480, "billing")).await;
    let refused = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput {
            adjustment_timesheet: Some(adjustment()),
            ..ResolveInput::credited("Agreed", 10_000)
        },
        DisputeBy::User { id: cleo.id },
    )
    .await;
    assert!(refused.is_err());
    assert!(t.reload(&invoice).await.disputed);
}

/// A usage record needs hours and a line text.
#[renox::test]
async fn a_usage_record_needs_hours_and_a_text() {
    let t = boot().await;
    let none = usage::record(
        t.db(),
        UsageDraft::hours(PartyId(1), original(), "normal", 0, 4_000),
    )
    .await;
    assert!(none.is_err());
    let blank = usage::record(
        t.db(),
        UsageDraft::hours(PartyId(1), original(), "normal", 1_000, 4_000).description(" "),
    )
    .await;
    assert!(blank.is_err());
}
