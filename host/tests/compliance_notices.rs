//! BL-PRP-27 (#277), the notification types and their give-up reactions of the
//! Property client run by the real modules of the host, with the e-mail
//! provider answering that every address is a hard bounce:
//!
//! - a `lease.renewal_window` e-mail that hard-bounces marks the contact
//!   invalid and opens a task for the property manager of the lease's
//!   building (not another building's);
//! - a `lease.payment_demand` whose e-mail gave up keeps a statutory-notice
//!   record and a task for the property manager, due in one working day, to
//!   deliver it another way;
//! - a staff reminder (`property.lease_unsigned`) that fails opens no task.

mod support;

use host::roles::PROPERTY_MANAGER;
use property::UnitUse;
use property::arrears::{self, ArrearsDemand};
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_kernel::BusinessCalendar;
use wl_kernel::ClientConfig;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::notify::NotifyChannel;
use wl_notify::channels::ProviderFuture;
use wl_notify::{
    ChannelProvider, Message, MessageStatus, Outgoing, SendError, StatutoryNotice, WlNotify,
};
use wl_parties::{ContactPoint, ContactStatus, ContactType};
use wl_ticketing::Ticket;

/// An e-mail provider that answers every send with a hard bounce.
struct Bounces;

impl ChannelProvider for Bounces {
    fn name(&self) -> &'static str {
        "bounces"
    }

    fn channel(&self) -> NotifyChannel {
        NotifyChannel::Email
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        _outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
        Box::pin(async {
            Err(SendError::hard(
                wl_notify::channels::HardError::HardBounce,
                "550",
                "mailbox unknown",
            ))
        })
    }
}

