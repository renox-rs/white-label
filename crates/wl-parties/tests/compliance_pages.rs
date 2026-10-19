//! The back office of a confirmed match (#39, part 5): the confirm action
//! with its "waiting for a second reviewer" state on the screening task page,
//! the freeze and report task pages, and the 404 and 403 rules.

mod support;

use renox::auth::permissions;
use renox::prelude::*;
use wl_kernel::PartyKind;
use wl_kernel::approvals::{self, ApprovalEnd};
use wl_parties::{
    ComplianceTask, ComplianceTaskFund, ComplianceTaskKind, ComplianceTaskStatus, NewCandidate,
    NewParty, Party, Screening, ScreeningRun, ScreeningTask, ScreeningTrigger, WlParties, create,
    permissions as perm, record_run,
};

use support::{Booted, boot_custom};

async fn holder(t: &Booted, name: &str) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(t.db(), name, &email, "password123")
        .await
        .unwrap();
    permissions::define_role(
        t.db(),
        "compliance",
        &[
            perm::VIEW,
            perm::BLOCK,
            perm::BLOCK_REASON_VIEW,
            perm::SCREENING_REVIEW,
        ],
    )
    .await
    .unwrap();
    user.assign_role(t.db(), "compliance").await.unwrap();
    user
}

struct World {
    t: Booted,
    a: User,
    b: User,
    /// Reads the tasks but may not review.
    reader: User,
    /// Sees parties only.
    partner: User,
    omar: Party,
}

