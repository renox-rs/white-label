//! Recovering a written-off invoice (wl-invoicing §4 `recover`): the
//! acceptance checks AC1-AC3 and AC5-AC8 of #66. AC4 (payments end to end)
//! is in `host/tests/recover.rs`. A test cannot build an `AuthUser`, so the
//! money comes in through `Invoicer::allocate`, as `wl-payments` calls it.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_invoicing::bad_debt_relief::{self, ReliefRun};
use wl_invoicing::vat_summary::{VatSummary, VatSummaryStatus};
use wl_invoicing::{
    AdjustmentKind, AdjustmentStatus, Allocation, Invoice, InvoiceLine, Recovery, VatAdjustment,
};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, VatCategory};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{LegalEntity, Subject};

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

/// How the bad-debt relief of the written-off invoice (AED 50.00 on AED
/// 1,050.00) stands when the money comes in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Relief {
    Pending,
    Proposed,
    Included,
    Excluded,
}

struct Case {
    t: Booted,
    pgp: LegalEntity,
    invoice: Invoice,
    relief: VatAdjustment,
    q3: VatSummary,
    /// The summary of today's period, when the case made one.
    now: Option<VatSummary>,
    accountant: User,
}

/// The invoice (supply 2026-03-15, notice 2026-07-01) written off for AED
/// 1,050.00. Q1 is locked and Q3 open, so the rule can propose the relief
/// on Q3; `relief` says how far it went. `open_now` makes an `open` summary
/// for today's period too.
async fn case(relief: Relief, open_now: bool) -> Case {
    let t = boot().await;
    let accountant = t.accountant("Zaid").await;
    let pgp = t.entity("PGP").await;
    t.summary(&pgp, supply(), VatSummaryStatus::Locked).await;
    let q3 = t.summary(&pgp, first_day(), VatSummaryStatus::Open).await;
    let now = if open_now {
        Some(t.summary(&pgp, today(), VatSummaryStatus::Open).await)
    } else {
        None
    };
    let (invoice, relief_row) = t.written_off(None, supply(), Some(day(2026, 7, 1))).await;
    assert_eq!(invoice.legal_entity_id, Some(pgp.id));
    assert_eq!(invoice.status, InvoiceStatus::WrittenOff);
    assert_eq!((invoice.balance, relief_row.amount), (105_000, 5_000));
    let c = Case {
        t,
        pgp,
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
        assert_eq!(c.relief().await.status, AdjustmentStatus::Proposed);
    }
    let decision = match relief {
        Relief::Included => Some(("include", "Meets the rules")),
        Relief::Excluded => Some(("exclude", "Customer paid")),
        _ => None,
    };
    if let Some((action, reason)) = decision {
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

    /// Allocates `amount` fils of payment 7 through the contract.
    async fn recover(&self, amount: i64) -> Result<()> {
        wl_kernel::invoicing::invoicer(self.t.state())
            .allocate(
                self.t.state(),
                InvoiceId(self.invoice.id),
                Subject::new("payments.payment", 7),
                amount,
            )
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

    async fn allocations(&self) -> Vec<Allocation> {
        Allocation::query()
            .where_(Allocation::INVOICE_ID.eq(self.invoice.id))
            .get(self.t.db())
            .await
            .unwrap()
    }

    async fn recoveries(&self) -> Vec<Recovery> {
        Recovery::of_invoice(self.t.db(), self.invoice.id)
            .await
            .unwrap()
    }

    async fn recover_history(&self) -> Vec<String> {
        self.t
            .history(&self.invoice)
            .await
            .into_iter()
            .filter(|name| name.starts_with("recover"))
            .collect()
    }

    async fn line_count(&self) -> usize {
        InvoiceLine::query()
            .where_(InvoiceLine::INVOICE_ID.eq(self.invoice.id))
            .count(self.t.db())
            .await
            .unwrap() as usize
    }
}

/// What a recovery must never write.
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

fn refused(result: Result<()>) {
    assert!(
        matches!(result, Err(Error::Status(code, _)) if code == StatusCode::UNPROCESSABLE_ENTITY),
        "{result:?}"
    );
}

/// AC1 and AC2: a part, then the rest, of a claimed relief.
#[renox::test]
async fn a_claimed_relief_is_recovered_in_two_parts() {
    let c = case(Relief::Included, true).await;
    let before = c.invoice().await;
    let lines_before = c.line_count().await;
    assert_eq!(c.relief().await.status, AdjustmentStatus::Included);
    let open_now = c.now.as_ref().unwrap();

    // AC1: AED 525.00.
    c.recover(52_500)
        .await
        .expect("the first part is recovered");
    let part = c.invoice().await;
    assert_eq!(part.status, InvoiceStatus::WrittenOff);
    assert_eq!(part.write_off_recovered_amount, 52_500);
    assert_eq!(part.balance, 52_500);
    assert_eq!(part.allocated_total, 52_500);
    assert_eq!(c.allocations().await.len(), 1);
    assert_eq!(frozen(&part), frozen(&before));
    assert_eq!(c.line_count().await, lines_before);
    let adjustments = c.recovery_adjustments().await;
    assert_eq!(adjustments.len(), 1);
    assert_eq!(adjustments[0].amount, 2_500);
    assert_eq!(adjustments[0].status, AdjustmentStatus::Proposed);
    assert_eq!(adjustments[0].summary_id, Some(open_now.id));
    assert_eq!(adjustments[0].legal_entity_id, Some(c.pgp.id));
    assert_eq!(adjustments[0].source_id, c.invoice.id);
    // The claimed relief itself is not touched.
    let relief = c.relief().await;
    assert_eq!(
        (relief.status, relief.amount),
        (AdjustmentStatus::Included, 5_000)
    );

    let notices = c.t.notices.of_type("invoicing.bad_debt_recovered");
    assert_eq!(notices.len(), 1);
    assert!(matches!(
        notices[0].recipients.as_slice(),
        [NotifyRecipient::User { id }] if *id == c.accountant.id
    ));
    assert_eq!(notices[0].data["amount"], "AED 525.00");
    assert_eq!(notices[0].data["recovered_total"], "AED 525.00");
    assert_eq!(notices[0].data["relief_claimed"], "yes");
    assert_eq!(
        notices[0].data["link"],
        format!("/invoices/{}", c.invoice.id).as_str()
    );
    let events = c.t.recovered.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].invoice, InvoiceId(c.invoice.id));
    assert_eq!(events[0].source, Subject::new("payments.payment", 7));
    assert_eq!(events[0].amount, 52_500);
    assert!(events[0].relief_claimed);
    assert_eq!(events[0].recovered_on, today());
    assert!(c.t.paid.all().is_empty());
    assert_eq!(c.recover_history().await, ["recover"]);

    // AC2: the rest.
    c.recover(52_500).await.expect("the rest is recovered");
    let paid = c.invoice().await;
    assert_eq!(paid.status, InvoiceStatus::Paid);
    assert_eq!(paid.write_off_recovered_amount, 105_000);
    assert_eq!(paid.balance, 0);
    assert_eq!(frozen(&paid), frozen(&before));
    assert_eq!(c.t.paid.all().len(), 1);
    assert_eq!(c.t.paid.all()[0].invoice, InvoiceId(c.invoice.id));
    let adjustments = c.recovery_adjustments().await;
    assert_eq!(adjustments.len(), 2);
    assert_eq!(adjustments[1].amount, 2_500);
    assert_eq!(adjustments[1].status, AdjustmentStatus::Proposed);
    assert_eq!(c.recover_history().await, ["recover", "recover_in_full"]);
    assert_eq!(c.t.recovered.all().len(), 2);
    assert_eq!(c.t.notices.of_type("invoicing.bad_debt_recovered").len(), 2);
    let rows = c.recoveries().await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].recovered_total, 105_000);
    assert_eq!(rows[1].vat_share, 2_500);
    assert!(rows.iter().all(|r| r.relief_claimed));
    assert_eq!(c.allocations().await.len(), 2);
}

