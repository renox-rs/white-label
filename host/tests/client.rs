//! The host reads the client's `client.toml` (wl-kernel §5.1).

use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::{ClientConfig, LegalEntity};

mod support;

#[renox::test]
async fn the_demo_client_boots_with_its_legal_entities() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let default = LegalEntity::default_entity(app.db()).await.unwrap();
    assert_eq!(default.code, "PGP");
    assert_eq!(LegalEntity::query().count(app.db()).await.unwrap(), 2);
    // The client's brand colour is the theme every page links.
    app.get("/").await.assert_ok().assert_see("/_renox/theme/");
}

/// #55 AC10: the demo boots with `notify`; wl-parties answers `contacts`
/// for the notifier, and a message to a seeded party goes out by Renox mail.
#[renox::test]
async fn the_demo_client_sends_notifications_to_its_parties() {
    use wl_kernel::notify::{self, MessageId, NotificationTypeDef, NotifyChannel, NotifyRequest};
    use wl_kernel::parties::directory;

    let app = TestApp::new(host::app().provide(support::noon_clock())).await;
    app.kernel().seed().await.unwrap();
    let al_noor = wl_parties::Party::query()
        .where_(wl_parties::Party::LEGAL_NAME_EN.eq("Al Noor Trading LLC"))
        .first(app.db())
        .await
        .unwrap()
        .expect("the seeded party");
    let contacts = directory(app.state())
        .contacts(app.db(), al_noor.party_id())
        .await
        .unwrap()
        .expect("wl-parties answers");
    let email = contacts.point_for(NotifyChannel::Email).expect("an e-mail");
    assert_eq!(email.address, "accounts@alnoortrading.example");
    assert!(email.primary);

    notify::register_type(
        NotificationTypeDef::new("host_test.notice", "property").template(
            NotifyChannel::Email,
            "en",
            Some("Notice {{ number }}"),
            "Your lease {{ number }} is renewed.",
        ),
    );
    let request = NotifyRequest::new(
        "host_test.notice",
        wl_kernel::Subject::new("property.lease", 1),
    )
    .to_party(al_noor.party_id())
    .data(renox::serde_json::json!({ "number": "L-0001" }));
    let state = app.state();
    let ids = notify::notifier(state)
        .notify(state, request)
        .await
        .unwrap();
    let [MessageId(id)] = ids[..] else {
        panic!("one message: {ids:?}")
    };
    app.run_jobs().await;
    let message = wl_notify::Message::find(app.db(), id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(message.status, wl_notify::MessageStatus::Sent);
    app.assert_mail_sent("accounts@alnoortrading.example", "Notice L-0001");
}

#[test]
fn a_client_of_an_unbuilt_app_or_module_is_refused() {
    let demo = std::fs::read_to_string(host::DEMO_CLIENT).unwrap();
    let manpower =
        ClientConfig::parse(&demo.replace(r#"app     = "property""#, r#"app = "manpower""#))
            .unwrap();
    let err = host::app_for(manpower).err().unwrap();
    assert!(err.0.contains("app `manpower` is not built yet"), "{err}");

    let with_inventory = ClientConfig::parse(&demo.replace(
        r#"enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "sales", "hr", "portal"]"#,
        r#"enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "sales", "hr", "portal", "inventory"]"#,
    ))
    .unwrap();
    let err = host::app_for(with_inventory).err().unwrap();
    assert!(err.0.contains("`inventory` is not built yet"), "{err}");
}

#[test]
fn an_unreadable_brand_colour_stops_the_boot() {
    let demo = std::fs::read_to_string(host::DEMO_CLIENT).unwrap();
    let pale = ClientConfig::parse(&demo.replace("#0F766E", "#FFF59D")).unwrap();
    let err = host::app_for(pale).err().unwrap();
    assert!(err.0.contains("the nearest colour that is"), "{err}");
}

#[renox::test]
async fn admins_find_the_legal_entities_in_the_sidebar() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let mut admin = User::register(app.db(), "Rana", "rana@example.com", "secret123")
        .await
        .unwrap();
    admin.email_verified_at = Some(renox::db::now());
    admin.save(app.db()).await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    admin
        .assign_role(app.db(), host::roles::ADMIN)
        .await
        .unwrap();
    app.acting_as(&admin);
    app.get("/legal-entities")
        .await
        .assert_ok()
        .assert_see("Palm Grove Facilities FZ-LLC")
        .assert_see(r#"class="rx-sidebar__link" href="/legal-entities" aria-current="page""#);
}

/// #45 AC10: the demo boots with `documents` enabled, so the kernel guard is
/// the documents module's (a kind that guards a transition is enforced), and
/// the admin role reaches the module's pages.
#[renox::test]
async fn the_demo_client_boots_with_documents_enabled() {
    use wl_kernel::documents::{self, BlockMode};

    struct Anyone;
    impl documents::BlockSubjects for Anyone {
        fn subjects<'a>(
            &'a self,
            _state: &'a AppState,
            _entry: &'a documents::BlockEntry,
            record_id: i64,
        ) -> documents::DocumentFuture<'a, Vec<wl_kernel::Subject>> {
            Box::pin(async move { Ok(vec![wl_kernel::Subject::new("hr.employee", record_id)]) })
        }
    }

    let app = TestApp::new(host::app()).await;
    // `wl:install` first, as the entity-scoped demo seeders expect.
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    documents::register_block_subjects("host_test.activate", Anyone);
    let permit = wl_documents::KindSeed::new("host_test_permit", "Host test permit")
        .subject_type("hr.employee")
        .override_mode(documents::OverrideMode::Never)
        .blocks("host_test.activate", BlockMode::Block);
    wl_documents::kinds::install_seeds(app.db(), [&permit])
        .await
        .unwrap();
    let refused = documents::guard(app.state())
        .check(app.state(), "host_test.activate", 1, documents::today())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        refused.message(),
        "Host test permit missing; override: not allowed"
    );

    let mut admin = User::register(app.db(), "Rana", "rana@example.com", "secret123")
        .await
        .unwrap();
    admin.email_verified_at = Some(renox::db::now());
    admin.save(app.db()).await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    admin
        .assign_role(app.db(), host::roles::ADMIN)
        .await
        .unwrap();
    app.acting_as(&admin);
    app.get("/documents/review").await.assert_ok();
    app.get("/document-kinds").await.assert_ok();
}

