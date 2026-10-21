//! BL-CON-09 (#288): registering, reducing, extending, releasing and
//! expiring a guarantee (`CON.RUL.11`). The accountant registers the
//! guarantee and runs `issue`, `reduce`, `extend` and `release` with the bank
//! document of each step; 60 days before the expiry the accountant gets a task
//! to ask the bank for an extension and the project manager is told; the day
//! after the expiry the system expires the guarantee and tells both.

mod support;

use construction::expiry;
use construction::guarantee::documents_of;
use construction::{Guarantee, GuaranteeKind, GuaranteeStatus, Project, ProjectStatus};
use renox::prelude::*;
use wl_kernel::documents::dubai_date;
use wl_kernel::tickets::Assignee;

use support::{Booted, boot, date, users_of};

/// AED in fils.
const fn aed(amount: i64) -> i64 {
    amount * 100
}

fn path(guarantee: &Guarantee, step: &str) -> String {
    format!("/construction/guarantees/{}/{step}", guarantee.id)
}

/// A project in `status` whose defects liability period ends on 2028-06-15.
async fn project(t: &Booted, manager: &User, status: ProjectStatus) -> Project {
    let project = t.project(status, Some(manager)).await;
    let ends = Some(date(2028, 6, 15));
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(t.db(), &[("defects_liability_ends_on", &ends)])
        .await
        .unwrap();
    t.reload(&project).await
}

/// An `active` performance bond of AED 500,000 expiring on 2027-09-30.
async fn bond(t: &Booted, project: &Project) -> Guarantee {
    t.guarantee(
        project,
        GuaranteeKind::Performance,
        aed(500_000),
        GuaranteeStatus::Active,
        Some(date(2027, 9, 30)),
    )
    .await
}

/// Done when 1 and 2: a performance bond AED 500,000 expiring 2027-09-30 on a
/// project whose defects liability period ends 2028-06-15 gives the
/// accountant a task on 2027-08-01 and the project manager a notice; `extend`
/// to 2028-07-31 with the bank amendment resets the reminders, and the next
/// task comes on 2028-06-01.
#[renox::test]
async fn the_accountant_asks_the_bank_for_an_extension_sixty_days_before_the_expiry() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = project(&t, &pm, ProjectStatus::PracticalCompletion).await;
    let held = bond(&t, &project).await;

    t.on(date(2027, 7, 31), async {
        expiry::daily(t.state()).await.unwrap();
    })
    .await;
    assert!(
        t.tasks
            .with_key("construction.guarantee_extension:")
            .is_empty()
    );

    t.on(date(2027, 8, 1), async {
        expiry::daily(t.state()).await.unwrap();
        // The same day again makes nothing new.
        expiry::daily(t.state()).await.unwrap();
    })
    .await;
    let tasks = t.tasks.with_key("construction.guarantee_extension:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].ticket.source_key.as_deref(),
        Some(format!("construction.guarantee_extension:{}:2027-09-30", held.id).as_str())
    );
    assert_eq!(
        tasks[0].ticket.assignee,
        Some(Assignee::user(accountant.id))
    );
    assert!(tasks[0].ticket.title.contains("performance bond"));
    assert!(tasks[0].ticket.description.contains("AED 500,000.00"));
    assert_eq!(dubai_date(tasks[0].at), date(2027, 8, 1));
    assert_eq!(
        tasks[0].ticket.due_at.map(dubai_date),
        Some(date(2027, 9, 30))
    );
    let told = t.notices.notices_titled("expires on 2027-09-30");
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), [pm.id]);
    assert_eq!(
        t.reload_guarantee(held.id).await.reminded_for,
        Some(date(2027, 9, 30))
    );

    // The extension with the bank's amendment resets the reminders.
    t.on(date(2027, 9, 1), async {
        t.post(
            &accountant,
            &path(&held, "extend"),
            &[
                ("expires_on", "2028-07-31"),
                ("document", "AMENDMENT-1.pdf"),
            ],
        )
        .await
        .assert_ok();
    })
    .await;
    let extended = t.reload_guarantee(held.id).await;
    assert_eq!(extended.status, GuaranteeStatus::Active);
    assert_eq!(extended.expires_on, Some(date(2028, 7, 31)));
    assert_eq!(extended.reminded_for, None);
    let documents = documents_of(t.db(), held.id).await.unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].transition, "extend");
    assert_eq!(documents[0].reference, "AMENDMENT-1.pdf");

    t.on(date(2028, 5, 31), async {
        expiry::daily(t.state()).await.unwrap();
    })
    .await;
    assert_eq!(
        t.tasks.with_key("construction.guarantee_extension:").len(),
        1,
        "not yet"
    );
    t.on(date(2028, 6, 1), async {
        expiry::daily(t.state()).await.unwrap();
    })
    .await;
    let tasks = t.tasks.with_key("construction.guarantee_extension:");
    assert_eq!(tasks.len(), 2);
    assert_eq!(
        tasks[1].ticket.source_key.as_deref(),
        Some(format!("construction.guarantee_extension:{}:2028-07-31", held.id).as_str())
    );
    assert_eq!(dubai_date(tasks[1].at), date(2028, 6, 1));
    assert_eq!(t.notices.notices_titled("expires on 2028-07-31").len(), 1);
}

