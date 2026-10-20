//! BL-HRM-27 (#217): each step is done the way its kind says (a task with
//! evidence, a checklist run that passed, an accepted document, the SOPs
//! acknowledged); a step of the employee is recorded by the employee or in
//! person with the signature and a witness; HR waives a step that does not
//! apply and another HR user reopens a mistake; the onboarding completes after
//! the last required step (`HRM.ACL.10`).

mod support;

use renox::prelude::*;
use wl_hr::steps::{self, DoneInput, InPerson};
use wl_hr::{
    Employee, EmployeeStatus, NewEmployee, Onboarding, OnboardingStatus, OnboardingStep,
    StepRefused, StepStatus, employees,
};
use wl_kernel::documents::{DocumentAccepted, SopWithdrawn};
use wl_kernel::tickets::{ChecklistCompleted, ChecklistOutcome, ChecklistRunId};
use wl_kernel::{DocumentId, PartyId, SopId};

use support::{Booted, add_template, boot, boot_with, branch, driver_template, nov};

/// The first onboarding of `employee`.
async fn onboarding_of(t: &Booted, employee: &Employee) -> Onboarding {
    t.onboardings(employee).await.remove(0)
}

fn in_person(signature: &str) -> DoneInput {
    DoneInput {
        in_person: Some(InPerson {
            signature: signature.into(),
        }),
        ..DoneInput::default()
    }
}

/// An employee at branch B without an account.
async fn walk_in(t: &Booted, party: i64) -> Employee {
    employees::create(
        t.state(),
        NewEmployee::new(PartyId(party), "driver", nov(2))
            .scope(branch())
            .status(EmployeeStatus::Onboarding),
    )
    .await
    .unwrap()
}

/// AC1 and AC2: the checklist run of "PPE handed over" completing *passed*
/// makes the step `done`; *failed* leaves it `open` with the failed items and
/// a new run can be started. A checklist step is not ticked by hand.
#[renox::test]
async fn hrm27_a_checklist_step_follows_its_run() {
    let t = boot().await;
    let hana = t.user_in_branch("Hana", "hr").await;
    let (ravi, _) = t.ravi().await;
    let onboarding = onboarding_of(&t, &ravi).await;
    let step = t.step(&onboarding, "Uniform or PPE handed over").await;

    let by_hand = steps::mark_done(t.state(), step.id, hana.id, DoneInput::default())
        .await
        .unwrap();
    assert_eq!(by_hand.unwrap_err(), StepRefused::ByEvidence);

    let started = steps::start_checklist(t.state(), step.id, hana.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(started.checklist_run_id, Some(100));
    {
        let runs = t.checklists.started.lock().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, "ppe_handover");
        assert_eq!(runs[0].1, step.subject());
    }
    let completed = |run: i64, outcome, items: &[&str]| ChecklistCompleted {
        run: ChecklistRunId(run),
        subject: step.subject(),
        outcome,
        failed_items: items.iter().map(|i| (*i).to_owned()).collect(),
    };
    t.state()
        .emit(completed(100, ChecklistOutcome::Failed, &["Size M boots"]))
        .await
        .unwrap();
    let failed = t.reload(&step).await;
    assert_eq!(failed.status, StepStatus::Open);
    assert_eq!(failed.failed_items.as_deref(), Some("Size M boots"));

    let again = steps::start_checklist(t.state(), step.id, hana.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.checklist_run_id, Some(101));
    assert_eq!(again.failed_items, None);
    t.state()
        .emit(completed(101, ChecklistOutcome::Passed, &[]))
        .await
        .unwrap();
    let done = t.reload(&step).await;
    assert_eq!(done.status, StepStatus::Done);
    assert_eq!(done.checklist_run_id, Some(101));
    // A second event for a step that is done is ignored.
    t.state()
        .emit(completed(101, ChecklistOutcome::Passed, &[]))
        .await
        .unwrap();
    assert_eq!(t.reload(&step).await.status, StepStatus::Done);
}

