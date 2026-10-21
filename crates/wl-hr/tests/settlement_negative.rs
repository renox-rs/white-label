//! BL-HRM-23 (#213), `HRM.ACL.06`, `HRM.ACL.04`: recoveries are offset against
//! the dues of a final settlement only within the Art. 25 caps; what is above
//! them is a receivable from the employee with a task for the accountant, and
//! no payment is made for it.

mod support;

use renox::prelude::*;
use wl_hr::{
    Employee, EmployeeStatus, FinalSettlement, Receivable, SettlementPayment, SettlementStatus,
};

use wl_kernel::tickets::Assignee;

use support::Booted;

/// A prepares and approves nothing, B approves, P pays, C is the accountant
/// who gets the recovery task. E, Ravi Kumar (party 61), is the employee.
struct World {
    t: Booted,
    a: User,
    b: User,
    p: User,
    accountant: User,
    e: Employee,
}

async fn world() -> World {
    let t = support::boot().await;
    let a = t.user_with("Amal", "settle_prep").await;
    let b = t.user_with("Basel", "settle_appr").await;
    let p = t.user_with("Pia", "settle_pay").await;
    let accountant = t.user_with("Abdul", "accountant").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    World {
        t,
        a,
        b,
        p,
        accountant,
        e,
    }
}

/// A `draft` settlement of the employee.
async fn draft(w: &World) -> FinalSettlement {
    w.t.settlement_in(&w.e, SettlementStatus::Draft, |_| {})
        .await
}

/// A enters `fields`, B approves; returns the approved settlement.
async fn approved_with(
    w: &World,
    settlement: &FinalSettlement,
    fields: &[(&str, &str)],
) -> FinalSettlement {
    w.t.settle(&w.a, settlement, "amounts", fields)
        .await
        .assert_status(303);
    w.t.settle_transition(&w.b, settlement, "approve")
        .await
        .assert_status(303);
    w.t.settlement(settlement).await
}

async fn payments(w: &World, settlement: &FinalSettlement) -> Vec<SettlementPayment> {
    SettlementPayment::of_settlement(w.t.db(), settlement.id)
        .await
        .unwrap()
}

/// AC1: given dues AED 4,000 and an advance balance AED 6,000, with cap 50%
/// (AED 2,000 offset), net paid AED 2,000 and a receivable of AED 4,000 with an
/// accountant task.
#[renox::test]
async fn hrm23_ac1_an_advance_above_half_the_dues_leaves_a_receivable() {
    let w = world().await;
    let settlement = draft(&w).await;
    let approved = approved_with(
        &w,
        &settlement,
        &[("unpaid_salary", "4000"), ("advance_recoveries", "6000")],
    )
    .await;
    assert_eq!(approved.status, SettlementStatus::Approved);
    assert_eq!(approved.recoveries_offset, 200_000);
    assert_eq!(approved.net, 200_000);
    assert_eq!(approved.owed_by_employee, 400_000);

    w.t.settle(
        &w.p,
        &settlement,
        "pay",
        &[
            ("amount", "2000"),
            ("method", "transfer"),
            ("reference", "TRF-1"),
        ],
    )
    .await
    .assert_status(303);
    let paid = w.t.settlement(&settlement).await;
    assert_eq!(paid.status, SettlementStatus::Paid);
    let made = payments(&w, &settlement).await;
    assert_eq!(made.len(), 1, "no payment is made for the receivable");
    assert_eq!(made[0].amount, 200_000);
    let receivables = Receivable::of_employee(w.t.db(), w.e.id).await.unwrap();
    assert_eq!(receivables.len(), 1);
    assert_eq!(receivables[0].amount, 400_000);
    assert_eq!(receivables[0].party_id, 61);
    assert_eq!(receivables[0].settlement_id, Some(settlement.id));
    let tasks = w.t.tasks.with_key("hr.settlement_receivable:");
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].title.contains("4000.00"));
    assert_eq!(tasks[0].assignee, Some(Assignee::user(w.accountant.id)));
    assert_eq!(w.t.settlement_paid.all().len(), 1);
    assert_eq!(w.t.settlement_paid.all()[0].amount, 200_000);
}

/// AC2: given dues AED 0 and no recoveries (net 0), `pay` succeeds with no
/// payment and no receivable.
#[renox::test]
async fn hrm23_ac2_a_net_of_zero_is_paid_with_no_payment_and_no_receivable() {
    let w = world().await;
    let settlement = draft(&w).await;
    approved_with(&w, &settlement, &[("unpaid_salary", "0")]).await;
    w.t.settle(&w.p, &settlement, "pay", &[("amount", "")])
        .await
        .assert_status(303);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Paid
    );
    assert!(payments(&w, &settlement).await.is_empty());
    assert!(
        Receivable::of_employee(w.t.db(), w.e.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(w.t.tasks.with_key("hr.settlement_receivable:").is_empty());
}

/// AC3: given dues AED 0 and an advance balance AED 1,000, `pay` succeeds with
/// no payment and a receivable of AED 1,000 with an accountant task.
#[renox::test]
async fn hrm23_ac3_recoveries_with_no_dues_are_all_a_receivable() {
    let w = world().await;
    let settlement = draft(&w).await;
    let approved = approved_with(&w, &settlement, &[("advance_recoveries", "1000")]).await;
    assert_eq!((approved.net, approved.recoveries_offset), (0, 0));
    assert_eq!(approved.owed_by_employee, 100_000);
    w.t.settle(&w.p, &settlement, "pay", &[("amount", "")])
        .await
        .assert_status(303);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Paid
    );
    assert!(payments(&w, &settlement).await.is_empty());
    let receivables = Receivable::of_employee(w.t.db(), w.e.id).await.unwrap();
    assert_eq!(receivables.len(), 1);
    assert_eq!(receivables[0].amount, 100_000);
    assert_eq!(w.t.tasks.with_key("hr.settlement_receivable:").len(), 1);
}

