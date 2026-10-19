//! Reversing a recovery (wl-invoicing §4 `unallocate`, #70 part 4, A10):
//! the invoice goes back to `written_off`, the recovered amount comes
//! down, and the VAT side is undone from the share stored on the recovery.
//! The money comes in through `Invoicer::allocate` and goes out through
//! `Invoicer::unallocate`, as `wl-payments` does.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::bad_debt_relief;
use wl_invoicing::vat_summary::{VatSummary, VatSummaryStatus};
use wl_invoicing::{
    AdjustmentKind, AdjustmentStatus, Allocation, Invoice, Recovery, Unallocation,
    UnallocationOutcome, VatAdjustment,
};
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, ReopenReason, Reopened, Unallocate};

use support::{Booted, boot};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a date")
}

fn supply() -> NaiveDate {
    day(2026, 3, 15)
}

fn first_day() -> NaiveDate {
    day(2026, 9, 16)
}

fn payment() -> Subject {
    Subject::new("payments.payment", 7)
}

/// How the relief of the written-off invoice (AED 50.00 on AED 1,050.00)
/// stands when the money comes in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Relief {
    Pending,
    Included,
    Excluded,
}

struct Case {
    t: Booted,
    invoice: Invoice,
    relief: VatAdjustment,
    q3: VatSummary,
    now: VatSummary,
    accountant: User,
}

async fn case(relief: Relief) -> Case {
    let t = boot().await;
    let accountant = t.accountant("Zaid").await;
    let pgp = t.entity("PGP").await;
    t.summary(&pgp, supply(), VatSummaryStatus::Locked).await;
    let q3 = t.summary(&pgp, first_day(), VatSummaryStatus::Open).await;
    let now = t.summary(&pgp, today(), VatSummaryStatus::Open).await;
    let (invoice, relief_row) = t.written_off(None, supply(), Some(day(2026, 7, 1))).await;
    assert_eq!(invoice.status, InvoiceStatus::WrittenOff);
    let c = Case {
        t,
        invoice,
        relief: relief_row,
        q3,
        now,
        accountant,
    };
    if relief != Relief::Pending {
        bad_debt_relief::run(c.t.state(), first_day())
            .await
            .expect("the rule runs");
        let (action, reason) = if relief == Relief::Included {
            ("include", "Meets the rules")
        } else {
            ("exclude", "Customer paid")
        };
        c.t.post_decision(&c.q3, &c.relief().await, action, &c.accountant, reason)
            .await
            .assert_ok();
    }
    c
}

impl Case {
    async fn invoice(&self) -> Invoice {
        self.t.reload(&self.invoice).await
    }

    async fn relief(&self) -> VatAdjustment {
        self.t.adjustment(self.relief.id).await
    }

    async fn recover(&self, amount: i64) {
        wl_kernel::invoicing::invoicer(self.t.state())
            .allocate(
                self.t.state(),
                InvoiceId(self.invoice.id),
                payment(),
                amount,
            )
            .await
            .expect("the recovery runs");
    }

    async fn reverse(&self, amount: i64, key: &str) -> Result<Reopened> {
        let request = Unallocate::new(
            InvoiceId(self.invoice.id),
            payment(),
            amount,
            key,
            ReopenReason::PaymentReversed,
        )
        .reversal_reason("bank_return");
        wl_kernel::invoicing::invoicer(self.t.state())
            .unallocate(self.t.state(), request)
            .await
    }