/// #64 AC1: the demo boots with `invoicing`; `/invoices` answers 200 with
/// `invoices.view` and 403 without it.
#[renox::test]
async fn the_demo_client_lists_invoices_for_those_who_may_see_them() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    for (name, role, allowed) in [
        ("Rima", host::roles::RECEIVABLES, true),
        ("Mona", host::roles::MEMBER, false),
    ] {
        let email = format!("{}@example.com", name.to_lowercase());
        let mut user = User::register(app.db(), name, &email, "secret123")
            .await
            .unwrap();
        user.email_verified_at = Some(renox::db::now());
        user.save(app.db()).await.unwrap();
        user.assign_role(app.db(), role).await.unwrap();
        app.acting_as(&user);
        let page = app.get("/invoices").await;
        if allowed {
            page.assert_ok();
        } else {
            page.assert_forbidden();
        }
    }
}

/// #146 AC1: the demo boots with `ticketing`; `/tickets` answers 200 with
/// `tickets.view` and 403 without it, and the kernel's task creator is the
/// module's (a ticket opened through it is numbered).
#[renox::test]
async fn the_demo_client_lists_tickets_for_those_who_may_see_them() {
    use wl_kernel::tickets::{OpenTicket, task_creator};

    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    let opened = task_creator(app.state())
        .open(
            app.state(),
            OpenTicket::new(
                wl_kernel::Subject::new("property.unit", 12),
                "general",
                "Leaking tap",
            ),
        )
        .await
        .unwrap()
        .expect("ticketing is on in the demo");
    assert!(opened.number.starts_with("TKT-"), "{}", opened.number);

    for (name, role, allowed) in [
        ("Carla", host::roles::COORDINATOR, true),
        ("Tariq", host::roles::TECHNICIAN, true),
        ("Mona", host::roles::MEMBER, false),
    ] {
        let email = format!("{}@example.com", name.to_lowercase());
        let mut user = User::register(app.db(), name, &email, "secret123")
            .await
            .unwrap();
        user.email_verified_at = Some(renox::db::now());
        user.save(app.db()).await.unwrap();
        user.assign_role(app.db(), role).await.unwrap();
        app.acting_as(&user);
        let page = app.get("/tickets").await;
        if allowed {
            page.assert_ok().assert_see(&opened.number);
        } else {
            page.assert_forbidden();
        }
    }
}

