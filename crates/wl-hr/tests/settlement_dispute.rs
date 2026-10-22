//! BL-HRM-22 (#212), `HRM.ACL.04`: a final settlement is disputed by the
//! worker, a MoHRE complaint or a court case, the undisputed amount is still
//! paid, and the dispute is resolved `upheld` or `revised` by a second user.

mod support;

use renox::prelude::*;
use wl_hr::{
    Employee, EmployeeStatus, FinalSettlement, Receivable, SettlementPayment, SettlementStatus,
};

use support::{Booted, nov};

const DISPUTED: &str = "hr.settlement_disputed";

/// A prepares (and also approves and pays, to show A cannot approve their own
/// figures), B approves, H, C and M are HR, the accountant and the MD who are
/// told. E, Ravi Kumar, is the employee.
struct World {
    t: Booted,
    a: User,
    b: User,
    hr: User,
    accountant: User,
    md: User,
    e: Employee,
}

async fn world() -> World {
    world_on(support::boot().await).await
}

async fn world_on(t: Booted) -> World {
    let a = t.user_with("Amal", "settle_all").await;
    let b = t.user_with("Basel", "settle_appr").await;
    let hr = t.user_with("Hira", "hr").await;
    let accountant = t.user_with("Abdul", "accountant").await;
    let md = t.user_with("Mazen", "owner").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    t.parties.name(61, "Ravi Kumar");
    World {
        t,
        a,
        b,
        hr,
        accountant,
        md,
        e,
    }
}

/// An `approved` settlement with a net of AED 12,000.
async fn approved(w: &World) -> FinalSettlement {
    w.t.settlement_in(&w.e, SettlementStatus::Approved, |s| {
        s.unpaid_salary = 1_200_000;
        s.net = 1_200_000;
        s.prepared_by = Some(w.a.id);
    })
    .await
}

/// A `paid` settlement with a net of AED 12,000, paid in full.
async fn paid(w: &World) -> FinalSettlement {
    let settlement =
        w.t.settlement_in(&w.e, SettlementStatus::Paid, |s| {
            s.unpaid_salary = 1_200_000;
            s.net = 1_200_000;
            s.paid_on = Some(nov(1));
        })
        .await;
    w.t.paid_against(&settlement, 1_200_000).await;
    settlement
}

/// A disputes `settlement`: a MoHRE complaint, C-9, `undisputed` dirhams.
async fn dispute(w: &World, settlement: &FinalSettlement, undisputed: &str) {
    w.t.settle(
        &w.a,
        settlement,
        "dispute",
        &[
            ("source", "mohre_complaint"),
            ("reference", "C-9"),
            ("undisputed_amount", undisputed),
        ],
    )
    .await
    .assert_status(303);
}

async fn paid_sum(w: &World, settlement: &FinalSettlement) -> i64 {
    SettlementPayment::of_settlement(w.t.db(), settlement.id)
        .await
        .unwrap()
        .iter()
        .map(|p| p.amount)
        .sum()
}

/// AC1: given a `paid` settlement, `dispute` with source "MoHRE complaint" and
/// reference "C-9" makes it `disputed`, opens a labour-case ticket and tells
/// HR, the accountant and the MD.
#[renox::test]
async fn hrm22_ac1_a_paid_settlement_is_disputed_with_a_ticket_and_notices() {
    let w = world().await;
    let settlement = paid(&w).await;
    dispute(&w, &settlement, "12000").await;
    let settlement = w.t.settlement(&settlement).await;
    assert_eq!(settlement.status, SettlementStatus::Disputed);
    assert_eq!(
        settlement.dispute_source.as_deref(),
        Some("mohre_complaint")
    );
    assert_eq!(settlement.dispute_reference.as_deref(), Some("C-9"));
    assert_eq!(settlement.disputed_from.as_deref(), Some("paid"));
    let tickets = w.t.tasks.with_key("hr.settlement_dispute:");
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].category_key, "labour_case");
    assert!(tickets[0].title.contains("C-9"));
    assert_eq!(settlement.dispute_ticket, Some(1));
    let notices = w.t.notices.of_type(DISPUTED);
    assert_eq!(notices.len(), 1);
    let mut told = vec![w.hr.id, w.accountant.id, w.md.id];
    told.sort_unstable();
    assert_eq!(support::users(&notices[0]), told);
    let events = w.t.settlement_disputed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reference, "C-9");
    assert_eq!(events[0].source, "mohre_complaint");
}

