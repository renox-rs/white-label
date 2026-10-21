//! BL-CON-01 (#280): terminating a project mid-way (`terminate`,
//! `CON.ACL.15`). The project manager requests it with the reason, the notice
//! and the effective date; the MD approves; the project is `terminated`,
//! attendance stops after the effective date, crews are released, purchasing
//! is wound down, the site store is counted, a final account IPA is drafted,
//! retention becomes due, guarantees are followed up, open snags are
//! cancelled and the client is told.

mod support;

use construction::{
    Guarantee, GuaranteeKind, GuaranteeStatus, Ipa, IpaKind, IpaStatus, ProjectStatus, Retention,
    RetentionPart, RetentionStatus, permissions,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_attendance::service::{self, NewRecord};
use wl_attendance::{AttendanceFlag, AttendanceStatus, FlagType};
use wl_inventory::{
    PurchaseRequest, PurchaseRequestStatus, StockCount, StockCountStatus, stock_count,
};
use wl_kernel::approvals::approve;
use wl_kernel::documents::dubai_date;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::Assignee;

use support::{Booted, boot, date, dubai};

/// The message of a refusal (a 422).
fn refusal(error: Error) -> String {
    match error {
        Error::Status(status, message) => {
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{message}");
            message
        }
        other => panic!("not a refusal: {other:?}"),
    }
}

/// A purchase request at `location` in `status`, inserted as a fixture.
async fn request(t: &Booted, location: i64, status: PurchaseRequestStatus) -> PurchaseRequest {
    let mut request = PurchaseRequest::new(location);
    request.status = status;
    let mut tx = t.db().begin().await.unwrap();
    request
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    request
}

async fn reload_request(t: &Booted, id: i64) -> PurchaseRequest {
    PurchaseRequest::find_or_404(t.db(), id).await.unwrap()
}

/// Done when 1: project manager A requests `terminate` with the reason
/// `main_contractor_insolvent`, the notice and the effective date 2027-05-31:
/// an approval request goes to the MD and the project stays `in_progress`.
/// A cannot approve; the MD approves and the project is `terminated` and
/// `ProjectTerminated` is emitted.
#[renox::test]
async fn the_md_approves_the_termination_the_project_manager_asked_for() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let md = t.md("Bader").await;
    let project = t.project(ProjectStatus::InProgress, Some(&a)).await;

    t.ask_to_terminate(&a, &project, "main_contractor_insolvent", "2027-05-31")
        .await
        .assert_ok();

    let waiting = t.reload(&project).await;
    assert_eq!(waiting.status, ProjectStatus::InProgress);
    assert_eq!(waiting.termination_effective_on, None);
    let request = t.pending_request(&project).await;
    assert_eq!(request.requester_id, a.id);
    assert!(t.terminated.all().is_empty());

    // The requester cannot approve, nor can a project manager.
    let refused = approve(t.state(), request.id, &a, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(t.reload(&project).await.status, ProjectStatus::InProgress);

    approve(t.state(), request.id, &md, "agreed").await.unwrap();

    let ended = t.reload(&project).await;
    assert_eq!(ended.status, ProjectStatus::Terminated);
    assert_eq!(
        ended.termination_reason.as_deref(),
        Some("main_contractor_insolvent")
    );
    assert_eq!(
        ended.termination_notice.as_deref(),
        Some("TERMINATION-NOTICE-17.pdf")
    );
    assert_eq!(ended.termination_effective_on, Some(date(2027, 5, 31)));
    assert_eq!(ended.termination_requested_by, Some(a.id));
    assert!(ended.terminated_at.is_some());
    let events = t.terminated.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].project, project.subject());
    assert_eq!(events[0].reason, "main_contractor_insolvent");
    assert_eq!(events[0].effective_on, date(2027, 5, 31));
    assert_eq!(events[0].requested_by, Some(a.id));
}