async fn world() -> World {
    let t = boot_custom(WlParties::new().screening(true), false, |app| app).await;
    let a = holder(&t, "Alia").await;
    let b = holder(&t, "Basma").await;
    let reader = t.user("Rana", &[perm::VIEW, perm::BLOCK_REASON_VIEW]).await;
    let partner = t.user("Pia", &[perm::VIEW]).await;
    let omar = create(
        t.app.state(),
        NewParty {
            kind: PartyKind::Person,
            legal_name_en: "Omar K.".into(),
            source: "test".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    World {
        t,
        a,
        b,
        reader,
        partner,
        omar,
    }
}

impl World {
    async fn flag(&self) -> ScreeningTask {
        let candidate = NewCandidate {
            name: "Omar K.".into(),
            list_name: "UN consolidated".into(),
            score: 0.91,
            list_entry: Some("QDi.001".into()),
        };
        record_run(
            self.t.app.state(),
            &self.omar,
            ScreeningRun::automatic(ScreeningTrigger::Review, vec![candidate]),
            None,
        )
        .await
        .unwrap();
        ScreeningTask::open_for_party(self.t.db(), self.omar.id)
            .await
            .unwrap()
            .expect("a task")
    }

    async fn tasks(&self, kind: ComplianceTaskKind) -> ComplianceTask {
        ComplianceTask::query()
            .where_(ComplianceTask::KIND.eq(kind))
            .first(self.t.db())
            .await
            .unwrap()
            .expect("a task")
    }
}

#[renox::test]
async fn the_confirm_action_waits_for_a_second_reviewer_and_opens_the_tasks() {
    let w = world().await;
    let task = w.flag().await;
    let detail = format!("/parties/screening-tasks/{}", task.id);
    let confirm = format!("{detail}/confirm");

    // Reviewer A sees the button; a reader without `screening.review` does not.
    w.t.app.acting_as(&w.a);
    w.t.app
        .get(&detail)
        .await
        .assert_ok()
        .assert_see("Confirm match");
    w.t.app.acting_as(&w.reader);
    w.t.app
        .get(&detail)
        .await
        .assert_ok()
        .assert_dont_see("Confirm match");
    w.t.app
        .post(&confirm, &[("note", "Same DOB")])
        .await
        .assert_forbidden();

    // A blank note is refused.
    w.t.app.acting_as(&w.a);
    w.t.app
        .htmx()
        .post(&confirm, &[("note", "  ")])
        .await
        .assert_invalid("note");

    // A confirms: the page now waits for a second reviewer.
    w.t.app
        .post(&confirm, &[("note", "Same DOB")])
        .await
        .assert_ok();
    let page = w.t.app.get(&detail).await;
    page.assert_ok()
        .assert_see("Waiting for a second reviewer")
        .assert_dont_see("Confirm match");
    assert!(
        ComplianceTask::open_tasks(w.t.db())
            .await
            .unwrap()
            .is_empty()
    );

    // B approves: the tasks exist and the page links to them.
    let request = wl_kernel::ApprovalRequest::query()
        .first(w.t.db())
        .await
        .unwrap()
        .expect("a request");
    let decided = approvals::approve(w.t.app.state(), request.id, &w.b, "ok")
        .await
        .unwrap()
        .expect("ended");
    assert_eq!(decided.decision, ApprovalEnd::Approved);
    let freeze = w.tasks(ComplianceTaskKind::Freeze).await;
    let report = w.tasks(ComplianceTaskKind::Report).await;
    w.t.app.acting_as(&w.a);
    w.t.app
        .get(&detail)
        .await
        .assert_ok()
        .assert_see(&format!("/parties/compliance-tasks/{}", freeze.id));

    // The list and the freeze page.
    let list = w.t.app.get("/parties/compliance-tasks").await;
    list.assert_ok()
        .assert_see(&format!("/parties/compliance-tasks/{}", freeze.id))
        .assert_see(&format!("/parties/compliance-tasks/{}", report.id));
    let freeze_url = format!("/parties/compliance-tasks/{}", freeze.id);
    w.t.app
        .get(&freeze_url)
        .await
        .assert_ok()
        .assert_see("Omar K.")
        .assert_see("Record freeze")
        .assert_see("No module reported funds");

    // A funds line by hand, then edited and confirmed.
    w.t.app
        .post(
            &format!("{freeze_url}/funds"),
            &[
                ("label", "Deposit D-1"),
                ("kind", "deposit"),
                ("amount", "1,250.50"),
            ],
        )
        .await
        .assert_ok();
    let fund = ComplianceTaskFund::query()
        .where_(ComplianceTaskFund::TASK_ID.eq(freeze.id))
        .first(w.t.db())
        .await
        .unwrap()
        .expect("a line");
    assert_eq!((fund.amount_fils, fund.confirmed), (125_050, false));
    w.t.app.get(&freeze_url).await.assert_see("1,250.50");
    w.t.app
        .post(
            &format!("{freeze_url}/funds/{}", fund.id),
            &[("amount", "1000"), ("state", "confirmed")],
        )
        .await
        .assert_ok();
    let fund = ComplianceTaskFund::find(w.t.db(), fund.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((fund.amount_fils, fund.confirmed), (100_000, true));
    w.t.app
        .htmx()
        .post(
            &format!("{freeze_url}/funds"),
            &[("label", ""), ("amount", "1")],
        )
        .await
        .assert_invalid("label");
    w.t.app
        .htmx()
        .post(
            &format!("{freeze_url}/funds"),
            &[("label", "x"), ("amount", "abc")],
        )
        .await
        .assert_invalid("amount");

    // The reader cannot complete (403); A records the freeze (time defaults to now).
    w.t.app.acting_as(&w.reader);
    w.t.app
        .post(&format!("{freeze_url}/complete"), &[("frozen_at", "")])
        .await
        .assert_forbidden();
    w.t.app
        .post(
            &format!("{freeze_url}/funds"),
            &[("label", "x"), ("amount", "1")],
        )
        .await
        .assert_forbidden();
    w.t.app.acting_as(&w.a);
    w.t.app
        .post(&format!("{freeze_url}/complete"), &[("frozen_at", "")])
        .await
        .assert_ok();
    let freeze = ComplianceTask::find(w.t.db(), freeze.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(freeze.status, ComplianceTaskStatus::Done);
    assert!(freeze.frozen_at.is_some());

    // The report needs a reference and a date.
    let report_url = format!("/parties/compliance-tasks/{}", report.id);
    w.t.app
        .get(&report_url)
        .await
        .assert_ok()
        .assert_see("Record report");
    w.t.app
        .htmx()
        .post(
            &format!("{report_url}/complete"),
            &[("reference", ""), ("reported_on", "")],
        )
        .await
        .assert_invalid("reference");
    w.t.app
        .post(
            &format!("{report_url}/complete"),
            &[("reference", "GOAML-1"), ("reported_on", "2026-11-04")],
        )
        .await
        .assert_ok();
    let report = ComplianceTask::find(w.t.db(), report.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.status, ComplianceTaskStatus::Done);
    assert_eq!(report.reference.as_deref(), Some("GOAML-1"));
    w.t.app
        .get(&report_url)
        .await
        .assert_ok()
        .assert_see("GOAML-1");
}

#[renox::test]
async fn a_rejected_request_leaves_the_match_open_and_non_holders_get_404() {
    let w = world().await;
    let task = w.flag().await;
    let detail = format!("/parties/screening-tasks/{}", task.id);

    // Without `parties.block_reason.view` every route is a 404.
    w.t.app.acting_as(&w.partner);
    w.t.app
        .get("/parties/compliance-tasks")
        .await
        .assert_not_found();
    w.t.app
        .get("/parties/compliance-tasks/1")
        .await
        .assert_not_found();
    w.t.app
        .post(&format!("{detail}/confirm"), &[("note", "x")])
        .await
        .assert_not_found();
    w.t.app
        .post("/parties/compliance-tasks/1/complete", &[("frozen_at", "")])
        .await
        .assert_not_found();
    w.t.app
        .post(
            "/parties/compliance-tasks/1/funds",
            &[("label", "x"), ("amount", "1")],
        )
        .await
        .assert_not_found();

    // A holder gets 404 for a task that does not exist.
    w.t.app.acting_as(&w.reader);
    w.t.app
        .get("/parties/compliance-tasks/99999")
        .await
        .assert_not_found();
    w.t.app.get("/parties/compliance-tasks").await.assert_ok();

    // B rejects: the screening stays a possible match and no task exists.
    w.t.app.acting_as(&w.a);
    w.t.app
        .post(&format!("{detail}/confirm"), &[("note", "Same DOB")])
        .await
        .assert_ok();
    let request = wl_kernel::ApprovalRequest::query()
        .first(w.t.db())
        .await
        .unwrap()
        .expect("a request");
    approvals::reject(w.t.app.state(), request.id, &w.b, "no")
        .await
        .unwrap();
    let screening = Screening::query().first(w.t.db()).await.unwrap().unwrap();
    assert_eq!(screening.status, wl_kernel::ScreeningStatus::PossibleMatch);
    assert!(
        ComplianceTask::open_tasks(w.t.db())
            .await
            .unwrap()
            .is_empty()
    );
    w.t.app
        .get(&detail)
        .await
        .assert_ok()
        .assert_see("Confirm match");
}