/// #56 AC1, AC5 and AC6 in the demo: with `notify` and `ticketing` both on, a
/// message that gives up opens a real ticket in the `task` category for the
/// holder of the type's staff role, due one working day after the give-up,
/// and a second give-up for the same recipient comments on it.
#[renox::test]
async fn a_give_up_opens_a_real_ticket_for_the_staff_role() {
    use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRecipient};
    use wl_kernel::notify::{GiveUpReaction, NotifyRequest};
    use wl_kernel::{Subject, calendar};
    use wl_notify::{Message, MessageStatus};
    use wl_ticketing::{Category, Comment, Ticket, TicketStatus};

    use std::sync::Arc;
    use wl_notify::channels::{ProviderFuture, SmsGateway, SmsGatewayHandle};

    /// A gateway that refuses every number for good.
    struct Refusing;
    impl SmsGateway for Refusing {
        fn send<'a>(
            &'a self,
            _state: &'a AppState,
            _to: &'a str,
            _text: &'a str,
        ) -> ProviderFuture<'a, renox::Result> {
            Box::pin(async { Err(renox::Error::permanent_message("number refused")) })
        }
    }

    const TYPE: &str = "host_test.give_up";
    let app = TestApp::new(
        host::app()
            .provide(SmsGatewayHandle(Arc::new(Refusing)))
            .provide(support::noon_clock()),
    )
    .await;
    notify::register_type(
        NotificationTypeDef::new(TYPE, "property")
            .channels(&[NotifyChannel::Sms])
            .give_up(&[GiveUpReaction::StaffTask])
            .staff_role(host::roles::COORDINATOR)
            .template(NotifyChannel::Sms, "en", None, "Secret body {{ number }}."),
    );
    app.kernel().seed().await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    let carla = User::register(app.db(), "Carla", "carla@example.com", "secret123")
        .await
        .unwrap();
    carla
        .assign_role(app.db(), host::roles::COORDINATOR)
        .await
        .unwrap();

    // The gateway refuses the number for good, so the only channel of the
    // type fails and the message gives up at once.
    async fn send(app: &TestApp) -> Vec<wl_kernel::notify::MessageId> {
        let request = NotifyRequest::new(TYPE, Subject::new("property.unit", 12))
            .subject_label("Unit 12")
            .to(NotifyRecipient::Address {
                channel: NotifyChannel::Sms,
                address: "+971500000000".to_owned(),
            })
            .data(renox::serde_json::json!({ "number": "L-0001" }));
        let state = app.state();
        app.at_travelled_time(notify::notifier(state).notify(state, request))
            .await
            .unwrap()
    }
    let ids = send(&app).await;
    app.run_all_jobs().await;
    let message = Message::find(app.db(), ids[0].0).await.unwrap().unwrap();
    assert_eq!(message.status, MessageStatus::Abandoned);

    let tickets = Ticket::query().get(app.db()).await.unwrap();
    assert_eq!(tickets.len(), 1, "one task");
    let ticket = &tickets[0];
    let task = Category::by_key(app.db(), "task").await.unwrap().unwrap();
    assert_eq!(ticket.category_id, task.id);
    assert_eq!(ticket.status, TicketStatus::Assigned);
    assert_eq!(ticket.assignee_user_id, Some(carla.id));
    assert_eq!(ticket.title, "Could not reach ****0000 about Unit 12");
    assert!(ticket.description.contains("SMS ****0000: number refused"));
    assert!(!ticket.description.contains("Secret"));
    let gave_up_at = message.updated_at.expect("the give-up time");
    assert_eq!(
        ticket.due_at,
        Some(calendar::calendar(app.state()).add_working_days(gave_up_at, 1))
    );
    assert!(
        ticket
            .source_key
            .as_deref()
            .is_some_and(|k| k.starts_with("notify.give_up:property.unit:12:"))
    );

    // The same recipient again, while the task is open: a comment.
    send(&app).await;
    app.run_all_jobs().await;
    assert_eq!(Ticket::query().count(app.db()).await.unwrap(), 1);
    let comments = Comment::of_ticket(app.db(), ticket.id).await.unwrap();
    assert_eq!(comments.len(), 1);
    assert!(!comments[0].text.contains("Secret"));
}

