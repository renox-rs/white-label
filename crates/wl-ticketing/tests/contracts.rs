//! BL-TKT-04 (#149): when a service contract ends (`expire`, or the
//! termination date of a `terminate`), `WRK.RUL.18` cancels the planned
//! visits due after the end date, leaves started visits alone, gives the
//! coordinator one task for the other open visits, ends the maintenance plans
//! and, for a compliance contract without a replacement, raises
//! `no_contract` on each covered subject; activating a replacement clears it.

mod support;

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::tickets::{Assignee, ComplianceFlagReason, OpenTicket, TicketId};
use wl_ticketing::contract_end::{
    self, CONTRACT_ENDED_REASON, NO_CONTRACT_NOTICE, VISIT_CANCELLED_NOTICE, task_key,
};
use wl_ticketing::flags::{ComplianceFlag, REPLACEMENT_ACTIVE};
use wl_ticketing::plans::{MaintenancePlans, PlanStatus};
use wl_ticketing::{
    Category, ContractStatus, Coverage, ServiceContract, SlaPolicy, Ticket, TicketStatus, Tickets,
};

use support::contracts::{World, date, tower_a_cleaning, tower_a_lifts, world};
use support::{BLUEFIRE, COOLAIR, dubai};

/// AMC-2026-0007: lifts of Tower A, 2026-01-01 to 2026-12-31, with the
/// planned visits of the issue: 12-20 (assigned), 12-31 (open), 2027-01-15
/// and 2027-02-15 (assigned), and one started on 12-10.
struct Lifts {
    contract: wl_ticketing::ServiceContract,
    dec20: Ticket,
    dec31: Ticket,
    jan15: Ticket,
    feb15: Ticket,
    started: Ticket,
}

async fn lifts(w: &World) -> Lifts {
    let contract = w
        .contract(
            COOLAIR,
            "2026-01-01",
            "2026-12-31",
            &[(tower_a_lifts(), "lifts")],
        )
        .await;
    let vendor = Some(Assignee::party(COOLAIR));
    let dec20 = w
        .visit(&contract, "lifts", tower_a_lifts(), "2026-12-20", vendor)
        .await;
    let dec31 = w
        .visit(&contract, "lifts", tower_a_lifts(), "2026-12-31", None)
        .await;
    let jan15 = w
        .visit(&contract, "lifts", tower_a_lifts(), "2027-01-15", vendor)
        .await;
    let feb15 = w
        .visit(&contract, "lifts", tower_a_lifts(), "2027-02-15", vendor)
        .await;
    let started = w
        .visit(&contract, "lifts", tower_a_lifts(), "2026-12-10", vendor)
        .await;
    let started = w.start(&started).await;
    Lifts {
        contract,
        dec20,
        dec31,
        jan15,
        feb15,
        started,
    }
}

/// The contract expires: the nightly run of 1 January 2027 finds the end
/// date passed.
async fn expire(w: &World, lifts: &Lifts) {
    let report = contract_end::run(w.t.state(), date("2027-01-01"))
        .await
        .expect("the nightly run");
    assert_eq!(report.flagged_expiring, 1);
    assert_eq!(report.expired, 1);
    assert_eq!(report.ended.len(), 1);
    assert_eq!(report.ended[0].contract, lifts.contract.id);
}