/// `CON.RUL.11` asks for an extension only while the project has not reached
/// the guarantee's release point: completion for the advance guarantee, the
/// end of the defects liability period for a bond.
#[renox::test]
async fn no_task_once_the_project_reached_the_release_point() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let _accountant = t.accountant("Aisha").await;
    // Completed: an advance guarantee has done its work, a bond has not.
    let completed = project(&t, &pm, ProjectStatus::PracticalCompletion).await;
    let advance = t
        .guarantee(
            &completed,
            GuaranteeKind::Advance,
            aed(150_000),
            GuaranteeStatus::Active,
            Some(date(2027, 9, 30)),
        )
        .await;
    let performance = bond(&t, &completed).await;
    // The defects liability period has ended: the bond is released, not
    // extended.
    let ended = project(&t, &pm, ProjectStatus::PracticalCompletion).await;
    Project::query()
        .where_(Project::ID.eq(ended.id))
        .update(
            t.db(),
            &[("defects_liability_ends_on", &Some(date(2027, 8, 10)))],
        )
        .await
        .unwrap();
    let released_soon = bond(&t, &ended).await;
    // A guarantee that was called is not extended, and does not expire.
    let called = t
        .guarantee(
            &completed,
            GuaranteeKind::Performance,
            aed(10_000),
            GuaranteeStatus::Called,
            Some(date(2027, 9, 30)),
        )
        .await;

    t.on(date(2027, 8, 15), async {
        // 2027-08-01 was missed: the run makes the task up.
        let run = expiry::run_on(t.state(), date(2027, 8, 15)).await.unwrap();
        assert_eq!(run.reminded.len(), 1);
        assert_eq!(run.reminded[0].id, performance.id);
        assert!(run.expired.is_empty());
    })
    .await;
    let keys = t.tasks.with_key("construction.guarantee_extension:");
    assert_eq!(keys.len(), 1);
    for other in [&advance, &released_soon, &called] {
        assert_eq!(
            t.reload_guarantee(other.id).await.reminded_for,
            None,
            "{:?}",
            other.kind
        );
    }
}

