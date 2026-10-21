//! BL-CLN-04 (#352): `CLN.RUL.10` asks the patient for a new insurance card
//! two days before an insured appointment whose policy ends before the visit
//! or whose card has expired, flags the appointment for the front desk, and
//! clears the flag only when staff accept a card valid on the day. Through the
//! daily run, the real routes and the kernel's notifier and document events.

mod support;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::Subject;
use wl_kernel::documents::{DocumentAccepted, DocumentId, DocumentUploaded};
use wl_kernel::notify::NotifyRecipient;
use wl_scheduling::Booking;
use wl_scheduling::pages::local_text;

use clinics::card_requests::{self, CARD_KIND, CARD_UPLOAD_REQUEST};
use clinics::flags::{AppointmentFlag, FlagCause};
use clinics::policies::{self, NewPolicy};
use clinics::{InsurancePolicy, appointments};
use renox::chrono::NaiveDate;
use support::{Clinic, PATIENT, PATIENT_EMAIL, at, boot, boot_real_notify, clock_at, day, dubai};

const PAYER: PartyId = PartyId(900);

/// A doctor, the consultation and a way to make policies.
struct Scene {
    t: Clinic,
    doctor: wl_scheduling::Resource,
    service: wl_scheduling::ServiceType,
}

async fn scene() -> Scene {
    scene_on(boot().await).await
}

async fn scene_on(t: Clinic) -> Scene {
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    Scene { t, doctor, service }
}

impl Scene {
    /// A policy of the patient valid through `valid_to`, its card through
    /// `card_to`.
    async fn policy(&self, valid_to: i64, card_to: Option<i64>) -> InsurancePolicy {
        let mut new =
            NewPolicy::new(PATIENT, PAYER, "Gold", "M-1", day(-200)).valid_to(day(valid_to));
        if let Some(card) = card_to {
            new = new.card_valid_to(day(card));
        }
        policies::create(self.t.state(), None, new).await.unwrap()
    }

    /// An appointment on `date` at 09:00, insured with `policy`.
    async fn insured(&self, date: i64, policy: &InsurancePolicy) -> Booking {
        self.insured_at(date, 9, policy).await
    }

    /// Like [`insured`](Scene::insured) at `hour:00` (the doctor has one
    /// booking at a time).
    async fn insured_at(&self, date: i64, hour: u32, policy: &InsurancePolicy) -> Booking {
        let booking = self
            .t
            .book(&self.service, &self.doctor, at(day(date), hour))
            .await;
        appointments::attach_policy(self.t.state(), booking.id, policy.id)
            .await
            .unwrap();
        booking
    }

    fn asked(&self) -> Vec<wl_kernel::notify::NotifyRequest> {
        self.t
            .messages
            .sent()
            .into_iter()
            .filter(|r| r.type_key == CARD_UPLOAD_REQUEST)
            .collect()
    }

    async fn flags(&self, booking: &Booking) -> Vec<AppointmentFlag> {
        AppointmentFlag::open_of(self.t.db(), booking.id)
            .await
            .unwrap()
    }

    /// Staff accept a card valid through `date` for the policy.
    async fn accept(&self, policy: &InsurancePolicy, date: i64) -> renox::testing::TestResponse {
        let rita = self.t.receptionist().await;
        self.t.app.acting_as(&rita);
        self.t
            .app
            .htmx()
            .post(
                &format!("/clinics/policies/{}/card", policy.id),
                &[("card_valid_to", day(date).to_string().as_str())],
            )
            .await
    }
}