/// AC1: `expire` cancels the 2027 visits with `contract_ended` and tells the
/// vendor; the coordinator has one task listing the 12-20 and 12-31 visits;
/// the visit in progress is untouched.
#[renox::test]
async fn tkt04_expiring_cancels_the_late_visits_and_gives_the_coordinator_one_task() {
    let w = world().await;
    let l = lifts(&w).await;
    // A reactive ticket on the covered subject is not a visit.
    let reactive = Tickets::open_at(
        w.t.state(),
        OpenTicket::new(tower_a_lifts(), "lifts", "Stuck lift").assign_to(Assignee::party(COOLAIR)),
        dubai("2026-12-28", 9, 0),
    )
    .await
    .unwrap();
    assert!(
        w.t.ticket(reactive.id.0).await.contract_id.is_some(),
        "handled under the contract"
    );

    expire(&w, &l).await;

    for late in [&l.jan15, &l.feb15] {
        let visit = w.t.reload(late).await;
        assert_eq!(visit.status, TicketStatus::Cancelled);
        assert_eq!(visit.cancel_reason.as_deref(), Some(CONTRACT_ENDED_REASON));
    }
    assert_eq!(
        w.told_party(VISIT_CANCELLED_NOTICE, COOLAIR),
        2,
        "the vendor is told"
    );
    assert_eq!(w.t.reload(&l.dec20).await.status, TicketStatus::Assigned);
    assert_eq!(
        w.t.reload(&l.dec31).await.status,
        TicketStatus::Open,
        "due on the end date"
    );
    assert_eq!(
        w.t.reload(&l.started).await.status,
        TicketStatus::InProgress
    );
    assert_eq!(
        w.t.ticket(reactive.id.0).await.status,
        TicketStatus::Assigned,
        "a reactive ticket is not cancelled"
    );

    let key = task_key(l.contract.id);
    let tasks = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(key.as_str()))
        .get(w.t.db())
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1, "one task");
    let task = &tasks[0];
    assert_eq!(task.assignee_user_id, Some(w.coordinator.id));
    assert!(task.due_at.is_some(), "due one working day later");
    assert!(task.description.contains(&l.dec20.number));
    assert!(task.description.contains(&l.dec31.number));
    assert!(!task.description.contains(&l.jan15.number));
    assert!(!task.description.contains(&l.feb15.number));
    assert!(!task.description.contains(&l.started.number));

    let contract = w.reload(&l.contract).await;
    assert_eq!(contract.status, ContractStatus::Expired);
    assert!(contract.ended_at.is_some());
    let plans = MaintenancePlans::of_contract(w.t.db(), contract.id)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1, "one plan per covered subject and category");
    assert!(plans.iter().all(|p| p.status == PlanStatus::Ended));
}

/// AC2: no other active lift contract covers Tower A: `no_contract` is
/// raised and the property manager told; `ServiceContractEnded` lists 2
/// cancelled and 2 left.
#[renox::test]
async fn tkt04_a_compliance_contract_without_a_replacement_flags_the_subject() {
    let w = world().await;
    let l = lifts(&w).await;
    assert!(l.contract.compliance_contract);
    expire(&w, &l).await;

    let open = ComplianceFlag::open(w.t.db()).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].reason, ComplianceFlagReason::NoContract);
    assert_eq!(open[0].subject(), tower_a_lifts());
    assert_eq!(open[0].contract_id, Some(l.contract.id));
    assert_eq!(w.raised.all().len(), 1);
    assert_eq!(w.raised.all()[0].reason, "no_contract");
    assert_eq!(w.told(NO_CONTRACT_NOTICE, &w.manager), 1);
    assert_eq!(w.told(NO_CONTRACT_NOTICE, &w.coordinator), 0);

    let ended = w.ended.all();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].contract.0, l.contract.id);
    assert_eq!(ended[0].outcome, "expired");
    assert_eq!(
        ended[0].visits_cancelled,
        [TicketId(l.jan15.id), TicketId(l.feb15.id)]
    );
    assert_eq!(
        ended[0].visits_left,
        [TicketId(l.dec20.id), TicketId(l.dec31.id)]
    );
}

