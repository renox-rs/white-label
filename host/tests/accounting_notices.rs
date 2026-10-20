//! BL-ACC-24 (#348), notices, through the real host of the 04 Accounting
//! client with a mail provider that bounces: a document request that gives up
//! marks the contact invalid, gives the engagement manager a task due the
//! next working day and shows *Notice not delivered* with no reminder until
//! the proof is recorded; a return approval request that gives up has
//! reached the other client approver and gives the manager a task; a KPI pack
//! notice that gives up marks the contact invalid and gives the manager a
//! task; with e-mail failing a statutory deadline notice gives the partner a
//! delivery task and stays visible in the portal (04 §4.2, §9 *Notices*,
//! `NTF.RUL.01`).

mod support;

use accounting::request_lists::{self, REQUEST_LIST_NOTICE, RequestList};
use accounting::{Engagement, EngagementStatus, Return, ReturnStatus, ReturnType};
use host::roles::{ADMIN, ENGAGEMENT_MANAGER, PARTNER};
use renox::chrono::{Days, Duration};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{BouncingMail, accounting_booted_with_mail, company, person, tenant};
use wl_kernel::PartyId;
use wl_notify::StatutoryNotice;
use wl_parties::{ContactPoint, ContactStatus, ContactType};
use wl_ticketing::Ticket;

/// An `active` engagement of `company` run by `manager`.
async fn active_engagement(app: &TestApp, company: PartyId, manager: &User) -> Engagement {
    let mut engagement = Engagement {
        client_party_id: company.0,
        title: "Bookkeeping and VAT".to_owned(),
        retainer_fee: 500_000,
        letter_signed: true,
        manager_user_id: Some(manager.id),
        status: EngagementStatus::Active,
        ..Engagement::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    engagement
        .insert_in_state(&mut tx, EngagementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Engagement::find_or_404(app.db(), engagement.id)
        .await
        .unwrap()
}

/// The contact point of `party`.
async fn contact(app: &TestApp, party: PartyId) -> ContactPoint {
    ContactPoint::query()
        .where_(ContactPoint::PARTY_ID.eq(party.0))
        .where_(ContactPoint::CONTACT_TYPE.eq(ContactType::Email))
        .first(app.db())
        .await
        .unwrap()
        .expect("an e-mail contact point")
}

/// Done when 1, and `Integrations` of #347: the request e-mail bounces.
#[renox::test]
async fn a_document_request_that_gives_up_waits_for_the_proof_before_any_reminder() {
    let mail = BouncingMail::default();
    let app = accounting_booted_with_mail(mail.clone()).await;
    let manager = person(&app, "Mona", ENGAGEMENT_MANAGER).await;
    let admin = person(&app, "Ada", ADMIN).await;
    let client = company(&app, "Gulf Trading A LLC").await;
    let engagement = active_engagement(&app, client, &manager).await;

    request_lists::run_period_end(app.state(), "2026-10-31".parse().unwrap())
        .await
        .unwrap();
    app.run_all_jobs().await;
    let list = RequestList::of_engagement(app.db(), engagement.id)
        .await
        .unwrap()
        .remove(0);
    assert!(!mail.attempts().is_empty(), "the request was e-mailed");

    // The contact point is invalid; the manager has one statutory task.
    assert_eq!(contact(&app, client).await.status, ContactStatus::Invalid);
    let record = StatutoryNotice::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a statutory notice record");
    assert_eq!(record.type_key, REQUEST_LIST_NOTICE);
    let tasks = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(format!("notify.statutory:{}", record.id).as_str()))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee_user_id, Some(manager.id));
    let due = tasks[0].due_at.expect("a due time");
    let wait = due - renox::db::now();
    assert!(
        wait > Duration::zero() && wait <= Duration::days(4),
        "{wait}"
    );

    // The engagement shows the request as not delivered.
    app.acting_as(&manager);
    app.get(&format!("/accounting/engagements/{}", engagement.id))
        .await
        .assert_ok()
        .assert_see("Notice not delivered");

    // No reminder is due while the notice is undelivered.
    let due_date = list.due_date.expect("a due date");
    let later = due_date + Days::new(8);
    let held = request_lists::remind(app.state(), later).await.unwrap();
    assert_eq!(held.undelivered, [list.id]);
    assert!(held.reminded.is_empty());

    // The proof is recorded; the reminder is due.
    app.acting_as(&admin);
    app.htmx()
        .post_multipart(
            &format!("/statutory-notices/{}/proof", record.id),
            &[
                ("method", "call"),
                ("delivered_on", &wl_kernel::documents::today().to_string()),
                ("note", "Phoned the finance contact"),
            ],
            &[],
        )
        .await
        .assert_ok();
    let after = request_lists::remind(app.state(), later).await.unwrap();
    assert_eq!(after.reminded, [list.id]);
}