/// AC3: an unclaimed relief is reduced by the VAT share, and excluded once
/// the write-off is recovered. The same for `pending` and `proposed`.
#[renox::test]
async fn an_unclaimed_relief_is_reduced_then_excluded() {
    for stage in [Relief::Pending, Relief::Proposed] {
        let c = case(stage, true).await;
        let expected = if stage == Relief::Pending {
            AdjustmentStatus::Pending
        } else {
            AdjustmentStatus::Proposed
        };
        c.recover(52_500).await.unwrap();
        assert!(c.recovery_adjustments().await.is_empty());
        let relief = c.relief().await;
        assert_eq!((relief.status, relief.amount), (expected, 2_500));
        assert!(!c.t.recovered.all()[0].relief_claimed);
        assert_eq!(c.invoice().await.status, InvoiceStatus::WrittenOff);
        let notices = c.t.notices.of_type("invoicing.bad_debt_recovered");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].data["relief_claimed"], "no");

        c.recover(52_500).await.unwrap();
        let relief = c.relief().await;
        assert_eq!(
            (relief.status, relief.amount),
            (AdjustmentStatus::Excluded, 0)
        );
        assert_eq!(relief.reason.as_deref(), Some("recovered before claim"));
        assert!(c.recovery_adjustments().await.is_empty());
        assert_eq!(c.invoice().await.status, InvoiceStatus::Paid);
    }
}

