//! BL-EDU-05 (#440): a membership and its enrolment stay in step
//! (`EDU.RUL.14`). Pausing the subscription freezes the enrolment and freezing
//! the enrolment pauses the subscription, and the same for resume and
//! unfreeze, without a loop and without a transition run twice.
//!
//! The subscriptions are a stand-in that keeps wl-invoicing's rules (a pause
//! cause per reason, a resume when none is left, `SubscriptionPaused` and
//! `SubscriptionResumed` after each); wl-invoicing's own test proves the events
//! (`crates/wl-invoicing/tests/subscription_pause.rs`).

mod support;

use edtech::enrolments::{self, NewEnrolment};
use edtech::{Enrolment, EnrolmentFreeze, EnrolmentStatus, FreezeCause};
use renox::prelude::*;
use renox::state_machine::history;
use support::{Centre, boot, day};
use wl_kernel::Subject;
use wl_kernel::subscriptions::{PauseCause, SubscriptionStatus};

/// An active membership enrolment of Sara and its agreement.
async fn membership(t: &Centre) -> (Enrolment, Subject) {
    let sara = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::membership(sara, "club", "Conversation club", 31_500).terms_accepted(),
    )
    .await
    .unwrap();
    let active = enrolments::activate(t.state(), Some(&desk), e.id)
        .await
        .unwrap();
    assert_eq!(active.status, EnrolmentStatus::Active);
    let subject = active.subject();
    assert_eq!(t.subs.status_of(&subject), Some(SubscriptionStatus::Active));
    (active, subject)
}

async fn reload(t: &Centre, e: &Enrolment) -> Enrolment {
    Enrolment::find_or_404(t.db(), e.id).await.unwrap()
}

async fn transitions(t: &Centre, e: &Enrolment) -> Vec<String> {
    history(t.db(), "edtech_enrolments", e.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.transition)
        .collect()
}

/// Given membership M `active` and enrolment E `active`, when the guardian
/// pauses M for 14 days, E is `frozen` (cause *linked*, no day counted against
/// the allowance, nothing asked of the subscription in return); when M
/// resumes, E is `active`.
#[renox::test]
async fn edu_05_pausing_the_membership_freezes_the_enrolment_and_resuming_unfreezes_it() {
    let t = boot().await;
    let (e, subject) = membership(&t).await;

    t.subs.user_pauses(t.state(), &subject, day(14)).await;

    let frozen = reload(&t, &e).await;
    assert_eq!(frozen.status, EnrolmentStatus::Frozen);
    let rows = EnrolmentFreeze::open_of(t.db(), e.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cause, FreezeCause::Linked);
    assert_eq!(rows[0].counted_days, 0);
    assert_eq!(rows[0].until_on, Some(day(14)));
    assert!(t.subs.pauses().is_empty(), "no second pause is asked for");
    assert_eq!(t.subs.status_of(&subject), Some(SubscriptionStatus::Paused));

    t.subs.user_resumes(t.state(), &subject, false).await;

    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Active);
    assert!(EnrolmentFreeze::open_of(t.db(), e.id).await.unwrap().is_empty());
    assert!(t.subs.resumes().is_empty(), "no second resume is asked for");
}

/// When staff freeze E, M is `paused` with the agreement's cause; the days are
/// counted once, by the enrolment; unfreezing E resumes M.
#[renox::test]
async fn edu_05_freezing_the_enrolment_pauses_the_membership_and_the_allowance_is_counted_once() {
    let t = boot().await;
    let (e, subject) = membership(&t).await;
    let desk = t.desk().await;

    enrolments::freeze(t.state(), Some(&desk), e.id, FreezeCause::Student, Some(day(14)))
        .await
        .unwrap();

    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Frozen);
    assert_eq!(t.subs.status_of(&subject), Some(SubscriptionStatus::Paused));
    assert_eq!(t.subs.causes_of(&subject), [PauseCause::Agreement]);
    assert_eq!(
        t.subs.pauses(),
        [(subject.clone(), PauseCause::Agreement)],
        "one pause, asked once"
    );
    // The enrolment counted the 15 days (today to the 14th, both counted); the
    // pause of the subscription is the agreement's and counts none.
    let used = EnrolmentFreeze::counted_in_year(t.db(), 1, day(0)).await.unwrap();
    assert_eq!(used, 15);

    enrolments::unfreeze(t.state(), Some(&desk), e.id, FreezeCause::Student)
        .await
        .unwrap();

    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Active);
    assert_eq!(t.subs.status_of(&subject), Some(SubscriptionStatus::Active));
    assert_eq!(t.subs.resumes(), [(subject.clone(), PauseCause::Agreement)]);
}