/// AC3 and AC11: Ravi has no account; HR user A records the SOP
/// acknowledgement in person with Ravi's drawn signature; A is the witness
/// and Ravi is told on his own channel. Not without the signature.
#[renox::test]
async fn hrm27_hr_records_the_sop_acknowledgement_in_person_with_a_witness() {
    let t = boot().await;
    let a = t.user_in_branch("Amal", "hr").await;
    let viewer = t.user_in_branch("Vera", "manager").await;
    t.sops.set_pending(&[1]);
    let ravi = walk_in(&t, 90).await;
    let onboarding = onboarding_of(&t, &ravi).await;
    let step = t.step(&onboarding, "SOPs acknowledged").await;
    assert!(
        step.assignees(t.db()).await.unwrap().is_empty(),
        "no account: in person only"
    );

    let refuse = |by: i64, input: DoneInput| {
        let state = t.state().clone();
        let id = step.id;
        async move {
            steps::mark_done(&state, id, by, input)
                .await
                .unwrap()
                .unwrap_err()
        }
    };
    assert_eq!(
        refuse(a.id, DoneInput::default()).await,
        StepRefused::EmployeeOnly
    );
    assert_eq!(
        refuse(a.id, in_person("  ")).await,
        StepRefused::SignatureRequired
    );
    assert_eq!(
        refuse(viewer.id, in_person("sig")).await,
        StepRefused::NotManager
    );

    let done = steps::mark_done(
        t.state(),
        step.id,
        a.id,
        in_person("data:image/png;base64,AAAA"),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(done.status, StepStatus::Done);
    assert_eq!(done.witness_id, Some(a.id));
    assert_eq!(
        done.signature.as_deref(),
        Some("data:image/png;base64,AAAA")
    );
    {
        let acknowledged = t.sops.in_person.lock().unwrap();
        assert_eq!(acknowledged.len(), 1);
        assert_eq!(acknowledged[0].0.employee, ravi.subject());
        assert_eq!(acknowledged[0].0.role, "driver");
        assert_eq!(acknowledged[0].2, a.id);
    }
    let told = t.notices.of_type("hr.onboarding_in_person_recorded");
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].recipients.len(), 1);
    assert_eq!(told[0].data["witness"], "Amal");
    assert_eq!(told[0].data["step"], "SOPs acknowledged");
}

/// AC4: HR user A cannot mark Ravi's `task` step "read the handbook" (owner
/// employee) done without Ravi; Ravi can, in his own session; A can in
/// person.
#[renox::test]
async fn hrm27_an_employee_step_is_recorded_by_the_employee_or_in_person() {
    let t = boot().await;
    add_template(&t, driver_template()).await;
    let a = t.user_in_branch("Amal", "hr").await;
    let (ravi, account) = t.ravi().await;
    let onboarding = onboarding_of(&t, &ravi).await;
    let handbook = t.step(&onboarding, "Read the handbook").await;
    assert_eq!(handbook.assignees(t.db()).await.unwrap(), [account.id]);

    let refused = steps::mark_done(t.state(), handbook.id, a.id, DoneInput::default())
        .await
        .unwrap();
    assert_eq!(refused.unwrap_err(), StepRefused::EmployeeOnly);
    assert_eq!(t.reload(&handbook).await.status, StepStatus::Open);

    let done = steps::mark_done(t.state(), handbook.id, account.id, DoneInput::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, StepStatus::Done);
    assert_eq!(done.done_by, Some(account.id));
    assert_eq!(done.witness_id, None);

    // In person: the employee is recorded as the one who did it.
    let licence = t.step(&onboarding, "Copy of driving licence").await;
    let done = steps::mark_done(
        t.state(),
        licence.id,
        a.id,
        DoneInput {
            note: Some("licence no. 4412".into()),
            in_person: Some(InPerson {
                signature: "sig".into(),
            }),
            ..DoneInput::default()
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(done.done_by, Some(account.id));
    assert_eq!(done.witness_id, Some(a.id));
}

/// AC5: A, the only HR holder of `onboarding.manage` in the branch and so the
/// sole assignee of "system account", cannot waive it; owner O (through the
/// owner role, not an assignee) can, with a reason.
#[renox::test]
async fn hrm27_an_assignee_cannot_waive_but_the_owner_can_with_a_reason() {
    let t = boot().await;
    let a = t.user_in_branch("Amal", "hr").await;
    let owner = t.user_with("Omar", "owner").await;
    let (ravi, _) = t.ravi().await;
    let onboarding = onboarding_of(&t, &ravi).await;
    let step = t.step(&onboarding, "System account and access").await;
    assert_eq!(step.assignees(t.db()).await.unwrap(), [a.id]);

    let waive = |by: i64, reason: &'static str| {
        let state = t.state().clone();
        let id = step.id;
        async move { steps::waive(&state, id, by, reason).await.unwrap() }
    };
    assert_eq!(
        waive(a.id, "not needed").await.unwrap_err(),
        StepRefused::AssigneeCannotWaive
    );
    assert_eq!(
        waive(owner.id, "  ").await.unwrap_err(),
        StepRefused::ReasonRequired
    );
    let nobody = t.user("Nobody").await;
    assert_eq!(
        waive(nobody.id, "x").await.unwrap_err(),
        StepRefused::NotManager
    );
    let waived = waive(owner.id, "uses a shared account").await.unwrap();
    assert_eq!(waived.status, StepStatus::Waived);
    assert_eq!(waived.waived_by, Some(owner.id));
    assert_eq!(
        waived.waive_reason.as_deref(),
        Some("uses a shared account")
    );
}

/// AC6: a `task` step with *evidence required* is not marked done without a
/// file or note; a step without is.
#[renox::test]
async fn hrm27_a_step_that_needs_evidence_is_not_done_without_it() {
    let t = boot().await;
    add_template(&t, driver_template()).await;
    let (ravi, account) = t.ravi().await;
    let onboarding = onboarding_of(&t, &ravi).await;
    let licence = t.step(&onboarding, "Copy of driving licence").await;
    assert!(licence.evidence_required);
    let refused = steps::mark_done(t.state(), licence.id, account.id, DoneInput::default())
        .await
        .unwrap();
    assert_eq!(refused.unwrap_err(), StepRefused::EvidenceRequired);
    let blank = DoneInput {
        note: Some("   ".into()),
        ..DoneInput::default()
    };
    assert!(
        steps::mark_done(t.state(), licence.id, account.id, blank)
            .await
            .unwrap()
            .is_err()
    );
    let with_file = DoneInput {
        document_id: Some(41),
        ..DoneInput::default()
    };
    let done = steps::mark_done(t.state(), licence.id, account.id, with_file)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, StepStatus::Done);
    assert_eq!(done.evidence_document_id, Some(41));
}

