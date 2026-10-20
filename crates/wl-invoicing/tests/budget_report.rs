//! Budget vs actual and the warning on overspending (wl-invoicing §5.15 steps
//! 2 to 4, `FIN.BUD.01`, `FIN.RUL.24`, `FIN.ACL.22`): the acceptance checks of
//! #89.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::budget_report::{self, Report, ReportQuery};
use wl_invoicing::credit_notes::{self, By};
use wl_invoicing::{BudgetKind, CompanyBudgetLine, CreditNote, StartChoice, permissions as perm};
use wl_kernel::LegalEntity;
use wl_kernel::budgets::company_budgets;
use wl_kernel::invoicing::{CreditLine, VatCategory};
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, boot};

const NOTICE: &str = "invoicing.budget_variance";

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, m, d).unwrap()
}

/// A user who also holds `role` (made once).
async fn with_role(t: &Booted, name: &str, role: &str) -> User {
    let user = t.helper(name, &[perm::VIEW]).await;
    user.assign_role(t.db(), role).await.unwrap();
    user
}

/// The report of `entities` for `month` of 2027, with the drawings lines when
/// asked.
async fn report(
    t: &Booted,
    entities: &[&LegalEntity],
    month: u32,
    ytd: bool,
    drawings_lines: bool,
) -> Report {
    budget_report::build(
        t.db(),
        &t.config(),
        &ReportQuery {
            entities: entities.iter().map(|e| e.id).collect(),
            month: day(month, 1),
            ytd,
            drawings_lines,
            uncategorised: "Uncategorised".into(),
        },
    )
    .await
    .unwrap()
}

/// The row called `name` among `rows`.
fn row<'a>(rows: &'a [budget_report::Row], name: &str) -> &'a budget_report::Row {
    rows.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no row {name}"))
}

/// The recipients (user ids, sorted) of the variance notices so far.
fn told(t: &Booted) -> Vec<Vec<i64>> {
    t.notices
        .of_type(NOTICE)
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

/// Given an approved 2027 budget for E1 with *Marketing* AED 10,000 in March,
/// and approved supplier bills in March for AED 6,000 + 5% VAT and AED 4,500 +
/// 5% VAT coded to marketing, the March actual is AED 10,500 and the variance
/// AED +500 (5%), unfavourable. The second bill makes the actual reach 100%:
/// the marketing owner role and the finance manager are notified once; a third
/// bill in March does not notify again.
#[renox::test]
async fn the_actual_leaves_out_recoverable_vat_and_warns_once() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    t.approved_budget("PGP", 2027, &[("marketing", 3, 1_000_000)])
        .await;
    let owner = with_role(&t, "Mona", "marketing_manager").await;
    let manager = with_role(&t, "Fares", "finance_manager").await;

    t.approved_bill(
        &e1,
        "SB-1",
        day(3, 10),
        &[("Flyers", 600_000, "marketing", VatCategory::Standard)],
    )
    .await;
    let march = report(&t, &[&e1], 3, false, false).await;
    let marketing = row(&march.expense, "Marketing");
    assert_eq!((marketing.budget, marketing.actual), (1_000_000, 600_000));
    assert!(t.notices.of_type(NOTICE).is_empty());

    t.approved_bill(
        &e1,
        "SB-2",
        day(3, 20),
        &[("Radio spot", 450_000, "marketing", VatCategory::Standard)],
    )
    .await;
    let march = report(&t, &[&e1], 3, false, false).await;
    let marketing = row(&march.expense, "Marketing");
    assert_eq!(marketing.actual, 1_050_000);
    assert_eq!(marketing.variance(), 50_000);
    assert_eq!(marketing.percent_bp(), Some(500));
    assert!(marketing.unfavourable());
    let mut both = vec![owner.id, manager.id];
    both.sort_unstable();
    assert_eq!(told(&t), [both]);
    let events = t.variance_reached.all();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (
            events[0].category.as_str(),
            events[0].budget,
            events[0].actual
        ),
        ("marketing", 1_000_000, 1_050_000)
    );
    assert_eq!(events[0].percent_used, Some(105));
    assert_eq!(events[0].month, day(3, 1));

    t.approved_bill(
        &e1,
        "SB-3",
        day(3, 25),
        &[("Banner", 20_000, "marketing", VatCategory::Standard)],
    )
    .await;
    assert_eq!(told(&t).len(), 1);
    assert_eq!(t.variance_reached.all().len(), 1);
    // A month with no budget line yet has the same category at budget 0.
    let year = report(&t, &[&e1], 3, true, false).await;
    assert_eq!(row(&year.expense, "Marketing").budget, 1_000_000);
}

