//! The company budget (wl-invoicing §3 *Company budget*, §4, §5.15 step 1,
//! `FIN.BUD.01`, `FIN.RUL.26`): the acceptance checks of #88.

mod support;

use renox::chrono::{Datelike, Months, NaiveDate};
use renox::prelude::*;
use wl_invoicing::budget_machine::EXPIRY_HOURS;
use wl_invoicing::budget::month_start;
use wl_invoicing::budget_category::{self, CategoryInput};
use wl_invoicing::budget_lines::{self, LineInput, NewBudget};
use wl_invoicing::budget_reminder;
use wl_invoicing::{
    BudgetCategory, BudgetKind, BudgetStatus, CompanyBudget, CompanyBudgetLine, MappingKind,
    StartChoice, WlInvoicing, permissions as perm,
};
use wl_kernel::approvals::{self, ApprovalStatus};
use wl_kernel::budgets::{CompanyBudgetId, company_budgets};
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, boot, boot_with};

const RENT: i64 = 2_000_000;

/// The lines of `budget`, as stored now.
async fn lines(t: &Booted, budget: &CompanyBudget) -> Vec<CompanyBudgetLine> {
    CompanyBudgetLine::of(t.db(), budget.id).await.unwrap()
}

/// A 422 with a message that holds `word`.
fn assert_refused<T: std::fmt::Debug>(result: Result<T>, word: &str) {
    match result {
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => {
            assert!(message.contains(word), "{message}");
        }
        other => panic!("expected a refusal with `{word}`, got {other:?}"),
    }
}

/// Submits `budget` as `a` and approves it as `b`; returns it as stored.
async fn approve(t: &Booted, budget: &CompanyBudget, a: &User, b: &User) -> CompanyBudget {
    t.submit_budget(budget, a).await;
    let request = t.budget_request(budget).await;
    approvals::approve(t.state(), request.id, b, "")
        .await
        .unwrap()
        .expect("the approval runs");
    t.budget_now(budget).await
}

/// An approved budget of entity `code` for `year` with rent in every month.
async fn approved_year(t: &Booted, code: &str, year: i64, a: &User, b: &User) -> CompanyBudget {
    let draft = t.new_budget(code, year, StartChoice::Empty, a).await;
    t.fill(&draft, "rent_utilities", RENT, 1, 12).await;
    approve(t, &draft, a, b).await
}

/// Given entity E1 and fiscal year 2027, accountant A creates a draft with
/// 12 lines for *Rent and utilities* (AED 20,000 each) and submits it; an
/// approval request goes to the holders of `budgets.approve`. Owner B returns
/// it with a reason; it is `draft` again and A is told. A submits again and
/// B approves; version 1 is `approved`; event `CompanyBudgetApproved`.
#[renox::test]
async fn a_budget_is_submitted_returned_and_approved() {
    let t = boot().await;
    let a = t.budget_preparer("Amal").await;
    let b = t.budget_approver("Bilal").await;
    let budget = t.new_budget("PGP", 2027, StartChoice::Empty, &a).await;
    assert_eq!((budget.version, budget.status), (1, BudgetStatus::Draft));
    assert_eq!(budget.start_choice, StartChoice::Empty);
    assert_eq!(budget.prepared_by, Some(a.id));
    t.fill(&budget, "rent_utilities", RENT, 1, 12).await;
    let stored = lines(&t, &budget).await;
    assert_eq!(stored.len(), 12);
    assert!(stored.iter().all(|l| l.amount == RENT && !l.locked));

    t.submit_budget(&budget, &a).await;
    let submitted = t.budget_now(&budget).await;
    assert_eq!(submitted.status, BudgetStatus::Submitted);
    assert_eq!(submitted.submitted_by, Some(a.id));
    let request = t.budget_request(&budget).await;
    assert_eq!(request.action, "invoicing.budget_approval");
    assert_eq!(request.requester_id, a.id);
    assert_eq!(request.status, ApprovalStatus::Pending);
    assert_eq!(submitted.approval_request_id, Some(request.id));
    let approvers = approvals::notify::step_approvers(t.db(), &request)
        .await
        .unwrap();
    assert!(approvers.iter().any(|u| u.id == b.id));
    assert!(!approvers.iter().any(|u| u.id == a.id));
    assert_eq!(t.notices_of(&b, "approval-requested").await.len(), 1);

    approvals::return_request(t.state(), request.id, &b, "add marketing")
        .await
        .unwrap();
    let returned = t.budget_now(&budget).await;
    assert_eq!(returned.status, BudgetStatus::Draft);
    assert_eq!(returned.return_reason.as_deref(), Some("add marketing"));
    let told = t.notices_of(&a, "approval-decided").await;
    assert_eq!(told.len(), 1);
    assert!(told[0].contains("invoicing.company_budget"), "{}", told[0]);
    assert!(t.budget_approved.all().is_empty());

    t.fill(&budget, "marketing", 500_000, 1, 12).await;
    t.submit_budget(&budget, &a).await;
    let again = t.budget_request(&budget).await;
    assert_ne!(again.id, request.id);
    approvals::approve(t.state(), again.id, &b, "")
        .await
        .unwrap()
        .expect("the approval runs");
    let approved = t.budget_now(&budget).await;
    assert_eq!(approved.status, BudgetStatus::Approved);
    assert_eq!(approved.approved_by, Some(b.id));
    assert!(approved.approved_at.is_some());
    assert_eq!(approved.return_reason, None);
    let events = t.budget_approved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].budget, CompanyBudgetId(budget.id));
    assert_eq!(events[0].fiscal_year, 2027);
    assert_eq!(events[0].version, 1);
    assert_eq!(events[0].superseded, None);
    assert_eq!(
        t.budget_history(&budget).await,
        ["submit", "back_to_draft", "submit", "submit_approved"]
    );
}