/// AC1 with the real wl-ticketing: the labour-case ticket is a ticket of the
/// `labour_case` category about the employee, opened from the settlement, and
/// it is closed when the dispute is resolved (AC3).
#[renox::test]
async fn hrm22_ac1_the_labour_case_is_a_real_ticket_opened_and_closed_with_the_dispute() {
    let w = world_on(support::boot_with_ticketing().await).await;
    let settlement = approved(&w).await;
    dispute(&w, &settlement, "10000").await;

    let settlement = w.t.settlement(&settlement).await;
    let tickets = w.t.tickets().await;
    assert_eq!(tickets.len(), 1);
    let (ticket, category) = &tickets[0];
    assert_eq!(category, "labour_case");
    assert_eq!(settlement.dispute_ticket, Some(ticket.id));
    let subject = |kind: &str, id: i64| wl_kernel::Subject::new(kind, id);
    assert_eq!(
        subject(&ticket.subject_type, ticket.subject_id),
        w.e.subject()
    );
    assert_eq!(
        subject(
            ticket.source_type.as_deref().unwrap(),
            ticket.source_id.unwrap()
        ),
        settlement.subject()
    );
    assert!(ticket.title.contains("C-9"));
    assert!(ticket.description.contains("C-9"));
    assert!(ticket.is_open());

    w.t.settle(
        &w.b,
        &settlement,
        "resolve",
        &[("outcome", "upheld"), ("document", "Award-3")],
    )
    .await
    .assert_status(303);
    let tickets = w.t.tickets().await;
    assert_eq!(tickets.len(), 1);
    assert!(!tickets[0].0.is_open(), "{:?}", tickets[0].0.status);
}

/// AC1 (guard): a dispute without a reference, or without the undisputed
/// amount, is refused.
#[renox::test]
async fn hrm22_a_dispute_needs_a_source_a_reference_and_the_undisputed_amount() {
    let w = world().await;
    let settlement = approved(&w).await;
    w.t.settle(
        &w.a,
        &settlement,
        "dispute",
        &[("source", "court"), ("reference", "")],
    )
    .await
    .assert_status(422);
    w.t.settle(
        &w.a,
        &settlement,
        "dispute",
        &[("source", "court"), ("reference", "CASE-1")],
    )
    .await
    .assert_status(422);
    w.t.settle(
        &w.a,
        &settlement,
        "dispute",
        &[
            ("source", "court"),
            ("reference", "CASE-1"),
            ("undisputed_amount", "12000.01"),
        ],
    )
    .await
    .assert_status(422);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Approved
    );
}

/// AC2: given `approved` AED 12,000 with AED 10,000 undisputed, a payment of
/// AED 10,000 is recorded while `disputed`.
#[renox::test]
async fn hrm22_ac2_the_undisputed_amount_is_paid_while_disputed() {
    let w = world().await;
    let settlement = approved(&w).await;
    dispute(&w, &settlement, "10000").await;
    w.t.settle(
        &w.a,
        &settlement,
        "payments",
        &[
            ("amount", "10000"),
            ("method", "transfer"),
            ("reference", "TRF-1"),
        ],
    )
    .await
    .assert_status(303);
    let stored = w.t.settlement(&settlement).await;
    assert_eq!(stored.status, SettlementStatus::Disputed);
    assert_eq!(stored.undisputed_amount, 1_000_000);
    assert_eq!(paid_sum(&w, &settlement).await, 1_000_000);
}

/// AC3: A enters revised amounts of AED 11,000 while `disputed`; A's `resolve`
/// is refused; B runs `resolve` `revised` with the decision: `approved`, new
/// statement, `pay` for AED 1,000; the ticket is closed.
#[renox::test]
async fn hrm22_ac3_revised_amounts_are_resolved_by_another_user_and_the_difference_is_paid() {
    let w = world().await;
    let settlement = approved(&w).await;
    dispute(&w, &settlement, "10000").await;
    w.t.settle(
        &w.a,
        &settlement,
        "payments",
        &[
            ("amount", "10000"),
            ("method", "transfer"),
            ("reference", "TRF-1"),
        ],
    )
    .await
    .assert_status(303);
    w.t.settle(&w.a, &settlement, "amounts", &[("unpaid_salary", "11000")])
        .await
        .assert_status(303);
    let revised = w.t.settlement(&settlement).await;
    assert_eq!(revised.net, 1_100_000);
    assert_eq!(revised.revised_by, Some(w.a.id));

    let decision = [("outcome", "revised"), ("document", "Agreement-7")];
    w.t.settle(&w.a, &settlement, "resolve", &decision)
        .await
        .assert_status(422);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Disputed
    );

    w.t.settle(&w.b, &settlement, "resolve", &decision)
        .await
        .assert_status(303);
    let resolved = w.t.settlement(&settlement).await;
    assert_eq!(resolved.status, SettlementStatus::Approved);
    assert_eq!(resolved.statement_version, 2);
    assert_eq!(resolved.approved_by, Some(w.b.id));
    assert_eq!(resolved.dispute_outcome.as_deref(), Some("revised"));
    assert_eq!(resolved.resolution_document.as_deref(), Some("Agreement-7"));
    let closed = w.t.tasks.completed();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].0, 1);

    w.t.settle(
        &w.a,
        &settlement,
        "pay",
        &[
            ("amount", "1000"),
            ("method", "transfer"),
            ("reference", "TRF-2"),
        ],
    )
    .await
    .assert_status(303);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Paid
    );
    assert_eq!(paid_sum(&w, &settlement).await, 1_100_000);
}