/// The request needs a reason, a notice and an effective date, and a project
/// that is `in_progress` or `on_hold`; nothing is written otherwise.
#[renox::test]
async fn a_request_without_its_inputs_or_from_the_wrong_state_is_refused() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let project = t.project(ProjectStatus::InProgress, Some(&a)).await;
    let path = format!("/construction/projects/{}/terminate", project.id);

    t.post(
        &a,
        &path,
        &[("reason", "bored"), ("effective_on", "2027-05-31")],
    )
    .await
    .assert_status(422);
    t.post(
        &a,
        &path,
        &[
            ("reason", "agreed"),
            ("notice", " "),
            ("effective_on", "2027-05-31"),
        ],
    )
    .await
    .assert_status(422);
    t.post(&a, &path, &[("reason", "agreed"), ("notice", "N.pdf")])
        .await
        .assert_status(422);
    assert_eq!(t.reload(&project).await.status, ProjectStatus::InProgress);
    assert!(
        wl_kernel::approvals::ApprovalRequest::pending_about(t.db(), &project.subject())
            .await
            .unwrap()
            .is_empty()
    );

    // A project that has not started, and one already terminated, cannot be
    // terminated; neither can a user who does not manage it.
    let setup = t.project(ProjectStatus::Setup, Some(&a)).await;
    let outsider = t.user_with("Sam", "md").await;
    t.ask_to_terminate(&a, &setup, "agreed", "2027-05-31")
        .await
        .assert_status(422);
    t.ask_to_terminate(&outsider, &project, "agreed", "2027-05-31")
        .await
        .assert_status(403);
    assert_eq!(t.reload(&project).await.status, ProjectStatus::InProgress);

    // A project on hold can be terminated too.
    let held = t.project(ProjectStatus::OnHold, Some(&a)).await;
    t.ask_to_terminate(&a, &held, "terminated_by_client", "2027-05-31")
        .await
        .assert_ok();
    assert_eq!(t.pending_request(&held).await.requester_id, a.id);
}

/// Done when 3: attendance for 2027-06-01 on the project is refused;
/// attendance for 2027-05-31 is accepted.
#[renox::test]
async fn attendance_stops_the_day_after_the_effective_date() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let md = t.md("Bader").await;
    let boss = t.user_with("Bashir", "md").await;
    let project = t.project(ProjectStatus::InProgress, Some(&a)).await;
    let site = t.work_location(&project).await;
    let worker = t.staff.employ(1);
    t.ask_to_terminate(&a, &project, "main_contractor_insolvent", "2027-05-31")
        .await
        .assert_ok();
    t.approve(&md, &project).await;
    assert_eq!(t.reload(&project).await.status, ProjectStatus::Terminated);

    t.on(date(2027, 5, 31), async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(
                worker,
                site.id,
                dubai(date(2027, 5, 31), 7, 0),
                Some(dubai(date(2027, 5, 31), 15, 0)),
                boss.id,
            ),
        )
        .await
        .unwrap();
        assert_eq!(record.date, date(2027, 5, 31));
    })
    .await;

    t.on(date(2027, 6, 1), async {
        let refused = service::record(
            t.state(),
            NewRecord::supervisor(
                worker,
                site.id,
                dubai(date(2027, 6, 1), 7, 0),
                Some(dubai(date(2027, 6, 1), 15, 0)),
                boss.id,
            ),
        )
        .await
        .unwrap_err();
        let message = refusal(refused);
        assert!(message.contains("2027-05-31"), "{message}");
    })
    .await;
}

/// Done when 4: with the MD approving on 2027-05-20 (effective date
/// 2027-05-31), attendance for 2027-05-25 is accepted and the project manager
/// has the reassign task from 2027-05-20, due on the effective date.
#[renox::test]
async fn a_future_effective_date_keeps_the_crews_working_until_then() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let md = t.md("Bader").await;
    let boss = t.user_with("Bashir", "md").await;
    let project = t.project(ProjectStatus::InProgress, Some(&a)).await;
    let site = t.work_location(&project).await;
    let worker = t.staff.employ(1);
    t.ask_to_terminate(&a, &project, "terminated_by_client", "2027-05-31")
        .await
        .assert_ok();

    t.on(date(2027, 5, 20), async { t.approve(&md, &project).await })
        .await;

    assert_eq!(t.reload(&project).await.status, ProjectStatus::Terminated);
    let tasks = t.tasks.with_key("construction.terminate_reassign:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].ticket.assignee,
        Some(wl_kernel::tickets::Assignee::user(a.id))
    );
    assert_eq!(
        dubai_date(tasks[0].at),
        date(2027, 5, 20),
        "the task is made on the day of the approval"
    );
    assert_eq!(
        tasks[0].ticket.due_at.map(dubai_date),
        Some(date(2027, 5, 31))
    );

    t.on(date(2027, 5, 25), async {
        service::record(
            t.state(),
            NewRecord::supervisor(
                worker,
                site.id,
                dubai(date(2027, 5, 25), 7, 0),
                Some(dubai(date(2027, 5, 25), 15, 0)),
                boss.id,
            ),
        )
        .await
        .unwrap();
    })
    .await;
}

