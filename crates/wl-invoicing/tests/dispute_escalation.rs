//! Escalating an open dispute (wl-invoicing §5.10, `FIN.RUL.15`, #68 part 4):
//! the steps, no repeat, the end at resolution, the settings, the recipients
//! and a failing notifier. Dates are fixed through `escalation::run`.

mod support;

use renox::auth::permissions::{self, Scope};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::dispute::{self, Dispute};
use wl_invoicing::dispute_resolve::{self, ResolveInput};
use wl_invoicing::permissions as perm;
use wl_invoicing::{EscalationLog, Invoice, WlInvoicing, escalation};
use wl_kernel::approvals::ApprovalPolicy;
use wl_kernel::invoicing::{DisputeBy, DisputeRequest, InvoiceId, VatCategory};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{LegalEntity, RuleRun, RuleRunSpec, RuleRunStatus, RunOutcome, run_rule};

use support::{Booted, boot, boot_with};

const LINE: [(i64, VatCategory); 1] = [(100_000, VatCategory::Standard)];
const ROLE: &str = "finance_manager";

fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

/// A user `name` holding `finance_manager`, for `entity` or globally.
async fn manager(t: &Booted, name: &str, entity: Option<&LegalEntity>) -> User {
    let user = User::register(
        t.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "password123",
    )
    .await
    .unwrap();
    permissions::define_role(t.db(), ROLE, &["invoices.view", "receivables.view"])
        .await
        .unwrap();
    match entity {
        Some(entity) => user
            .assign_role_in(t.db(), ROLE, &Scope::of_id::<LegalEntity>(entity.id))
            .await
            .unwrap(),
        None => user.assign_role(t.db(), ROLE).await.unwrap(),
    }
    user
}

/// The entity of `invoice`.
async fn entity_of(t: &Booted, invoice: &Invoice) -> LegalEntity {
    LegalEntity::find(t.db(), invoice.legal_entity_id.expect("an entity"))
        .await
        .unwrap()
        .unwrap()
}

/// Raises a dispute on `invoice` and says it was raised on `on`, 09:00 UTC.
async fn raise(t: &Booted, invoice: &Invoice, by: &User, on: NaiveDate) -> i64 {
    let request = DisputeRequest::new(
        InvoiceId(invoice.id),
        "wrong hours",
        DisputeBy::User { id: by.id },
    );
    let id = dispute::raise(t.state(), request.amount(20_000))
        .await
        .unwrap();
    let mut row = Dispute::find(t.db(), id).await.unwrap().unwrap();
    row.raised_at = on.and_hms_opt(9, 0, 0).unwrap().and_utc();
    row.save(t.db()).await.unwrap();
    id
}

/// Resolves the open dispute of `invoice`, `credited` or `upheld`.
async fn resolve(t: &Booted, invoice: &Invoice, credited: bool) {
    let by = t
        .helper("Cleo", &[perm::VIEW, perm::DISPUTE, perm::CREDIT])
        .await;
    let input = if credited {
        // A credit above the limit waits for an approval; this one does not.
        ApprovalPolicy::query()
            .where_(ApprovalPolicy::ACTION.eq("invoicing.dispute_credit"))
            .update(t.db(), &[("threshold", &10_000_000_i64)])
            .await
            .unwrap();
        ResolveInput::credited("Agreed", 20_000)
    } else {
        ResolveInput::upheld("Hours verified")
    };
    let done =
        dispute_resolve::resolve(t.state(), invoice.id, input, DisputeBy::User { id: by.id })
            .await
            .unwrap();
    assert!(done.is_resolved());
}

fn told_since(t: &Booted, before: usize) -> Vec<i64> {
    t.notices.of_type(escalation::ESCALATED)[before..]
        .iter()
        .flat_map(|request| request.recipients.iter())
        .map(|recipient| match recipient {
            NotifyRecipient::User { id } => *id,
            other => panic!("not a user: {other:?}"),
        })
        .collect()
}

/// Runs the escalation on `on` and returns the users the new notices went
/// to, sorted.
async fn run(t: &Booted, on: NaiveDate) -> Vec<i64> {
    let before = t.notices.of_type(escalation::ESCALATED).len();
    let sent = escalation::run(t.state(), on).await.unwrap();
    let users = sorted(told_since(t, before));
    assert_eq!(sent, users.len(), "the count is the notices sent");
    users
}

fn sorted(mut ids: Vec<i64>) -> Vec<i64> {
    ids.sort_unstable();
    ids
}

/// A, an invoice issued by A, and F, finance manager of its entity.
async fn world(t: &Booted) -> (User, User, Invoice) {
    let a = t.user_a().await;
    let invoice = t.issued(&LINE, Some(t.day(10)), &a).await;
    let entity = entity_of(t, &invoice).await;
    let f = manager(t, "Farah", Some(&entity)).await;
    (a, f, invoice)
}