/// AC3: activating AMC-2027-0001 covering Tower A lifts clears the flag.
#[renox::test]
async fn tkt04_activating_a_replacement_clears_the_flag() {
    let w = world().await;
    let l = lifts(&w).await;
    expire(&w, &l).await;
    assert_eq!(ComplianceFlag::open(w.t.db()).await.unwrap().len(), 1);

    let next = w
        .contract(
            BLUEFIRE,
            "2027-01-01",
            "2027-12-31",
            &[(tower_a_lifts(), "lifts")],
        )
        .await;
    assert_eq!(next.status, ContractStatus::Active);
    assert!(ComplianceFlag::open(w.t.db()).await.unwrap().is_empty());
    let cleared = w.cleared.all();
    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0].reason, "no_contract");
    assert_eq!(cleared[0].contract, Some(l.contract.id));
    let flag = ComplianceFlag::query()
        .first(w.t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(flag.cleared_by.as_deref(), Some(REPLACEMENT_ACTIVE));
    assert_eq!(
        w.activated.all().len(),
        2,
        "ServiceContractActivated for both contracts"
    );
}

/// A contract that another `active` one replaces raises no flag.
#[renox::test]
async fn tkt04_no_flag_is_raised_while_another_active_contract_covers_the_subject() {
    let w = world().await;
    let l = lifts(&w).await;
    let other = w
        .contract(
            BLUEFIRE,
            "2026-06-01",
            "2027-12-31",
            &[(tower_a_lifts(), "lifts")],
        )
        .await;
    expire(&w, &l).await;
    assert!(ComplianceFlag::open(w.t.db()).await.unwrap().is_empty());
    assert!(w.raised.all().is_empty());
    assert_eq!(w.told(NO_CONTRACT_NOTICE, &w.manager), 0);
    assert_eq!(w.reload(&other).await.status, ContractStatus::Active);
}

/// AC4: a cleaning contract ending the same way raises no flag.
#[renox::test]
async fn tkt04_a_cleaning_contract_raises_no_flag() {
    let w = world().await;
    let contract = w
        .contract(
            COOLAIR,
            "2026-01-01",
            "2026-12-31",
            &[(tower_a_cleaning(), "cleaning")],
        )
        .await;
    assert!(!contract.compliance_contract);
    let late = w
        .visit(
            &contract,
            "cleaning",
            tower_a_cleaning(),
            "2027-01-15",
            Some(Assignee::party(COOLAIR)),
        )
        .await;
    let report = contract_end::run(w.t.state(), date("2027-01-01"))
        .await
        .unwrap();
    assert_eq!(report.ended.len(), 1);
    assert_eq!(w.t.reload(&late).await.status, TicketStatus::Cancelled);
    assert!(ComplianceFlag::open(w.t.db()).await.unwrap().is_empty());
    assert!(w.raised.all().is_empty());
    assert_eq!(w.told(NO_CONTRACT_NOTICE, &w.manager), 0);
}

