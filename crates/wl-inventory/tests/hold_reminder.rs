//! Reminding the deciders of an open stock hold and escalating it (wl-inventory
//! §6 `STK.RUL.07`, issue #232): reminders every H hours, the escalation role
//! every 2 x H, nothing for a hold that is decided, closed or cancelled, and
//! the most specific setting for H and for the role.

mod support;

use std::time::Duration;

use renox::auth::permissions;
use renox::prelude::*;
use wl_inventory::hold_reminder::{ESCALATION_NOTICE, REMINDER_NOTICE, TASK};
use wl_inventory::permissions as perm;
use wl_inventory::{HoldStatus, Item, Location, Lot, LotStatus, StockHold, WlInventory};
use wl_kernel::Subject;
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, boot_with};

/// Waits `hours` hours and `minutes` minutes.
fn wait(t: &Booted, hours: u64, minutes: u64) {
    t.app.travel(Duration::from_secs(hours * 3600 + minutes * 60));
}

/// The users each notice of `kind` went to, with the record it was about.
fn told(t: &Booted, kind: &str) -> Vec<(Subject, Vec<i64>)> {
    t.notices
        .all()
        .iter()
        .filter(|request| request.type_key == kind)
        .map(|request| {
            (
                request.subject.clone(),
                request
                    .recipients
                    .iter()
                    .filter_map(|recipient| match recipient {
                        NotifyRecipient::User { id } => Some(*id),
                        _ => None,
                    })
                    .collect(),
            )
        })
        .collect()
}

/// A 06 outlet: H = 2 hours and the kitchen manager is the escalation role.
async fn boot_kitchen() -> Booted {
    let t = boot_with(
        WlInventory::new()
            .hold_reminder_hours(2)
            .hold_escalation_role("kitchen_manager"),
    )
    .await;
    permissions::define_role(t.db(), "kitchen_manager", &[perm::STOCK_VIEW])
        .await
        .unwrap();
    t
}

/// A *temperature* hold on the chicken in Fridge 2, placed now.
async fn fridge_hold(t: &Booted, holder: &User) -> (Location, StockHold) {
    let mut item = Item::new("CHICKEN", "CHICKEN", "kg");
    item.average_cost = 3_000;
    item.save(t.db()).await.unwrap();
    let fridge = t.named_location("FRIDGE-2", "Fridge 2").await;
    t.stock_in(&item, &fridge, None, 5_000, 3_000).await;
    let id = fridge.id.to_string();
    t.hold_as(
        holder,
        &[
            ("reason", "temperature"),
            ("reference", "ticketing.ticket:9"),
            ("location", id.as_str()),
        ],
    )
    .await
    .assert_ok();
    let hold = t.all_holds().await.pop().expect("a hold");
    (fridge, hold)
}