/// User C, holding both `budgets.manage` and `budgets.approve`, submits a
/// version and cannot approve it.
#[renox::test]
async fn the_preparer_cannot_approve_their_own_budget() {
    let t = boot().await;
    let c = t
        .helper(
            "Cora",
            &[perm::BUDGETS_VIEW, perm::BUDGETS_MANAGE, perm::BUDGETS_APPROVE],
        )
        .await;
    let budget = t.new_budget("PGP", 2027, StartChoice::Empty, &c).await;
    t.fill(&budget, "rent_utilities", RENT, 1, 12).await;
    t.submit_budget(&budget, &c).await;
    let request = t.budget_request(&budget).await;
    let approvers = approvals::notify::step_approvers(t.db(), &request)
        .await
        .unwrap();
    assert!(!approvers.iter().any(|u| u.id == c.id));
    assert!(
        approvals::approve(t.state(), request.id, &c, "")
            .await
            .is_err()
    );
    assert_eq!(
        t.budget_now(&budget).await.status,
        BudgetStatus::Submitted
    );
    assert!(t.budget_approved.all().is_empty());
}

/// A first 2028 version created as a copy of last year's budget has the lines
/// of the approved 2027 version moved to 2028; created empty it has no lines.
#[renox::test]
async fn a_first_budget_starts_empty_or_as_a_copy_of_last_years() {
    let t = boot_with(WlInvoicing::new().budget_revise_past_months(true)).await;
    let a = t.budget_preparer("Amal").await;
    let b = t.budget_approver("Bilal").await;
    approved_year(&t, "PGP", 2027, &a, &b).await;
    // Version 2 of 2027 changes December; it is the one in force.
    let revision = t.new_budget("PGP", 2027, StartChoice::Empty, &a).await;
    assert_eq!((revision.version, revision.start_choice), (2, StartChoice::Revision));
    let december = t.category("rent_utilities").await;
    budget_lines::set_lines(
        t.db(),
        revision.id,
        &[LineInput {
            category_id: december.id,
            month: revision.months()[11],
            amount: 2_500_000,
            note: Some("Rent went up".into()),
        }],
    )
    .await
    .unwrap();
    budget_lines::set_revision_reason(t.db(), revision.id, "Rent went up")
        .await
        .unwrap();
    approve(&t, &revision, &a, &b).await;

    let copy = t.new_budget("PGP", 2028, StartChoice::LastBudget, &a).await;
    assert_eq!((copy.version, copy.start_choice), (1, StartChoice::LastBudget));
    let copied = lines(&t, &copy).await;
    assert_eq!(copied.len(), 12);
    let first = NaiveDate::from_ymd_opt(2028, 1, 1).unwrap();
    assert_eq!(copied.iter().map(|l| l.month).min(), Some(first));
    assert!(copied.iter().all(|l| l.month.year() == 2028 && !l.locked));
    let last = copied
        .iter()
        .find(|l| l.month == NaiveDate::from_ymd_opt(2028, 12, 1).unwrap())
        .unwrap();
    assert_eq!((last.amount, last.note.as_deref()), (2_500_000, Some("Rent went up")));

    let empty = t.new_budget("PGP", 2029, StartChoice::Empty, &a).await;
    assert!(lines(&t, &empty).await.is_empty());

    // No approved budget last year to copy.
    let entity = t.entity("PGP").await;
    let refused = budget_lines::create(
        t.state(),
        NewBudget {
            legal_entity_id: entity.id,
            fiscal_year: 2031,
            name: String::new(),
            start: StartChoice::LastBudget,
            revision_reason: None,
        },
        Some(a.id),
    )
    .await;
    assert_refused(refused, "no approved budget");
}