/// A return `InReview` for `company`'s engagement.
async fn return_in_review(app: &TestApp, engagement: &Engagement) -> Return {
    let mut ret = Return {
        client_party_id: engagement.client_party_id,
        engagement_id: engagement.id,
        return_type: ReturnType::Vat,
        period_end: "2026-10-31".parse().unwrap(),
        due_date: "2026-11-28".parse().unwrap(),
        version: 1,
        figures: Some("Output VAT 12,000; input VAT 4,000".to_owned()),
        workpapers_attached: true,
        preparer_user_id: Some(999_999),
        status: ReturnStatus::InReview,
        ..Return::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    ret.insert_in_state(&mut tx, ReturnStatus::InReview, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Return::find_or_404(app.db(), ret.id).await.unwrap()
}

/// A portal user acting for `company` as an owner, with an e-mail address.
async fn approver(app: &TestApp, name: &str, company: PartyId) -> (User, PartyId) {
    let person_party = tenant(app, name).await;
    let user = person(app, name, "member").await;
    wl_portal::PortalAccess::register(
        app.state(),
        user.id,
        wl_portal::NewGrant::new(person_party, company, "client", &["owner"]),
    )
    .await
    .unwrap();
    (user, person_party)
}

/// Done when 2 (first part): a return approval request to two client
/// approvers; one bounces; the other has it; the manager gets a task.
#[renox::test]
async fn a_return_approval_request_that_gives_up_has_reached_the_other_approver_and_gives_a_task() {
    let mail = BouncingMail::default();
    let app = accounting_booted_with_mail(mail.clone()).await;
    let manager = person(&app, "Mona", ENGAGEMENT_MANAGER).await;
    let client = company(&app, "Gulf Trading B LLC").await;
    let engagement = active_engagement(&app, client, &manager).await;
    let (_first, first_party) = approver(&app, "Omar Haddad", client).await;
    let (_second, second_party) = approver(&app, "Lina Saeed", client).await;
    let second_address = contact(&app, second_party).await.value;
    mail.accept(&second_address);
    let ret = return_in_review(&app, &engagement).await;

    app.acting_as(&manager);
    app.htmx()
        .post(&format!("/accounting/returns/{}/review", ret.id), &[])
        .await
        .assert_ok();
    app.run_all_jobs().await;

    let sent_to: Vec<String> = mail.attempts().iter().map(|m| m.address.clone()).collect();
    assert!(
        sent_to.contains(&second_address),
        "the other approver has it: {sent_to:?}"
    );
    assert_eq!(
        contact(&app, first_party).await.status,
        ContactStatus::Invalid
    );
    assert_eq!(
        contact(&app, second_party).await.status,
        ContactStatus::Valid
    );
    let tasks = Ticket::query()
        .where_(Ticket::ASSIGNEE_USER_ID.eq(manager.id))
        .get(app.db())
        .await
        .unwrap();
    assert!(
        tasks.iter().any(|t| t.title.contains("Omar Haddad")),
        "a task to reach the first approver: {:?}",
        tasks.iter().map(|t| &t.title).collect::<Vec<_>>()
    );
}

/// Done when 2 (second part): a published KPI pack whose notice gives up marks
/// the contact invalid and gives the engagement manager a task.
#[renox::test]
async fn a_kpi_pack_notice_that_gives_up_marks_the_contact_and_tasks_the_manager() {
    let mail = BouncingMail::default();
    let app = accounting_booted_with_mail(mail.clone()).await;
    let manager = person(&app, "Mona", ENGAGEMENT_MANAGER).await;
    let client = company(&app, "Gulf Trading C LLC").await;
    let (_user, party) = approver(&app, "Omar Haddad", client).await;
    app.acting_as(&manager);
    let csv = "code,name,debit,credit\n4000,Sales,,100.00\n1000,Bank,100.00,\n";
    app.htmx()
        .post(
            "/accounting/kpi-packs/import",
            &[
                ("client", &client.0.to_string()),
                ("period", "2026-10"),
                ("csv", csv),
            ],
        )
        .await
        .assert_ok();
    let pack = accounting::KpiPack::query()
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    for (code, category) in [("4000", "revenue"), ("1000", "assets")] {
        app.htmx()
            .post(
                &format!("/accounting/kpi-packs/{}/map", pack.id),
                &[("code", code), ("category", category)],
            )
            .await
            .assert_ok();
    }
    app.htmx()
        .post(
            &format!("/accounting/kpi-packs/{}/commentary", pack.id),
            &[("commentary", "A good month.")],
        )
        .await
        .assert_ok();
    app.htmx()
        .post(&format!("/accounting/kpi-packs/{}/publish", pack.id), &[])
        .await
        .assert_ok();
    app.run_all_jobs().await;

    assert_eq!(contact(&app, party).await.status, ContactStatus::Invalid);
    let tasks = Ticket::query()
        .where_(Ticket::ASSIGNEE_USER_ID.eq(manager.id))
        .get(app.db())
        .await
        .unwrap();
    assert!(
        !tasks.is_empty(),
        "the manager has a task to reach the client"
    );
}

/// Integrations (#347): with e-mail failing, a statutory deadline notice gives
/// the partner a delivery task and the notice stays visible to the client
/// approver in the portal.
#[renox::test]
async fn with_email_failing_a_statutory_notice_gives_a_task_and_stays_in_the_portal() {
    let mail = BouncingMail::default();
    let app = accounting_booted_with_mail(mail.clone()).await;
    let manager = person(&app, "Mona", ENGAGEMENT_MANAGER).await;
    let partner = person(&app, "Pia", PARTNER).await;
    let client = company(&app, "Gulf Trading D LLC").await;
    let engagement = active_engagement(&app, client, &manager).await;
    let (approver_user, _party) = approver(&app, "Omar Haddad", client).await;

    app.acting_as(&manager);
    app.htmx()
        .post(
            &format!("/accounting/engagements/{}/suspend", engagement.id),
            &[("reason", "client_request")],
        )
        .await
        .assert_ok();
    app.run_all_jobs().await;

    let record = StatutoryNotice::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a statutory notice record");
    let tasks = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(format!("notify.statutory:{}", record.id).as_str()))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee_user_id, Some(partner.id));
    let notice =
        accounting::deadline_notices::DeadlineNotice::of_engagement(app.db(), engagement.id)
            .await
            .unwrap()
            .remove(0);
    app.acting_as(&approver_user);
    app.get(&format!(
        "/accounting/portal/deadline-notices/{}",
        notice.id
    ))
    .await
    .assert_ok();
}