/// AC3 (guard): `revised` needs revised amounts that somebody entered.
#[renox::test]
async fn hrm22_resolving_as_revised_needs_amounts_entered_while_disputed() {
    let w = world().await;
    let settlement = approved(&w).await;
    dispute(&w, &settlement, "0").await;
    w.t.settle(
        &w.b,
        &settlement,
        "resolve",
        &[("outcome", "revised"), ("document", "Agreement-7")],
    )
    .await
    .assert_status(422);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Disputed
    );
}

/// AC4: `resolve` without a document is refused.
#[renox::test]
async fn hrm22_ac4_resolve_without_a_document_is_refused() {
    let w = world().await;
    let settlement = approved(&w).await;
    dispute(&w, &settlement, "0").await;
    w.t.settle(
        &w.b,
        &settlement,
        "resolve",
        &[("outcome", "upheld"), ("document", "  ")],
    )
    .await
    .assert_status(422);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Disputed
    );
}

/// AC5: given `disputed` from `approved` with AED 10,000 of AED 12,000 paid,
/// `resolve` `upheld` makes it `approved`; `pay` for AED 2,000 makes it `paid`.
/// A second dispute after the resolution is allowed.
#[renox::test]
async fn hrm22_ac5_upheld_goes_back_to_approved_when_the_net_is_not_fully_paid() {
    let w = world().await;
    let settlement = approved(&w).await;
    w.t.settle(
        &w.a,
        &settlement,
        "payments",
        &[
            ("amount", "10000"),
            ("method", "wps"),
            ("reference", "SIF-1"),
        ],
    )
    .await
    .assert_status(303);
    dispute(&w, &settlement, "10000").await;
    w.t.settle(
        &w.b,
        &settlement,
        "resolve",
        &[("outcome", "upheld"), ("document", "Decision-3")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Approved
    );
    w.t.settle(
        &w.a,
        &settlement,
        "pay",
        &[
            ("amount", "2000"),
            ("method", "transfer"),
            ("reference", "TRF-9"),
        ],
    )
    .await
    .assert_status(303);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Paid
    );
    // A later claim: disputed again from `paid`.
    dispute(&w, &settlement, "12000").await;
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Disputed
    );
}

/// AC5: given `disputed` from `paid`, `upheld` makes it `closed`.
#[renox::test]
async fn hrm22_ac5_upheld_closes_a_settlement_that_is_paid_in_full() {
    let w = world().await;
    let settlement = paid(&w).await;
    dispute(&w, &settlement, "12000").await;
    w.t.settle(
        &w.b,
        &settlement,
        "resolve",
        &[("outcome", "upheld"), ("document", "Judgment-2")],
    )
    .await
    .assert_status(303);
    let closed = w.t.settlement(&settlement).await;
    assert_eq!(closed.status, SettlementStatus::Closed);
    assert_eq!(closed.dispute_outcome.as_deref(), Some("upheld"));
    assert!(closed.closed_on.is_some());
    assert_eq!(w.t.tasks.completed().len(), 1);
}

/// A revised net below what was already paid makes the excess a receivable
/// from the employee, with a task for the accountant.
#[renox::test]
async fn hrm22_a_revised_net_below_the_amount_paid_makes_the_excess_a_receivable() {
    let w = world().await;
    let settlement = paid(&w).await;
    dispute(&w, &settlement, "12000").await;
    w.t.settle(&w.a, &settlement, "amounts", &[("unpaid_salary", "9000")])
        .await
        .assert_status(303);
    w.t.settle(
        &w.b,
        &settlement,
        "resolve",
        &[("outcome", "revised"), ("document", "Judgment-5")],
    )
    .await
    .assert_status(303);
    let stored = w.t.settlement(&settlement).await;
    assert_eq!(stored.status, SettlementStatus::Approved);
    assert_eq!(stored.overpaid, 300_000);
    let receivables = Receivable::of_employee(w.t.db(), w.e.id).await.unwrap();
    assert_eq!(receivables.len(), 1);
    assert_eq!(receivables[0].amount, 300_000);
    assert_eq!(receivables[0].settlement_id, Some(settlement.id));
    assert_eq!(w.t.tasks.with_key("hr.settlement_receivable:").len(), 1);
    // Nothing more is owed or paid: `pay` with no amount settles it.
    w.t.settle(&w.a, &settlement, "pay", &[("amount", "")])
        .await
        .assert_status(303);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Paid
    );
    assert_eq!(
        Receivable::of_employee(w.t.db(), w.e.id)
            .await
            .unwrap()
            .len(),
        1
    );
}