fn long(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

/// Done when 1: an insured appointment on day 5 and a policy valid through day
/// 3: at 02:00 of day 3 the appointment is flagged for the front desk and the
/// patient gets one message with the upload link. (The hold for the quiet
/// hours is proved with the real wl-notify in
/// `a_card_request_queued_at_night_is_held_until_08_00`.)
#[renox::test]
async fn a_policy_that_ends_before_the_visit_is_asked_for_a_new_card() {
    let s = scene().await;
    let policy = s.policy(3, None).await;
    let booking = s.insured(5, &policy).await;
    assert!(s.asked().is_empty(), "five days ahead: nothing yet");

    let ran = card_requests::run_daily(s.t.state(), at(day(3), 2))
        .await
        .unwrap();
    assert_eq!(ran, [booking.id]);

    let flags = s.flags(&booking).await;
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].cause, FlagCause::InsuranceCard);
    let asked = s.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].recipients,
        [NotifyRecipient::Party { id: PATIENT }]
    );

    // The same day again: still one message and one flag.
    let again = card_requests::run_daily(s.t.state(), at(day(3), 2))
        .await
        .unwrap();
    assert!(again.is_empty());
    assert_eq!(s.asked().len(), 1);
    assert_eq!(s.flags(&booking).await.len(), 1);

    // The front desk sees it.
    s.t.app.acting_as(&s.t.receptionist().await);
    s.t.app
        .get(&format!("/clinics/appointments/{}", booking.id))
        .await
        .assert_ok()
        .assert_see("Insurance card to renew");
}

/// Done when 2: a card that expires on day 4 on a policy valid for months: the
/// same happens.
#[renox::test]
async fn an_expiring_card_is_asked_for_too() {
    let s = scene().await;
    let policy = s.policy(300, Some(4)).await;
    let booking = s.insured(5, &policy).await;

    card_requests::run_daily(s.t.state(), at(day(3), 2))
        .await
        .unwrap();
    assert_eq!(s.flags(&booking).await.len(), 1);
    assert_eq!(s.asked().len(), 1);
}