/// A category with an actual of AED 800 and no budget line shows a variance of
/// AED +800 and no percentage; the first expense posted warns (budget 0).
#[renox::test]
async fn an_actual_with_no_budget_line_shows_no_percentage() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    t.approved_budget("PGP", 2027, &[("marketing", 3, 1_000_000)])
        .await;
    let manager = with_role(&t, "Fares", "finance_manager").await;
    t.approved_bill(
        &e1,
        "SB-1",
        day(3, 10),
        &[("Diesel", 80_000, "fuel", VatCategory::Exempt)],
    )
    .await;
    let march = report(&t, &[&e1], 3, false, false).await;
    let fuel = row(&march.expense, "Transport and fuel");
    assert_eq!((fuel.budget, fuel.actual), (0, 80_000));
    assert_eq!(fuel.variance(), 80_000);
    assert_eq!(fuel.percent_bp(), None);
    assert_eq!(told(&t), [vec![manager.id]]);

    let viewer = t.helper("Vera", &[perm::BUDGETS_VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .get("/budget-report?month=2027-03")
        .await
        .assert_ok()
        .assert_see("Transport and fuel")
        .assert_see("+800.00")
        .assert_see("\u{2014}");
}

/// An invoice line with a charge type mapped to no category appears in
/// *Uncategorised*; a mapped one is income of its category by the date of
/// supply; a credit note in a later month reduces the income of its own month.
#[renox::test]
async fn income_follows_the_charge_type_and_the_credit_note_its_own_month() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let clerk = t.user_a().await;
    t.approved_budget("PGP", 2027, &[("sales_services", 3, 2_000_000)])
        .await;
    let rent = t
        .invoice_with_charge(&e1, "rent", 1_000_000, day(3, 10), &clerk)
        .await;
    t.invoice_with_charge(&e1, "mystery", 500_000, day(3, 12), &clerk)
        .await;
    // An invoice dated in April is not March income.
    t.invoice_with_charge(&e1, "rent", 300_000, day(4, 2), &clerk)
        .await;

    let march = report(&t, &[&e1], 3, false, false).await;
    assert_eq!(row(&march.income, "Sales and services").actual, 1_000_000);
    assert_eq!(row(&march.income, "Sales and services").budget, 2_000_000);
    assert_eq!(row(&march.income, "Uncategorised").actual, 500_000);
    assert_eq!(march.income_total().1, 1_500_000);
    // Income below its budget is unfavourable.
    assert!(row(&march.income, "Sales and services").unfavourable());

    let credit = t.helper("Cleo", &[perm::VIEW, perm::CREDIT]).await;
    let draft = credit_notes::draft(
        t.db(),
        &rent,
        &[CreditLine::new(1, 200_000)],
        "Damaged",
        Some(credit.id),
    )
    .await
    .unwrap();
    let note = credit_notes::issue(t.state(), By::System(Some(credit.id)), &draft)
        .await
        .unwrap();
    CreditNote::query()
        .where_(CreditNote::ID.eq(note.id))
        .update(t.db(), &[("issue_date", &Some(day(4, 5)))])
        .await
        .unwrap();
    let march = report(&t, &[&e1], 3, false, false).await;
    assert_eq!(row(&march.income, "Sales and services").actual, 1_000_000);
    let april = report(&t, &[&e1], 4, false, false).await;
    assert_eq!(
        row(&april.income, "Sales and services").actual,
        300_000 - 200_000
    );
    // The year to date adds them up.
    let ytd = report(&t, &[&e1], 4, true, false).await;
    assert_eq!(
        row(&ytd.income, "Sales and services").actual,
        1_000_000 + 300_000 - 200_000
    );
}