/// AC7 and AC9: an optional step still `open` when the last required step is
/// done: the onboarding is `completed`, `OnboardingCompleted` is emitted, HR
/// is told, and the optional step stays `open`.
#[renox::test]
async fn hrm27_the_onboarding_completes_after_the_last_required_step() {
    let t = boot_with(wl_hr::WlHr::new(), false).await;
    add_template(
        &t,
        wl_hr::TemplateInput::for_role("Clerk", "clerk")
            .step(wl_hr::StepInput::task("Sign the contract", "hr", 1))
            .step(wl_hr::StepInput::task("Meet the team", "hr", 2).optional()),
    )
    .await;
    let hana = t.user_in_branch("Hana", "hr").await;
    let clerk = employees::create(
        t.state(),
        NewEmployee::new(PartyId(91), "clerk", nov(2)).scope(branch()),
    )
    .await
    .unwrap();
    let onboarding = onboarding_of(&t, &clerk).await;
    let required = t.step(&onboarding, "Sign the contract").await;
    let optional = t.step(&onboarding, "Meet the team").await;
    assert!(t.completed.all().is_empty());

    steps::mark_done(t.state(), required.id, hana.id, DoneInput::default())
        .await
        .unwrap()
        .unwrap();
    let after = Onboarding::find_or_404(t.db(), onboarding.id)
        .await
        .unwrap();
    assert_eq!(after.status, OnboardingStatus::Completed);
    assert_eq!(t.reload(&optional).await.status, StepStatus::Open);
    let completed = t.completed.all();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].employee.0, clerk.id);
    assert_eq!(completed[0].trigger, "hire");
    assert_eq!(t.notices.of_type("hr.onboarding_completed").len(), 1);
    // The optional step can still be done; a completed onboarding is not
    // reopened.
    let late = steps::mark_done(t.state(), optional.id, hana.id, DoneInput::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(late.status, StepStatus::Done);
    assert_eq!(t.completed.all().len(), 1, "completed once");
    let reopened = steps::reopen(t.state(), required.id, hana.id, "again")
        .await
        .unwrap();
    assert_eq!(reopened.unwrap_err(), StepRefused::OnboardingClosed);
}