/// A revision copies the approved lines, version + 1; the lines of the months
/// that had ended are locked and editing one is refused. A revision needs its
/// reason to be submitted. When approved, the version it replaced becomes
/// `superseded` in the same step. A third version is refused while version 2
/// is a draft.
#[renox::test]
async fn a_revision_locks_the_ended_months_and_supersedes_the_old_version() {
    // A fiscal year that started three months ago: three months have ended.
    let started = month_start(today())
        .checked_sub_months(Months::new(3))
        .unwrap();
    let t = boot_with(WlInvoicing::new().budget_fiscal_start_month(started.month())).await;
    let a = t.budget_preparer("Amal").await;
    let b = t.budget_approver("Bilal").await;
    let year = i64::from(started.year());
    let first = approved_year(&t, "PGP", year, &a, &b).await;
    assert_eq!(first.start_month, i64::from(started.month()));

    let revision = t.new_budget("PGP", year, StartChoice::Empty, &a).await;
    assert_eq!((revision.version, revision.start_choice), (2, StartChoice::Revision));
    let copied = lines(&t, &revision).await;
    assert_eq!(copied.len(), 12);
    let locked: Vec<bool> = {
        let mut ordered = copied.clone();
        ordered.sort_by_key(|l| l.month);
        ordered.iter().map(|l| l.locked).collect()
    };
    assert_eq!(&locked[..4], [true, true, true, false]);
    assert!(locked[4..].iter().all(|l| !l));

    // Editing a locked month is refused, a later month is allowed.
    let rent = t.category("rent_utilities").await;
    let months = revision.months();
    let set = |month: NaiveDate| LineInput {
        category_id: rent.id,
        month,
        amount: 1,
        note: None,
    };
    assert_refused(
        budget_lines::set_lines(t.db(), revision.id, &[set(months[2])]).await,
        "locked",
    );
    assert_refused(
        budget_lines::delete_line(t.db(), revision.id, rent.id, months[2]).await,
        "locked",
    );
    budget_lines::set_lines(t.db(), revision.id, &[set(months[3])])
        .await
        .unwrap();

    // A third version is refused while version 2 is a draft.
    let entity = t.entity("PGP").await;
    let third = budget_lines::create(
        t.state(),
        NewBudget {
            legal_entity_id: entity.id,
            fiscal_year: year,
            name: String::new(),
            start: StartChoice::Empty,
            revision_reason: None,
        },
        Some(a.id),
    )
    .await;
    assert_refused(third, "already has a draft");

    // Without its reason the revision is not submitted.
    t.post_as(&format!("/budgets/{}/submit", revision.id), &a, &[])
        .await
        .assert_redirect(&format!("/budgets/{}", revision.id));
    assert_eq!(t.budget_now(&revision).await.status, BudgetStatus::Draft);
    budget_lines::set_revision_reason(t.db(), revision.id, "Rent went up")
        .await
        .unwrap();
    let approved = approve(&t, &revision, &a, &b).await;
    assert_eq!(approved.status, BudgetStatus::Approved);
    assert_eq!(approved.supersedes_id, Some(first.id));
    assert_eq!(t.budget_now(&first).await.status, BudgetStatus::Superseded);
    let events = t.budget_approved.all();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].superseded, Some(CompanyBudgetId(first.id)));
    // One approved version for the entity and year.
    let approved_count = CompanyBudget::query()
        .where_(CompanyBudget::LEGAL_ENTITY_ID.eq(entity.id))
        .where_(CompanyBudget::FISCAL_YEAR.eq(year))
        .where_(CompanyBudget::STATUS.eq(BudgetStatus::Approved))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(approved_count, 1);
    // The lines of a submitted, approved or superseded version are not edited.
    assert_refused(
        budget_lines::set_lines(t.db(), revision.id, &[set(months[5])]).await,
        "draft",
    );
}