/// An approved supplier bill line of AED 3,000 in March coded to *Owner
/// drawings* is on the owner drawings line, below the operating result, and
/// not in expense. With *Owner drawings* budgeted AED 2,500 for March it
/// notifies the finance manager once (the category has no owner role).
#[renox::test]
async fn owner_drawings_stay_out_of_the_operating_result() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    t.approved_budget(
        "PGP",
        2027,
        &[("owner_drawings", 3, 250_000), ("marketing", 3, 1_000_000)],
    )
    .await;
    let manager = with_role(&t, "Fares", "finance_manager").await;
    t.approved_bill(
        &e1,
        "SB-1",
        day(3, 11),
        &[(
            "Personal bill",
            300_000,
            "owner_drawings",
            VatCategory::Exempt,
        )],
    )
    .await;
    t.approved_bill(
        &e1,
        "SB-2",
        day(3, 12),
        &[("Flyers", 100_000, "marketing", VatCategory::Exempt)],
    )
    .await;
    let march = report(&t, &[&e1], 3, false, true).await;
    assert_eq!(march.drawings_total(), (250_000, 300_000));
    assert_eq!(march.expense_total().1, 100_000);
    assert!(
        march
            .expense
            .iter()
            .all(|r| r.kind != BudgetKind::OwnerDrawings)
    );
    assert_eq!(march.operating(), (-1_000_000, -100_000));
    assert_eq!(march.after_drawings(), (-1_250_000, -400_000));
    assert_eq!(march.drawings_items.len(), 1);
    assert_eq!(march.drawings_items[0].label, "Personal bill");
    // The finance manager is told once about the drawings.
    assert_eq!(told(&t), [vec![manager.id]]);
    assert_eq!(t.variance_reached.all().len(), 1);
    assert_eq!(t.variance_reached.all()[0].category, "owner_drawings");
}

/// The VAT of a drawings line is not recovered: it stays in the amount.
#[renox::test]
async fn the_vat_of_a_drawings_line_stays_in_the_cost() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    t.approved_bill(
        &e1,
        "SB-1",
        day(3, 11),
        &[(
            "Personal bill",
            300_000,
            "owner_drawings",
            VatCategory::Standard,
        )],
    )
    .await;
    let march = report(&t, &[&e1], 3, false, false).await;
    assert_eq!(march.drawings_total().1, 315_000);
}