/// An `on_hold` visit due after the end date is cancelled like an open one.
#[renox::test]
async fn tkt04_an_on_hold_visit_after_the_end_date_is_cancelled() {
    let w = world().await;
    let contract = w
        .contract(
            COOLAIR,
            "2026-01-01",
            "2026-12-31",
            &[(tower_a_cleaning(), "cleaning")],
        )
        .await;
    let mut held = Ticket {
        number: "TKT-2026-9500".into(),
        subject_type: tower_a_cleaning().subject_type,
        subject_id: tower_a_cleaning().subject_id,
        category_id: Category::by_key(w.t.db(), "cleaning")
            .await
            .unwrap()
            .unwrap()
            .id,
        title: "Held visit".into(),
        assignee_party_id: Some(COOLAIR.0),
        planned_due_date: Some(date("2027-02-01")),
        contract_id: Some(contract.id),
        status: TicketStatus::OnHold,
        ..Ticket::default()
    };
    let mut tx = w.t.db().begin().await.unwrap();
    held.insert_in_state(&mut tx, TicketStatus::OnHold, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    contract_end::run(w.t.state(), date("2027-01-01"))
        .await
        .unwrap();
    assert_eq!(w.t.reload(&held).await.status, TicketStatus::Cancelled);
}

/// AC5: a termination entered on 2026-11-10 with termination date
/// 2026-11-30 cancels nothing until 2026-11-30; a ticket raised on a covered
/// subject on 2026-11-20 still gets the contract's vendor proposed and its
/// SLA; `ServiceContractEnded` is emitted on 2026-11-30.
#[renox::test]
async fn tkt04_a_termination_with_a_future_date_runs_on_that_date() {
    let w = world().await;
    let policy = SlaPolicy::create(
        w.t.db(),
        SlaPolicy {
            key: "amc".into(),
            name_en: "AMC".into(),
            ..SlaPolicy::default()
        },
    )
    .await
    .unwrap();
    let mut draft = w
        .draft(
            COOLAIR,
            "2026-01-01",
            "2027-12-31",
            &[(tower_a_lifts(), "lifts")],
        )
        .await;
    draft.sla_policy_id = Some(policy.id);
    draft.save(w.t.db()).await.unwrap();
    let contract = w.activate(&draft).await;
    let visit = w
        .visit(
            &contract,
            "lifts",
            tower_a_lifts(),
            "2026-12-15",
            Some(Assignee::party(COOLAIR)),
        )
        .await;

    w.terminate(&contract, "Poor service", "2026-11-30")
        .await
        .assert_ok();
    let terminated = w.reload(&contract).await;
    assert_eq!(terminated.status, ContractStatus::Terminated);
    assert_eq!(terminated.termination_date, Some(date("2026-11-30")));
    assert_eq!(
        terminated.termination_reason.as_deref(),
        Some("Poor service")
    );
    assert!(terminated.ended_at.is_none());
    assert!(w.ended.all().is_empty());

    // Until the date it still covers its subjects.
    let opened = Tickets::open_at(
        w.t.state(),
        OpenTicket::new(tower_a_lifts(), "lifts", "Lift stuck"),
        dubai("2026-11-20", 9, 0),
    )
    .await
    .unwrap();
    let raised = w.t.ticket(opened.id.0).await;
    assert_eq!(raised.contract_id, Some(contract.id));
    assert_eq!(
        raised.sla_policy_id,
        Some(policy.id),
        "the contract's SLA policy"
    );
    let proposed =
        wl_ticketing::ServiceContracts::proposed_vendor(w.t.state(), &raised, date("2026-11-20"))
            .await
            .unwrap();
    assert_eq!(proposed, Some(COOLAIR));
    w.t.app.acting_as(&w.coordinator);
    w.t.app
        .get(&format!("/tickets/{}", raised.id))
        .await
        .assert_ok()
        .assert_see("P-00005");

    let before = contract_end::run(w.t.state(), date("2026-11-29"))
        .await
        .unwrap();
    assert!(before.ended.is_empty());
    assert_eq!(
        w.t.reload(&visit).await.status,
        TicketStatus::Assigned,
        "nothing is cancelled yet"
    );
    assert!(w.ended.all().is_empty());

    let on = contract_end::run(w.t.state(), date("2026-11-30"))
        .await
        .unwrap();
    assert_eq!(on.ended.len(), 1);
    assert_eq!(w.t.reload(&visit).await.status, TicketStatus::Cancelled);
    let ended = w.ended.all();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].outcome, "terminated");
    assert_eq!(ended[0].visits_cancelled, [TicketId(visit.id)]);

    // From the date on it covers nothing.
    let after = wl_ticketing::ServiceContracts::coverage(
        w.t.db(),
        &tower_a_lifts(),
        raised.category_id,
        date("2026-12-01"),
    )
    .await
    .unwrap();
    assert!(after.is_none());
}

