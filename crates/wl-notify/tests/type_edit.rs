//! Editing the give-up reactions of a notification type on
//! `/notification-types/{key}` (#58 AC6, A7, A8).

mod support;

use renox::prelude::Model;
use wl_kernel::notify::{Category, GiveUpReaction};
use wl_notify::permissions as perm;
use wl_notify::{NotificationType, SendError};

use support::{BLUE_DUNES, RECEIPT, RED_ROCK, REMINDER, boot, boot_invalid};

async fn stored(t: &support::Booted, key: &str) -> NotificationType {
    NotificationType::by_key(t.db(), key)
        .await
        .unwrap()
        .unwrap()
}

/// The edit form of a type renders with its reactions ticked, and saving
/// writes them (and the staff role) to `wl_notify_types`.
#[renox::test]
async fn the_edit_form_saves_the_reactions_and_the_staff_role() {
    let t = boot().await;
    let path = format!("/notification-types/{REMINDER}");
    let nobody = t.user("Nadia", &[perm::LOG_VIEW]).await;
    t.app.acting_as(&nobody);
    t.app.get(&path).await.assert_forbidden();
    t.app.htmx().post(&path, &[]).await.assert_status(403);

    let admin = t.user("Amal", &[perm::TEMPLATES_MANAGE]).await;
    t.app.acting_as(&admin);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_view("wl-notify/type_edit.html")
        .assert_see("Mark contact invalid")
        .assert_see("Alternative recipient")
        .assert_see("Staff task")
        .assert_see("Staff role told")
        .assert_see("collector");
    t.app
        .get("/notification-types/no.such.type")
        .await
        .assert_status(404);

    t.app
        .htmx()
        .post(
            &path,
            &[
                ("mark_contact_invalid", "on"),
                ("staff_task", "on"),
                ("staff_role", " finance "),
            ],
        )
        .await
        .assert_status(303);
    let kind = stored(&t, REMINDER).await;
    assert_eq!(
        kind.give_up.0,
        [
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::StaffTask
        ]
    );
    assert_eq!(kind.staff_role.as_deref(), Some("finance"));
}

/// AC6: `mark_contact_invalid` alone is a 422 for a non-marketing type and
/// changes nothing; a marketing type may have it alone.
#[renox::test]
async fn the_edit_form_refuses_mark_contact_invalid_alone() {
    let t = boot().await;
    let admin = t.user("Amal", &[perm::TEMPLATES_MANAGE]).await;
    t.app.acting_as(&admin);
    let path = format!("/notification-types/{REMINDER}");
    let before = stored(&t, REMINDER).await;

    let refused = t
        .app
        .htmx()
        .post(&path, &[("mark_contact_invalid", "on")])
        .await;
    refused.assert_status(422).assert_see("add Staff task.");
    let after = stored(&t, REMINDER).await;
    assert_eq!(after.give_up.0, before.give_up.0);
    assert_eq!(after.staff_role, before.staff_role);

    let mut marketing = stored(&t, REMINDER).await;
    marketing.category = Category::Marketing;
    marketing.save(t.db()).await.unwrap();
    t.app
        .htmx()
        .post(&path, &[("mark_contact_invalid", "on")])
        .await
        .assert_status(303);
    assert_eq!(
        stored(&t, REMINDER).await.give_up.0,
        [GiveUpReaction::MarkContactInvalid]
    );
}

/// AC6: with `staff_task` added the save succeeds and the next give-up
/// uses the new reactions: a type that marked nothing now marks the
/// contact point.
#[renox::test]
async fn the_next_give_up_uses_the_saved_reactions() {
    let t = boot_invalid().await;
    let hard = || {
        SendError::hard(
            wl_notify::channels::HardError::HardBounce,
            "550",
            "no mailbox",
        )
    };

    // `receipt.issued` has no reaction: a hard bounce marks nothing.
    t.email.then([Err(hard())]);
    t.notify_party(RECEIPT, BLUE_DUNES).await;
    t.app.run_all_jobs().await;
    assert!(t.directory.invalidations().is_empty());

    let admin = t.user("Amal", &[perm::TEMPLATES_MANAGE]).await;
    t.app.acting_as(&admin);
    t.app
        .htmx()
        .post(
            &format!("/notification-types/{RECEIPT}"),
            &[("mark_contact_invalid", "on"), ("staff_task", "on")],
        )
        .await
        .assert_status(303);

    t.email.then([Err(hard())]);
    t.notify_party(RECEIPT, RED_ROCK).await;
    t.app.run_all_jobs().await;
    let marked: Vec<_> = t
        .directory
        .invalidations()
        .iter()
        .map(|i| i.point)
        .collect();
    assert_eq!(marked, [31], "Red Rock's e-mail");
    assert_eq!(t.invalidated.all().len(), 1);
}
