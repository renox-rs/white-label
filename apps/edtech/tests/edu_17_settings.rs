//! BL-EDU-17 (#452), the app's side: the 10 values of the shared document,
//! scheduling, notification, alert, sales, staff and connection settings that
//! the app carries as tables (`edtech::defaults`, `edtech::seed`), the types
//! it registers, and the tutor of a one-to-one booking asked through the real
//! wl-scheduling. What the shared modules do with the values is proved through
//! the real host in `host/tests/edtech_settings.rs`.

mod support;

use edtech::defaults;
use edtech::notices::{self, FAMILY_NOTICE, NOTICE, TERMS_REQUEST};
use edtech::seed;
use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use support::{Centre, boot, day, next_weekday};
use wl_kernel::PartyId;
use wl_kernel::notify::{
    GROUP_GUARDIAN, GROUP_INVOICES, GROUP_SIGNATURES, GiveUpReaction, NotifyChannel,
};
use wl_scheduling::{AvailabilityRule, ServiceType, resource, service};

/// The registered definition of `key`.
fn def(key: &str) -> wl_kernel::notify::NotificationTypeDef {
    notices::notification_types()
        .into_iter()
        .find(|d| d.key == key)
        .unwrap_or_else(|| panic!("{key} is a type of the app"))
}

/// Notifications: a message to a guardian or a student that gives up goes to
/// the other guardian, then becomes a task for the counsellor; an adult
/// student without a guardian has no other contact, so the task comes at once.
#[test]
fn edu_17_the_family_notice_tries_the_other_guardian_then_the_counsellor() {
    let family = def(FAMILY_NOTICE);
    assert_eq!(family.group, GROUP_GUARDIAN);
    assert_eq!(
        family.give_up,
        [
            GiveUpReaction::AlternativeRecipient,
            GiveUpReaction::StaffTask
        ]
    );
    assert_eq!(family.staff_role.as_deref(), Some("counsellor"));
    assert!(!family.statutory);
    // WhatsApp first, e-mail when WhatsApp is down (§8).
    assert_eq!(
        family.channels,
        [NotifyChannel::Whatsapp, NotifyChannel::Email]
    );
}

/// Notification types: the terms, refund policy and consent requests are
/// *Portal invitations, document requests, signature requests*, with a task
/// for the counsellor; staff-only types name no group and take `none`; none is
/// statutory.
#[test]
fn edu_17_the_requests_go_to_the_counsellor_and_staff_types_take_none() {
    let request = def(TERMS_REQUEST);
    assert_eq!(request.group, GROUP_SIGNATURES);
    assert_eq!(
        request.give_up,
        [
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::StaffTask
        ]
    );
    assert_eq!(request.staff_role.as_deref(), Some("counsellor"));

    let staff = def(NOTICE);
    assert_eq!(staff.group, "");
    assert_eq!(staff.give_up, [GiveUpReaction::None]);

    assert!(notices::notification_types().iter().all(|d| !d.statutory));
}

/// Notification types: the invoice, membership charge and reminder types of
/// the shared modules go to the accountant after the contact is marked invalid
/// and the other guardian or payer contact is tried.
#[test]
fn edu_17_an_invoice_message_that_gives_up_ends_in_an_accountant_task() {
    assert_eq!(defaults::NOTIFICATION_TYPES.len(), 6);
    for spec in &defaults::NOTIFICATION_TYPES {
        assert_eq!(spec.group, GROUP_INVOICES, "{}", spec.key);
        assert_eq!(
            spec.give_up,
            [
                GiveUpReaction::MarkContactInvalid,
                GiveUpReaction::AlternativeRecipient,
                GiveUpReaction::StaffTask
            ],
            "{}",
            spec.key
        );
        assert_eq!(spec.role, Some("accountant"), "{}", spec.key);
    }
}

/// Scheduling: the one-to-one service type asks the police clearance of its
/// tutor; the trial class and the placement test ask nothing; no service type
/// is seeded with a deposit or the check-in (the host installs them without).
#[test]
fn edu_17_only_the_one_to_one_asks_a_document_of_its_tutor() {
    for spec in seed::SERVICE_TYPES {
        if spec.code == seed::ONE_TO_ONE_SERVICE {
            assert_eq!(spec.qualifications, "police_clearance");
        } else {
            assert_eq!(spec.qualifications, "", "{}", spec.code);
        }
    }
    assert_eq!(seed::ESCALATION_HOURS, 2);
    assert_eq!(seed::BOOKING_LOOK_AHEAD_DAYS, 30);
}

/// Look-ahead: 30 days for the tutor kinds, 60 for the centre permit and the
/// course approval.
#[test]
fn edu_17_the_look_ahead_windows_are_30_and_60_days() {
    assert_eq!(seed::TUTOR_LOOK_AHEAD_DAYS, 30);
    assert_eq!(seed::CENTRE_LOOK_AHEAD_DAYS, 60);
    assert_eq!(edtech::tutor_alerts::WINDOW_DAYS, 30);
    assert_eq!(edtech::tutor_alerts::ESCALATION_HOURS, 24);
    assert_eq!(edtech::enrolments::HOLD_HOURS, 48);
}