/// AC6: a termination entered with today's date runs `WRK.RUL.18` at once.
#[renox::test]
async fn tkt04_a_termination_dated_today_runs_at_once() {
    let w = world().await;
    let contract = w
        .contract(
            COOLAIR,
            "2026-01-01",
            "2099-12-31",
            &[(tower_a_lifts(), "lifts")],
        )
        .await;
    let late = w
        .visit(
            &contract,
            "lifts",
            tower_a_lifts(),
            "2099-06-01",
            Some(Assignee::party(COOLAIR)),
        )
        .await;
    let today = wl_kernel::documents::today().to_string();
    w.terminate(&contract, "Vendor closed", &today)
        .await
        .assert_ok();

    assert_eq!(w.t.reload(&late).await.status, TicketStatus::Cancelled);
    assert!(w.reload(&contract).await.ended_at.is_some());
    assert_eq!(w.ended.all().len(), 1);
    assert_eq!(
        ComplianceFlag::open(w.t.db()).await.unwrap().len(),
        1,
        "no_contract"
    );
}

/// A terminate needs a reason, and a contract that is not in force cannot
/// be terminated.
#[renox::test]
async fn tkt04_terminating_needs_a_reason_and_a_contract_in_force() {
    let w = world().await;
    let contract = w
        .contract(
            COOLAIR,
            "2026-01-01",
            "2099-12-31",
            &[(tower_a_lifts(), "lifts")],
        )
        .await;
    w.terminate(&contract, "  ", "2099-01-01")
        .await
        .assert_status(422);
    assert_eq!(w.reload(&contract).await.status, ContractStatus::Active);
    let draft = w
        .draft(
            COOLAIR,
            "2026-01-01",
            "2099-12-31",
            &[(tower_a_lifts(), "lifts")],
        )
        .await;
    w.terminate(&draft, "Never mind", "2099-01-01")
        .await
        .assert_status(422);
    // A draft without coverage cannot be activated.
    let empty = w.draft(COOLAIR, "2026-01-01", "2099-12-31", &[]).await;
    w.try_activate(&empty).await.assert_status(422);
}

/// The end is handled once: a second run, or `handle` again, does nothing.
#[renox::test]
async fn tkt04_the_end_is_handled_once() {
    let w = world().await;
    let l = lifts(&w).await;
    expire(&w, &l).await;
    let again = contract_end::run(w.t.state(), date("2027-01-02"))
        .await
        .unwrap();
    assert!(again.ended.is_empty());
    assert!(
        contract_end::handle(w.t.state(), l.contract.id, date("2027-01-02"))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(w.ended.all().len(), 1);
    assert_eq!(w.raised.all().len(), 1);
    let tasks = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(task_key(l.contract.id).as_str()))
        .count(w.t.db())
        .await
        .unwrap();
    assert_eq!(tasks, 1);
}

/// AC7: a reassignment from the task records the reason `contract_ended`.
#[renox::test]
async fn tkt04_a_reassignment_from_the_task_records_contract_ended() {
    let w = world().await;
    let l = lifts(&w).await;
    expire(&w, &l).await;

    w.t.app.acting_as(&w.coordinator);
    let imran = w.imran.id.to_string();
    w.t.app
        .htmx()
        .post(
            &format!(
                "/service-contracts/{}/visits/{}/reassign",
                l.contract.id, l.dec20.id
            ),
            &[("assignee_user_id", imran.as_str())],
        )
        .await
        .assert_ok();
    let visit = w.t.reload(&l.dec20).await;
    assert_eq!(visit.assignee_user_id, Some(w.imran.id));
    let history = w.t.history(&visit).await;
    assert_eq!(
        history.last().unwrap().reason_code.as_deref(),
        Some("contract_ended")
    );

    // Another contract's visit is not reachable through this page.
    let other = w
        .contract(
            BLUEFIRE,
            "2026-06-01",
            "2027-12-31",
            &[(tower_a_cleaning(), "cleaning")],
        )
        .await;
    let theirs = w
        .visit(
            &other,
            "cleaning",
            tower_a_cleaning(),
            "2026-12-01",
            Some(Assignee::party(BLUEFIRE)),
        )
        .await;
    w.t.app
        .htmx()
        .post(
            &format!(
                "/service-contracts/{}/visits/{}/reassign",
                l.contract.id, theirs.id
            ),
            &[("assignee_user_id", imran.as_str())],
        )
        .await
        .assert_status(404);
}