/// AC5: an excluded relief is left alone.
#[renox::test]
async fn an_excluded_relief_is_left_alone() {
    let c = case(Relief::Excluded, true).await;
    assert_eq!(c.relief().await.status, AdjustmentStatus::Excluded);
    c.recover(52_500).await.unwrap();
    assert!(c.recovery_adjustments().await.is_empty());
    let after = c.relief().await;
    assert_eq!(
        (after.status, after.amount),
        (AdjustmentStatus::Excluded, 5_000)
    );
    assert!(!c.t.recovered.all()[0].relief_claimed);
    let rows = c.recoveries().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].recovery_adjustment_id, None);
}

/// AC5: with no `open` summary the recovery adjustment waits as `pending`,
/// and the next `FIN.RUL.14` run after one exists proposes it. A pending
/// relief is not proposed by that path without its own conditions.
#[renox::test]
async fn a_recovery_waits_for_an_open_summary() {
    let c = case(Relief::Included, false).await;
    c.recover(52_500).await.unwrap();
    let adjustments = c.recovery_adjustments().await;
    assert_eq!(adjustments.len(), 1);
    assert_eq!(adjustments[0].status, AdjustmentStatus::Pending);
    assert_eq!(adjustments[0].summary_id, None);
    let ran = bad_debt_relief::run(c.t.state(), today()).await.unwrap();
    assert_eq!((ran.proposed, ran.recoveries_proposed), (0, 0));
    assert_eq!(ran.recoveries_waiting, 1);

    let now = c.t.summary(&c.pgp, today(), VatSummaryStatus::Open).await;
    let ran = bad_debt_relief::run(c.t.state(), today()).await.unwrap();
    assert_eq!(
        ran,
        ReliefRun {
            recoveries_proposed: 1,
            ..Default::default()
        }
    );
    let adjustment = c.t.adjustment(adjustments[0].id).await;
    assert_eq!(adjustment.status, AdjustmentStatus::Proposed);
    assert_eq!(adjustment.summary_id, Some(now.id));
    assert_eq!(adjustment.amount, 2_500);
}

/// AC5: a pending relief is not proposed by the recovery path; the rule's
/// own conditions (here: the supply period is not locked) still decide.
#[renox::test]
async fn a_pending_relief_keeps_its_own_conditions() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    t.summary(&pgp, today(), VatSummaryStatus::Open).await;
    let (invoice, relief) = t.written_off(None, supply(), Some(day(2026, 7, 1))).await;
    t.pay(&invoice, 7, 52_500).await;
    let ran = bad_debt_relief::run(t.state(), today()).await.unwrap();
    assert_eq!(ran.proposed, 0);
    let relief = t.adjustment(relief.id).await;
    assert_eq!(
        (relief.status, relief.amount),
        (AdjustmentStatus::Pending, 2_500)
    );
}