/// Done when 3 and 5: a valid policy and card: nothing happens; a self-pay
/// appointment is never asked; and the message names the clinic, the day, the
/// time and the link, and no service.
#[renox::test]
async fn a_valid_cover_is_left_alone_and_the_message_names_no_service() {
    let s = scene().await;
    let good = s.policy(300, Some(300)).await;
    s.insured(5, &good).await;
    // A self-pay appointment: no policy, nothing asked.
    s.t.book(&s.service, &s.doctor, at(day(5), 11)).await;
    let bad = s.policy(3, None).await;
    assert!(
        card_requests::run_daily(s.t.state(), at(day(3), 2))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(s.asked().is_empty());

    // An insured one that needs a card: the text.
    let booking = s.insured_at(5, 10, &bad).await;
    card_requests::run_daily(s.t.state(), at(day(3), 2))
        .await
        .unwrap();
    let asked = s.asked();
    assert_eq!(asked.len(), 1);
    let body = support::Messages::body(&asked[0]);
    assert!(body.contains("Palm Grove Clinic LLC"), "{body}");
    assert!(body.contains(&long(day(5))), "{body}");
    assert!(body.contains("10:00"), "{body}");
    assert!(
        body.contains(&format!("/portal/link/{}/document_upload", PATIENT.0)),
        "{body}"
    );
    assert!(!body.to_lowercase().contains("consultation"), "{body}");
    assert_eq!(s.flags(&booking).await.len(), 1);
}

/// Done when 4: staff accept an uploaded card; an upload alone clears nothing,
/// a card that does not cover the day clears nothing, a card valid to a year
/// from now clears the flag.
#[renox::test]
async fn only_a_card_that_staff_accept_and_that_covers_the_day_clears_the_flag() {
    let s = scene().await;
    let policy = s.policy(3, None).await;
    let booking = s.insured(5, &policy).await;
    card_requests::run_daily(s.t.state(), at(day(3), 2))
        .await
        .unwrap();
    assert_eq!(s.flags(&booking).await.len(), 1);

    // The patient uploads: wl-documents takes it for review. Nothing clears.
    let subject = Subject::new("clinics.insurance_policy", policy.id);
    s.t.state()
        .emit(DocumentUploaded {
            document: DocumentId(41),
            subject: subject.clone(),
            kind: CARD_KIND.to_owned(),
            uploaded_by: None,
            source: "portal".to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(
        s.flags(&booking).await.len(),
        1,
        "an upload alone does not clear it"
    );

    // A card valid to day 4 does not reach the day of the visit.
    s.accept(&policy, 4).await.assert_ok();
    assert_eq!(s.flags(&booking).await.len(), 1);
    // One valid to next year does; the policy's own end moves with it.
    s.accept(&policy, 365).await.assert_ok();
    assert!(s.flags(&booking).await.is_empty());
    let all = AppointmentFlag::of_booking(s.t.db(), booking.id)
        .await
        .unwrap();
    assert_eq!(all[0].clear_reason.as_deref(), Some("card_accepted"));
    let renewed = InsurancePolicy::find_or_404(s.t.db(), policy.id)
        .await
        .unwrap();
    assert_eq!(renewed.card_valid_to, Some(day(365)));
    assert_eq!(renewed.valid_to, Some(day(365)));

    // The viewer cannot accept one; an expired card is refused.
    s.t.app.acting_as(&s.t.viewer().await);
    s.t.app
        .htmx()
        .post(
            &format!("/clinics/policies/{}/card", policy.id),
            &[("card_valid_to", day(365).to_string().as_str())],
        )
        .await
        .assert_forbidden();
    s.accept(&policy, -3).await.assert_status(422);
}

/// The same through wl-documents: when it accepts the card of the policy, the
/// expiry date it took is the card's last valid day.
#[renox::test]
async fn the_document_desk_accepting_the_card_clears_the_flag() {
    let s = scene().await;
    let policy = s.policy(300, Some(4)).await;
    let booking = s.insured(5, &policy).await;
    card_requests::run_daily(s.t.state(), at(day(3), 2))
        .await
        .unwrap();
    assert_eq!(s.flags(&booking).await.len(), 1);

    let accepted = DocumentAccepted {
        document: DocumentId(41),
        subject: Subject::new("clinics.insurance_policy", policy.id),
        kind: CARD_KIND.to_owned(),
        expires_on: Some(day(365)),
    };
    s.t.state().emit(accepted).await.unwrap();
    assert!(s.flags(&booking).await.is_empty());
    // Another kind of document of the policy changes nothing.
    let other = DocumentAccepted {
        document: DocumentId(42),
        subject: Subject::new("clinics.insurance_policy", policy.id),
        kind: "something_else".to_owned(),
        expires_on: Some(day(10)),
    };
    s.t.state().emit(other).await.unwrap();
    let kept = InsurancePolicy::find_or_404(s.t.db(), policy.id)
        .await
        .unwrap();
    assert_eq!(kept.card_valid_to, Some(day(365)));
}

/// An appointment booked inside the two days is checked at booking; a
/// reschedule runs the check again for the new day; a cancellation takes the
/// flag away; the guardian of a dependant is the one asked.
#[renox::test]
async fn booking_rescheduling_cancelling_and_dependants() {
    let s = scene().await;
    let policy = s.policy(0, None).await;

    // Booked for tomorrow with a policy that ended today: asked at once.
    let soon = s.insured(1, &policy).await;
    assert_eq!(s.asked().len(), 1);
    let flags = s.flags(&soon).await;
    assert_eq!(
        (flags.len(), flags[0].origin.as_str()),
        (1, "policy_attached")
    );

    // Cancelled: no flag.
    let rita = s.t.receptionist().await;
    s.t.app.acting_as(&rita);
    s.t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/cancel", soon.id),
            &[("reason_code", clinics::reasons::COVERAGE_NOT_CONFIRMED)],
        )
        .await
        .assert_status(303);
    assert!(s.flags(&soon).await.is_empty());

    // Rescheduled: the check runs again for the new day.
    let later = s.insured(5, &policy).await;
    assert!(s.flags(&later).await.is_empty(), "five days ahead");
    s.t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/reschedule", later.id),
            &[("starts_at", local_text(at(day(2), 9)).as_str())],
        )
        .await
        .assert_status(303);
    assert_eq!(s.flags(&later).await.len(), 1, "now two days ahead");
    assert_eq!(s.asked().len(), 2);

    // A dependant: the guardian is asked, with the guardian's own link.
    s.t.guardians.set(PartyId(72), PartyId(80));
    let child_policy = policies::create(
        s.t.state(),
        None,
        NewPolicy::new(PartyId(72), PAYER, "Gold", "M-72", day(-200)).valid_to(day(0)),
    )
    .await
    .unwrap();
    let child =
        s.t.book_for(PartyId(72), &s.service, &s.doctor, at(day(1), 15))
            .await;
    appointments::attach_policy(s.t.state(), child.id, child_policy.id)
        .await
        .unwrap();
    let asked = s.asked();
    let last = asked.last().unwrap();
    assert_eq!(
        last.recipients,
        [NotifyRecipient::Party { id: PartyId(80) }]
    );
    assert!(support::Messages::body(last).contains("/portal/link/80/document_upload"));
}

/// Done when 1, with the real wl-notify: the request is made at 22:15, inside
/// the quiet hours (21:00-08:00) of a normal message to a patient, so it is
/// deferred, nothing reaches the provider, and the release at 08:00 sends it.
#[renox::test]
async fn a_card_request_queued_at_night_is_held_until_08_00() {
    use wl_notify::{Message, MessageStatus, release};

    let s = scene_on(boot_real_notify().await).await;
    let policy = s.policy(3, None).await;
    let booking = s.insured(5, &policy).await;
    let date = day(3).to_string();

    clock_at(&s.t, dubai(&date, 22, 15));
    let ran = s
        .t
        .app
        .at_travelled_time(card_requests::run_daily(s.t.state(), dubai(&date, 22, 15)))
        .await
        .unwrap();
    assert_eq!(ran, [booking.id]);
    s.t.app.run_jobs().await;

    let messages = Message::query().order_by(Message::ID).get(s.t.db()).await.unwrap();
    let asked: Vec<_> = messages
        .iter()
        .filter(|m| m.type_key == CARD_UPLOAD_REQUEST)
        .collect();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].status, MessageStatus::Deferred);
    assert_eq!(asked[0].scheduled_for, Some(dubai(&day(4).to_string(), 8, 0)));
    assert!(s.t.outbox.sent().is_empty(), "nothing sent in the quiet hours");

    // Still held one minute before 08:00.
    clock_at(&s.t, dubai(&day(4).to_string(), 7, 59));
    assert_eq!(
        s.t.app
            .at_travelled_time(release::run_due(s.t.state(), renox::db::now()))
            .await
            .unwrap(),
        0
    );
    s.t.app.run_jobs().await;
    assert!(s.t.outbox.sent().is_empty());

    // At 08:00 the scheduled release queues it and the send job sends it.
    clock_at(&s.t, dubai(&day(4).to_string(), 8, 0));
    s.t.app.run_scheduled(release::TASK).await.unwrap();
    s.t.app.run_jobs().await;
    let sent = s.t.outbox.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].address, PATIENT_EMAIL);
    let message = Message::find(s.t.db(), asked[0].id).await.unwrap().unwrap();
    assert_eq!(message.status, MessageStatus::Sent);
}