    async fn recovery_adjustments(&self) -> Vec<VatAdjustment> {
        VatAdjustment::query()
            .where_(VatAdjustment::KIND.eq(AdjustmentKind::BadDebtRecovery))
            .where_(VatAdjustment::SOURCE_ID.eq(self.invoice.id))
            .order_by(VatAdjustment::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }

    async fn raised_reliefs(&self) -> Vec<VatAdjustment> {
        VatAdjustment::query()
            .where_(VatAdjustment::KIND.eq(AdjustmentKind::BadDebtRelief))
            .where_(VatAdjustment::SOURCE_ID.eq(self.invoice.id))
            .where_(VatAdjustment::REVERSES_RECOVERY_ID.is_not_null())
            .order_by(VatAdjustment::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }

    async fn recoveries(&self) -> Vec<Recovery> {
        Recovery::of_invoice(self.t.db(), self.invoice.id)
            .await
            .unwrap()
    }

    async fn allocations(&self) -> Vec<Allocation> {
        Allocation::query()
            .where_(Allocation::INVOICE_ID.eq(self.invoice.id))
            .order_by(Allocation::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }

    async fn logs(&self) -> Vec<Unallocation> {
        Unallocation::query()
            .where_(Unallocation::INVOICE_ID.eq(self.invoice.id))
            .order_by(Unallocation::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }
}

/// What a reversal must never write.
fn frozen(invoice: &Invoice) -> impl PartialEq + std::fmt::Debug {
    (
        invoice.net_total,
        invoice.vat_total,
        invoice.gross_total,
        invoice.write_off_amount,
        invoice.write_off_date,
        invoice.write_off_notice_on,
        invoice.write_off_reason_code.clone(),
        invoice.write_off_approval_id,
    )
}

/// AC5: written off for AED 1,050.00, recovered in full (relief and
/// recovery adjustment both included), then the recovery payment reversed.
#[renox::test]
async fn a_reversed_recovery_in_full_puts_the_invoice_back_to_written_off() {
    let c = case(Relief::Included).await;
    let before = c.invoice().await;
    c.recover(105_000).await;
    let paid = c.invoice().await;
    assert_eq!(paid.status, InvoiceStatus::Paid);
    let adjustment = c.recovery_adjustments().await.remove(0);
    assert_eq!(adjustment.status, AdjustmentStatus::Proposed);
    assert_eq!(adjustment.summary_id, Some(c.now.id));
    c.t.post_decision(&c.now, &adjustment, "include", &c.accountant, "ok")
        .await
        .assert_ok();

    let reopened = c.reverse(105_000, "payments.allocation:41").await.unwrap();
    assert!(!reopened.skipped);
    assert_eq!(reopened.new_status, InvoiceStatus::WrittenOff);
    assert_eq!(reopened.balance, 105_000);

    let after = c.invoice().await;
    assert_eq!(after.status, InvoiceStatus::WrittenOff);
    assert_eq!(after.write_off_recovered_amount, 0);
    assert_eq!((after.balance, after.allocated_total), (105_000, 0));
    assert_eq!(frozen(&after), frozen(&before));

    // The allocation and the recovery are marked, never deleted.
    let allocations = c.allocations().await;
    assert_eq!(allocations.len(), 1);
    assert!(allocations[0].removed_at.is_some());
    assert_eq!(
        allocations[0].removed_reason.as_deref(),
        Some("recovery_reversed")
    );
    let recoveries = c.recoveries().await;
    assert_eq!(recoveries.len(), 1);
    assert!(recoveries[0].is_reversed());
    assert_eq!(
        recoveries[0].reversed_key.as_deref(),
        Some("payments.allocation:41")
    );
    assert_eq!(
        recoveries[0].reversed_effect.as_deref(),
        Some("relief_created")
    );

    // The VAT was included, so a new relief for the share is proposed on
    // the open summary. The old rows are not touched.
    let raised = c.raised_reliefs().await;
    assert_eq!(raised.len(), 1);
    assert_eq!(raised[0].amount, 5_000);
    assert_eq!(raised[0].status, AdjustmentStatus::Proposed);
    assert_eq!(raised[0].summary_id, Some(c.now.id));
    assert_eq!(raised[0].reverses_recovery_id, Some(recoveries[0].id));
    assert_eq!(c.relief().await.status, AdjustmentStatus::Included);
    assert_eq!(
        c.t.adjustment(adjustment.id).await.status,
        AdjustmentStatus::Included
    );

    // One log row, one event, the notices.
    let logs = c.logs().await;
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].outcome, UnallocationOutcome::Done);
    assert_eq!(logs[0].reason, ReopenReason::RecoveryReversed);
    assert_eq!(logs[0].transition.as_deref(), Some("unallocate_recovery"));
    let events = c.t.invoice_reopened.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, ReopenReason::RecoveryReversed);
    assert_eq!(events[0].new_status, InvoiceStatus::WrittenOff);
    assert_eq!(events[0].amount, 105_000);
    let notices = c.t.notices.of_type("invoicing.recovery_reversed");
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].data["amount"], "AED 1,050.00");
    assert_eq!(notices[0].data["recovered_total"], "AED 0.00");
    assert!(
        notices[0].data["vat_effect"]
            .as_str()
            .unwrap()
            .contains("New bad-debt relief")
    );
    let told = c.t.notices.of_type("invoicing.invoice_reopened");
    assert!(!told.is_empty());
    assert!(
        told[0].data["reason"]
            .as_str()
            .unwrap()
            .contains("recovery reversed")
    );
}