/// A user with `budgets.view` but not `drawings.approve` sees the drawings
/// total (AED 3,000) but not its lines; a holder of `drawings.approve` sees
/// both.
#[renox::test]
async fn the_drawings_lines_are_only_for_holders_of_drawings_approve() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    t.approved_bill(
        &e1,
        "SB-1",
        day(3, 11),
        &[(
            "Villa decorator",
            300_000,
            "owner_drawings",
            VatCategory::Exempt,
        )],
    )
    .await;
    let viewer = t.helper("Vera", &[perm::BUDGETS_VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .get("/budget-report?month=2027-03")
        .await
        .assert_ok()
        .assert_see("Total owner drawings")
        .assert_see("3,000.00")
        .assert_dont_see("Villa decorator")
        .assert_dont_see("Owner drawings lines");
    let owner = t
        .helper("Olga", &[perm::BUDGETS_VIEW, perm::DRAWINGS_APPROVE])
        .await;
    t.app.acting_as(&owner);
    t.app
        .get("/budget-report?month=2027-03")
        .await
        .assert_ok()
        .assert_see("3,000.00")
        .assert_see("Villa decorator")
        .assert_see("Owner drawings lines");
}

/// For entity E3 with no approved 2027 budget, approved bills post actual and
/// nobody is notified; the report says there is no approved budget.
#[renox::test]
async fn an_entity_without_a_budget_posts_actual_and_warns_nobody() {
    let t = boot().await;
    let e3 = t.new_entity("E3").await;
    with_role(&t, "Fares", "finance_manager").await;
    t.approved_bill(
        &e3,
        "SB-1",
        day(3, 11),
        &[("Flyers", 100_000, "marketing", VatCategory::Exempt)],
    )
    .await;
    assert!(t.notices.of_type(NOTICE).is_empty());
    assert!(t.variance_reached.all().is_empty());
    let march = report(&t, &[&e3], 3, false, false).await;
    assert_eq!(march.without_budget, [e3.id]);
    let marketing = row(&march.expense, "Marketing");
    assert_eq!((marketing.budget, marketing.actual), (0, 100_000));

    let admin = t.helper("Ada", &[perm::BUDGETS_VIEW]).await;
    t.app.acting_as(&admin);
    t.app
        .get(&format!("/budget-report?month=2027-03&entity={}", e3.id))
        .await
        .assert_ok()
        .assert_see("No approved budget for the year");
}

/// The all-entities view shows the sums of E1 and E2; a user scoped to E1 sees
/// E1 only, also in the all-entities view, and cannot ask for E2.
#[renox::test]
async fn the_all_entities_view_adds_up_and_a_scoped_user_sees_their_entity() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let e2 = t.entity("PGFZ").await;
    t.approved_budget("PGP", 2027, &[("marketing", 3, 1_000_000)])
        .await;
    t.approved_budget("PGFZ", 2027, &[("marketing", 3, 500_000)])
        .await;
    t.approved_bill(
        &e1,
        "SB-1",
        day(3, 11),
        &[("Flyers", 100_000, "marketing", VatCategory::Exempt)],
    )
    .await;
    t.approved_bill(
        &e2,
        "SB-2",
        day(3, 11),
        &[("Posters", 200_000, "marketing", VatCategory::Exempt)],
    )
    .await;
    let both = report(&t, &[&e1, &e2], 3, false, false).await;
    let marketing = row(&both.expense, "Marketing");
    assert_eq!((marketing.budget, marketing.actual), (1_500_000, 300_000));
    let only = report(&t, &[&e1], 3, false, false).await;
    assert_eq!(row(&only.expense, "Marketing").actual, 100_000);

    let scoped = t
        .scoped_user("Sam", &[perm::VIEW, perm::BUDGETS_VIEW], e1.id)
        .await;
    t.app.acting_as(&scoped);
    t.app
        .get("/budget-report?month=2027-03")
        .await
        .assert_ok()
        .assert_see("1,000.00")
        .assert_see("PGP")
        .assert_dont_see("PGFZ")
        .assert_dont_see("3,000.00");
    t.app
        .get(&format!("/budget-report?month=2027-03&entity={}", e2.id))
        .await
        .assert_forbidden();

    let global = t.helper("Gina", &[perm::BUDGETS_VIEW]).await;
    t.app.acting_as(&global);
    t.app
        .get("/budget-report?month=2027-03")
        .await
        .assert_ok()
        .assert_see("3,000.00")
        .assert_see("PGFZ");
    // Without the permission the page is forbidden.
    let nobody = t.helper("Nadia", &[perm::VIEW]).await;
    t.app.acting_as(&nobody);
    t.app.get("/budget-report").await.assert_forbidden();
}