/// `submit` with a line whose category is inactive is refused.
#[renox::test]
async fn a_line_with_an_inactive_category_is_not_submitted() {
    let t = boot().await;
    let a = t.budget_preparer("Amal").await;
    let budget = t.new_budget("PGP", 2027, StartChoice::Empty, &a).await;
    t.fill(&budget, "marketing", 100_000, 1, 3).await;
    let marketing = t.category("marketing").await;
    BudgetCategory::query()
        .where_(BudgetCategory::ID.eq(marketing.id))
        .update(t.db(), &[("active", &false)])
        .await
        .unwrap();
    // A new line cannot use it either.
    assert_refused(
        budget_lines::set_lines(
            t.db(),
            budget.id,
            &[LineInput {
                category_id: marketing.id,
                month: budget.months()[4],
                amount: 5,
                note: None,
            }],
        )
        .await,
        "not found or not active",
    );
    t.post_as(&format!("/budgets/{}/submit", budget.id), &a, &[])
        .await
        .assert_redirect(&format!("/budgets/{}", budget.id));
    assert_eq!(t.budget_now(&budget).await.status, BudgetStatus::Draft);
    assert!(
        approvals::pending_for(t.db(), &budget.subject())
            .await
            .unwrap()
            .is_empty()
    );
    // A budget with no line is not submitted either.
    let empty = t.new_budget("PGFZ", 2027, StartChoice::Empty, &a).await;
    t.post_as(&format!("/budgets/{}/submit", empty.id), &a, &[])
        .await
        .assert_redirect(&format!("/budgets/{}", empty.id));
    assert_eq!(t.budget_now(&empty).await.status, BudgetStatus::Draft);
}

/// A submitted version whose approval expires is `draft` again with the
/// reason "approval expired" and the approved version stays in force. A
/// discarded draft needs a reason and frees the entity and year for a new
/// version.
#[renox::test]
async fn an_expired_approval_returns_the_draft_and_a_discard_frees_the_year() {
    let t = boot_with(WlInvoicing::new().budget_revise_past_months(true)).await;
    let a = t.budget_preparer("Amal").await;
    let b = t.budget_approver("Bilal").await;
    let first = approved_year(&t, "PGP", 2027, &a, &b).await;

    let revision = t.new_budget("PGP", 2027, StartChoice::Empty, &a).await;
    budget_lines::set_revision_reason(t.db(), revision.id, "Another look")
        .await
        .unwrap();
    t.submit_budget(&revision, &a).await;
    let request = t.budget_request(&revision).await;
    t.app
        .travel(std::time::Duration::from_secs((EXPIRY_HOURS as u64 + 1) * 60 * 60));
    t.app
        .at_travelled_time(approvals::expire(t.state(), request.id))
        .await
        .unwrap();
    let back = t.budget_now(&revision).await;
    assert_eq!(back.status, BudgetStatus::Draft);
    assert_eq!(back.return_reason.as_deref(), Some("approval expired"));
    assert_eq!(t.budget_now(&first).await.status, BudgetStatus::Approved);

    // A discard without a reason is refused.
    t.post_as(&format!("/budgets/{}/discard", revision.id), &a, &[("reason", " ")])
        .await
        .assert_status(422);
    assert_eq!(t.budget_now(&revision).await.status, BudgetStatus::Draft);
    t.post_as(
        &format!("/budgets/{}/discard", revision.id),
        &a,
        &[("reason", "Not needed")],
    )
    .await
    .assert_redirect("/budgets");
    let discarded = t.budget_now(&revision).await;
    assert_eq!(discarded.status, BudgetStatus::Discarded);
    assert_eq!(discarded.discard_reason.as_deref(), Some("Not needed"));
    assert_eq!(t.budget_now(&first).await.status, BudgetStatus::Approved);
    // The lines of a discarded version are not edited, and it is not reopened.
    assert_refused(
        budget_lines::set_lines(t.db(), discarded.id, &[]).await,
        "draft",
    );

    let next = t.new_budget("PGP", 2027, StartChoice::Empty, &a).await;
    assert_eq!(next.version, 3);
    assert_eq!(next.status, BudgetStatus::Draft);
}