/// AC4: given dues AED 10,000 and an overpayment AED 3,000, the offset is
/// AED 2,000 (20%), net AED 8,000 is paid and the receivable is AED 1,000.
#[renox::test]
async fn hrm23_ac4_an_overpayment_is_offset_to_20_percent_of_the_dues() {
    let w = world().await;
    // The overpayment recovery came from a supplementary run.
    let settlement =
        w.t.settlement_in(&w.e, SettlementStatus::Draft, |s| s.recoveries = 300_000)
            .await;
    let approved = approved_with(&w, &settlement, &[("unpaid_salary", "10000")]).await;
    assert_eq!(approved.recoveries_offset, 200_000);
    assert_eq!(approved.net, 800_000);
    w.t.settle(
        &w.p,
        &settlement,
        "pay",
        &[
            ("amount", "8000"),
            ("method", "wps"),
            ("reference", "SIF-3"),
        ],
    )
    .await
    .assert_status(303);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Paid
    );
    let made = payments(&w, &settlement).await;
    assert_eq!(made.iter().map(|p| p.amount).sum::<i64>(), 800_000);
    let receivables = Receivable::of_employee(w.t.db(), w.e.id).await.unwrap();
    assert_eq!(receivables.len(), 1);
    assert_eq!(receivables[0].amount, 100_000);
}

/// The caps are by type: a fine is offset to 5% of the dues, damage to five
/// days of the last wage, and all together to 50%.
#[renox::test]
async fn hrm23_each_type_is_capped_on_its_own_and_all_together_at_half_the_dues() {
    let w = world().await;
    // The last salary structure: AED 3,000 a month, so damage is at most 500.
    w.t.salary(&w.e, 300_000, 1_000).await;
    let settlement = draft(&w).await;
    let approved = approved_with(
        &w,
        &settlement,
        &[
            ("unpaid_salary", "10000"),
            ("fine_recoveries", "1000"),
            ("damage_recoveries", "900"),
        ],
    )
    .await;
    // Fine 5% of 10 000 = 500, damage 5/30 of 3 000 = 500.
    assert_eq!(approved.recoveries_offset, 100_000);
    assert_eq!(approved.net, 900_000);
    assert_eq!(approved.owed_by_employee, 90_000);
}

/// A recovery that a supplementary run adds before the settlement is paid is
/// offset within the caps as well, and `pay` takes the net as it stands then.
#[renox::test]
async fn hrm23_a_recovery_added_after_approval_is_offset_within_the_caps_at_pay() {
    let w = world().await;
    let settlement = draft(&w).await;
    approved_with(&w, &settlement, &[("unpaid_salary", "10000")]).await;
    // A supplementary run adds an overpayment of AED 3,000 (fixture).
    FinalSettlement::query()
        .where_(FinalSettlement::ID.eq(settlement.id))
        .update(w.t.db(), &[("recoveries", &300_000_i64)])
        .await
        .unwrap();
    // The net was 10 000; it is 8 000 now: 10 000 is more than is owed.
    w.t.settle(
        &w.p,
        &settlement,
        "pay",
        &[("amount", "10000"), ("method", "transfer")],
    )
    .await
    .assert_status(422);
    w.t.settle(
        &w.p,
        &settlement,
        "pay",
        &[("amount", "8000"), ("method", "transfer")],
    )
    .await
    .assert_status(303);
    let paid = w.t.settlement(&settlement).await;
    assert_eq!(paid.net, 800_000);
    assert_eq!(paid.owed_by_employee, 100_000);
}

/// A payment that does not cover the net, or none for a net above 0, does not
/// pay the settlement; the preparer cannot approve their own figures.
#[renox::test]
async fn hrm23_pay_needs_the_whole_net_and_a_second_user_approves() {
    let w = world().await;
    let settlement = draft(&w).await;
    w.t.settle(&w.a, &settlement, "amounts", &[("unpaid_salary", "5000")])
        .await
        .assert_status(303);
    // A prepared it with a role that also approves would still be refused.
    let same = w.t.user_with("Amira", "settle_all").await;
    w.t.settle(&same, &settlement, "amounts", &[("unpaid_salary", "5000")])
        .await
        .assert_status(303);
    w.t.settle_transition(&same, &settlement, "approve")
        .await
        .assert_status(422);
    w.t.settle_transition(&w.b, &settlement, "approve")
        .await
        .assert_status(303);
    w.t.settle(
        &w.p,
        &settlement,
        "pay",
        &[("amount", "4999.99"), ("method", "transfer")],
    )
    .await
    .assert_status(422);
    w.t.settle(&w.p, &settlement, "pay", &[("amount", "")])
        .await
        .assert_status(422);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Approved
    );
    assert!(payments(&w, &settlement).await.is_empty());
}