/// Done when 5 to 7: 2 open purchase requests without POs are `cancelled`;
/// procurement has one task listing the 3 open POs; a stock count is open for
/// the site store; a final account IPA in `draft` exists; both retention
/// parts of AED 50,000 are `due`; the accountant has one task per active
/// guarantee; 4 open snag tickets are `cancelled` with the reason; the client
/// contacts get the e-mail.
#[renox::test]
async fn the_wind_down_after_the_approval() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let md = t.md("Bader").await;
    let project = t.project(ProjectStatus::InProgress, Some(&a)).await;
    let store = t.store(&project, "SITE-STORE").await;
    let subject = project.subject();

    // Purchasing: 2 requests with no order, one already ordered, 3 open
    // orders.
    let draft = request(&t, store.id, PurchaseRequestStatus::Draft).await;
    let approved = request(&t, store.id, PurchaseRequestStatus::Approved).await;
    let submitted = request(&t, store.id, PurchaseRequestStatus::Submitted).await;
    let ordered = request(&t, store.id, PurchaseRequestStatus::Ordered).await;
    // Procurement and the accountant hold their roles: the tasks go to them.
    let buyer = t.named("Pia", permissions::PROCUREMENT_ROLE).await;
    let books = t.accountant("Ali").await;
    for number in ["PO-2027-0001", "PO-2027-0002", "PO-2027-0003"] {
        t.order(&store, number).await;
    }
    // Money: two retention parts of AED 50,000, a draft IPA, three guarantees.
    for part in [RetentionPart::First, RetentionPart::Final] {
        Retention::new(project.id, part, 5_000_000)
            .save(t.db())
            .await
            .unwrap();
    }
    let mut interim = Ipa::new(project.id);
    interim.claimed_amount = 12_000_000;
    interim.save(t.db()).await.unwrap();
    for (kind, status) in [
        (GuaranteeKind::Advance, GuaranteeStatus::Active),
        (GuaranteeKind::Performance, GuaranteeStatus::Active),
        (GuaranteeKind::RetentionBond, GuaranteeStatus::Draft),
    ] {
        let mut guarantee = Guarantee::new(project.id, kind, 2_000_000);
        guarantee.status = status;
        let mut tx = t.db().begin().await.unwrap();
        guarantee
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    // Four open snags and a defect already closed elsewhere.
    for n in 1..=4 {
        t.tasks.seed(&subject, "snag", &format!("Snag {n}"));
    }
    t.tasks.seed(&subject, "general", "A general ticket");

    t.ask_to_terminate(&a, &project, "main_contractor_insolvent", "2027-05-31")
        .await
        .assert_ok();
    // Nothing was touched while the request waits.
    assert_eq!(
        reload_request(&t, draft.id).await.status,
        PurchaseRequestStatus::Draft
    );
    t.approve(&md, &project).await;

    // Purchase requests without an order are cancelled.
    for id in [draft.id, submitted.id, approved.id] {
        let found = reload_request(&t, id).await;
        assert_eq!(found.status, PurchaseRequestStatus::Cancelled);
        assert_eq!(found.cancel_reason.as_deref(), Some("project terminated"));
    }
    assert_eq!(
        reload_request(&t, ordered.id).await.status,
        PurchaseRequestStatus::Ordered
    );

    // One task for procurement lists the 3 open orders.
    let orders = t.tasks.with_key("construction.terminate_orders:");
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].ticket.assignee, Some(Assignee::user(buyer.id)));
    for number in ["PO-2027-0001", "PO-2027-0002", "PO-2027-0003"] {
        assert!(
            orders[0].ticket.description.contains(number),
            "{}",
            orders[0].ticket.description
        );
    }

    // A stock count is open for the site store.
    let counts = stock_count::unfinished_at(t.db(), store.id).await.unwrap();
    assert_eq!(counts.len(), 1);
    assert_eq!(counts[0].status, StockCountStatus::Open);
    assert_eq!(counts[0].source.as_deref(), Some("construction.project:1"));

    // The final account IPA is the draft that existed, valued to the
    // effective date.
    let ipas = Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(ipas.len(), 1);
    assert_eq!(ipas[0].id, interim.id);
    assert_eq!(ipas[0].kind, IpaKind::FinalAccount);
    assert_eq!(ipas[0].status, IpaStatus::Draft);
    assert_eq!(ipas[0].valuation_on, Some(date(2027, 5, 31)));

    // Both retention parts are due.
    let parts = Retention::query()
        .where_(Retention::PROJECT_ID.eq(project.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(parts.len(), 2);
    for part in &parts {
        assert_eq!(part.status, RetentionStatus::Due);
        assert_eq!(part.amount, 5_000_000);
        assert!(part.due_since.is_some());
    }

    // One task per active guarantee.
    let guarantees = t.tasks.with_key("construction.terminate_guarantee:");
    assert_eq!(guarantees.len(), 2);
    assert!(
        guarantees
            .iter()
            .all(|task| task.ticket.assignee == Some(Assignee::user(books.id)))
    );

    // The 4 snags are cancelled with the reason; the other ticket stays.
    let cancelled = t.tasks.cancelled_of(&subject, "snag");
    assert_eq!(cancelled.len(), 4);
    assert!(
        cancelled
            .iter()
            .all(|snag| snag.cancelled.as_deref() == Some("project terminated"))
    );
    assert!(t.tasks.cancelled_of(&subject, "general").is_empty());

    // The client contacts are e-mailed with the effective date.
    let told = t.notices.of_type(construction::termination::NOTICE);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["date"], "2027-05-31");
    assert!(told[0].recipients.iter().any(|r| matches!(
        r,
        NotifyRecipient::Contact { of, .. } if *of == support::CLIENT_PARTY
    )));

    // Nothing is deleted, and the wind-down ran once.
    let stored = t.reload(&project).await;
    assert!(stored.wound_down_at.is_some());
    let again = construction::termination::after_termination(t.state(), project.id)
        .await
        .unwrap();
    assert!(!again.ran);
    assert_eq!(t.terminated.all().len(), 1);
    assert_eq!(StockCount::query().count(t.db()).await.unwrap(), 1);
}