/// AC7 for a recovery: the same key twice changes nothing.
#[renox::test]
async fn the_same_key_twice_reverses_a_recovery_once() {
    let c = case(Relief::Included).await;
    c.recover(105_000).await;
    c.reverse(105_000, "k:1").await.unwrap();
    let again = c.reverse(105_000, "k:1").await.unwrap();
    assert!(again.skipped);
    assert_eq!(c.logs().await.len(), 1);
    assert_eq!(c.t.invoice_reopened.all().len(), 1);
    assert_eq!(c.t.notices.of_type("invoicing.recovery_reversed").len(), 1);
    assert!(c.raised_reliefs().await.is_empty());
    assert_eq!(c.invoice().await.write_off_recovered_amount, 0);
}

/// AC5: with no `open` summary the new relief waits as `pending`; the next
/// `FIN.RUL.14` run after one exists proposes it, without the rule's other
/// conditions.
#[renox::test]
async fn the_new_relief_waits_for_an_open_summary() {
    let c = case(Relief::Included).await;
    c.recover(105_000).await;
    let adjustment = c.recovery_adjustments().await.remove(0);
    c.t.post_decision(&c.now, &adjustment, "include", &c.accountant, "ok")
        .await
        .assert_ok();
    // Today's summary is closed for good.
    VatSummary::query()
        .where_(VatSummary::ID.eq(c.now.id))
        .update(c.t.db(), &[("status", &"locked")])
        .await
        .unwrap();
    c.reverse(105_000, "k:1").await.unwrap();
    let raised = c.raised_reliefs().await;
    assert_eq!(raised.len(), 1);
    assert_eq!(raised[0].status, AdjustmentStatus::Pending);
    assert_eq!(raised[0].summary_id, None);

    let ran = bad_debt_relief::run(c.t.state(), today()).await.unwrap();
    assert_eq!((ran.proposed, ran.waiting_for_open_summary), (0, 1));

    VatSummary::query()
        .where_(VatSummary::ID.eq(c.now.id))
        .update(c.t.db(), &[("status", &"open")])
        .await
        .unwrap();
    let ran = bad_debt_relief::run(c.t.state(), today()).await.unwrap();
    assert_eq!(ran.proposed, 1);
    let proposed = c.t.adjustment(raised[0].id).await;
    assert_eq!(proposed.status, AdjustmentStatus::Proposed);
    assert_eq!(proposed.summary_id, Some(c.now.id));
    assert_eq!(proposed.amount, 5_000);
}

/// AC6: a part recovery while the relief is still pending reduced it; the
/// reversal gives the share back.
#[renox::test]
async fn a_reversed_part_recovery_restores_a_pending_relief() {
    let c = case(Relief::Pending).await;
    c.recover(52_500).await;
    assert_eq!(c.relief().await.amount, 2_500);
    c.reverse(52_500, "k:1").await.unwrap();
    let after = c.invoice().await;
    assert_eq!(after.status, InvoiceStatus::WrittenOff);
    assert_eq!(
        (after.balance, after.write_off_recovered_amount),
        (105_000, 0)
    );
    let relief = c.relief().await;
    assert_eq!(
        (relief.status, relief.amount),
        (AdjustmentStatus::Pending, 5_000)
    );
    assert!(c.raised_reliefs().await.is_empty());
    assert_eq!(
        c.recoveries().await[0].reversed_effect.as_deref(),
        Some("relief_restored")
    );
}

/// AC6: a relief excluded "recovered before claim" is pending again at the
/// restored amount.
#[renox::test]
async fn a_reversed_full_recovery_reinstates_the_excluded_relief() {
    let c = case(Relief::Pending).await;
    c.recover(105_000).await;
    let excluded = c.relief().await;
    assert_eq!(
        (excluded.status, excluded.amount),
        (AdjustmentStatus::Excluded, 0)
    );
    c.reverse(105_000, "k:1").await.unwrap();
    let relief = c.relief().await;
    assert_eq!(
        (relief.status, relief.amount),
        (AdjustmentStatus::Pending, 5_000)
    );
    assert_eq!(relief.reason, None);
    assert_eq!(c.invoice().await.status, InvoiceStatus::WrittenOff);
    assert_eq!(
        c.recoveries().await[0].reversed_effect.as_deref(),
        Some("relief_reinstated")
    );
}

/// AC6: an open recovery adjustment is reduced; at 0.00 it is excluded.
#[renox::test]
async fn an_open_recovery_adjustment_is_reduced_to_zero_and_excluded() {
    let c = case(Relief::Included).await;
    c.recover(52_500).await;
    let adjustment = c.recovery_adjustments().await.remove(0);
    assert_eq!(
        (adjustment.status, adjustment.amount),
        (AdjustmentStatus::Proposed, 2_500)
    );
    c.reverse(52_500, "k:1").await.unwrap();
    let after = c.t.adjustment(adjustment.id).await;
    assert_eq!(
        (after.status, after.amount),
        (AdjustmentStatus::Excluded, 0)
    );
    assert_eq!(after.reason.as_deref(), Some("recovery reversed"));
    assert!(c.raised_reliefs().await.is_empty());
    assert_eq!(c.invoice().await.status, InvoiceStatus::WrittenOff);
    assert_eq!(
        c.recoveries().await[0].reversed_effect.as_deref(),
        Some("adjustment_excluded")
    );
}