/// AC6: the shares of 333.33 then 716.67 on a claimed relief of 50.00.
#[renox::test]
async fn part_recoveries_share_the_vat_by_the_amounts_so_far() {
    let c = case(Relief::Included, true).await;
    c.recover(33_333).await.unwrap();
    c.recover(71_667).await.unwrap();
    let shares: Vec<i64> = c
        .recovery_adjustments()
        .await
        .iter()
        .map(|a| a.amount)
        .collect();
    assert_eq!(shares, [1_587, 3_413]);
    assert_eq!(shares.iter().sum::<i64>(), 5_000);
    assert_eq!(c.invoice().await.status, InvoiceStatus::Paid);
}

/// AC7: `recover` is refused outside `written_off`, for an amount of 0 or
/// less and for more than is left; nothing is written. It is offered to no
/// one, and an open invoice is still allocated.
#[renox::test]
async fn recover_is_refused_when_it_does_not_apply() {
    let c = case(Relief::Included, true).await;
    let source = Subject::new("payments.payment", 7);
    for amount in [0, -100, 105_001] {
        refused(c.recover(amount).await);
    }
    let rest = c.invoice().await;
    assert_eq!(rest.status, InvoiceStatus::WrittenOff);
    assert_eq!(
        (rest.balance, rest.write_off_recovered_amount),
        (105_000, 0)
    );
    assert!(c.allocations().await.is_empty());
    assert!(c.recoveries().await.is_empty());
    assert!(c.recovery_adjustments().await.is_empty());
    assert!(c.t.recovered.all().is_empty());
    assert!(
        c.t.notices
            .of_type("invoicing.bad_debt_recovered")
            .is_empty()
    );

    // After a part, the refusal says what is left.
    c.recover(52_500).await.unwrap();
    match c.recover(52_501).await {
        Err(Error::Status(_, message)) => {
            assert!(
                message.contains("Only AED 525.00 is left to recover"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(c.allocations().await.len(), 1);

    // Offered to no user.
    let admin = c.t.admin_a().await;
    for user in [&admin, &c.accountant] {
        let offered = c.t.available(&c.invoice, user).await;
        assert!(!offered.contains("recover"), "{offered}");
    }

    // Not from any other state: the service says `NotFrom`.
    let clerk = c.t.user_b().await;
    let issued =
        c.t.issued(&[(100_000, VatCategory::Standard)], None, &clerk)
            .await;
    let result = wl_invoicing::recover::recover(c.t.state(), &issued, &source, 1_000, None).await;
    assert!(
        matches!(result, Err(TransitionError::NotFrom(_))),
        "{result:?}"
    );
    let issued = c.t.reload(&issued).await;
    assert_eq!(
        (issued.status, issued.balance),
        (InvoiceStatus::Issued, 105_000)
    );

    // An open invoice still allocates.
    c.t.pay(&issued, 8, 5_000).await;
    let issued = c.t.reload(&issued).await;
    assert_eq!(
        (issued.status, issued.balance),
        (InvoiceStatus::PartiallyPaid, 100_000)
    );
    assert_eq!(
        c.t.history(&issued).await.last().map(String::as_str),
        Some("allocate")
    );
    assert!(
        Recovery::of_invoice(c.t.db(), issued.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC8: the invoice page shows the recovered amount, what is left, and the
/// recovery with its VAT effect; the VAT summary page lists the adjustment.
#[renox::test]
async fn the_pages_show_the_recovery() {
    let c = case(Relief::Included, true).await;
    c.recover(52_500).await.unwrap();
    let now = c.now.as_ref().unwrap();

    c.t.app.acting_as(&c.accountant);
    c.t.app
        .get(&format!("/invoices/{}", c.invoice.id))
        .await
        .assert_ok()
        .assert_see("Recovered so far")
        .assert_see("Left to recover")
        .assert_see("AED 525.00")
        .assert_see("AED 25.00")
        .assert_see("/payments/7");
    c.t.app
        .get(&format!("/vat-summaries/{}", now.id))
        .await
        .assert_ok()
        .assert_see("Bad-debt recovery")
        .assert_see(&c.invoice.number.clone().unwrap())
        .assert_see("AED 25.00");
}