/// Done when 3: an advance guarantee of AED 150,000 with unrecovered advance
/// AED 90,000 cannot be reduced to AED 80,000; AED 90,000 is allowed with the
/// amendment.
#[renox::test]
async fn an_advance_guarantee_is_not_reduced_below_the_unrecovered_advance() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = project(&t, &pm, ProjectStatus::InProgress).await;
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(
            t.db(),
            &[
                ("advance_paid", &aed(150_000)),
                ("advance_recovered", &aed(60_000)),
            ],
        )
        .await
        .unwrap();
    let advance = t
        .guarantee(
            &project,
            GuaranteeKind::Advance,
            aed(150_000),
            GuaranteeStatus::Active,
            Some(date(2027, 12, 31)),
        )
        .await;
    let reduce = path(&advance, "reduce");

    let refused = t
        .post(
            &accountant,
            &reduce,
            &[
                ("amount", &aed(80_000).to_string()),
                ("document", "AMENDMENT-3.pdf"),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("AED 90,000.00"),
        "the unrecovered advance is shown: {}",
        refused.text()
    );
    assert_eq!(t.reload_guarantee(advance.id).await.amount, aed(150_000));

    // Without the amendment, or by a user who may not manage guarantees.
    t.post(
        &accountant,
        &reduce,
        &[("amount", &aed(90_000).to_string()), ("document", " ")],
    )
    .await
    .assert_status(422);
    t.post(
        &pm,
        &reduce,
        &[
            ("amount", &aed(90_000).to_string()),
            ("document", "AMENDMENT-3.pdf"),
        ],
    )
    .await
    .assert_status(403);
    // It must reduce.
    t.post(
        &accountant,
        &reduce,
        &[
            ("amount", &aed(150_000).to_string()),
            ("document", "AMENDMENT-3.pdf"),
        ],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reload_guarantee(advance.id).await.amount, aed(150_000));

    t.post(
        &accountant,
        &reduce,
        &[
            ("amount", &aed(90_000).to_string()),
            ("document", "AMENDMENT-3.pdf"),
        ],
    )
    .await
    .assert_ok();
    let reduced = t.reload_guarantee(advance.id).await;
    assert_eq!(reduced.status, GuaranteeStatus::Active);
    assert_eq!(reduced.amount, aed(90_000));
    let documents = documents_of(t.db(), advance.id).await.unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].transition, "reduce");

    // A performance bond may be reduced to any amount.
    let performance = t
        .guarantee(
            &project,
            GuaranteeKind::Performance,
            aed(500_000),
            GuaranteeStatus::Active,
            Some(date(2027, 12, 31)),
        )
        .await;
    t.post(
        &accountant,
        &path(&performance, "reduce"),
        &[
            ("amount", &aed(10_000).to_string()),
            ("document", "CONSENT-1.pdf"),
        ],
    )
    .await
    .assert_ok();
    assert_eq!(t.reload_guarantee(performance.id).await.amount, aed(10_000));
}

/// Done when 4: with no extension, on 2027-10-01 the bond is `expired` and the
/// accountant and project manager are notified. On the expiry day itself it is
/// still `active`.
#[renox::test]
async fn a_guarantee_expires_the_day_after_its_expiry_and_tells_both() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = project(&t, &pm, ProjectStatus::PracticalCompletion).await;
    let held = bond(&t, &project).await;
    // A draft is not issued yet and never expires.
    let other = t
        .guarantee(
            &project,
            GuaranteeKind::Performance,
            aed(500_000),
            GuaranteeStatus::Draft,
            Some(date(2027, 1, 1)),
        )
        .await;

    t.on(date(2027, 9, 30), async {
        let run = expiry::run_on(t.state(), date(2027, 9, 30)).await.unwrap();
        assert!(run.expired.is_empty());
    })
    .await;
    assert_eq!(
        t.reload_guarantee(held.id).await.status,
        GuaranteeStatus::Active
    );
    assert!(t.notices.notices_titled("expired").is_empty());

    t.on(date(2027, 10, 1), async {
        expiry::daily(t.state()).await.unwrap();
    })
    .await;
    let expired = t.reload_guarantee(held.id).await;
    assert_eq!(expired.status, GuaranteeStatus::Expired);
    assert_eq!(
        t.reload_guarantee(other.id).await.status,
        GuaranteeStatus::Draft
    );
    let told = t.notices.notices_titled("expired");
    assert_eq!(told.len(), 2, "the accountant and the project manager");
    let mut users: Vec<i64> = told.iter().flat_map(users_of).collect();
    users.sort_unstable();
    let mut wanted = vec![accountant.id, pm.id];
    wanted.sort_unstable();
    assert_eq!(users, wanted);
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("2027-09-30")
    );

    // It expires once.
    t.on(date(2027, 10, 2), async {
        let run = expiry::run_on(t.state(), date(2027, 10, 2)).await.unwrap();
        assert!(run.expired.is_empty());
    })
    .await;
    assert_eq!(t.notices.notices_titled("expired").len(), 2);
    // An expired guarantee is finished: it cannot be extended or released.
    t.post(
        &accountant,
        &path(&expired, "release"),
        &[("document", "CANCELLATION-LETTER.pdf")],
    )
    .await
    .assert_status(422);
}