/// Saving category *Fuel* mapped to expense category *fuel*, already mapped to
/// *Transport and fuel*, is refused. Other checks of a category: an owner
/// drawings category needs its owner party, the kind of a category with a
/// budget line cannot change, and exactly one category of that kind is
/// seeded.
#[renox::test]
async fn a_category_cannot_repeat_a_mapping_key_of_another() {
    let t = boot().await;
    let fuel = CategoryInput {
        code: "fuel".into(),
        name_en: "Fuel".into(),
        kind: BudgetKind::Expense,
        mappings: vec![(MappingKind::ExpenseCategory, "Fuel".into())],
        active: true,
        ..Default::default()
    };
    let refused = budget_category::save(t.db(), None, fuel.clone()).await;
    assert_refused(refused, "Transport and fuel");
    assert!(BudgetCategory::by_code(t.db(), "fuel").await.unwrap().is_none());

    // The same category with a key nobody uses is saved, and keeps it when
    // it is saved again.
    let free = CategoryInput {
        mappings: vec![(MappingKind::CostCode, "diesel".into())],
        ..fuel.clone()
    };
    let saved = budget_category::save(t.db(), None, free.clone()).await.unwrap();
    budget_category::save(t.db(), Some(saved.id), free).await.unwrap();
    let keys = budget_category::mappings_of(t.db(), saved.id).await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key_value, "diesel");
    assert_refused(
        budget_category::save(t.db(), None, CategoryInput { code: "diesel".into(), mappings: vec![(MappingKind::CostCode, "DIESEL ".into())], ..fuel.clone() })
            .await,
        "Fuel",
    );

    // A key of another kind does not fit the category.
    let wrong = CategoryInput {
        code: "wrong".into(),
        mappings: vec![(MappingKind::ChargeType, "x".into())],
        ..fuel.clone()
    };
    assert_refused(budget_category::save(t.db(), None, wrong).await, "cannot be mapped");

    // An owner drawings category needs its owner party.
    let drawings = CategoryInput {
        code: "partner_b".into(),
        name_en: "Partner B drawings".into(),
        kind: BudgetKind::OwnerDrawings,
        active: true,
        ..Default::default()
    };
    assert_refused(
        budget_category::save(t.db(), None, drawings.clone()).await,
        "owner party",
    );
    let with_party = CategoryInput {
        owner_party_id: Some(2),
        ..drawings
    };
    let partner = budget_category::save(t.db(), None, with_party).await.unwrap();
    assert_eq!(partner.owner_party_id, Some(2));
    let seeded = BudgetCategory::query()
        .where_(BudgetCategory::KIND.eq(BudgetKind::OwnerDrawings))
        .where_(BudgetCategory::CODE.eq("owner_drawings"))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(seeded, 1);

    // The kind of a category with a budget line cannot change.
    let a = t.budget_preparer("Amal").await;
    let budget = t.new_budget("PGP", 2027, StartChoice::Empty, &a).await;
    t.fill(&budget, "marketing", 1, 1, 1).await;
    let marketing = t.category("marketing").await;
    let change = CategoryInput {
        code: "marketing".into(),
        name_en: "Marketing".into(),
        kind: BudgetKind::Income,
        active: true,
        ..Default::default()
    };
    assert_refused(
        budget_category::save(t.db(), Some(marketing.id), change).await,
        "kind cannot change",
    );
    // Seeding again changes nothing.
    assert_eq!(budget_category::install(t.db()).await.unwrap(), 0);
}

