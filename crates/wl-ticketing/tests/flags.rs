//! BL-TKT-03 (#148): compliance flags and `WRK.RUL.14`: a planned visit not
//! resolved three days after its planned due date tells the coordinator, and
//! for a compliance-critical category the manager too and raises the flag
//! `visit_missed`, cleared when the visit is resolved.

mod support;

use renox::auth::permissions;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::compliance::{ComplianceAlert, is_critical};
use wl_kernel::tickets::{
    ComplianceFlagCleared, ComplianceFlagRaised, ComplianceFlagReason, OpenTicket,
};
use wl_kernel::Subject;
use wl_ticketing::flags::{self, ComplianceFlag, Raise, VISIT_RESOLVED};
use wl_ticketing::visit_check::{self, NOTIFICATION_TYPE, RULE};
use wl_ticketing::{Category, Priority, Ticket, TicketStatus, Tickets, WlTicketing, permissions as perm};

use support::{Booted, Recorded, boot_app, dubai, record, CLIENT};

/// "Tower A fire alarm" and "Tower A cleaning".
fn alarm() -> Subject {
    Subject::new("property.building_system", 4)
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

struct World {
    t: Booted,
    raised: Recorded<ComplianceFlagRaised>,
    cleared: Recorded<ComplianceFlagCleared>,
    coordinator: User,
    manager: User,
    technician: User,
}

async fn world() -> World {
    let raised = Recorded::new();
    let cleared = Recorded::new();
    let (r, c) = (raised.clone(), cleared.clone());
    let t = boot_app(
        CLIENT,
        WlTicketing::new().compliance_manager_role("property_manager"),
        move |app| record(record(app, &r), &c),
    )
    .await;
    permissions::define_role(t.db(), "coordinator", &[perm::VIEW, perm::ASSIGN, perm::WORK])
        .await
        .unwrap();
    for role in ["property_manager", "admin"] {
        permissions::define_role(t.db(), role, &[perm::VIEW]).await.unwrap();
    }
    let staff = |name: &'static str, role: &'static str| {
        let t = &t;
        async move {
            let email = format!("{}@example.com", name.to_lowercase());
            let user = User::register(t.db(), name, &email, "password123")
                .await
                .unwrap();
            user.assign_role(t.db(), role).await.unwrap();
            user
        }
    };
    let coordinator = staff("Cora", "coordinator").await;
    let manager = staff("Pam", "property_manager").await;
    let technician = t.technician("Tim").await;
    for (key, name, critical) in [
        ("fire_safety", "Fire and life safety", true),
        ("cleaning", "Cleaning", false),
    ] {
        Category::create(
            t.db(),
            Category {
                key: key.into(),
                name_en: name.into(),
                default_priority: Priority::Normal,
                compliance_critical: critical,
                active: true,
                ..Category::default()
            },
        )
        .await
        .unwrap();
    }
    World {
        t,
        raised,
        cleared,
        coordinator,
        manager,
        technician,
    }
}

impl World {
    /// A planned visit of `category` on `subject`, planned due on `due`,
    /// with its appointment on 2 November at 10:00, assigned to the
    /// technician.
    async fn visit(&self, category: &str, subject: Subject, due: NaiveDate) -> Ticket {
        let request = OpenTicket::new(subject, category, "Quarterly visit").planned_due(due);
        let opened = Tickets::open_at(self.t.state(), request, dubai("2026-10-25", 8, 0))
            .await
            .unwrap();
        let ticket = self.t.ticket(opened.id.0).await;
        let tech = self.technician.id.to_string();
        self.t
            .assign(
                &self.coordinator,
                &ticket,
                "2026-10-26 09:00",
                &[("user", tech.as_str()), ("visit", "2026-11-02T10:00:00")],
            )
            .await
            .assert_ok();
        self.t.reload(&ticket).await
    }

    fn told(&self, user: &User) -> usize {
        self.t
            .notices
            .of_type(NOTIFICATION_TYPE)
            .iter()
            .filter(|r| {
                r.recipients
                    .iter()
                    .any(|rc| matches!(rc, wl_kernel::notify::NotifyRecipient::User { id } if *id == user.id))
            })
            .count()
    }
}

/// AC1: a planned fire-safety visit with planned due date 1 November (its
/// appointment on 2 November) and not resolved: on 4 November the
/// coordinator and the property manager are told, the flag `visit_missed` is
/// raised and `ComplianceFlagRaised` is emitted; not before, and once.
#[renox::test]
async fn tkt03_a_missed_fire_safety_visit_tells_both_and_raises_the_flag() {
    let w = world().await;
    let visit = w.visit("fire_safety", alarm(), day(1)).await;
    assert_eq!(visit.planned_due_date, Some(day(1)));

    let early = visit_check::run(w.t.state(), day(3)).await.unwrap();
    assert_eq!(early.missed, 0);
    assert!(w.t.notices.of_type(NOTIFICATION_TYPE).is_empty());

    let report = visit_check::run(w.t.state(), day(4)).await.unwrap();
    assert_eq!((report.missed, report.flags, report.notices), (1, 1, 2));
    assert_eq!(w.told(&w.coordinator), 1);
    assert_eq!(w.told(&w.manager), 1);
    let open = ComplianceFlag::open(w.t.db()).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].reason, ComplianceFlagReason::VisitMissed);
    assert_eq!(open[0].subject(), alarm());
    assert_eq!(open[0].ticket_id, Some(visit.id));
    let events = w.raised.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].subject, alarm());
    assert_eq!(events[0].category, "fire_safety");
    assert_eq!(events[0].reason, "visit_missed");
    assert!(w.t.reload(&visit).await.missed_notified_at.is_some());

    // The next night tells nobody again and raises nothing.
    let again = visit_check::run(w.t.state(), day(5)).await.unwrap();
    assert_eq!((again.missed, again.flags, again.notices), (0, 0, 0));
    assert_eq!(w.raised.all().len(), 1);
    assert_eq!(w.told(&w.coordinator), 1);
}