/// Alerts: the three compliance-critical rules, with the company owner as the
/// owner of the alert.
#[test]
fn edu_17_the_critical_rules_are_doc_rul_03_and_the_two_edu_rules() {
    assert_eq!(
        defaults::CRITICAL_RULES,
        ["DOC.RUL.03", "EDU.RUL.12", "EDU.RUL.13"]
    );
    assert_eq!(defaults::ALERT_ROLE, "owner");
    assert_eq!(edtech::tutor_alerts::RULE, "EDU.RUL.12");
    assert_eq!(edtech::permits::RULE, "EDU.RUL.13");
}

/// Integrations: the manual fallbacks say what to do while the connection is
/// down, in both languages.
#[test]
fn edu_17_the_fallbacks_say_what_the_desk_and_the_tutor_do() {
    let gateway = defaults::GATEWAY_FALLBACK_EN;
    assert!(gateway.contains("card terminal") && gateway.contains("cash"));
    assert!(gateway.contains("payment links are sent again"));
    assert!(defaults::WHATSAPP_FALLBACK_EN.contains("e-mail"));
    assert!(defaults::ATTENDANCE_FALLBACK_EN.contains("on paper"));
    assert!(defaults::ATTENDANCE_FALLBACK_EN.contains("24 hours"));
    for arabic in [
        defaults::GATEWAY_FALLBACK_AR,
        defaults::WHATSAPP_FALLBACK_AR,
        defaults::EMAIL_FALLBACK_AR,
        defaults::ATTENDANCE_FALLBACK_AR,
    ] {
        assert!(!arabic.trim().is_empty());
    }
    // The tutor's own window for the attendance is the 24 hours of the fallback.
    assert_eq!(defaults::TUTOR_ATTENDANCE_HOURS, 24);
}

/// Lead sources and campaigns: eight sources (not the aggregator), none
/// requires a campaign, six campaign channels recorded as the module's own.
#[test]
fn edu_17_the_lead_sources_and_the_campaign_channels() {
    assert_eq!(
        defaults::LEAD_SOURCES,
        [
            "referral", "walk_in", "website", "social", "paid_ads", "event", "partner", "other"
        ]
    );
    assert!(defaults::CAMPAIGN_REQUIRED_SOURCES.is_empty());
    let names: Vec<&str> = defaults::CAMPAIGN_CHANNELS.iter().map(|c| c.0).collect();
    assert_eq!(
        names,
        [
            "Instagram",
            "TikTok",
            "Search ads",
            "WhatsApp",
            "Email",
            "School and community events"
        ]
    );
}

/// Loyalty: fees of AED 1,000 earn 100 points; 100 points are an AED 10
/// discount, at most 10% of the fee; points expire after 12 months with a
/// reminder 30 days before; an adjustment above 500 points needs a manager.
#[test]
fn edu_17_the_loyalty_values_earn_100_points_for_1000_dirhams() {
    let paid_aed = 1_000;
    assert_eq!(
        paid_aed / defaults::LOYALTY_EARN_PER_AED * defaults::LOYALTY_POINTS,
        100
    );
    let discount = defaults::LOYALTY_MIN_REDEEM * defaults::LOYALTY_POINT_VALUE_FILS;
    assert_eq!(discount, 1_000, "AED 10 in fils");
    // At most 10% of a fee of AED 100: AED 10 passes, AED 10.01 does not.
    let fee_fils = 10_000;
    let cap = fee_fils * defaults::LOYALTY_MAX_SHARE_BPS / 10_000;
    assert!(discount <= cap && discount + 1 > cap);
    assert_eq!(defaults::LOYALTY_VALIDITY_MONTHS, 12);
    assert_eq!(defaults::LOYALTY_REMINDER_DAYS, 30);
    assert_eq!(defaults::LOYALTY_ADJUSTMENT_LIMIT, 500);
    assert_eq!(
        (defaults::MARKETING_CAP, defaults::BROADCAST_APPROVAL_ABOVE),
        (2, 200)
    );
    assert_eq!(defaults::SEND_WINDOW_MINUTES, (420, 1260));
    // Points earned on 2026-11-01 expire a year on, reminded on 2027-10-02.
    let earned = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
    let expires = NaiveDate::from_ymd_opt(2027, 11, 1).unwrap();
    assert_eq!(
        expires - TimeDelta::days(defaults::LOYALTY_REMINDER_DAYS),
        NaiveDate::from_ymd_opt(2027, 10, 2).unwrap()
    );
    assert!(earned < expires);
}