/// Done when 1: a 06 hold placed at 10:00 (H = 2): the deciders are reminded
/// at 12:00 and again at 14:00, the kitchen manager is told at 14:00 and not
/// before, and then every further 2 x H.
#[renox::test]
async fn deciders_are_reminded_every_h_hours_and_the_kitchen_manager_every_two_h() {
    let t = boot_kitchen().await;
    let holder = t.holder("Hal").await;
    let chef = t.releaser("Chef").await;
    let kim = t.user_with("Kim", "kitchen_manager").await;
    let (_fridge, hold) = fridge_hold(&t, &holder).await;

    // 11:59: not yet.
    wait(&t, 1, 59);
    t.app.run_scheduled(TASK).await.unwrap();
    assert!(told(&t, REMINDER_NOTICE).is_empty());

    // 12:00: the deciders are reminded, nobody else.
    wait(&t, 0, 1);
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(
        told(&t, REMINDER_NOTICE),
        vec![(hold.subject(), vec![chef.id])]
    );
    assert!(told(&t, ESCALATION_NOTICE).is_empty());

    // 13:00: the job runs again and sends nothing twice.
    wait(&t, 1, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(told(&t, REMINDER_NOTICE).len(), 1);
    assert!(told(&t, ESCALATION_NOTICE).is_empty());

    // 14:00: the second reminder, and the kitchen manager is told.
    wait(&t, 1, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(told(&t, REMINDER_NOTICE).len(), 2);
    assert_eq!(
        told(&t, ESCALATION_NOTICE),
        vec![(hold.subject(), vec![kim.id])]
    );

    // 16:00: the third reminder, and again every further 2 x H.
    wait(&t, 2, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(told(&t, REMINDER_NOTICE).len(), 3);
    assert_eq!(told(&t, ESCALATION_NOTICE).len(), 1);
    // 18:00: the fourth reminder and the second escalation.
    wait(&t, 2, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(told(&t, REMINDER_NOTICE).len(), 4);
    assert_eq!(told(&t, ESCALATION_NOTICE).len(), 2);
    let kept = StockHold::find_or_404(t.db(), hold.id).await.unwrap();
    assert_eq!((kept.reminders_sent, kept.escalations_sent), (4, 2));
}

/// Done when 2: the last line decided at 13:00: no notice at 14:00; the hold
/// closes, so nothing is reminded for it any more.
#[renox::test]
async fn nothing_is_sent_for_a_hold_decided_before_the_escalation() {
    let t = boot_kitchen().await;
    let holder = t.holder("Hal").await;
    let chef = t.releaser("Chef").await;
    let _kim = t.user_with("Kim", "kitchen_manager").await;
    let (_fridge, hold) = fridge_hold(&t, &holder).await;

    wait(&t, 2, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(told(&t, REMINDER_NOTICE).len(), 1);

    // 13:00: the chef decides the last line.
    wait(&t, 1, 0);
    let line = t.hold_lines(&hold).await.remove(0);
    t.decide_as(&chef, &line, "release").await.assert_ok();
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);

    // 14:00: no reminder, no escalation.
    wait(&t, 1, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(told(&t, REMINDER_NOTICE).len(), 1);
    assert!(told(&t, ESCALATION_NOTICE).is_empty());
}

/// A cancelled hold is never reminded.
#[renox::test]
async fn a_cancelled_hold_is_not_reminded() {
    let t = boot_kitchen().await;
    let holder = t.holder("Hal").await;
    let chef = t.releaser("Chef").await;
    let _kim = t.user_with("Kim", "kitchen_manager").await;
    let (_fridge, hold) = fridge_hold(&t, &holder).await;

    t.cancel_hold_as(&chef, &hold, "").await.assert_ok();
    wait(&t, 5, 0);
    t.app.run_scheduled(TASK).await.unwrap();

    assert!(told(&t, REMINDER_NOTICE).is_empty());
    assert!(told(&t, ESCALATION_NOTICE).is_empty());
}

/// A line waiting for a second person's approval is still undecided: the
/// reminders go on.
#[renox::test]
async fn a_line_waiting_for_approval_is_still_reminded() {
    let t = boot_with(
        WlInventory::new()
            .hold_reminder_hours(2)
            .hold_second_person_reasons(&["temperature"]),
    )
    .await;
    let holder = t.holder("Hal").await;
    let chef = t.releaser("Chef").await;
    let _manager = t.releaser("Mia").await;
    let (_fridge, hold) = fridge_hold(&t, &holder).await;
    let line = t.hold_lines(&hold).await.remove(0);
    t.decide_as(&chef, &line, "release").await.assert_ok();
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Open);

    wait(&t, 2, 0);
    t.app.run_scheduled(TASK).await.unwrap();

    let reminders = told(&t, REMINDER_NOTICE);
    assert_eq!(reminders.len(), 1);
    assert!(reminders[0].1.contains(&chef.id));
}

/// The escalation goes to the business owner (here the company admin) when no
/// role is set.
#[renox::test]
async fn the_escalation_goes_to_the_owner_unless_a_role_is_set() {
    let t = boot_with(WlInventory::new().hold_reminder_hours(1)).await;
    permissions::define_role(t.db(), "admin", &[perm::STOCK_VIEW])
        .await
        .unwrap();
    let ada = t.user_with("Ada", "admin").await;
    let holder = t.holder("Hal").await;
    let _chef = t.releaser("Chef").await;
    let (_fridge, hold) = fridge_hold(&t, &holder).await;

    wait(&t, 2, 0);
    t.app.run_scheduled(TASK).await.unwrap();

    assert_eq!(
        told(&t, ESCALATION_NOTICE),
        vec![(hold.subject(), vec![ada.id])]
    );
}

/// The most specific H wins: the location over the reason over the default;
/// the same for the escalation role.
#[renox::test]
async fn the_location_beats_the_reason_and_the_reason_beats_the_default() {
    let t = boot_with(
        WlInventory::new()
            .hold_reminder_hours(24)
            .hold_reminder_hours_for_reason("quality", 6)
            .hold_reminder_hours_at("FRIDGE-2", 2)
            .hold_escalation_role("owner")
            .hold_escalation_role_for_reason("quality", "quality_lead")
            .hold_escalation_role_at("FRIDGE-2", "kitchen_manager"),
    )
    .await;
    for role in ["owner", "quality_lead", "kitchen_manager"] {
        permissions::define_role(t.db(), role, &[perm::STOCK_VIEW])
            .await
            .unwrap();
    }
    let owner = t.user_with("Olga", "owner").await;
    let lead = t.user_with("Quin", "quality_lead").await;
    let kim = t.user_with("Kim", "kitchen_manager").await;
    let holder = t.holder("Hal").await;
    let _chef = t.releaser("Chef").await;
    let (_fridge, fridge_hold) = fridge_hold(&t, &holder).await;
    // A quality hold on a lot, and an `other` hold on a branch.
    let vaccine = t.tracked_item("VACCINE").await;
    let lot = t.lot(&vaccine, "V-1", LotStatus::Released).await;
    let store = t.named_location("STORE-A", "Store A").await;
    t.stock_in(&vaccine, &store, Some(&lot), 4_000, 1_000).await;
    let lot_id = lot.id.to_string();
    t.hold_as(
        &holder,
        &[
            ("reason", "quality"),
            ("reference", "ticketing.ticket:10"),
            ("lots", lot_id.as_str()),
        ],
    )
    .await
    .assert_ok();
    let quality_hold = t.all_holds().await.pop().unwrap();
    let gauze = t.item("GAUZE").await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&gauze, &branch, None, 4_000, 100).await;
    let branch_id = branch.id.to_string();
    t.hold_as(
        &holder,
        &[
            ("reason", "other"),
            ("reference", "ticketing.ticket:11"),
            ("location", branch_id.as_str()),
        ],
    )
    .await
    .assert_ok();
    let other_hold = t.all_holds().await.pop().unwrap();
    assert!(Lot::find(t.db(), lot.id).await.unwrap().is_some());

    // 2 hours: only the fridge (H = 2).
    wait(&t, 2, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    let subjects: Vec<Subject> = told(&t, REMINDER_NOTICE)
        .into_iter()
        .map(|(subject, _)| subject)
        .collect();
    assert_eq!(subjects, vec![fridge_hold.subject()]);

    // 6 hours: the quality hold too (H = 6), the fridge for the third time
    // and, at 2 x H = 4 hours, its escalation role.
    wait(&t, 4, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    let subjects: Vec<Subject> = told(&t, REMINDER_NOTICE)
        .into_iter()
        .map(|(subject, _)| subject)
        .collect();
    assert!(subjects.contains(&quality_hold.subject()));
    assert!(!subjects.contains(&other_hold.subject()));
    assert_eq!(
        told(&t, ESCALATION_NOTICE)
            .into_iter()
            .filter(|(subject, _)| *subject == fridge_hold.subject())
            .map(|(_, people)| people)
            .collect::<Vec<_>>(),
        vec![vec![kim.id]]
    );

    // 24 hours: the `other` hold falls back to the default H.
    wait(&t, 18, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    let subjects: Vec<Subject> = told(&t, REMINDER_NOTICE)
        .into_iter()
        .map(|(subject, _)| subject)
        .collect();
    assert!(subjects.contains(&other_hold.subject()));
    let escalations = told(&t, ESCALATION_NOTICE);
    assert!(escalations.contains(&(quality_hold.subject(), vec![lead.id])));
    assert!(escalations.contains(&(fridge_hold.subject(), vec![kim.id])));
    // The default role is the owner's, once the other hold is 48 hours old.
    assert!(!escalations.iter().any(|(_, people)| people == &vec![owner.id]));
    wait(&t, 24, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    assert!(
        told(&t, ESCALATION_NOTICE)
            .contains(&(other_hold.subject(), vec![owner.id]))
    );
}

/// By default H is 24 hours: the first reminder after a day, the first
/// escalation after two.
#[renox::test]
async fn the_default_is_a_reminder_after_a_day_and_an_escalation_after_two() {
    let t = boot_with(WlInventory::new()).await;
    let holder = t.holder("Hal").await;
    let chef = t.releaser("Chef").await;
    let (_fridge, hold) = fridge_hold(&t, &holder).await;

    wait(&t, 23, 59);
    t.app.run_scheduled(TASK).await.unwrap();
    assert!(told(&t, REMINDER_NOTICE).is_empty());

    wait(&t, 0, 1);
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(
        told(&t, REMINDER_NOTICE),
        vec![(hold.subject(), vec![chef.id])]
    );
    assert!(told(&t, ESCALATION_NOTICE).is_empty());

    // After 48 hours the owner would be told; nobody holds the owner's role
    // or the admin's here, so the escalation is counted and nobody is told.
    wait(&t, 24, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(told(&t, REMINDER_NOTICE).len(), 2);
    let kept = StockHold::find_or_404(t.db(), hold.id).await.unwrap();
    assert_eq!(kept.escalations_sent, 1);
    assert!(told(&t, ESCALATION_NOTICE).is_empty());
}