/// AC2: a planned cleaning visit missed the same way only tells the
/// coordinator, and no flag is raised.
#[renox::test]
async fn tkt03_a_missed_cleaning_visit_only_tells_the_coordinator() {
    let w = world().await;
    w.visit("cleaning", alarm(), day(1)).await;
    let report = visit_check::run(w.t.state(), day(4)).await.unwrap();
    assert_eq!((report.missed, report.flags, report.notices), (1, 0, 1));
    assert_eq!(w.told(&w.coordinator), 1);
    assert_eq!(w.told(&w.manager), 0);
    assert!(ComplianceFlag::open(w.t.db()).await.unwrap().is_empty());
    assert!(w.raised.all().is_empty());
}

/// AC3: when the visit is resolved on 6 November the flag clears with "visit
/// resolved" and `ComplianceFlagCleared` is emitted; a visit cancelled by
/// the coordinator does not clear its flag.
#[renox::test]
async fn tkt03_resolving_the_visit_clears_the_flag_but_cancelling_does_not() {
    let w = world().await;
    let visit = w.visit("fire_safety", alarm(), day(1)).await;
    let other = w
        .visit("fire_safety", Subject::new("property.building_system", 5), day(1))
        .await;
    visit_check::run(w.t.state(), day(4)).await.unwrap();
    assert_eq!(ComplianceFlag::open(w.t.db()).await.unwrap().len(), 2);

    // A user cannot cancel a compliance-critical planned visit (BL-PRP-24,
    // `PRP.ACL.15`); the system's cancel when the contract ends does, and the
    // flag stays.
    w.t.run(&w.coordinator, &other, "cancel", "2026-11-05 09:00", &[("reason", "Not needed")])
        .await
        .assert_status(422);
    wl_ticketing::work::auto_cancel(w.t.state(), &other, "The contract ended")
        .await
        .unwrap();
    assert_eq!(w.t.reload(&other).await.status, TicketStatus::Cancelled);
    assert_eq!(ComplianceFlag::open(w.t.db()).await.unwrap().len(), 2);
    assert!(w.cleared.all().is_empty());

    // Resolved: that visit's flag clears.
    w.t.run(&w.technician, &visit, "start", "2026-11-06 09:00", &[])
        .await
        .assert_ok();
    // A compliance-critical visit is resolved with the service report attached.
    w.t.app.acting_as(&w.technician);
    w.t.app
        .htmx()
        .post(
            &format!("/tickets/{}/service-report", visit.id),
            &[("reference", "SR-2026-114")],
        )
        .await
        .assert_ok();
    w.t.run(&w.technician, &visit, "resolve", "2026-11-06 11:00", &[])
        .await
        .assert_ok();
    let open = ComplianceFlag::open(w.t.db()).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].subject_id, 5);
    let cleared = ComplianceFlag::find(w.t.db(), 1).await.unwrap().unwrap();
    assert_eq!(cleared.cleared_by.as_deref(), Some(VISIT_RESOLVED));
    assert!(cleared.cleared_at.is_some());
    let events = w.cleared.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].subject, alarm());
    assert_eq!(events[0].reason, "visit_missed");
    assert_eq!(w.t.reload(&visit).await.status, TicketStatus::Resolved);
}