/// AC1: nothing on day 13, the owner on day 14, the owner and F on day 28,
/// and both again on day 42; the notice names the invoice and the step.
#[renox::test]
async fn each_whole_period_tells_the_owner_and_from_the_second_step_the_manager() {
    let t = boot().await;
    let (a, f, invoice) = world(&t).await;
    raise(&t, &invoice, &a, day(11, 1)).await;

    assert!(run(&t, day(11, 14)).await.is_empty());
    assert_eq!(run(&t, day(11, 15)).await, [a.id]);
    assert_eq!(run(&t, day(11, 29)).await, sorted(vec![a.id, f.id]));
    assert_eq!(run(&t, day(12, 13)).await, sorted(vec![a.id, f.id]));

    let notices = t.notices.of_type(escalation::ESCALATED);
    assert_eq!(notices.len(), 5);
    assert_eq!(notices[0].subject, invoice.subject());
    assert_eq!(notices[0].data["step"], 1);
    assert_eq!(notices[0].data["days"], 14);
    assert_eq!(notices[0].data["reason"], "wrong hours");
    assert_eq!(notices[0].data["amount"], "AED 200.00");
    assert_eq!(notices[1].data["step"], 2);
    assert_eq!(notices[1].data["days"], 28);
    assert_eq!(notices[4].data["step"], 3);
    let manager_copy = notices[1..3]
        .iter()
        .find(|n| n.recipients == [NotifyRecipient::User { id: f.id }])
        .expect("a copy for F");
    assert_eq!(manager_copy.data["owner"], a.name.as_str());
}

/// AC2: the same day, or the next one, sends nothing new; the second run of
/// the occurrence is skipped and the log has one row per dispute, step and
/// user.
#[renox::test]
async fn a_step_is_never_sent_twice() {
    let t = boot().await;
    let (a, f, invoice) = world(&t).await;
    let id = raise(&t, &invoice, &a, day(11, 1)).await;

    assert_eq!(run(&t, day(11, 15)).await, [a.id]);
    assert!(run(&t, day(11, 15)).await.is_empty());
    assert!(run(&t, day(11, 16)).await.is_empty());
    assert_eq!(run(&t, day(11, 29)).await, sorted(vec![a.id, f.id]));
    assert!(run(&t, day(11, 29)).await.is_empty());

    let runs = RuleRun::query()
        .where_(RuleRun::RULE.eq(escalation::RULE))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|r| r.status == RuleRunStatus::Succeeded));
    let key = escalation::occurrence_key(id, 1);
    let spec = RuleRunSpec::new(escalation::RULE, invoice.subject(), key);
    let again = run_rule(t.state(), spec, |_| async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(again, RunOutcome::Skipped);

    assert_eq!(EscalationLog::told(t.db(), id, 1).await.unwrap(), [a.id]);
    assert_eq!(
        EscalationLog::told(t.db(), id, 2).await.unwrap(),
        sorted(vec![a.id, f.id])
    );
}

/// AC3: resolved (`upheld` or `credited`), nothing is sent any more.
#[renox::test]
async fn a_resolved_dispute_is_not_escalated() {
    for credited in [false, true] {
        let t = boot().await;
        let (a, _f, invoice) = world(&t).await;
        raise(&t, &invoice, &a, day(11, 1)).await;
        assert_eq!(run(&t, day(11, 15)).await, [a.id]);

        resolve(&t, &invoice, credited).await;
        assert!(run(&t, day(11, 29)).await.is_empty());
        assert!(run(&t, day(12, 13)).await.is_empty());
        assert!(run(&t, day(12, 27)).await.is_empty());
    }
}

/// AC4: with `.dispute_escalation_days(7)` the first notice is on day 7.
#[renox::test]
async fn the_period_is_a_setting() {
    let t = boot_with(WlInvoicing::new().dispute_escalation_days(7)).await;
    let (a, f, invoice) = world(&t).await;
    raise(&t, &invoice, &a, day(11, 1)).await;

    assert!(run(&t, day(11, 7)).await.is_empty());
    assert_eq!(run(&t, day(11, 8)).await, [a.id]);
    assert_eq!(run(&t, day(11, 15)).await, sorted(vec![a.id, f.id]));
}

/// AC5: a new dispute after a resolution counts from its own date and starts
/// again at the owner.
#[renox::test]
async fn a_new_dispute_starts_again() {
    let t = boot().await;
    let (a, f, invoice) = world(&t).await;
    raise(&t, &invoice, &a, day(11, 1)).await;
    resolve(&t, &invoice, false).await;
    raise(&t, &invoice, &a, day(11, 22)).await;

    assert!(run(&t, day(11, 29)).await.is_empty());
    assert_eq!(run(&t, day(12, 6)).await, [a.id]);
    assert_eq!(run(&t, day(12, 20)).await, sorted(vec![a.id, f.id]));
}

