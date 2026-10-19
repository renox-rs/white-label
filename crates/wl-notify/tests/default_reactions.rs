//! The default give-up reaction of a type registered without a list
//! (#60 AC1 to AC6). The types come from `support`; the providers are
//! scripted, the staff task is the module's own over a recording task
//! creator. `install_defaults.rs` covers the backfill (AC8) and
//! `undelivered.rs` the query (AC9).

mod support;

use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::Subject;
use wl_kernel::notify::{self, ContactPurpose, GiveUpReaction, NotifyChannel, NotifyRequest};
use wl_kernel::tickets::Assignee;
use wl_notify::{NotificationType, ReactionRun, SendError, failure_notice};

use support::{
    Booted, PAIR_STUDENT, SARA, SEEDED_BOOKING, SEEDED_BOOKING_URGENT, SEEDED_GUARDIAN_URGENT,
    SEEDED_INVOICE, SEEDED_OTP, SEEDED_PLAIN, SEEDED_PLAIN_URGENT, SEEDED_SECURITY,
    SEEDED_STATUTORY, boot, boot_alternative, boot_staff,
};

use GiveUpReaction::{AlternativeRecipient, MarkContactInvalid, StaffTask, StatutoryNotice};

const ALERT: &str = "notify-give-up-alert";

async fn stored(t: &Booted, key: &str) -> NotificationType {
    NotificationType::by_key(t.db(), key)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no stored type {key}"))
}

/// The kernel registry and the installed row agree on the reactions.
async fn assert_reactions(t: &Booted, key: &str, want: &[GiveUpReaction], role: Option<&str>) {
    let registered = notify::notification_type(key).unwrap();
    assert_eq!(registered.give_up, want, "registered {key}");
    assert_eq!(registered.staff_role.as_deref(), role, "role of {key}");
    let row = stored(t, key).await;
    assert_eq!(row.give_up.0, want, "stored {key}");
    assert_eq!(row.staff_role.as_deref(), role, "stored role of {key}");
}

/// AC1, AC2, AC3 and AC4: the stored rows hold the defaults.
#[renox::test]
async fn installed_rows_hold_the_default_of_their_group() {
    let t = boot().await;
    assert_reactions(
        &t,
        SEEDED_INVOICE,
        &[MarkContactInvalid, AlternativeRecipient, StaffTask],
        Some("receivables"),
    )
    .await;
    // Statutory wins over urgency: no staff task.
    assert_reactions(
        &t,
        SEEDED_STATUTORY,
        &[MarkContactInvalid, StatutoryNotice],
        None,
    )
    .await;
    // OTP: nothing, although the type is urgent.
    assert_reactions(&t, SEEDED_OTP, &[GiveUpReaction::None], None).await;
    assert_reactions(&t, SEEDED_PLAIN, &[StaffTask], None).await;
    assert_reactions(&t, SEEDED_PLAIN_URGENT, &[StaffTask], None).await;
    assert_reactions(
        &t,
        SEEDED_GUARDIAN_URGENT,
        &[AlternativeRecipient, StaffTask],
        None,
    )
    .await;
    assert_reactions(
        &t,
        SEEDED_BOOKING_URGENT,
        &[MarkContactInvalid, StaffTask],
        None,
    )
    .await;
    assert_reactions(&t, SEEDED_BOOKING, &[MarkContactInvalid], None).await;
    assert_reactions(
        &t,
        SEEDED_SECURITY,
        &[MarkContactInvalid, StaffTask],
        Some("hr"),
    )
    .await;
}

/// AC3: an urgent OTP type whose SMS gives up opens no task and sends no
/// alert; `failure_notice` names the other open channel.
#[renox::test]
async fn an_otp_give_up_opens_no_task_and_sends_no_alert() {
    let t = boot_staff().await;
    let admin = t.holder("Dana", COMPANY_ADMIN_ROLE, None).await;

    t.give_up_on_sara(SEEDED_OTP, 90, "Login code").await;
    assert!(t.tasks.opened().is_empty());
    assert!(t.notices(&admin, ALERT).await.is_empty());
    assert!(t.calls().is_empty(), "no other reaction ran either");

    let failure = failure_notice(t.app.state(), NotifyChannel::Sms);
    assert_eq!(failure.text, "Code could not be sent; try email");
    assert_eq!(failure.other_channels, [NotifyChannel::Email]);
}