/// Done when 5: `release` without the original or the bank's cancellation
/// letter is refused; with it the guarantee is `released`.
#[renox::test]
async fn a_release_needs_the_original_or_the_cancellation_letter() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = project(&t, &pm, ProjectStatus::PracticalCompletion).await;
    let held = bond(&t, &project).await;
    let release = path(&held, "release");

    let refused = t.post(&accountant, &release, &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("original"), "{}", refused.text());
    t.post(&accountant, &release, &[("document", "  ")])
        .await
        .assert_status(422);
    t.post(&pm, &release, &[("document", "CANCELLATION-LETTER.pdf")])
        .await
        .assert_status(403);
    assert_eq!(
        t.reload_guarantee(held.id).await.status,
        GuaranteeStatus::Active
    );

    t.post(
        &accountant,
        &release,
        &[("document", "CANCELLATION-LETTER.pdf")],
    )
    .await
    .assert_ok();
    let released = t.reload_guarantee(held.id).await;
    assert_eq!(released.status, GuaranteeStatus::Released);
    let documents = documents_of(t.db(), held.id).await.unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].transition, "release");
    assert_eq!(documents[0].reference, "CANCELLATION-LETTER.pdf");
    // A released guarantee is finished.
    t.post(&accountant, &release, &[("document", "AGAIN.pdf")])
        .await
        .assert_status(422);
}