/// AC8: the `document` step "Emirates ID" waived and the document already
/// accepted: reopened by another user, it is `done` at once; a waived
/// `checklist` step reopened is `open` and needs a new run.
#[renox::test]
async fn hrm27_a_reopened_document_step_is_done_again_and_a_checklist_step_is_open() {
    let t = boot().await;
    add_template(&t, driver_template()).await;
    let a = t.user_in_branch("Amal", "hr").await;
    let owner = t.user_with("Omar", "owner").await;
    let (ravi, _) = t.ravi().await;
    let onboarding = onboarding_of(&t, &ravi).await;
    let emirates = t.step(&onboarding, "Emirates ID").await;
    let ppe = t.step(&onboarding, "Uniform or PPE handed over").await;
    for step in [&emirates, &ppe] {
        steps::waive(t.state(), step.id, owner.id, "handled elsewhere")
            .await
            .unwrap()
            .unwrap();
    }
    t.docs.accepted(&ravi.subject(), "emirates_id", 77);

    // The one who waived cannot reopen; another holder can, with a reason.
    let same = steps::reopen(t.state(), emirates.id, owner.id, "oops")
        .await
        .unwrap();
    assert_eq!(same.unwrap_err(), StepRefused::SameUser);
    let blank = steps::reopen(t.state(), emirates.id, a.id, " ")
        .await
        .unwrap();
    assert_eq!(blank.unwrap_err(), StepRefused::ReasonRequired);
    let reopened = steps::reopen(t.state(), emirates.id, a.id, "the ID is in")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reopened.status, StepStatus::Done);
    assert_eq!(reopened.evidence_document_id, Some(77));
    let reopened = steps::reopen(t.state(), ppe.id, a.id, "needs a new run")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reopened.status, StepStatus::Open);
    assert_eq!(reopened.checklist_run_id, None);
    // A step done by the system from its evidence is not reopened by hand.
    let done_by_system = steps::reopen(t.state(), emirates.id, owner.id, "again")
        .await
        .unwrap();
    assert_eq!(done_by_system.unwrap_err(), StepRefused::CannotReopen);
}

/// AC10: A marked "system account" done by mistake: A cannot reopen it; HR
/// user B reopens it with a reason, the step is `open` again and the history
/// keeps both records.
#[renox::test]
async fn hrm27_another_hr_user_reopens_a_step_marked_by_mistake_and_the_history_stays() {
    let t = boot().await;
    let a = t.user_in_branch("Amal", "hr").await;
    let b = t.user_in_branch("Basel", "hr").await;
    let (ravi, _) = t.ravi().await;
    let onboarding = onboarding_of(&t, &ravi).await;
    let step = t.step(&onboarding, "System account and access").await;
    steps::mark_done(t.state(), step.id, a.id, DoneInput::default())
        .await
        .unwrap()
        .unwrap();

    let own = steps::reopen(t.state(), step.id, a.id, "ticked by mistake")
        .await
        .unwrap();
    assert_eq!(own.unwrap_err(), StepRefused::SameUser);
    t.notices.clear();
    let reopened = steps::reopen(t.state(), step.id, b.id, "ticked by mistake")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reopened.status, StepStatus::Open);
    assert_eq!(reopened.done_by, None);
    assert_eq!(
        reopened.due_date, step.due_date,
        "the due date is unchanged"
    );
    let history = renox::state_machine::history(t.db(), OnboardingStep::TABLE, step.id)
        .await
        .unwrap();
    let names: Vec<&str> = history.iter().map(|h| h.transition.as_str()).collect();
    assert_eq!(names, ["mark_done", "reopen"]);
    // The assignees are told.
    assert_eq!(t.notices.of_type("hr.onboarding_step_reopened").len(), 1);
}