/// On 2027-12-02, with no approved 2028 budget for an entity, the accountant
/// and the finance manager are notified (`FIN.RUL.26`); once; again on the
/// first day of the year if still none; a version approved in between stops
/// the second notice.
#[renox::test]
async fn the_accountant_and_the_finance_manager_are_told_before_a_year_without_a_budget() {
    let t = boot().await;
    let accountant = t.accountant("Aya").await;
    let manager = {
        let user = t.helper("Fares", &[perm::BUDGETS_VIEW]).await;
        user.assign_role(t.db(), "finance_manager").await.unwrap();
        user
    };
    let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();

    assert_eq!(budget_reminder::run(t.state(), day(2027, 12, 1)).await.unwrap(), 0);
    // Two entities, so two notices, each to both people.
    assert_eq!(budget_reminder::run(t.state(), day(2027, 12, 2)).await.unwrap(), 2);
    let sent = t.notices.of_type("invoicing.budget_reminder");
    assert_eq!(sent.len(), 2);
    for request in &sent {
        let mut ids: Vec<i64> = request
            .recipients
            .iter()
            .filter_map(|r| match r {
                NotifyRecipient::User { id } => Some(*id),
                _ => None,
            })
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, [accountant.id.min(manager.id), accountant.id.max(manager.id)]);
    }
    // Once at N days.
    assert_eq!(budget_reminder::run(t.state(), day(2027, 12, 20)).await.unwrap(), 0);

    // PGP approves its 2028 budget; only PGFZ is told on the first day.
    let a = t.budget_preparer("Amal").await;
    let b = t.budget_approver("Bilal").await;
    approved_year(&t, "PGP", 2028, &a, &b).await;
    assert_eq!(budget_reminder::run(t.state(), day(2028, 1, 1)).await.unwrap(), 1);
    assert_eq!(t.notices.of_type("invoicing.budget_reminder").len(), 3);
    assert_eq!(budget_reminder::run(t.state(), day(2028, 1, 2)).await.unwrap(), 0);
}

/// The CSV import fills a draft: a dry run reads and checks everything and
/// stores nothing; a row with an unknown category, a month outside the year
/// or a bad amount is rejected with its row number; two rows with the same
/// category and month are rejected with both row numbers; a file with a
/// rejected row stores nothing.
#[renox::test]
async fn the_csv_import_checks_every_row_first() {
    let t = boot().await;
    let a = t.budget_preparer("Amal").await;
    let budget = t.new_budget("PGP", 2027, StartChoice::Empty, &a).await;
    let good = "category,month,amount,note\nmarketing,2027-03,10000.00,Q1 push\nrent_utilities,2027-03,\"20,000\",\n";

    let dry = budget_lines::import_csv(t.db(), budget.id, good, true).await.unwrap();
    assert_eq!((dry.rows, dry.valid, dry.applied), (2, 2, false));
    assert!(dry.errors.is_empty());
    assert!(lines(&t, &budget).await.is_empty());

    let bad = "category,month,amount\nmarketing,2027-03,10\nfuell,2027-03,5\nmarketing,2027-13,5\nmarketing,2027-04,abc\nmarketing,2028-01,5\nmarketing,2027-03,7\n";
    let report = budget_lines::import_csv(t.db(), budget.id, bad, false).await.unwrap();
    let rows: Vec<usize> = report.errors.iter().map(|e| e.row).collect();
    assert_eq!(rows, [2, 3, 4, 5, 6, 7]);
    assert!(report.errors[0].message.contains("rows 2 and 7") || report.errors[0].message.contains("Rows 2 and 7"));
    assert!(report.errors[1].message.contains("fuell"));
    assert!(!report.applied);
    assert!(lines(&t, &budget).await.is_empty());

    let done = budget_lines::import_csv(t.db(), budget.id, good, false).await.unwrap();
    assert!(done.applied);
    let stored = lines(&t, &budget).await;
    assert_eq!(stored.len(), 2);
    let marketing = t.category("marketing").await;
    let line = stored.iter().find(|l| l.category_id == marketing.id).unwrap();
    assert_eq!((line.amount, line.note.as_deref()), (1_000_000, Some("Q1 push")));
    assert_eq!(line.month, NaiveDate::from_ymd_opt(2027, 3, 1).unwrap());

    // Semicolons, as a workbook saved in some locales has them.
    let semicolons = "category;month;amount\nmarketing;2027-04;5\n";
    assert!(
        budget_lines::import_csv(t.db(), budget.id, semicolons, false)
            .await
            .unwrap()
            .applied
    );

    // Only a draft is filled.
    t.submit_budget(&budget, &a).await;
    assert_eq!(t.budget_now(&budget).await.status, BudgetStatus::Submitted);
    assert_refused(
        budget_lines::import_csv(t.db(), budget.id, good, false).await,
        "draft",
    );
}