/// Given M already `paused` by a block, freezing E does not run a second
/// `pause`; unfreezing E asks nothing of M either.
#[renox::test]
async fn edu_05_a_membership_paused_by_a_block_is_not_paused_again() {
    let t = boot().await;
    let (e, subject) = membership(&t).await;
    let desk = t.desk().await;
    t.subs.block_pauses(t.state(), &subject).await;
    assert_eq!(t.subs.causes_of(&subject), [PauseCause::PartyBlock]);

    enrolments::freeze(t.state(), Some(&desk), e.id, FreezeCause::Student, Some(day(10)))
        .await
        .unwrap();

    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Frozen);
    assert!(t.subs.pauses().is_empty(), "no second pause");
    assert_eq!(t.subs.causes_of(&subject), [PauseCause::PartyBlock]);

    enrolments::unfreeze(t.state(), Some(&desk), e.id, FreezeCause::Student)
        .await
        .unwrap();
    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Active);
    assert!(t.subs.resumes().is_empty());
    assert_eq!(t.subs.status_of(&subject), Some(SubscriptionStatus::Paused));
}

/// No transition fires more than once in the history for one user action: a
/// pause and a resume by the guardian, then a freeze and an unfreeze by staff,
/// leave exactly four runs on E after its activation and one pause and one resume asked of M.
#[renox::test]
async fn edu_05_no_transition_runs_twice_for_one_user_action() {
    let t = boot().await;
    let (e, subject) = membership(&t).await;
    let desk = t.desk().await;

    t.subs.user_pauses(t.state(), &subject, day(14)).await;
    t.subs.user_resumes(t.state(), &subject, false).await;
    enrolments::freeze(t.state(), Some(&desk), e.id, FreezeCause::Student, Some(day(7)))
        .await
        .unwrap();
    enrolments::unfreeze(t.state(), Some(&desk), e.id, FreezeCause::Student)
        .await
        .unwrap();

    let names = transitions(&t, &e).await;
    let from = names
        .iter()
        .rposition(|name| name == "activate")
        .expect("the activation is in the history");
    assert_eq!(
        names[from..],
        [
            "activate",
            "freeze_system",
            "unfreeze_system",
            "freeze",
            "unfreeze"
        ]
    );
    assert_eq!(t.subs.pauses().len(), 1);
    assert_eq!(t.subs.resumes().len(), 1);
}

/// Given M paused while a cycle invoice from before the pause is overdue, on
/// resume M is `past_due` and E is `active`.
#[renox::test]
async fn edu_05_a_membership_resumed_into_past_due_still_unfreezes_the_enrolment() {
    let t = boot().await;
    let (e, subject) = membership(&t).await;
    t.subs.user_pauses(t.state(), &subject, day(14)).await;
    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Frozen);

    t.subs.user_resumes(t.state(), &subject, true).await;

    assert_eq!(t.subs.status_of(&subject), Some(SubscriptionStatus::PastDue));
    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Active);
}

/// The enrolment page shows the membership next to the enrolment's status.
#[renox::test]
async fn edu_05_the_page_shows_the_membership_paused_with_the_enrolment() {
    let t = boot().await;
    let (e, subject) = membership(&t).await;
    let desk = t.desk_user().await;
    t.app.acting_as(&desk);
    let path = format!("/edtech/enrolments/{}", e.id);
    t.app.get(&path).await.assert_ok().assert_see("Billing");

    t.subs.user_pauses(t.state(), &subject, day(14)).await;

    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Frozen")
        .assert_see("Paused")
        .assert_see("The membership is paused");
}
