//! The daily cash check (wl-invoicing §6 `FIN.RUL.23` and `FIN.RUL.25`): the
//! acceptance checks of #93. Today is Wednesday 2027-01-13; week 1 is Monday
//! 2027-01-11 to Sunday 2027-01-17, so week 4 starts on 2027-02-01 and week 6
//! on 2027-02-15.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_invoicing::cash_alert::{self, BELOW_THRESHOLD, CashAlert};
use wl_invoicing::cash_balance::{self, EnterBalance};
use wl_invoicing::supplier_bill::{SupplierBill, SupplierBillStatus};
use wl_invoicing::task::{InvoicingTask, TaskKind, TaskStatus};
use wl_invoicing::{WlInvoicing, permissions as perm};
use wl_kernel::LegalEntity;
use wl_kernel::cash_forecast::{
    CashForecastSource, ForecastFuture, ForecastLine, register_cash_forecast_source,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::Direction;

use support::{BLUE_DUNES, Booted, boot_with};

const THRESHOLD: i64 = 2_000_000;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, m, d).unwrap()
}

/// A booted app with the module `module`, whose clock reads 2027-01-13.
async fn booted(module: WlInvoicing) -> Booted {
    let t = boot_with(module).await;
    t.travel_to("2027-01-13", 2, 0);
    t
}

/// The module with a threshold of AED 20,000 for PGP.
fn module() -> WlInvoicing {
    WlInvoicing::new().cash_threshold("PGP", THRESHOLD)
}

/// A user who holds `role` (made once).
async fn with_role(t: &Booted, name: &str, role: &str) -> User {
    let user = t.helper(name, &[perm::VIEW]).await;
    user.assign_role(t.db(), role).await.unwrap();
    user
}

/// The finance manager and the company admin.
async fn recipients(t: &Booted) -> Vec<i64> {
    let manager = with_role(t, "Fatima", "finance_manager").await;
    let admin = with_role(t, "Boss", "admin").await;
    let mut ids = vec![manager.id, admin.id];
    ids.sort_unstable();
    ids
}

/// An approved supplier bill of `entity` with `balance` fils open, due `due`.
async fn bill(
    t: &Booted,
    entity: &LegalEntity,
    number: &str,
    due: NaiveDate,
    balance: i64,
) -> SupplierBill {
    let mut bill = SupplierBill {
        party_id: BLUE_DUNES.0,
        legal_entity_id: Some(entity.id),
        supplier_invoice_number: number.to_owned(),
        number_key: number.to_lowercase(),
        bill_date: day(1, 2),
        due_date: Some(due),
        currency: "AED".to_owned(),
        gross_total: balance,
        balance,
        status: SupplierBillStatus::Approved,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    bill.insert_in_state(&mut tx, SupplierBillStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    bill
}

/// Runs the daily check as of `on`; how many notices went out.
async fn run_on(t: &Booted, on: NaiveDate) -> usize {
    cash_alert::run(t.state(), on).await.unwrap()
}

/// The users told by the notices of `BELOW_THRESHOLD`, one list per notice.
fn told(t: &Booted) -> Vec<Vec<i64>> {
    t.notices
        .of_type(BELOW_THRESHOLD)
        .iter()
        .map(|request| {
            let mut ids: Vec<i64> = request
                .recipients
                .iter()
                .filter_map(|r| match r {
                    NotifyRecipient::User { id } => Some(*id),
                    _ => None,
                })
                .collect();
            ids.sort_unstable();
            ids
        })
        .collect()
}

/// E1 with a threshold of AED 20,000: AED 50,000 in the bank and a bill of AED
/// 35,000 due in week 6 make AED 15,000 there. The 06:00 run tells the finance
/// manager and the company admin the week and the balance, and emits the
/// event; the next day, with the same first week below, nobody is told.
#[renox::test]
async fn the_first_week_below_is_told_once() {
    let t = booted(module()).await;
    let both = recipients(&t).await;
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 12));
    bill(&t, &e1, "SB-W6", day(2, 16), 3_500_000).await;

    assert_eq!(run_on(&t, day(1, 13)).await, 1);
    assert_eq!(told(&t), std::slice::from_ref(&both));
    let notice = &t.notices.of_type(BELOW_THRESHOLD)[0];
    assert_eq!(notice.data["week"], 6);
    assert_eq!(notice.data["entity"], "PGP");
    assert_eq!(notice.data["balance"], "AED 15,000.00");
    assert_eq!(notice.data["threshold"], "AED 20,000.00");
    assert!(notice.data["payments"].as_str().unwrap().contains("SB-W6"));
    let events = t.below_threshold.all();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (
            events[0].week,
            events[0].week_start,
            events[0].projected_balance,
            events[0].threshold
        ),
        (6, day(2, 15), 1_500_000, THRESHOLD)
    );

    // The next day, week 6 still first below: nobody.
    assert_eq!(run_on(&t, day(1, 14)).await, 0);
    assert_eq!(told(&t).len(), 1);
    assert_eq!(t.below_threshold.all().len(), 1);

    // Only the last alert is stored.
    let row = CashAlert::of(t.db(), e1.id).await.unwrap().unwrap();
    assert_eq!(row.first_week_start, Some(day(2, 15)));
}