/// #57 AC1 and AC5 in the demo: with `notify` and `parties` on, a reminder
/// sent to the finance contacts of Blue Dunes LLC goes to Ali; when his
/// number is refused for good, the same reminder goes once to Mona, in
/// Arabic, on her mobile.
#[renox::test]
async fn a_give_up_sends_the_message_to_another_finance_contact() {
    use std::sync::{Arc, Mutex};

    use wl_kernel::notify::{
        self, ContactPurpose, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
    };
    use wl_kernel::{PartyKind, Subject};
    use wl_notify::channels::{ProviderFuture, SmsGateway, SmsGatewayHandle};
    use wl_notify::{Message, MessageStatus};
    use wl_parties::{
        CompanyDetails, ContactPoint, ContactType, Language, NewParty, NewRelationship,
        PersonDetails, RelationshipType, create, relate,
    };

    const ALI: &str = "+971500000311";
    const MONA: &str = "+971500000312";

    /// A gateway that refuses Ali's number for good and records the rest.
    struct Gateway(Arc<Mutex<Vec<(String, String)>>>);
    impl SmsGateway for Gateway {
        fn send<'a>(
            &'a self,
            _state: &'a AppState,
            to: &'a str,
            text: &'a str,
        ) -> ProviderFuture<'a, renox::Result> {
            let refused = to == ALI;
            if !refused {
                self.0
                    .lock()
                    .unwrap()
                    .push((to.to_owned(), text.to_owned()));
            }
            Box::pin(async move {
                if refused {
                    Err(renox::Error::permanent_message("number refused"))
                } else {
                    Ok(())
                }
            })
        }
    }

    const TYPE: &str = "host_test.reminder";
    let sent = Arc::new(Mutex::new(Vec::new()));
    let app =
        TestApp::new(
            host::app()
                .provide(SmsGatewayHandle(Arc::new(Gateway(sent.clone()))))
                .provide(support::noon_clock()),
        )
        .await;
    notify::register_type(
        NotificationTypeDef::new(TYPE, "invoicing")
            .channels(&[NotifyChannel::Sms])
            .give_up(&[GiveUpReaction::AlternativeRecipient])
            .template(
                NotifyChannel::Sms,
                "en",
                None,
                "Invoice {{ number }} is overdue.",
            )
            .template(
                NotifyChannel::Sms,
                "ar",
                None,
                "الفاتورة {{ number }} متأخرة.",
            ),
    );
    app.kernel().seed().await.unwrap();

    let state = app.state();
    let contact = |name: &str, mobile: &str, language| NewParty {
        kind: PartyKind::Person,
        legal_name_en: name.into(),
        default_language: language,
        person: Some(PersonDetails::default()),
        contacts: vec![ContactPoint {
            contact_type: ContactType::Mobile,
            value: mobile.into(),
            is_primary: true,
            ..Default::default()
        }],
        source: "back_office".into(),
        ..Default::default()
    };
    let blue_dunes = create(
        state,
        NewParty {
            kind: PartyKind::Company,
            legal_name_en: "Blue Dunes LLC".into(),
            company: Some(CompanyDetails::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let ali = create(state, contact("Ali", ALI, Language::En))
        .await
        .unwrap();
    let mona = create(state, contact("Mona", MONA, Language::Ar))
        .await
        .unwrap();
    for (person, primary) in [(&mona, false), (&ali, true)] {
        relate(
            state,
            NewRelationship {
                from: person.id,
                to: blue_dunes.id,
                relation_type: RelationshipType::ContactOf,
                position: Some("finance".into()),
                primary,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    let request = NotifyRequest::new(TYPE, Subject::new("invoicing.invoice", 7))
        .to_contact(blue_dunes.party_id(), ContactPurpose::contact_of("finance"))
        .data(renox::serde_json::json!({ "number": "INV-0007" }));
    let ids = app
        .at_travelled_time(notify::notifier(state).notify(state, request))
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    app.run_all_jobs().await;

    let messages = Message::query()
        .order_by(Message::ID)
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(messages.len(), 2, "Ali's and Mona's");
    assert_eq!(messages[0].party_id, Some(ali.id));
    assert_eq!(messages[0].status, MessageStatus::Abandoned);
    assert_eq!(messages[1].party_id, Some(mona.id));
    assert_eq!(messages[1].alternative_of, Some(ids[0].0));
    assert_eq!(messages[1].status, MessageStatus::Sent);
    assert_eq!(
        *sent.lock().unwrap(),
        [(MONA.to_owned(), "الفاتورة INV-0007 متأخرة.".to_owned())]
    );
}

/// #94 part 7: the payments pages render for the accountant: the list, the
/// record form, the tasks and a payment's page (with a removed allocation
/// shown and the reverse sheet offered).
#[renox::test]
async fn the_payment_pages_render_for_the_accountant() {
    use wl_kernel::payments::PaymentStatus;
    use wl_payments::{Allocation, Payment};

    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    let mut user = User::register(app.db(), "Amal", "amal@example.com", "secret123")
        .await
        .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    user.assign_role(app.db(), host::roles::ACCOUNTANT)
        .await
        .unwrap();
    app.acting_as(&user);
    let entity = LegalEntity::default_entity(app.db()).await.unwrap();
    let mut payment = Payment {
        number: Some("RCT-TEST-0001".to_owned()),
        legal_entity_id: entity.id,
        party_id: 1,
        amount: 300_000,
        currency: "AED".to_owned(),
        status: PaymentStatus::Received,
        ..Default::default()
    };
    payment.insert(app.db()).await.unwrap();
    let mut allocation = Allocation::new(
        payment.id,
        &wl_kernel::Subject::new("invoicing.invoice", 7),
        100_000,
        None,
    );
    allocation.removed_at = Some(renox::db::now());
    allocation.removed_reason = Some("bank_return".to_owned());
    allocation.insert(app.db()).await.unwrap();

    app.get("/payments")
        .await
        .assert_ok()
        .assert_see("Payments");
    app.get("/payments/new")
        .await
        .assert_ok()
        .assert_see("Party code");
    app.get("/payments/tasks")
        .await
        .assert_ok()
        .assert_see("No open tasks");
    app.get(&format!("/payments/{}", payment.id))
        .await
        .assert_ok()
        .assert_see("RCT-TEST-0001")
        .assert_see("Returned by the bank")
        .assert_see("Reverse payment");
}

/// #99 part 5: the refund and deposit pages render for the accountant: the
/// lists, a failed refund with its attempt and reason, and the deposit's
/// "Refund failed" badge with its link.
#[renox::test]
async fn the_refund_and_deposit_pages_render_for_the_accountant() {
    use renox::state_machine::StateMachine;
    use wl_kernel::payments::{PaymentMethod, RefundFailureKind};
    use wl_payments::deposit::DepositStatus;
    use wl_payments::refund::RefundStatus;
    use wl_payments::refund_attempt::AttemptOutcome;
    use wl_payments::{Deposit, Refund, RefundAttempt};

    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    let mut user = User::register(app.db(), "Amal", "amal@example.com", "secret123")
        .await
        .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    user.assign_role(app.db(), host::roles::ACCOUNTANT)
        .await
        .unwrap();
    app.acting_as(&user);
    let entity = LegalEntity::default_entity(app.db()).await.unwrap();
    let mut deposit = Deposit {
        party_id: 1,
        legal_entity_id: entity.id,
        source_type: "ticketing.ticket".to_owned(),
        source_id: 5,
        amount: 800_000,
        held_amount: 800_000,
        currency: "AED".to_owned(),
        refund_method: Some(PaymentMethod::Transfer),
        status: wl_payments::deposit::DepositStatus::Refunded,
        ..Default::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    deposit
        .insert_in_state(&mut tx, DepositStatus::Refunded, "fixture")
        .await
        .unwrap();
    let mut refund = Refund {
        party_id: 1,
        legal_entity_id: entity.id,
        source_type: "payments.deposit".to_owned(),
        source_id: deposit.id,
        source_kind: "deposit".to_owned(),
        amount: 800_000,
        currency: "AED".to_owned(),
        reason: "Lease ended".to_owned(),
        method: Some(PaymentMethod::Transfer),
        status: RefundStatus::Failed,
        ..Default::default()
    };
    refund
        .insert_in_state(&mut tx, RefundStatus::Failed, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    deposit.refund_id = Some(refund.id);
    deposit.save(app.db()).await.unwrap();
    let mut attempt = RefundAttempt {
        refund_id: refund.id,
        number: 1,
        method: PaymentMethod::Transfer,
        started_on: renox::chrono::NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
        outcome: AttemptOutcome::Failed,
        failure_kind: Some(RefundFailureKind::BankReturn),
        failure_reason: Some("account closed".to_owned()),
        ..Default::default()
    };
    attempt.insert(app.db()).await.unwrap();

    app.get("/refunds").await.assert_ok().assert_see("Refunds");
    app.get(&format!("/refunds/{}", refund.id))
        .await
        .assert_ok()
        .assert_see("account closed")
        .assert_see("Returned by the bank")
        .assert_see(&format!("/deposits/{}", deposit.id));
    app.get("/deposits")
        .await
        .assert_ok()
        .assert_see("Deposits");
    app.get(&format!("/deposits/{}", deposit.id))
        .await
        .assert_ok()
        .assert_see("Refund failed")
        .assert_see(&format!("/refunds/{}", refund.id));
}

/// #94 AC1: with the demo `client.toml` (`payments` enabled) the host boots,
/// and `/payments` answers 200 with `payments.view` and 403 without it.
#[renox::test]
async fn the_payment_list_needs_the_view_permission() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let mut viewer = User::register(app.db(), "Vera", "vera@example.com", "secret123")
        .await
        .unwrap();
    viewer.email_verified_at = Some(renox::db::now());
    viewer.save(app.db()).await.unwrap();
    renox::auth::permissions::define_role(app.db(), "payments_viewer", &["payments.view"])
        .await
        .unwrap();
    viewer
        .assign_role(app.db(), "payments_viewer")
        .await
        .unwrap();
    let mut nobody = User::register(app.db(), "Nora", "nora@example.com", "secret123")
        .await
        .unwrap();
    nobody.email_verified_at = Some(renox::db::now());
    nobody.save(app.db()).await.unwrap();

    app.acting_as(&viewer);
    app.get("/payments").await.assert_ok();
    app.acting_as(&nobody);
    app.get("/payments").await.assert_forbidden();
}

/// #74 AC1 and AC6: the demo boots with `invoicing`; `/supplier-bills`
/// answers 200 with `supplier_bills.view` and 403 without it, the kernel's
/// `supplier_bills(state)` shows a line of an existing bill, and the demo's
/// `property` app also holds a bill with the same contract, period and amount.
#[renox::test]
async fn the_demo_client_lists_supplier_bills_and_holds_a_repeated_contract_bill() {
    use wl_invoicing::bill_submit::submit;
    use wl_invoicing::credit_notes::By;
    use wl_invoicing::supplier_bill::{self, BillDraft};
    use wl_invoicing::{BillLineDraft, SupplierBill, SupplierBillStatus};
    use wl_kernel::invoicing::{
        SupplierBillLineId, SupplierBillStatus as KernelStatus, VatCategory, supplier_bills,
    };
    use wl_kernel::{PartyId, Subject};

    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    for (name, role, allowed) in [
        ("Rima", host::roles::ACCOUNTANT, true),
        ("Mona", host::roles::MEMBER, false),
    ] {
        let email = format!("{}@example.com", name.to_lowercase());
        let mut user = User::register(app.db(), name, &email, "secret123")
            .await
            .unwrap();
        user.email_verified_at = Some(renox::db::now());
        user.save(app.db()).await.unwrap();
        user.assign_role(app.db(), role).await.unwrap();
        app.acting_as(&user);
        let page = app.get("/supplier-bills").await;
        if allowed {
            page.assert_ok();
        } else {
            page.assert_forbidden();
        }
    }

    // The demo bills exist after seeding; the provider shows one of them.
    wl_invoicing::seed::bills(app.state()).await.unwrap();
    let bill = SupplierBill::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a demo bill");
    let line = supplier_bill::lines_of(app.db(), bill.id).await.unwrap()[0].clone();
    let shown = supplier_bills(app.state())
        .line(app.db(), SupplierBillLineId(line.id))
        .await
        .unwrap()
        .expect("the line");
    assert_eq!(shown.bill.0, bill.id);
    assert_ne!(shown.bill_status, KernelStatus::Draft);

    // `property`: the same contract, period and amount under another number.
    let day = |d| renox::chrono::NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
    let supplier = PartyId(bill.party_id);
    let contract = Subject::new("ticketing.contract", 3);
    let mut results = Vec::new();
    for number in ["C-1", "C-2"] {
        let draft = BillDraft::new(supplier, number, day(5))
            .contract(contract.clone(), Some((day(1), day(31))))
            .line(
                BillLineDraft::new("Cleaning", 100_000, VatCategory::Standard)
                    .service_date(day(10)),
            );
        let made = supplier_bill::create(app.db(), &draft, Some(1))
            .await
            .unwrap();
        let done = submit(app.state(), By::System(Some(1)), made.id)
            .await
            .unwrap();
        results.push(done.status);
    }
    assert_eq!(
        results,
        [SupplierBillStatus::Matched, SupplierBillStatus::Exception]
    );
}