/// One open flag per subject, category and reason: raising it again does
/// nothing; a category that is not compliance-critical raises none; a
/// cleared flag can be raised again.
#[renox::test]
async fn tkt03_one_open_flag_per_subject_category_and_reason() {
    let w = world().await;
    let state = w.t.state();
    let raise = |category: &str| Raise {
        subject: alarm(),
        category: category.to_owned(),
        reason: ComplianceFlagReason::NoContract,
        contract: Some(7),
        ticket: None,
    };
    let at = dubai("2026-11-04", 9, 0);
    let first = flags::raise(state, raise("fire_safety"), at).await.unwrap();
    assert!(first.is_some());
    assert!(flags::raise(state, raise("fire_safety"), at).await.unwrap().is_none());
    assert!(flags::raise(state, raise("cleaning"), at).await.unwrap().is_none());
    assert!(flags::raise(state, raise("unknown"), at).await.unwrap().is_none());
    assert_eq!(w.raised.all().len(), 1);
    assert_eq!(w.raised.all()[0].contract, Some(7));

    let id = first.unwrap().id;
    let done = flags::clear(state, id, "replacement contract active", at).await.unwrap();
    assert_eq!(done.unwrap().cleared_by.as_deref(), Some("replacement contract active"));
    assert!(flags::clear(state, id, "again", at).await.unwrap().is_none());
    assert_eq!(w.cleared.all().len(), 1);
    assert!(flags::raise(state, raise("fire_safety"), at).await.unwrap().is_some());
    assert_eq!(ComplianceFlag::open(w.t.db()).await.unwrap().len(), 1);
}

/// AC4: the compliance report lists the open flags of Tower A, with reason
/// and age, and not those of another subject.
#[renox::test]
async fn tkt03_the_compliance_report_lists_the_open_flags_of_a_subject() {
    let w = world().await;
    let visit = w.visit("fire_safety", alarm(), day(1)).await;
    w.visit("fire_safety", Subject::new("property.building_system", 9), day(1))
        .await;
    visit_check::run(w.t.state(), day(4)).await.unwrap();

    let viewer = w.t.user("Vera", &[perm::VIEW]).await;
    w.t.app.acting_as(&viewer);
    w.t.app
        .get("/compliance-flags?subject_type=property.building_system&subject_id=4")
        .await
        .assert_ok()
        .assert_see("property.building_system #4")
        .assert_see("Planned visit missed")
        .assert_see(&visit.number)
        .assert_dont_see("property.building_system #9");
    w.t.app
        .get("/compliance-flags")
        .await
        .assert_ok()
        .assert_see("property.building_system #9");

    let nobody = w.t.user("Nadia", &[]).await;
    w.t.app.acting_as(&nobody);
    w.t.app.get("/compliance-flags").await.assert_forbidden();
}

/// AC5: `WRK.RUL.14` is compliance-critical: a run that fails after five
/// attempts alerts the business owner (BL-KRN-08), and a run for a date
/// that has one does not run twice.
#[renox::test]
async fn tkt03_a_failed_run_alerts_the_business_owner() {
    use renox::testing::TestApp;
    use std::time::Duration;

    let w = world().await;
    assert!(is_critical(RULE));
    let admin = {
        let user = User::register(w.t.db(), "Ada", "ada@example.com", "password123")
            .await
            .unwrap();
        user.assign_role(w.t.db(), "admin").await.unwrap();
        user
    };
    w.visit("fire_safety", alarm(), day(1)).await;
    w.t.notices.fail_on(Some(NOTIFICATION_TYPE));

    let app: &TestApp = &w.t.app;
    for wait in [60_u64, 300, 900, 3600] {
        app.at_travelled_time(visit_check::run(w.t.state(), day(4)))
            .await
            .unwrap();
        app.travel(Duration::from_secs(wait));
    }
    app.at_travelled_time(visit_check::run(w.t.state(), day(4)))
        .await
        .unwrap();

    let alerts = ComplianceAlert::open(w.t.db()).await.unwrap();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].rule, RULE);
    assert!(
        w.t.notices
            .all()
            .iter()
            .any(|r| r.recipients.iter().any(|rc| matches!(
                rc,
                wl_kernel::notify::NotifyRecipient::User { id } if *id == admin.id
            ))),
        "the owner is told"
    );
}