/// A new bill that makes week 4 the first below gives a new notice naming week
/// 4; a later week does not.
#[renox::test]
async fn an_earlier_first_week_notifies_again() {
    let t = booted(module()).await;
    recipients(&t).await;
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 12));
    bill(&t, &e1, "SB-W6", day(2, 16), 3_500_000).await;
    run_on(&t, day(1, 13)).await;

    // A bill in week 8 does not move the first week below.
    bill(&t, &e1, "SB-W8", day(3, 2), 3_000_000).await;
    assert_eq!(run_on(&t, day(1, 14)).await, 0);

    // A bill in week 4 does.
    bill(&t, &e1, "SB-W4", day(2, 3), 4_000_000).await;
    assert_eq!(run_on(&t, day(1, 15)).await, 1);
    let notices = t.notices.of_type(BELOW_THRESHOLD);
    assert_eq!(notices.len(), 2);
    assert_eq!(notices[1].data["week"], 4);
    assert_eq!(t.below_threshold.all().len(), 2);
}

/// After a run with no week below, a later breach notifies again.
#[renox::test]
async fn a_breach_after_a_clear_run_notifies_again() {
    let t = booted(module()).await;
    recipients(&t).await;
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 12));
    let mut big = bill(&t, &e1, "SB-W6", day(2, 16), 3_500_000).await;
    assert_eq!(run_on(&t, day(1, 13)).await, 1);

    // The bill is paid: no week below.
    big.balance = 0;
    big.save(t.db()).await.unwrap();
    assert_eq!(run_on(&t, day(1, 14)).await, 0);
    let row = CashAlert::of(t.db(), e1.id).await.unwrap().unwrap();
    assert_eq!(row.first_week_start, None);

    // A new bill brings it back: told again, for the same week.
    bill(&t, &e1, "SB-AGAIN", day(2, 16), 3_500_000).await;
    assert_eq!(run_on(&t, day(1, 15)).await, 1);
    assert_eq!(t.notices.of_type(BELOW_THRESHOLD).len(), 2);
}

/// The income budget fill is left out unless the setting counts it: with AED
/// 30,000 of income fill in week 6 the rule still warns for week 6.
#[renox::test]
async fn the_income_fill_is_left_out_unless_counted() {
    // A budget of AED 120,000 of sales for February is AED 30,000 a week.
    for (counted, notices) in [(false, 1), (true, 0)] {
        let t = boot_with(module().cash_income_fill_counted(counted)).await;
        t.approved_budget("PGP", 2027, &[("sales_services", 2, 12_000_000)])
            .await;
        t.travel_to("2027-01-13", 2, 0);
        recipients(&t).await;
        let e1 = t.entity("PGP").await;
        t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 12));
        bill(&t, &e1, "SB-W6", day(2, 16), 3_500_000).await;
        assert_eq!(run_on(&t, day(1, 13)).await, notices, "counted: {counted}");
        if !counted {
            assert_eq!(t.notices.of_type(BELOW_THRESHOLD)[0].data["week"], 6);
        }
    }
}

/// Contracted amounts not yet invoiced count in the balance the rule checks:
/// a scheduled charge of AED 10,000 due in week 6 lifts week 6 over the
/// threshold.
struct Charges;

impl CashForecastSource for Charges {
    fn lines<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> ForecastFuture<'a, Vec<ForecastLine>> {
        Box::pin(async move {
            let entity = LegalEntity::find(&state.db, legal_entity_id).await?;
            if entity.map(|e| e.code) != Some("CHCO".to_owned()) {
                return Ok(Vec::new());
            }
            Ok(vec![ForecastLine::new(
                "Scheduled charge",
                "scheduled_charge",
                day(2, 16),
                1_000_000,
                Direction::In,
            )])
        })
    }
}

#[renox::test]
async fn a_scheduled_charge_not_yet_invoiced_counts() {
    register_cash_forecast_source("test.charges", Charges);
    // Only CHCO has a threshold: the seeded PGP has no bank balance and would
    // be below the one `module()` gives it.
    let t = booted(WlInvoicing::new().cash_threshold("CHCO", THRESHOLD)).await;
    recipients(&t).await;
    let co = t.new_entity("CHCO").await;
    t.forecast.statement(co.id, "BA1", 5_000_000, day(1, 12));
    bill(&t, &co, "SB-W6", day(2, 16), 3_500_000).await;
    // 50,000 - 35,000 + 10,000 = 25,000, above the threshold of 20,000.
    assert_eq!(run_on(&t, day(1, 13)).await, 0);
    assert!(t.notices.of_type(BELOW_THRESHOLD).is_empty());
}