/// AC12: the only SOP for drivers is withdrawn while Ravi's
/// `sop_acknowledgement` step is open: the step becomes `done`.
#[renox::test]
async fn hrm27_a_withdrawn_sop_completes_the_acknowledgement_step() {
    let t = boot().await;
    t.sops.set_pending(&[1]);
    let (ravi, _) = t.ravi().await;
    let onboarding = onboarding_of(&t, &ravi).await;
    let step = t.step(&onboarding, "SOPs acknowledged").await;
    assert_eq!(step.status, StepStatus::Open);

    t.sops.set_pending(&[]);
    t.state()
        .emit(SopWithdrawn {
            sop: SopId(1),
            document: DocumentId(3),
            version: 1,
            cancelled_acknowledgements: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(t.reload(&step).await.status, StepStatus::Done);
}

/// With no SOP for the role the step is done at start, noted *no SOPs*.
#[renox::test]
async fn hrm27_with_no_sop_for_the_role_the_step_is_done_at_start() {
    let t = boot().await;
    let (ravi, _) = t.ravi().await;
    let onboarding = onboarding_of(&t, &ravi).await;
    let step = t.step(&onboarding, "SOPs acknowledged").await;
    assert_eq!(step.status, StepStatus::Done);
    assert_eq!(step.note.as_deref(), Some("No SOPs"));
}

/// A document accepted on the employee after the start makes the `document`
/// step done; one accepted before makes it done at the start.
#[renox::test]
async fn hrm27_an_accepted_document_makes_the_document_step_done() {
    let t = boot().await;
    add_template(&t, driver_template()).await;
    let (ravi, _) = t.ravi().await;
    let onboarding = onboarding_of(&t, &ravi).await;
    let step = t.step(&onboarding, "Emirates ID").await;
    assert_eq!(step.status, StepStatus::Open);
    // Another kind or another employee changes nothing.
    t.state()
        .emit(DocumentAccepted {
            document: DocumentId(5),
            subject: ravi.subject(),
            kind: "passport".into(),
            expires_on: None,
        })
        .await
        .unwrap();
    assert_eq!(t.reload(&step).await.status, StepStatus::Open);
    t.state()
        .emit(DocumentAccepted {
            document: DocumentId(6),
            subject: ravi.subject(),
            kind: "emirates_id".into(),
            expires_on: None,
        })
        .await
        .unwrap();
    let done = t.reload(&step).await;
    assert_eq!(done.status, StepStatus::Done);
    assert_eq!(done.evidence_document_id, Some(6));

    // Accepted before the onboarding started: done at start.
    let active = employees::create(
        t.state(),
        NewEmployee::new(PartyId(95), "driver", nov(2))
            .scope(branch())
            .status(EmployeeStatus::Active),
    )
    .await
    .unwrap();
    t.docs.accepted(&active.subject(), "emirates_id", 9);
    let started = wl_hr::onboarding::start(
        t.state(),
        wl_hr::StartRequest {
            employee_id: active.id,
            trigger: wl_hr::StartTrigger::Manual,
            templates: Vec::new(),
            start_on: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    let early = t.step(&started, "Emirates ID").await;
    assert_eq!(early.status, StepStatus::Done);
    assert_eq!(early.evidence_document_id, Some(9));
}

/// The pages: an assignee marks a step done in their own session; another
/// user is refused; a reason is needed to waive.
#[renox::test]
async fn hrm27_the_step_actions_answer_for_the_signed_in_user() {
    let t = boot().await;
    add_template(&t, driver_template()).await;
    let a = t.user_in_branch("Amal", "hr").await;
    let hana = t.user_with("Hana", "hr").await;
    let (ravi, account) = t.ravi().await;
    let onboarding = onboarding_of(&t, &ravi).await;
    let handbook = t.step(&onboarding, "Read the handbook").await;
    let path = |action: &str, id: i64| format!("/hr/steps/{id}/{action}");

    t.app.acting_as(&a);
    t.app
        .post(&path("done", handbook.id), &[("note", "")])
        .await
        .assert_forbidden();
    t.app.acting_as(&account);
    t.app
        .post(&path("done", handbook.id), &[("note", "")])
        .await
        .assert_ok();
    assert_eq!(t.reload(&handbook).await.status, StepStatus::Done);

    let account_step = t.step(&onboarding, "System account and access").await;
    t.app.acting_as(&account);
    t.app
        .post(&path("waive", account_step.id), &[("reason", "no")])
        .await
        .assert_forbidden();
    t.app.acting_as(&hana);
    t.app
        .htmx()
        .post(&path("waive", account_step.id), &[("reason", "")])
        .await
        .assert_status(422);
    t.app.acting_as(&hana);
    t.app
        .get(&format!("/hr/employees/{}", ravi.id))
        .await
        .assert_ok()
        .assert_see("System account and access");
}