/// AC8: with lifts set to reassign only, the task offers only "reassign" for
/// the 12-20 and 12-31 visits and refuses a cancel; a cleaning contract's
/// task still offers reassign or cancel.
#[renox::test]
async fn tkt04_a_reassign_only_category_offers_no_cancel() {
    let w = world().await;
    let mut lifts_category = Category::by_key(w.t.db(), "lifts").await.unwrap().unwrap();
    lifts_category.contract_end_reassign_only = true;
    lifts_category.save(w.t.db()).await.unwrap();

    let l = lifts(&w).await;
    let cleaning = w
        .contract(
            COOLAIR,
            "2026-01-01",
            "2026-12-31",
            &[(tower_a_cleaning(), "cleaning")],
        )
        .await;
    let clean = w
        .visit(
            &cleaning,
            "cleaning",
            tower_a_cleaning(),
            "2026-12-20",
            Some(Assignee::party(COOLAIR)),
        )
        .await;
    let report = contract_end::run(w.t.state(), date("2027-01-01"))
        .await
        .unwrap();
    assert_eq!(report.ended.len(), 2);

    w.t.app.acting_as(&w.coordinator);
    let page =
        w.t.app
            .get(&format!("/service-contracts/{}", l.contract.id))
            .await;
    page.assert_ok();
    for visit in [&l.dec20, &l.dec31] {
        page.assert_see(&format!("reassign-visit-{}", visit.id));
        page.assert_dont_see(&format!("cancel-visit-{}", visit.id));
    }
    let page =
        w.t.app
            .get(&format!("/service-contracts/{}", cleaning.id))
            .await;
    page.assert_ok()
        .assert_see(&format!("reassign-visit-{}", clean.id))
        .assert_see(&format!("cancel-visit-{}", clean.id));

    w.t.app
        .htmx()
        .post(
            &format!(
                "/service-contracts/{}/visits/{}/cancel",
                l.contract.id, l.dec20.id
            ),
            &[("reason", "Not needed")],
        )
        .await
        .assert_status(422);
    assert_eq!(w.t.reload(&l.dec20).await.status, TicketStatus::Assigned);
    w.t.app
        .htmx()
        .post(
            &format!(
                "/service-contracts/{}/visits/{}/cancel",
                cleaning.id, clean.id
            ),
            &[("reason", "Not needed")],
        )
        .await
        .assert_ok();
    assert_eq!(w.t.reload(&clean).await.status, TicketStatus::Cancelled);
}

/// The contract page needs `service_contracts.view`, and activating or
/// terminating `service_contracts.manage`.
#[renox::test]
async fn tkt04_the_contract_page_and_its_actions_are_behind_permissions() {
    let w = world().await;
    let contract = w
        .contract(
            COOLAIR,
            "2026-01-01",
            "2099-12-31",
            &[(tower_a_lifts(), "lifts")],
        )
        .await;
    let nobody = w.t.user("Nobody", &[wl_ticketing::permissions::VIEW]).await;
    w.t.app.acting_as(&nobody);
    w.t.app
        .get(&format!("/service-contracts/{}", contract.id))
        .await
        .assert_forbidden();

    w.t.app.acting_as(&w.coordinator);
    w.t.app
        .get(&format!("/service-contracts/{}", contract.id))
        .await
        .assert_ok()
        .assert_see(&contract.number);
    w.t.app
        .htmx()
        .post(
            &format!("/service-contracts/{}/terminate", contract.id),
            &[("reason", "x"), ("termination_date", "2099-01-01")],
        )
        .await
        .assert_forbidden();
    assert_eq!(w.reload(&contract).await.status, ContractStatus::Active);
}