/// Onboarding and SOPs: the three checklists give tasks to the centre
/// manager, the academic coordinator (the tutor's) and the employee and end
/// with the SOPs; the coordinator owns teaching, assessment and safeguarding,
/// the centre manager the front desk and refunds.
#[test]
fn edu_17_the_checklists_and_the_sop_owners() {
    let roles: Vec<&str> = defaults::ONBOARDING.iter().map(|o| o.role).collect();
    assert_eq!(roles, ["tutor", "counsellor", "academic_coordinator"]);
    let tutor = &defaults::ONBOARDING[0];
    for owner in [
        "role:centre_manager",
        "role:academic_coordinator",
        "employee",
    ] {
        assert!(tutor.steps.iter().any(|s| s.1 == owner), "{owner}");
    }
    let owners: Vec<(&str, &str)> = defaults::SOPS
        .iter()
        .map(|s| (s.category, s.owner_role))
        .collect();
    assert_eq!(
        owners,
        [
            ("teaching", "academic_coordinator"),
            ("assessment", "academic_coordinator"),
            ("safeguarding", "academic_coordinator"),
            ("front_desk", "centre_manager"),
            ("refunds", "centre_manager"),
        ]
    );
}

/// `tutor` is free all week.
async fn free_all_week(t: &Centre, tutor: &wl_scheduling::Resource) {
    for weekday in 1..=7 {
        AvailabilityRule::new(tutor.id, weekday, 0, 1440)
            .save(t.db())
            .await
            .unwrap();
    }
}

/// The one-to-one service type of the seed, offered by `tutor` all week.
async fn one_to_one(t: &Centre, tutor: &wl_scheduling::Resource) -> ServiceType {
    let spec = seed::SERVICE_TYPES
        .iter()
        .find(|s| s.code == seed::ONE_TO_ONE_SERVICE)
        .expect("the one-to-one service type");
    let mut service_type = ServiceType::new(spec.code, spec.name_en, spec.minutes);
    service_type.required_qualifications = spec.qualifications.to_owned();
    service_type.save(t.db()).await.unwrap();
    resource::offer(t.db(), service_type.id, tutor.id)
        .await
        .unwrap();
    free_all_week(t, tutor).await;
    service_type
}

/// Books the one-to-one session of `service_type` with `tutor` on the first
/// `weekday` a week from now at 10:00: `booked`, or the refusal in words.
async fn book(
    t: &Centre,
    service_type: &ServiceType,
    tutor: &wl_scheduling::Resource,
    weekday: u32,
) -> String {
    let parent = t.person(40 + i64::from(weekday), "Parent");
    let lesson = next_weekday(day(7), weekday);
    match service::book(
        t.state(),
        wl_scheduling::NewBooking::new(
            PartyId(parent),
            service_type.id,
            &[tutor.id],
            support::at(lesson, 10, 0),
        ),
        wl_scheduling::Actor::staff(1),
        renox::db::now(),
    )
    .await
    {
        Ok(_) => "booked".to_owned(),
        Err(refusal) => format!("{refusal:?}"),
    }
}

/// Scheduling: booking a one-to-one session with a tutor without a police
/// clearance is refused (the clearance name is in the refusal); the same
/// booking with a valid one is made.
#[renox::test]
async fn edu_17_a_one_to_one_with_a_tutor_without_clearance_is_refused() {
    let t = boot().await;
    let (layla, employee) = t.employed_tutor("Layla Hassan", false).await;
    let service_type = one_to_one(&t, &layla).await;

    let refused = book(&t, &service_type, &layla, 2).await;
    assert_ne!(refused, "booked");
    assert!(refused.contains("police clearance"), "{refused}");

    t.documents.set(
        &Centre::employee_subject(employee),
        "police_clearance",
        Some(day(365)),
    );
    assert_eq!(book(&t, &service_type, &layla, 2).await, "booked");
}

/// Scheduling: a freelance tutor also needs the private tutor permit or the
/// freelance visa on the day of the booking; an employed tutor does not.
#[renox::test]
async fn edu_17_a_freelance_tutor_needs_the_permit_for_a_one_to_one() {
    let t = boot().await;
    let (fiona, freelance) = t.employed_tutor("Fiona Freelance", true).await;
    let (layla, employed) = t.employed_tutor("Layla Hassan", false).await;
    for employee in [freelance, employed] {
        t.documents.set(
            &Centre::employee_subject(employee),
            "police_clearance",
            Some(day(365)),
        );
    }
    let service_type = one_to_one(&t, &fiona).await;
    resource::offer(t.db(), service_type.id, layla.id)
        .await
        .unwrap();
    free_all_week(&t, &layla).await;

    let refused = book(&t, &service_type, &fiona, 2).await;
    assert!(
        refused.contains("private tutor permit or freelance visa"),
        "{refused}"
    );
    // The employed tutor holds no permit and is not asked for one.
    assert_eq!(book(&t, &service_type, &layla, 3).await, "booked");

    t.documents.set(
        &Centre::employee_subject(freelance),
        "tutor_permit",
        Some(day(365)),
    );
    assert_eq!(book(&t, &service_type, &fiona, 2).await, "booked");
}