/// AC6: without a working owner, every issuer is told; a receivables holder
/// who is also F gets one notice per step.
#[renox::test]
async fn without_an_owner_the_receivables_are_told() {
    let t = boot().await;
    let (a, f, mut invoice) = world(&t).await;
    let b = t.clerk("Bilal").await;
    let entity = entity_of(&t, &invoice).await;
    // B is a receivables holder and a finance manager of the entity.
    b.assign_role_in(t.db(), ROLE, &Scope::of_id::<LegalEntity>(entity.id))
        .await
        .unwrap();
    // The owner no longer holds `invoices.issue`.
    let viewer = t.viewer().await;
    invoice.owner_id = Some(viewer.id);
    invoice.save(t.db()).await.unwrap();
    let id = raise(&t, &invoice, &a, day(11, 1)).await;

    let step1 = run(&t, day(11, 15)).await;
    assert_eq!(step1, sorted(vec![a.id, b.id]), "every issuer, not F");
    let step2 = run(&t, day(11, 29)).await;
    assert_eq!(step2, sorted(vec![a.id, b.id, f.id]), "B once, and F");
    assert_eq!(EscalationLog::told(t.db(), id, 2).await.unwrap(), step2);
    assert_eq!(
        t.notices
            .of_type(escalation::ESCALATED)
            .iter()
            .filter(|n| n.recipients == [NotifyRecipient::User { id: b.id }])
            .count(),
        2,
        "one per step"
    );
}

/// AC7: a manager of another entity is never told; a global one is.
#[renox::test]
async fn managers_of_other_entities_are_not_told() {
    let t = boot().await;
    let (a, f, invoice) = world(&t).await;
    let other = t.new_entity("OTHER").await;
    let there = manager(&t, "There", Some(&other)).await;
    let global = manager(&t, "Global", None).await;
    raise(&t, &invoice, &a, day(11, 1)).await;

    assert_eq!(run(&t, day(11, 15)).await, [a.id]);
    let told = run(&t, day(11, 29)).await;
    assert_eq!(told, sorted(vec![a.id, f.id, global.id]));
    assert!(!told.contains(&there.id));
}

/// AC8: the run changes neither the invoice, the dispute nor the ticket.
#[renox::test]
async fn escalating_changes_nothing_else() {
    let t = boot().await;
    let (a, _f, invoice) = world(&t).await;
    let id = raise(&t, &invoice, &a, day(11, 1)).await;
    let invoice_before = format!("{:?}", t.reload(&invoice).await);
    let dispute_before = format!("{:?}", Dispute::find(t.db(), id).await.unwrap().unwrap());
    let opened = t.tickets.opened().len();

    for on in [day(11, 15), day(11, 29), day(12, 13)] {
        run(&t, on).await;
    }

    assert_eq!(format!("{:?}", t.reload(&invoice).await), invoice_before);
    assert_eq!(
        format!("{:?}", Dispute::find(t.db(), id).await.unwrap().unwrap()),
        dispute_before
    );
    assert_eq!(t.tickets.opened().len(), opened);
    assert!(t.tickets.resolved().is_empty());
    assert_eq!(t.disputed.all().len(), 1);
    assert!(t.resolved.all().is_empty());
}

/// Makes the retry of a failed attempt due now.
async fn due_now(t: &Booted, invoice: &Invoice, dispute: i64, step: u32) {
    let key = escalation::occurrence_key(dispute, step);
    let mut run = RuleRun::find_occurrence(t.db(), escalation::RULE, &invoice.subject(), &key)
        .await
        .unwrap()
        .expect("a rule run");
    assert_eq!(run.status, RuleRunStatus::Retrying);
    run.next_attempt_at = None;
    run.save(t.db()).await.unwrap();
}

/// AC9: a failing recipient fails the attempt and leaves no log row; the
/// retry tells only those not yet told; another dispute is still sent.
#[renox::test]
async fn a_failing_notifier_is_retried_for_those_not_yet_told() {
    let t = boot().await;
    let (a, f, invoice) = world(&t).await;
    let b = t.clerk("Bilal").await;
    let other = t.issued(&LINE, Some(t.day(10)), &b).await;
    let first = raise(&t, &invoice, &a, day(11, 1)).await;
    let second = raise(&t, &other, &b, day(11, 1)).await;

    // Step 1: A cannot be told, B can.
    t.notices.fail_for(&[a.id]);
    assert_eq!(run(&t, day(11, 15)).await, [b.id]);
    assert!(
        EscalationLog::told(t.db(), first, 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        EscalationLog::told(t.db(), second, 1).await.unwrap(),
        [b.id]
    );
    due_now(&t, &invoice, first, 1).await;
    t.notices.fail_for(&[]);
    assert_eq!(run(&t, day(11, 15)).await, [a.id]);
    assert_eq!(EscalationLog::told(t.db(), first, 1).await.unwrap(), [a.id]);

    // Step 2: A is told, F is not; the retry tells only F.
    t.notices.fail_for(&[f.id]);
    let told = run(&t, day(11, 29)).await;
    assert!(told.contains(&a.id) && !told.contains(&f.id));
    assert_eq!(EscalationLog::told(t.db(), first, 2).await.unwrap(), [a.id]);
    due_now(&t, &invoice, first, 2).await;
    t.notices.fail_for(&[]);
    assert_eq!(run(&t, day(11, 29)).await, [f.id]);
    assert_eq!(
        EscalationLog::told(t.db(), first, 2).await.unwrap(),
        sorted(vec![a.id, f.id])
    );
}