/// AC4: a type with no known group gives one task, and the alert when it
/// is urgent.
#[renox::test]
async fn a_type_with_no_group_gives_a_task_and_an_urgent_one_the_alert() {
    let t = boot_staff().await;
    let admin = t.holder("Dana", COMPANY_ADMIN_ROLE, None).await;

    t.give_up_on_sara(SEEDED_PLAIN, 91, "Plain").await;
    assert_eq!(t.tasks.opened().len(), 1);
    assert!(t.notices(&admin, ALERT).await.is_empty());

    t.give_up_on_sara(SEEDED_PLAIN_URGENT, 92, "Urgent").await;
    assert_eq!(t.tasks.opened().len(), 2);
    assert_eq!(t.notices(&admin, ALERT).await.len(), 1);
}

/// AC5: an urgent guardian type: the second guardian gets the message, and
/// when it gives up too, one task and one alert for the whole chain.
#[renox::test]
async fn an_urgent_guardian_give_up_gives_one_task_and_one_alert() {
    let t = boot_alternative().await;
    let admin = t.holder("Dana", COMPANY_ADMIN_ROLE, None).await;
    t.email
        .then((0..2).map(|_| Err(SendError::permanent("550", "mailbox unknown"))));
    t.sms
        .then((0..2).map(|_| Err(SendError::permanent("504", "timeout"))));
    let request = NotifyRequest::new(SEEDED_GUARDIAN_URGENT, Subject::new("invoicing.invoice", 7))
        .to_contact(PAIR_STUDENT, ContactPurpose::guardian_of());
    let state = t.app.state();
    t.app
        .at_travelled_time(notify::notifier(state).notify(state, request))
        .await
        .expect("the notifier queues the message");
    t.app.run_all_jobs().await;

    let messages = t.messages().await;
    assert!(
        messages.iter().any(|m| m.alternative_of.is_some()),
        "the other guardian was tried"
    );
    assert_eq!(t.tasks.opened().len(), 1, "one task, not two");
    assert_eq!(t.notices(&admin, ALERT).await.len(), 1, "one alert");
    assert_eq!(t.gave_up.all().len(), 2, "both chains gave up");
}

/// AC5: an urgent booking type opens a task and alerts; a normal one only
/// marks the contact invalid.
#[renox::test]
async fn bookings_open_a_task_only_when_urgent() {
    let t = boot_staff().await;
    let admin = t.holder("Dana", COMPANY_ADMIN_ROLE, None).await;

    t.give_up_on_sara(SEEDED_BOOKING, 93, "Booking").await;
    assert!(t.tasks.opened().is_empty());
    assert!(t.notices(&admin, ALERT).await.is_empty());
    assert!(
        t.calls().iter().all(|(r, _)| *r == MarkContactInvalid) && !t.calls().is_empty(),
        "only mark_contact_invalid ran"
    );

    t.give_up_on_sara(SEEDED_BOOKING_URGENT, 94, "Urgent booking")
        .await;
    assert_eq!(t.tasks.opened().len(), 1);
    assert_eq!(t.notices(&admin, ALERT).await.len(), 1);
}

/// AC6: an employee-security type gives HR the task, messages nobody else
/// and emits `MessageGaveUp`.
#[renox::test]
async fn an_employee_security_give_up_gives_hr_a_task_and_no_other_contact() {
    let t = boot_staff().await;
    let hr = t.holder("Huda", "hr", None).await;

    let event = t.give_up_on_sara(SEEDED_SECURITY, 95, "Security").await;
    assert_eq!(event.type_key, SEEDED_SECURITY);
    let opened = t.tasks.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].assignee, Some(Assignee::user(hr.id)));

    let messages = t.messages().await;
    assert_eq!(messages.len(), 2, "the e-mail and the SMS to the person");
    assert!(
        messages
            .iter()
            .all(|m| m.party_id == Some(SARA.0) && m.alternative_of.is_none())
    );
    let runs = ReactionRun::of_message(t.db(), event.message.0)
        .await
        .unwrap();
    let steps = runs[0].steps(t.db()).await.unwrap();
    assert!(
        steps.iter().all(|s| s.reaction != AlternativeRecipient),
        "never alternative_recipient"
    );
}