/// Done when 1, through the registered entry: the task is the 02:00
/// Asia/Dubai one; run with the clock moved to 02:00 of day 3 it flags the
/// appointment and asks the patient once, and run at 02:00 of day 1 (the visit
/// is four days away) it does nothing.
#[renox::test]
async fn the_registered_nightly_task_asks_for_the_card_at_the_moved_clock() {
    let s = scene().await;
    assert_eq!(card_requests::TASK_AT, "02:00");
    assert_eq!(card_requests::TASK_ZONE, "Asia/Dubai");
    let policy = s.policy(3, None).await;
    let booking = s.insured(5, &policy).await;

    clock_at(&s.t, at(day(1), 2));
    s.t.app
        .run_scheduled(card_requests::TASK)
        .await
        .expect("the task runs");
    assert!(s.flags(&booking).await.is_empty());
    assert!(s.asked().is_empty());

    clock_at(&s.t, at(day(3), 2));
    s.t.app
        .run_scheduled(card_requests::TASK)
        .await
        .expect("the task runs");
    assert_eq!(s.flags(&booking).await.len(), 1);
    assert_eq!(s.asked().len(), 1);

    // The same night again: still one message.
    s.t.app
        .run_scheduled(card_requests::TASK)
        .await
        .expect("the task runs");
    assert_eq!(s.asked().len(), 1);
}