/// A first 2028 version created as a copy of last year's actual (BL-INV-25
/// option) has, for *Marketing* in March, the March 2027 actual of AED 10,500.
#[renox::test]
async fn a_budget_can_start_from_last_years_actual() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let a = t.budget_preparer("Amal").await;
    t.approved_bill(
        &e1,
        "SB-1",
        day(3, 10),
        &[("Flyers", 600_000, "marketing", VatCategory::Standard)],
    )
    .await;
    t.approved_bill(
        &e1,
        "SB-2",
        day(3, 20),
        &[("Radio spot", 450_000, "marketing", VatCategory::Standard)],
    )
    .await;
    // A category that is inactive now is skipped.
    t.approved_bill(
        &e1,
        "SB-3",
        day(5, 3),
        &[("Postage", 10_000, "other", VatCategory::Exempt)],
    )
    .await;
    let other = t.category("other_expense").await;
    wl_invoicing::BudgetCategory::query()
        .where_(wl_invoicing::BudgetCategory::ID.eq(other.id))
        .update(t.db(), &[("active", &false)])
        .await
        .unwrap();
    let budget = t.new_budget("PGP", 2028, StartChoice::LastActual, &a).await;
    assert_eq!(budget.start_choice, StartChoice::LastActual);
    let lines = CompanyBudgetLine::of(t.db(), budget.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    let marketing = t.category("marketing").await;
    assert_eq!(lines[0].category_id, marketing.id);
    assert_eq!(lines[0].month, NaiveDate::from_ymd_opt(2028, 3, 1).unwrap());
    assert_eq!(lines[0].amount, 1_050_000);
}

/// The kernel reads the actual of a category and month.
#[renox::test]
async fn the_kernel_reads_the_actual() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    t.approved_bill(
        &e1,
        "SB-1",
        day(3, 10),
        &[("Flyers", 600_000, "marketing", VatCategory::Standard)],
    )
    .await;
    let budgets = company_budgets(t.state());
    assert_eq!(
        budgets
            .actual_amount(t.db(), e1.id, "marketing", day(3, 31))
            .await
            .unwrap(),
        600_000
    );
    assert_eq!(
        budgets
            .actual_amount(t.db(), e1.id, "marketing", day(4, 1))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        budgets
            .actual_amount(t.db(), e1.id, "nothing", day(3, 1))
            .await
            .unwrap(),
        0
    );
}

/// The paid payroll run of the stand-in for wl-hr: only the entity `HRCO`
/// has one (the registry of sources is process wide).
struct Payroll;

impl wl_kernel::budgets::ActualCostSource for Payroll {
    fn costs<'a>(
        &'a self,
        db: &'a Db,
        entities: &'a [i64],
        from: NaiveDate,
        to: NaiveDate,
    ) -> wl_kernel::budgets::BudgetFuture<'a, Vec<wl_kernel::budgets::ActualCost>> {
        Box::pin(async move {
            let mut found = Vec::new();
            for id in entities {
                let Some(entity) = LegalEntity::find(db, *id).await? else {
                    continue;
                };
                // The March run, employer cost AED 80,000, paid.
                if entity.code == "HRCO" && (from..=to).contains(&day(3, 31)) {
                    found.push(wl_kernel::budgets::ActualCost {
                        legal_entity_id: entity.id,
                        date: day(3, 31),
                        key_kind: "payroll".to_owned(),
                        key: "payroll".to_owned(),
                        amount: 8_000_000,
                        label: "Payroll March".to_owned(),
                        source: wl_kernel::Subject::new("hr.payroll_run", 7),
                    });
                }
            }
            Ok(found)
        })
    }
}

/// A paid March payroll run with an employer cost of AED 80,000 counts in
/// *Salaries and staff costs* in March (through the kernel's actual cost
/// sources, which wl-hr will register), and in no other month or entity.
#[renox::test]
async fn a_paid_payroll_run_counts_in_salaries_and_staff_costs() {
    wl_kernel::budgets::register_actual_costs("test.hr", Payroll);
    let t = boot().await;
    let co = t.new_entity("HRCO").await;
    let e1 = t.entity("PGP").await;
    let march = report(&t, &[&co], 3, false, false).await;
    let salaries = row(&march.expense, "Salaries and staff costs");
    assert_eq!(salaries.actual, 8_000_000);
    assert_eq!(march.expense_total().1, 8_000_000);
    let april = report(&t, &[&co], 4, false, false).await;
    assert!(april.expense.iter().all(|r| r.actual == 0));
    let other = report(&t, &[&e1], 3, false, false).await;
    assert!(other.expense.iter().all(|r| r.actual == 0));
}
