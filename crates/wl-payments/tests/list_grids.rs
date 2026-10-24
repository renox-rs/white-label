//! The bank statements, the expense claims and the follow-up tasks are
//! standard grids (#524): each fills the page (the grid follows the header,
//! with no stack, card or pane around it), searches, sorts and pages through
//! the query string (`?search=`, `?sort=`, `?per_page=`, `?page=`) and keeps
//! its filters as grid filters (`?in.format=`, `?in.status=`).

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_payments::expense_claim::{self, LineInput};
use wl_payments::followup::{FollowupKind, FollowupStatus, FollowupTask};
use wl_payments::permissions as perm;

use support::{Booted, boot};

/// The page at `path`, as the signed-in user gets it.
async fn load(t: &Booted, path: &str) -> String {
    let page = t.app.get(path).await;
    page.assert_ok();
    page.text()
}

/// Where `text` first stands in the page; text that is not there is last.
fn at(html: &str, text: &str) -> usize {
    html.find(text).unwrap_or(usize::MAX)
}

/// Whether the grid is the page's own block: no stack, card or pane between
/// the header and the grid.
fn fills_the_page(html: &str) -> bool {
    let (Some(header), Some(grid)) = (
        html.find("class=\"rx-page-header\""),
        html.find("<form class=\"rx-grid"),
    ) else {
        return false;
    };
    let between = &html[header..grid];
    header < grid
        && !between.contains("rx-stack")
        && !between.contains("rx-card")
        && !between.contains("rx-pane")
}