/// A relief that a person excluded stays excluded.
#[renox::test]
async fn a_relief_excluded_by_a_person_is_left_alone() {
    let c = case(Relief::Excluded).await;
    c.recover(52_500).await;
    c.reverse(52_500, "k:1").await.unwrap();
    let relief = c.relief().await;
    assert_eq!(
        (relief.status, relief.amount),
        (AdjustmentStatus::Excluded, 5_000)
    );
    assert!(c.raised_reliefs().await.is_empty());
    assert_eq!(
        c.recoveries().await[0].reversed_effect.as_deref(),
        Some("none")
    );
}

/// Two recoveries: the one reversed is the one that matches, and the
/// other stays.
#[renox::test]
async fn only_the_matching_recovery_is_reversed() {
    let c = case(Relief::Pending).await;
    c.recover(33_333).await;
    c.recover(71_667).await;
    assert_eq!(c.invoice().await.status, InvoiceStatus::Paid);
    c.reverse(33_333, "k:1").await.unwrap();
    let after = c.invoice().await;
    assert_eq!(after.status, InvoiceStatus::WrittenOff);
    assert_eq!(after.write_off_recovered_amount, 71_667);
    assert_eq!(after.balance, 33_333);
    let recoveries = c.recoveries().await;
    assert!(recoveries[0].is_reversed());
    assert!(!recoveries[1].is_reversed());
    let allocations = c.allocations().await;
    assert!(allocations[0].removed_at.is_some());
    assert!(allocations[1].removed_at.is_none());
}

/// A reversal that is not a recovery's money is refused and logged; so is
/// one for an amount that is not the recovery's.
#[renox::test]
async fn money_that_is_not_a_recovery_is_refused() {
    let c = case(Relief::Included).await;
    c.recover(52_500).await;
    let before = c.invoice().await;
    // A part of the recovery.
    assert!(c.reverse(10_000, "k:part").await.is_err());
    // Another payment.
    let other = Unallocate::new(
        InvoiceId(c.invoice.id),
        Subject::new("payments.payment", 99),
        52_500,
        "k:other",
        ReopenReason::PaymentReversed,
    );
    assert!(
        wl_kernel::invoicing::invoicer(c.t.state())
            .unallocate(c.t.state(), other)
            .await
            .is_err()
    );
    let after = c.invoice().await;
    assert_eq!(after.status, InvoiceStatus::WrittenOff);
    assert_eq!(
        (after.balance, after.write_off_recovered_amount),
        (before.balance, before.write_off_recovered_amount)
    );
    assert!(!c.recoveries().await[0].is_reversed());
    let logs = c.logs().await;
    assert_eq!(logs.len(), 2);
    assert!(
        logs.iter()
            .all(|l| l.outcome == UnallocationOutcome::Failed)
    );
    assert!(c.t.invoice_reopened.all().is_empty());
    assert!(
        c.t.notices
            .of_type("invoicing.recovery_reversed")
            .is_empty()
    );
}

/// A recovery reversed twice under two keys is not undone twice.
#[renox::test]
async fn a_recovery_is_reversed_only_once() {
    let c = case(Relief::Pending).await;
    c.recover(52_500).await;
    c.reverse(52_500, "k:1").await.unwrap();
    assert!(c.reverse(52_500, "k:2").await.is_err());
    assert_eq!(c.invoice().await.write_off_recovered_amount, 0);
    assert_eq!(c.invoice().await.balance, 105_000);
    assert_eq!(c.relief().await.amount, 5_000);
}

/// A recovered, written-off invoice can be recovered again after a
/// reversal.
#[renox::test]
async fn the_invoice_can_be_recovered_again_after_a_reversal() {
    let c = case(Relief::Pending).await;
    c.recover(105_000).await;
    c.reverse(105_000, "k:1").await.unwrap();
    c.recover(105_000).await;
    let paid = c.invoice().await;
    assert_eq!(paid.status, InvoiceStatus::Paid);
    assert_eq!(paid.write_off_recovered_amount, 105_000);
    assert_eq!(c.recoveries().await.len(), 2);
    assert_eq!(c.relief().await.status, AdjustmentStatus::Excluded);
}