/// The pages: the list and the page of a version for a holder of
/// `budgets.view`, the sheets and the form for a holder of `budgets.manage`;
/// the sheets set an amount over a range, import CSV and discard.
#[renox::test]
async fn the_budget_pages_follow_the_permissions() {
    let t = boot().await;
    let a = t.budget_preparer("Amal").await;
    let viewer = t.helper("Vera", &[perm::BUDGETS_VIEW]).await;
    let nobody = t.helper("Nadia", &[perm::VIEW]).await;
    let budget = t.new_budget("PGP", 2027, StartChoice::Empty, &a).await;
    t.fill(&budget, "rent_utilities", RENT, 1, 12).await;
    let page = format!("/budgets/{}", budget.id);

    t.app.acting_as(&viewer);
    t.app.get("/budgets").await.assert_ok().assert_see("Company budgets");
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Rent and utilities")
        .assert_see("20,000.00")
        .assert_see("Jan 2027");
    t.app.get("/budget-categories").await.assert_ok().assert_see("Owner drawings");
    t.app.get("/budgets/new").await.assert_status(403);
    t.post_as(&format!("{page}/submit"), &viewer, &[]).await.assert_status(403);
    t.app.acting_as(&nobody);
    t.app.get("/budgets").await.assert_status(403);

    t.app.acting_as(&a);
    t.app.get("/budgets/new").await.assert_ok().assert_see("Fiscal year");
    t.app.get(&page).await.assert_ok().assert_see("Submit for approval");

    // The set sheet: the same amount over a range of months.
    let marketing = t.category("marketing").await.id.to_string();
    t.post_as(
        &format!("{page}/lines"),
        &a,
        &[
            ("category_id", marketing.as_str()),
            ("from", "2027-02-01"),
            ("to", "2027-04-01"),
            ("amount", "1,500.50"),
        ],
    )
    .await
    .assert_ok();
    let stored = lines(&t, &budget).await;
    let months: Vec<_> = stored
        .iter()
        .filter(|l| l.category_id == marketing.parse::<i64>().unwrap())
        .map(|l| (l.month.month(), l.amount))
        .collect();
    assert_eq!(months, [(2, 150_050), (3, 150_050), (4, 150_050)]);
    t.post_as(
        &format!("{page}/lines"),
        &a,
        &[("category_id", marketing.as_str()), ("from", "2027-05-01"), ("amount", "-1")],
    )
    .await
    .assert_status(422);
    t.post_as(
        &format!("{page}/lines/clear"),
        &a,
        &[("category_id", marketing.as_str()), ("month", "2027-04-01")],
    )
    .await
    .assert_ok();
    assert_eq!(lines(&t, &budget).await.len(), 14);

    // The import sheet: a dry run, then the lines; a rejected row answers 422.
    let ok = "marketing,2027-06,100\n";
    t.post_as(&format!("{page}/import"), &a, &[("csv", ok), ("dry_run", "on")])
        .await
        .assert_ok();
    assert_eq!(lines(&t, &budget).await.len(), 14);
    t.post_as(&format!("{page}/import"), &a, &[("csv", ok)]).await.assert_ok();
    assert_eq!(lines(&t, &budget).await.len(), 15);
    t.post_as(&format!("{page}/import"), &a, &[("csv", "nothing,2027-06,100\n")])
        .await
        .assert_status(422);

    // The form makes a draft and opens its page.
    let pgfz = t.entity("PGFZ").await.id.to_string();
    t.post_as(
        "/budgets",
        &a,
        &[
            ("legal_entity_id", pgfz.as_str()),
            ("fiscal_year", "2028"),
            ("name", "Facilities 2028"),
            ("start", "empty"),
        ],
    )
    .await;
    let facilities = CompanyBudget::query()
        .where_(CompanyBudget::FISCAL_YEAR.eq(2028))
        .first(t.db())
        .await
        .unwrap()
        .expect("the draft is made");
    assert_eq!(facilities.name, "Facilities 2028");
    t.post_as(
        "/budgets",
        &a,
        &[("legal_entity_id", pgfz.as_str()), ("fiscal_year", "2028"), ("start", "empty")],
    )
    .await
    .assert_status(422);
}

