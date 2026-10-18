//! The host reads the client's `client.toml` (wl-kernel §5.1).

use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::{ClientConfig, LegalEntity};

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

    let app = TestApp::new(host::app()).await;
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
        r#"enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments"]"#,
        r#"enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "inventory"]"#,
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
    let app = TestApp::new(host::app().provide(SmsGatewayHandle(Arc::new(Refusing)))).await;
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