/// BL-INV-24 (#87): the kernel coverage of a contract on a day runs up to
/// its end date, or its termination date, and a renewal covers the days
/// after the old end.
#[renox::test]
async fn inv24_the_coverage_of_a_day_runs_up_to_the_end_or_termination_date() {
    use wl_kernel::tickets::{ContractCover, ServiceContractCoverage, ServiceContractId};
    use wl_ticketing::contract::ContractCoverage;

    let w = world().await;
    let contract = w
        .contract(
            COOLAIR,
            "2026-01-01",
            "2027-12-31",
            &[(tower_a_lifts(), "lifts")],
        )
        .await;
    let id = ServiceContractId(contract.id);
    let cover = |day: &str| {
        let db = w.t.db();
        let on = date(day);
        async move { ContractCoverage.covers(db, id, on).await.unwrap() }
    };
    assert_eq!(cover("2027-12-31").await, ContractCover::Covered);
    assert_eq!(
        cover("2028-01-01").await,
        ContractCover::AfterEnd {
            ends_on: date("2027-12-31")
        }
    );
    assert_eq!(
        ContractCoverage
            .covers(w.t.db(), ServiceContractId(9_999), date("2026-10-05"))
            .await
            .unwrap(),
        ContractCover::Unknown
    );

    w.terminate(&contract, "Poor service", "2026-11-30")
        .await
        .assert_ok();
    assert_eq!(cover("2026-11-30").await, ContractCover::Covered);
    assert_eq!(
        cover("2026-12-01").await,
        ContractCover::AfterEnd {
            ends_on: date("2026-11-30")
        }
    );

    // A renewal that starts the day after covers the later days.
    use renox::state_machine::StateMachine;
    let mut renewal = ServiceContract {
        number: "AMC-2026-0900".into(),
        vendor_party_id: COOLAIR.0,
        title: "Renewal".into(),
        start_date: date("2026-12-01"),
        end_date: date("2027-12-31"),
        previous_version_id: Some(contract.id),
        status: ContractStatus::Active,
        ..ServiceContract::default()
    };
    let mut tx = w.t.db().begin().await.unwrap();
    renewal
        .insert_in_state(&mut tx, ContractStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(cover("2026-12-15").await, ContractCover::Covered);
    assert_eq!(
        cover("2028-01-01").await,
        ContractCover::AfterEnd {
            ends_on: date("2027-12-31")
        }
    );
}

/// Edge case: the system cancel of the visits due after the end still runs
/// for a category set to reassign only; only the coordinator's cancel is
/// withheld.
#[renox::test]
async fn tkt04_the_system_cancel_runs_for_a_reassign_only_category() {
    let w = world().await;
    let mut lifts_category = Category::by_key(w.t.db(), "lifts").await.unwrap().unwrap();
    lifts_category.contract_end_reassign_only = true;
    lifts_category.save(w.t.db()).await.unwrap();
    let l = lifts(&w).await;

    expire(&w, &l).await;

    for late in [&l.jan15, &l.feb15] {
        let visit = w.t.reload(late).await;
        assert_eq!(visit.status, TicketStatus::Cancelled);
        assert_eq!(visit.cancel_reason.as_deref(), Some(CONTRACT_ENDED_REASON));
    }
    assert_eq!(w.t.reload(&l.dec20).await.status, TicketStatus::Assigned);
}

/// Edge case: no task is opened when no visit remains on or before the end
/// date (every open visit is due after it and is cancelled).
#[renox::test]
async fn tkt04_no_task_is_opened_when_no_visits_remain() {
    let w = world().await;
    let contract = w
        .contract(
            COOLAIR,
            "2026-01-01",
            "2026-12-31",
            &[(tower_a_lifts(), "lifts")],
        )
        .await;
    let vendor = Some(Assignee::party(COOLAIR));
    let jan = w
        .visit(&contract, "lifts", tower_a_lifts(), "2027-01-15", vendor)
        .await;
    let started = w
        .visit(&contract, "lifts", tower_a_lifts(), "2026-12-10", vendor)
        .await;
    w.start(&started).await;

    let report = contract_end::run(w.t.state(), date("2027-01-01"))
        .await
        .unwrap();
    assert_eq!(report.ended.len(), 1);
    assert_eq!(report.ended[0].cancelled, [jan.id]);
    assert!(report.ended[0].left.is_empty());
    assert_eq!(report.ended[0].task, None);
    let key = task_key(contract.id);
    let tasks = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(key.as_str()))
        .get(w.t.db())
        .await
        .unwrap();
    assert!(tasks.is_empty(), "no task: {tasks:?}");
    assert_eq!(w.ended.all()[0].visits_left, Vec::<TicketId>::new());
}