/// Done when 6: with `retention_due_on_termination = no` the retention parts
/// stay `held`.
#[renox::test]
async fn retention_stays_held_when_the_project_says_so() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let md = t.md("Bader").await;
    let project = t.project(ProjectStatus::InProgress, Some(&a)).await;
    construction::Project::query()
        .where_(construction::Project::ID.eq(project.id))
        .update(t.db(), &[("retention_due_on_termination", &false)])
        .await
        .unwrap();
    for part in [RetentionPart::First, RetentionPart::Final] {
        Retention::new(project.id, part, 5_000_000)
            .save(t.db())
            .await
            .unwrap();
    }

    t.ask_to_terminate(&a, &project, "agreed", "2027-05-31")
        .await
        .assert_ok();
    t.approve(&md, &project).await;

    assert_eq!(t.reload(&project).await.status, ProjectStatus::Terminated);
    let parts = Retention::query()
        .where_(Retention::PROJECT_ID.eq(project.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(parts.len(), 2);
    assert!(
        parts
            .iter()
            .all(|part| part.status == RetentionStatus::Held)
    );
}

/// Edge case: an IPA still `submitted` stays and is certified normally; a
/// new `draft` final account IPA is made beside it.
#[renox::test]
async fn a_submitted_ipa_stays_and_a_draft_final_account_is_made() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let md = t.md("Bader").await;
    let project = t.project(ProjectStatus::InProgress, Some(&a)).await;
    let mut submitted = Ipa::new(project.id);
    submitted.status = IpaStatus::Submitted;
    let mut tx = t.db().begin().await.unwrap();
    submitted
        .insert_in_state(&mut tx, IpaStatus::Submitted, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    t.ask_to_terminate(&a, &project, "agreed", "2027-05-31")
        .await
        .assert_ok();
    t.approve(&md, &project).await;

    let mut ipas = Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project.id))
        .order_by(Ipa::ID)
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(ipas.len(), 2);
    let made = ipas.pop().unwrap();
    let stays = ipas.pop().unwrap();
    assert_eq!(stays.id, submitted.id);
    assert_eq!(stays.status, IpaStatus::Submitted);
    assert_eq!(stays.kind, IpaKind::Interim);
    assert_eq!(made.kind, IpaKind::FinalAccount);
    assert_eq!(made.status, IpaStatus::Draft);
    assert_eq!(made.valuation_on, Some(date(2027, 5, 31)));
}

/// Edge case: an effective date in the past is allowed; attendance already
/// recorded after it is kept, flagged for the reviewer, and not deleted.
#[renox::test]
async fn attendance_recorded_after_a_past_effective_date_is_flagged() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let md = t.md("Bader").await;
    let boss = t.user_with("Bashir", "md").await;
    let project = t.project(ProjectStatus::InProgress, Some(&a)).await;
    let site = t.work_location(&project).await;
    let worker = t.staff.employ(1);

    t.on(date(2027, 6, 2), async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(
                worker,
                site.id,
                dubai(date(2027, 6, 2), 7, 0),
                Some(dubai(date(2027, 6, 2), 15, 0)),
                boss.id,
            ),
        )
        .await
        .unwrap();
        assert!(
            AttendanceFlag::of_record(t.db(), record.id)
                .await
                .unwrap()
                .is_empty()
        );

        t.ask_to_terminate(&a, &project, "terminated_by_client", "2027-05-31")
            .await
            .assert_ok();
        t.approve(&md, &project).await;

        let kept = wl_attendance::AttendanceRecord::find_or_404(t.db(), record.id)
            .await
            .unwrap();
        assert_eq!(kept.status, AttendanceStatus::Flagged);
        let flags = AttendanceFlag::of_record(t.db(), record.id).await.unwrap();
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].flag_type, FlagType::AfterLastDay);
        assert!(flags[0].note.contains("2027-05-31"), "{}", flags[0].note);
    })
    .await;
}