/// The category pages: a repeated mapping key answers 422 under the form, a
/// new category is saved and listed.
#[renox::test]
async fn the_category_form_saves_and_refuses() {
    let t = boot().await;
    let a = t.budget_preparer("Amal").await;
    let admin = t.helper("Ada", &[perm::BUDGETS_VIEW, perm::BUDGET_CATEGORIES_MANAGE]).await;
    t.post_as(
        "/budget-categories",
        &admin,
        &[
            ("code", "fuel"),
            ("name_en", "Fuel"),
            ("kind", "expense"),
            ("mappings", "expense_category: fuel"),
            ("active", "on"),
        ],
    )
    .await
    .assert_status(422);
    t.post_as(
        "/budget-categories",
        &admin,
        &[
            ("code", "fuel"),
            ("name_en", "Fuel"),
            ("kind", "expense"),
            ("mappings", "cost_code: diesel"),
            ("active", "on"),
        ],
    )
    .await;
    t.app.acting_as(&admin);
    t.app.get("/budget-categories").await.assert_ok().assert_see("diesel");
    t.app.get("/budget-categories/new").await.assert_ok();
    // A preparer without the permission cannot save one.
    t.post_as("/budget-categories", &a, &[("code", "x"), ("name_en", "X"), ("kind", "expense")])
        .await
        .assert_status(403);
}

/// The kernel contract: the approved amount of a category in a month, `None`
/// without an approved budget, 0 for a category with no line, and whether a
/// category is owner drawings.
#[renox::test]
async fn the_kernel_reads_the_approved_budget() {
    let t = boot().await;
    let a = t.budget_preparer("Amal").await;
    let b = t.budget_approver("Bilal").await;
    let entity = t.entity("PGP").await;
    let budgets = company_budgets(t.state());
    let march = NaiveDate::from_ymd_opt(2027, 3, 17).unwrap();
    assert_eq!(
        budgets.approved_amount(t.db(), entity.id, "rent_utilities", march).await.unwrap(),
        None
    );
    let draft = t.new_budget("PGP", 2027, StartChoice::Empty, &a).await;
    t.fill(&draft, "rent_utilities", RENT, 1, 12).await;
    // A draft is not in force.
    assert_eq!(
        budgets.approved_amount(t.db(), entity.id, "rent_utilities", march).await.unwrap(),
        None
    );
    approve(&t, &draft, &a, &b).await;
    assert_eq!(
        budgets.approved_amount(t.db(), entity.id, "rent_utilities", march).await.unwrap(),
        Some(RENT)
    );
    assert_eq!(
        budgets.approved_amount(t.db(), entity.id, "marketing", march).await.unwrap(),
        Some(0)
    );
    let other = t.entity("PGFZ").await;
    assert_eq!(
        budgets.approved_amount(t.db(), other.id, "rent_utilities", march).await.unwrap(),
        None
    );
    assert_eq!(
        budgets
            .approved_amount(t.db(), entity.id, "rent_utilities", NaiveDate::from_ymd_opt(2028, 1, 1).unwrap())
            .await
            .unwrap(),
        None
    );
    assert!(budgets.is_owner_drawings(t.db(), "owner_drawings").await.unwrap());
    assert!(!budgets.is_owner_drawings(t.db(), "marketing").await.unwrap());
    assert!(!budgets.is_owner_drawings(t.db(), "nothing").await.unwrap());
}