/// Edge case: a contract that was renewed does not end: `WRK.RUL.18` leaves
/// its visits alone, raises no flag and emits no event (the `renew`
/// transition itself belongs to the renewal issue; the row is placed in the
/// `renewed` state).
#[renox::test]
async fn tkt04_a_renewed_contract_does_not_trigger_the_rule() {
    let w = world().await;
    let mut contract = ServiceContract {
        number: "AMC-2026-0800".into(),
        vendor_party_id: COOLAIR.0,
        title: "Renewed".into(),
        start_date: date("2026-01-01"),
        end_date: date("2026-12-31"),
        status: ContractStatus::Renewed,
        compliance_contract: true,
        ..ServiceContract::default()
    };
    let mut tx = w.t.db().begin().await.unwrap();
    contract
        .insert_in_state(&mut tx, ContractStatus::Renewed, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Coverage::create(
        w.t.db(),
        Coverage {
            contract_id: contract.id,
            subject_type: tower_a_lifts().subject_type,
            subject_id: tower_a_lifts().subject_id,
            category_id: Category::by_key(w.t.db(), "lifts")
                .await
                .unwrap()
                .unwrap()
                .id,
            ..Coverage::default()
        },
    )
    .await
    .unwrap();
    let jan = w
        .visit(
            &contract,
            "lifts",
            tower_a_lifts(),
            "2027-01-15",
            Some(Assignee::party(COOLAIR)),
        )
        .await;

    let report = contract_end::run(w.t.state(), date("2027-01-01"))
        .await
        .unwrap();
    assert!(report.ended.is_empty() && report.expired == 0);
    assert!(
        contract_end::handle(w.t.state(), contract.id, date("2027-01-01"))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(w.t.reload(&jan).await.status, TicketStatus::Assigned);
    assert!(ComplianceFlag::open(w.t.db()).await.unwrap().is_empty());
    assert!(w.ended.all().is_empty());
    assert_eq!(w.reload(&contract).await.ended_at, None);
}

/// Done-when 1, edge case: the coordinator's task is due exactly one working
/// day (Asia/Dubai calendar) after the run: a run on Friday 1 January 2027
/// at 10:00 gives Monday 4 January 10:00; Thursday 31 December 2026 gives
/// Friday.
#[renox::test]
async fn tkt04_the_task_is_due_one_working_day_after_the_run() {
    use renox::chrono::Timelike;
    use wl_kernel::calendar::calendar;

    let w = world().await;
    lifts(&w).await;
    // Friday 2027-01-01, 10:00 Dubai (06:00 UTC).
    let target = dubai("2027-01-01", 10, 0);
    let seconds = (target - renox::db::now()).num_seconds();
    assert!(seconds > 0);
    w.t.app
        .travel(std::time::Duration::from_secs(seconds as u64 + 60));
    let report =
        w.t.app
            .at_travelled_time(contract_end::run(w.t.state(), date("2027-01-01")))
            .await
            .unwrap();
    let task = Ticket::find(w.t.db(), report.ended[0].task.expect("a task"))
        .await
        .unwrap()
        .unwrap();
    let due = task.due_at.expect("a due time");
    let local = calendar(w.t.state()).local(due);
    assert_eq!(local.date_naive(), date("2027-01-04"), "Monday");
    assert_eq!(local.hour(), 10, "{due} {local} {target}");
    assert!(local.minute() < 5, "{local}");
    assert!(due >= dubai("2027-01-04", 10, 0));
}