/// Bank statements: twelve uploads, one a month of 2026, eight of ENBD-1 and
/// four of ADCB-2; the latest period first, search on the account and the
/// file, the status and format filters, pages.
#[renox::test]
async fn pay_grid_the_bank_statements_search_sort_filter_and_page() {
    let t = boot().await;
    let dina = t
        .user("Dina", &[perm::VIEW, perm::BANK_RECONCILE])
        .await;
    t.app.acting_as(&dina);
    let entity = t.entity("PGP").await.id.to_string();
    for month in 1..=12_u32 {
        let start = NaiveDate::from_ymd_opt(2026, month, 1).unwrap();
        let end = if month == 12 {
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()
        } else {
            NaiveDate::from_ymd_opt(2026, month + 1, 1)
                .and_then(|d| d.pred_opt())
                .unwrap()
        };
        let account = if month <= 8 { "ENBD-1" } else { "ADCB-2" };
        let (file, start, end) = (format!("stmt-{month:02}"), start.to_string(), end.to_string());
        t.app
            .htmx()
            .post(
                "/bank-statements",
                &[
                    ("legal_entity_id", entity.as_str()),
                    ("bank_account", account),
                    ("file_name", file.as_str()),
                    ("format", "csv"),
                    ("period_start", start.as_str()),
                    ("period_end", end.as_str()),
                    ("closing_balance", "1,000"),
                    ("content", ""),
                ],
            )
            .await
            .assert_redirect("/bank-statements");
    }

    let first = load(&t, "/bank-statements?per_page=10").await;
    assert!(fills_the_page(&first));
    assert!(first.contains("1–10 of 12"));
    assert!(at(&first, "stmt-12") < at(&first, "stmt-11"), "latest period first");
    assert!(!first.contains("stmt-01") && !first.contains("stmt-02"));
    let second = load(&t, "/bank-statements?per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && second.contains("stmt-01"));
    let oldest = load(&t, "/bank-statements?sort=period_end&per_page=25").await;
    assert!(at(&oldest, "stmt-01") < at(&oldest, "stmt-12"));

    let adcb = load(&t, "/bank-statements?search=adcb").await;
    assert!(adcb.contains("1–4 of 4") && adcb.contains("stmt-09"));
    assert!(!adcb.contains("stmt-08"));
    let one = load(&t, "/bank-statements?search=stmt-05").await;
    assert!(one.contains("1–1 of 1"));
    let csv = load(
        &t,
        "/bank-statements?in.format=csv&in.status=uploaded&per_page=25",
    )
    .await;
    assert!(csv.contains("1–12 of 12"));
    let mt940 = load(&t, "/bank-statements?in.format=mt940").await;
    assert!(!mt940.contains("stmt-01"));
}

/// Expense claims: twelve drafts, nine of Dina and three of Omar; search by
/// the claimant, sort by the title, pages, and the actions stay.
#[renox::test]
async fn pay_grid_the_expense_claims_search_by_claimant_sort_and_page() {
    let t = boot().await;
    wl_payments::expense_category::install(t.db()).await.unwrap();
    let dina = t
        .user("Dina", &[perm::EXPENSES_VIEW, perm::EXPENSES_SUBMIT])
        .await;
    let omar = t
        .user("Omar", &[perm::EXPENSES_VIEW, perm::EXPENSES_SUBMIT])
        .await;
    let approver = t
        .user("Amal", &[perm::EXPENSES_VIEW, perm::EXPENSES_APPROVE])
        .await;
    let entity = t.entity("PGP").await.id;
    for n in 1..=12_i64 {
        let claimant = if n <= 9 { dina.id } else { omar.id };
        expense_claim::create(
            t.db(),
            claimant,
            entity,
            &format!("Claim {n:02}"),
            &[LineInput {
                expense_date: NaiveDate::from_ymd_opt(2026, 3, 10).unwrap(),
                merchant: "ENOC".to_owned(),
                category: "fuel".to_owned(),
                description: "Fuel".to_owned(),
                amount: 10_500 + n,
                vat_amount: 500,
            }],
        )
        .await
        .unwrap();
    }
    t.app.acting_as(&approver);

    let first = load(&t, "/expense-claims?per_page=10").await;
    assert!(fills_the_page(&first));
    assert!(first.contains("1–10 of 12"));
    assert!(at(&first, "Claim 12") < at(&first, "Claim 11"), "newest first");
    let second = load(&t, "/expense-claims?per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && second.contains("Claim 01"));

    let of_omar = load(&t, "/expense-claims?search=omar").await;
    assert!(of_omar.contains("1–3 of 3") && of_omar.contains("Claim 10"));
    assert!(!of_omar.contains("Claim 09"));
    let by_title = load(&t, "/expense-claims?sort=title&per_page=25").await;
    assert!(at(&by_title, "Claim 01") < at(&by_title, "Claim 02"));
    let drafts = load(&t, "/expense-claims?in.status=draft&per_page=25").await;
    assert!(drafts.contains("1–12 of 12"));
    // The actions column stays (a submitted claim offers approve and reject).
    t.app.acting_as(&dina);
    let mine = load(&t, "/expense-claims?per_page=25").await;
    assert!(mine.contains("submit-claim-"), "the submit action stays");
}

/// Follow-up tasks: twelve open tasks of the accountants, nine for Blue
/// Dunes and three for Quiet Sands; search on the party, sort by the amount,
/// pages, and "Mark done" stays.
#[renox::test]
async fn pay_grid_the_follow_up_tasks_search_sort_and_page() {
    let t = boot().await;
    for n in 1..=12_i64 {
        FollowupTask {
            kind: FollowupKind::RaiseCharge,
            party_id: if n <= 9 { 1 } else { 2 },
            amount: 100_000 * n,
            currency: "AED".to_owned(),
            text: format!("Task {n:02}: raise a charge"),
            role: "accountant".to_owned(),
            status: FollowupStatus::Open,
            ..Default::default()
        }
        .insert(t.db())
        .await
        .unwrap();
    }
    let cara = t.accountant("Cara").await;
    t.app.acting_as(&cara);

    let first = load(&t, "/payments/tasks?per_page=10").await;
    assert!(fills_the_page(&first));
    assert!(first.contains("1–10 of 12"));
    assert!(at(&first, "Task 01") < at(&first, "Task 02"), "oldest first");
    assert!(first.contains("task-done-"), "Mark done stays");
    let second = load(&t, "/payments/tasks?per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && second.contains("Task 12"));

    let quiet = load(&t, "/payments/tasks?search=quiet").await;
    assert!(quiet.contains("1–3 of 3") && quiet.contains("Task 10"));
    assert!(!quiet.contains("Task 09"));
    let largest = load(&t, "/payments/tasks?sort=-amount&per_page=25").await;
    assert!(at(&largest, "Task 12") < at(&largest, "Task 01"));
}