/// The Property host whose e-mail always hard-bounces.
async fn booted_bouncing() -> TestApp {
    let config = ClientConfig::load(host::DEMO_CLIENT).expect("the demo client");
    let tuning = host::Tuning::default().notify(|notify: WlNotify| notify.provider(Bounces));
    let app = host::app_for_tuned(config, tuning).expect("the app");
    let app = TestApp::new(app.provide(noon_clock()).module(Probe)).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// A tenant with an active lease of a unit in tower A, whose property manager
/// is Mona (tower A only); Paul manages tower B.
struct Scene {
    app: TestApp,
    tenant: wl_kernel::PartyId,
    lease: property::Lease,
    mona: User,
    paul: User,
}

async fn scene() -> Scene {
    let app = booted_bouncing().await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let unit = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    let lease = active_lease(&app, &unit, tenant, false).await;
    let mona = wl_parties_free_user(&app, "Mona").await;
    let (tower_b, _) = building(&app, "TWR-B").await;
    let paul = scoped_person(
        &app,
        "Paul",
        PROPERTY_MANAGER,
        &property::Building::scope_of(tower_b.id),
    )
    .await;
    // What the setting up queued (the tenant's messages) is sent now.
    app.run_all_jobs().await;
    Scene {
        app,
        tenant,
        lease,
        mona,
        paul,
    }
}

/// Mona, who `active_lease` made the property manager of tower A.
async fn wl_parties_free_user(app: &TestApp, name: &str) -> User {
    User::find_by_email(app.db(), &format!("{}@example.com", name.to_lowercase()))
        .await
        .unwrap()
        .expect("the user")
}

/// The tickets that exist now.
async fn tickets(app: &TestApp) -> Vec<Ticket> {
    Ticket::query()
        .order_by(Ticket::ID)
        .get(app.db())
        .await
        .unwrap()
}

/// The tenant's e-mail contact point.
async fn email_of(app: &TestApp, tenant: wl_kernel::PartyId) -> ContactPoint {
    ContactPoint::query()
        .where_(ContactPoint::PARTY_ID.eq(tenant.0))
        .where_(ContactPoint::CONTACT_TYPE.eq(ContactType::Email))
        .first(app.db())
        .await
        .unwrap()
        .expect("an e-mail contact point")
}

/// A `lease.renewal_window` e-mail that hard-bounces: the contact is marked
/// invalid and the property manager of the lease's building gets a task; the
/// property manager of another building does not.
#[renox::test]
async fn a_renewal_window_hard_bounce_marks_the_contact_invalid_and_tasks_the_leases_manager() {
    let s = scene().await;
    let before = tickets(&s.app).await.len();

    // The renewal window opens: the property managers and the tenant are told.
    property::renewal::flag_due(s.app.state(), day(0))
        .await
        .unwrap();
    s.app.run_all_jobs().await;

    let message = Message::query()
        .where_(Message::TYPE_KEY.eq("lease.renewal_window"))
        .where_(Message::PARTY_ID.eq(s.tenant.0))
        .first(s.app.db())
        .await
        .unwrap()
        .expect("the tenant's message");
    assert_eq!(message.status, MessageStatus::Abandoned, "it gave up");
    let email = email_of(&s.app, s.tenant).await;
    assert_eq!(email.status, ContactStatus::Invalid);
    assert_eq!(email.invalid_reason.as_deref(), Some("hard_bounce"));

    let new: Vec<Ticket> = tickets(&s.app).await.into_iter().skip(before).collect();
    let mine: Vec<&Ticket> = new
        .iter()
        .filter(|t| t.assignee_user_id == Some(s.mona.id))
        .collect();
    assert_eq!(mine.len(), 1, "one task for the lease's property manager");
    assert!(
        new.iter().all(|t| t.assignee_user_id != Some(s.paul.id)),
        "none for the manager of another building"
    );
    let _ = &s.lease;
}

/// A `lease.payment_demand` whose e-mail gave up: a statutory-notice record
/// and one task for the property manager to deliver it another way, due one
/// working day after it gave up.
#[renox::test]
async fn a_payment_demand_that_gave_up_gives_the_manager_a_task_due_in_one_working_day() {
    let s = scene().await;
    let clerk = person(&s.app, "Clara", host::roles::RECEIVABLES).await;
    invoiced_charge(
        &s.app,
        &clerk,
        &s.lease,
        1_000_000,
        VatCategory::Exempt,
        day(-31),
        1,
    )
    .await;
    arrears::run(s.app.state(), day(0)).await.unwrap();
    let demand = ArrearsDemand::of_root(s.app.db(), s.lease.root_id)
        .await
        .unwrap()
        .remove(0);
    let before = tickets(&s.app).await.len();

    arrears::send_demand_as(
        s.app.state(),
        Some(s.mona.id),
        s.lease.id,
        demand.id,
        day(0),
    )
    .await
    .unwrap();
    s.app.run_all_jobs().await;

    let notices = StatutoryNotice::query().get(s.app.db()).await.unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].type_key, "lease.payment_demand");
    assert!(!notices[0].has_proof());
    let task = tickets(&s.app)
        .await
        .into_iter()
        .skip(before)
        .find(|t| t.source_key.as_deref() == Some(&format!("notify.statutory:{}", notices[0].id)))
        .expect("the statutory-notice task");
    assert_eq!(
        task.assignee_user_id,
        Some(s.mona.id),
        "the lease's manager"
    );
    let due = BusinessCalendar::default().add_working_days(notices[0].gave_up_at, 1);
    assert_eq!(
        task.due_at,
        Some(due),
        "due one working day after it gave up"
    );
    assert!(task.title.contains("by another means"), "{}", task.title);
}

/// A staff reminder that fails creates no task: the reaction of a staff-only
/// type is none.
#[renox::test]
async fn a_staff_reminder_that_fails_creates_no_task() {
    let s = scene().await;
    let before = tickets(&s.app).await.len();

    let told = property::notices::lease_unsigned(s.app.state(), &s.lease, 15, day(-15))
        .await
        .unwrap();
    assert!(told.contains(&s.mona.id));
    s.app.run_all_jobs().await;

    let reminders = Message::query()
        .where_(Message::TYPE_KEY.eq("property.lease_unsigned"))
        .get(s.app.db())
        .await
        .unwrap();
    assert!(!reminders.is_empty());
    // Each failed: a first address gave up, one that bounced before is
    // suppressed.
    assert!(
        reminders.iter().all(|m| matches!(
            m.status,
            MessageStatus::Abandoned | MessageStatus::Suppressed
        )),
        "every reminder failed"
    );
    assert!(
        reminders
            .iter()
            .any(|m| m.status == MessageStatus::Abandoned),
        "one of them gave up"
    );
    assert_eq!(tickets(&s.app).await.len(), before, "no task was opened");
}