/// The accountant registers a guarantee as a `draft` and issues it with the
/// bank's document once its amount, beneficiary and expiry are set; the
/// extension must be later than the expiry and have its amendment.
#[renox::test]
async fn a_guarantee_is_registered_issued_and_extended_with_its_documents() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = project(&t, &pm, ProjectStatus::InProgress).await;
    let register = format!("/construction/projects/{}/guarantees", project.id);
    let amount = aed(500_000).to_string();
    let fields = |kind: &'static str| {
        vec![
            ("kind", kind),
            ("bank", "Emirates NBD"),
            ("beneficiary", "Gulf Build Contracting LLC"),
            ("amount", amount.as_str()),
            ("expires_on", "2027-09-30"),
        ]
    };

    t.post(&accountant, &register, &fields("sparkle"))
        .await
        .assert_status(422);
    t.post(&pm, &register, &fields("performance"))
        .await
        .assert_status(403);
    t.post(&accountant, &register, &fields("performance"))
        .await
        .assert_ok();

    let drafts = Guarantee::query()
        .where_(Guarantee::PROJECT_ID.eq(project.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(drafts.len(), 1);
    let draft = &drafts[0];
    assert_eq!(draft.status, GuaranteeStatus::Draft);
    assert_eq!(draft.kind, GuaranteeKind::Performance);
    assert_eq!(draft.bank, "Emirates NBD");
    assert_eq!(draft.beneficiary, "Gulf Build Contracting LLC");
    assert_eq!(draft.amount, aed(500_000));
    assert_eq!(draft.expires_on, Some(date(2027, 9, 30)));

    // `issue` needs the bank's document, and only a draft is issued.
    t.post(&accountant, &path(draft, "issue"), &[])
        .await
        .assert_status(422);
    t.post(
        &accountant,
        &path(draft, "extend"),
        &[("document", "A.pdf"), ("expires_on", "2028-01-01")],
    )
    .await
    .assert_status(422);
    t.post(
        &accountant,
        &path(draft, "issue"),
        &[("document", "BANK-GUARANTEE-1.pdf")],
    )
    .await
    .assert_ok();
    let issued = t.reload_guarantee(draft.id).await;
    assert_eq!(issued.status, GuaranteeStatus::Active);
    let documents = documents_of(t.db(), draft.id).await.unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].transition, "issue");
    t.post(
        &accountant,
        &path(draft, "issue"),
        &[("document", "BANK-GUARANTEE-1.pdf")],
    )
    .await
    .assert_status(422);

    // A guarantee without a beneficiary or an expiry is not issued.
    let bare = t
        .guarantee(
            &project,
            GuaranteeKind::Advance,
            aed(1_000),
            GuaranteeStatus::Draft,
            None,
        )
        .await;
    let refused = t
        .post(&accountant, &path(&bare, "issue"), &[("document", "B.pdf")])
        .await;
    refused.assert_status(422);

    // The extension is later than the expiry and has its amendment.
    let extend = path(&issued, "extend");
    for fields in [
        vec![
            ("expires_on", "2027-09-30"),
            ("document", "AMENDMENT-1.pdf"),
        ],
        vec![
            ("expires_on", "2027-01-01"),
            ("document", "AMENDMENT-1.pdf"),
        ],
        vec![("expires_on", "2028-07-31"), ("document", "")],
        vec![("document", "AMENDMENT-1.pdf")],
    ] {
        t.post(&accountant, &extend, &fields)
            .await
            .assert_status(422);
    }
    assert_eq!(
        t.reload_guarantee(issued.id).await.expires_on,
        Some(date(2027, 9, 30)),
        "the expiry does not change without the bank document"
    );
}

/// Project `close` gives the accountant one release task per `active`
/// performance or retention bond; an advance guarantee and a guarantee that
/// is not active get none.
#[renox::test]
async fn closing_the_project_gives_the_accountant_a_release_task_per_active_bond() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = project(&t, &pm, ProjectStatus::PracticalCompletion).await;
    let performance = bond(&t, &project).await;
    let retention_bond = t
        .guarantee(
            &project,
            GuaranteeKind::RetentionBond,
            aed(50_000),
            GuaranteeStatus::Active,
            Some(date(2028, 12, 31)),
        )
        .await;
    for (kind, status) in [
        (GuaranteeKind::Advance, GuaranteeStatus::Active),
        (GuaranteeKind::Performance, GuaranteeStatus::Released),
        (GuaranteeKind::RetentionBond, GuaranteeStatus::Draft),
    ] {
        t.guarantee(&project, kind, aed(1_000), status, None).await;
    }

    t.on(date(2028, 6, 16), async {
        t.post(
            &pm,
            &format!("/construction/projects/{}/close", project.id),
            &[],
        )
        .await
        .assert_ok();
    })
    .await;

    assert_eq!(t.reload(&project).await.status, ProjectStatus::Closed);
    let tasks = t.tasks.with_key("construction.guarantee_release:");
    assert_eq!(tasks.len(), 2);
    let keys: Vec<String> = tasks
        .iter()
        .map(|task| task.ticket.source_key.clone().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            format!("construction.guarantee_release:{}", performance.id),
            format!("construction.guarantee_release:{}", retention_bond.id),
        ]
    );
    for task in &tasks {
        assert_eq!(task.ticket.assignee, Some(Assignee::user(accountant.id)));
    }
    assert!(tasks[1].ticket.title.contains("retention bond"));
}