/// An entity with no bank balance and week 3 below its threshold gets one
/// notice naming week 3 with the warning; unchanged the next day, nobody. The
/// warning alone never causes a notice.
#[renox::test]
async fn an_entity_without_a_bank_balance_gets_one_notice_with_the_warning() {
    let t = booted(WlInvoicing::new().cash_account("PGP", "BA9")).await;
    recipients(&t).await;
    let e1 = t.entity("PGP").await;

    // No week below yet: the missing balance alone tells nobody.
    assert_eq!(run_on(&t, day(1, 13)).await, 0);

    bill(&t, &e1, "SB-W3", day(1, 27), 1_000_000).await;
    assert_eq!(run_on(&t, day(1, 14)).await, 1);
    let notice = &t.notices.of_type(BELOW_THRESHOLD)[0];
    assert_eq!(notice.data["week"], 3);
    assert!(
        notice.data["warnings"]
            .as_str()
            .unwrap()
            .contains("no balance known"),
        "{}",
        notice.data["warnings"]
    );
    assert_eq!(run_on(&t, day(1, 15)).await, 0);
    assert_eq!(t.notices.of_type(BELOW_THRESHOLD).len(), 1);
}

/// A role nobody holds falls back to the company admin.
#[renox::test]
async fn a_role_nobody_holds_falls_back_to_the_admin() {
    let t = booted(module()).await;
    let admin = with_role(&t, "Boss", "admin").await;
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 12));
    bill(&t, &e1, "SB-W6", day(2, 16), 3_500_000).await;
    assert_eq!(run_on(&t, day(1, 13)).await, 1);
    assert_eq!(told(&t), [vec![admin.id]]);
}

/// Nobody to tell: nothing is stored, so the next run tries again.
#[renox::test]
async fn nothing_is_stored_when_nobody_can_be_told() {
    let t = booted(module()).await;
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 12));
    bill(&t, &e1, "SB-W6", day(2, 16), 3_500_000).await;
    assert_eq!(run_on(&t, day(1, 13)).await, 0);
    assert!(CashAlert::of(t.db(), e1.id).await.unwrap().is_none());
    let admin = with_role(&t, "Boss", "admin").await;
    assert_eq!(run_on(&t, day(1, 14)).await, 1);
    assert_eq!(told(&t), [vec![admin.id]]);
}

/// Bank account BA1 whose latest balance is 8 days old gets one accountant
/// task; running again opens no second one; a newer balance closes it.
#[renox::test]
async fn a_stale_balance_gets_one_task_that_a_newer_balance_closes() {
    let t = booted(WlInvoicing::new()).await;
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 5));
    run_on(&t, day(1, 13)).await;
    run_on(&t, day(1, 13)).await;
    let tasks = InvoicingTask::query()
        .where_(InvoicingTask::KIND.eq(TaskKind::StatementBalanceStale))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, TaskStatus::Open);
    assert_eq!(tasks[0].role, "accountant");
    assert!(tasks[0].message.contains("BA1"), "{}", tasks[0].message);

    // Importing a statement (a newer closing balance) closes it.
    t.forecast.statement(e1.id, "BA1", 5_100_000, day(1, 12));
    run_on(&t, day(1, 13)).await;
    let task = InvoicingTask::find(t.db(), tasks[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    let all = InvoicingTask::query()
        .where_(InvoicingTask::KIND.eq(TaskKind::StatementBalanceStale))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(all, 1);
}

/// A balance entered by hand closes the task at once, and a voided one does
/// not count.
#[renox::test]
async fn an_opening_cash_balance_closes_the_task_and_a_voided_one_does_not_count() {
    let t = booted(WlInvoicing::new().cash_account("PGP", "BA2")).await;
    let e1 = t.entity("PGP").await;
    run_on(&t, day(1, 13)).await;
    let open = || async {
        InvoicingTask::query()
            .where_(InvoicingTask::KIND.eq(TaskKind::StatementBalanceStale))
            .where_(InvoicingTask::STATUS.eq(TaskStatus::Open))
            .count(t.db())
            .await
            .unwrap()
    };
    assert_eq!(open().await, 1);

    let balance = t
        .app
        .at_travelled_time(cash_balance::enter(
            t.db(),
            None,
            &EnterBalance {
                legal_entity_id: e1.id,
                bank_account: "BA2".into(),
                amount: 100,
                as_of: day(1, 12),
                note: None,
            },
        ))
        .await
        .unwrap();
    assert_eq!(open().await, 0);

    // Voided: the balance does not count, and the account is missing again
    // (a new task, for the balance it now sees: none).
    t.app
        .at_travelled_time(cash_balance::void(t.db(), balance.id, None, "wrong bank"))
        .await
        .unwrap();
    run_on(&t, day(1, 14)).await;
    assert_eq!(open().await, 1);
}

/// The check changes no document and stores only the last alert.
#[renox::test]
async fn the_check_changes_no_document() {
    let t = booted(module()).await;
    recipients(&t).await;
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 12));
    let held = bill(&t, &e1, "SB-W6", day(2, 16), 3_500_000).await;
    run_on(&t, day(1, 13)).await;
    let after = SupplierBill::find(t.db(), held.id).await.unwrap().unwrap();
    assert_eq!(
        (after.balance, after.status),
        (3_500_000, SupplierBillStatus::Approved)
    );
    assert_eq!(CashAlert::query().count(t.db()).await.unwrap(), 1);
}
