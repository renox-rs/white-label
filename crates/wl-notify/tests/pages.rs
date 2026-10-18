//! The back-office pages of wl-notify: the acceptance checks AC4 and AC9 of
//! #55, and the module's models and views against the schema and routes.

mod support;

use renox::prelude::Model;
use wl_kernel::notify::{Category, GiveUpReaction};
use wl_notify::permissions as perm;
use wl_notify::{MessageStatus, NotificationType, SendError};

use support::{BLUE_DUNES, REMINDER, boot};

/// AC4: the types page shows the statutory flag, the give-up reaction in
/// run order and the staff role told; it needs
/// `notifications.templates.manage`.
#[renox::test]
async fn the_types_page_shows_statutory_reaction_and_staff_role() {
    let t = boot().await;
    let nobody = t.user("Nadia", &[perm::LOG_VIEW]).await;
    t.app.acting_as(&nobody);
    t.app.get("/notification-types").await.assert_forbidden();

    let admin = t.user("Amal", &[perm::TEMPLATES_MANAGE]).await;
    t.app.acting_as(&admin);
    let page = t.app.get("/notification-types").await;
    page.assert_ok()
        .assert_view("wl-notify/types.html")
        .assert_see("Notification types")
        .assert_see("Statutory")
        .assert_see("Give-up reaction")
        .assert_see("Staff role told");
    let html = page.text();
    let row = |key: &str| {
        let start = html
            .find(&format!(">{key}<"))
            .unwrap_or_else(|| panic!("no row for `{key}`"));
        let end = html[start..]
            .find("</tr>")
            .map_or(html.len(), |end| start + end);
        html[start..end].to_owned()
    };

    let reminder = row(REMINDER);
    for text in [
        ">No<",
        "Mark contact invalid, Alternative recipient, Staff task",
        "Collector",
        "Email → SMS",
    ] {
        assert!(reminder.contains(text), "`{text}` in {reminder}");
    }
    let notice = row(support::NOTICE);
    for text in ["Yes", "Mark contact invalid, Statutory notice", "—"] {
        assert!(notice.contains(text), "`{text}` in {notice}");
    }
    assert!(!notice.contains(">No<"), "{notice}");
    let receipt = row(support::RECEIPT);
    assert!(receipt.contains(">None<"), "{receipt}");
}

/// AC9: the log shows the given-up SMS as "Not delivered (gave up)" and the
/// e-mail as "Not delivered (fell back to SMS)"; neither counts as sent. It
/// needs `notifications.log.view`.
#[renox::test]
async fn the_log_never_shows_a_given_up_message_as_sent() {
    let t = boot().await;
    t.email
        .then([Err(SendError::permanent("550", "mailbox unknown"))]);
    t.sms
        .then((0..3).map(|_| Err(SendError::transient("504", "gateway timeout"))));
    t.notify_party(REMINDER, BLUE_DUNES).await;
    t.app.run_all_jobs().await;
    let statuses: Vec<_> = t.messages().await.iter().map(|m| m.status).collect();
    assert_eq!(
        statuses,
        [MessageStatus::Abandoned, MessageStatus::Abandoned]
    );

    let nobody = t.user("Nadia", &[perm::TEMPLATES_MANAGE]).await;
    t.app.acting_as(&nobody);
    t.app.get("/notification-log").await.assert_forbidden();

    let viewer = t.user("Vera", &[perm::LOG_VIEW]).await;
    t.app.acting_as(&viewer);
    let page = t.app.get("/notification-log").await;
    page.assert_ok()
        .assert_view("wl-notify/log.html")
        .assert_see("Not delivered (gave up)")
        .assert_see("Not delivered (fell back to SMS)")
        .assert_see("550 mailbox unknown")
        .assert_see("504 gateway timeout");
    let html = page.text();
    // Only the counter's label says "Sent", and it counts nothing; no badge
    // reads "Sent" or "Delivered".
    assert!(!html.contains(">Delivered<"), "{html}");
    let sent_label = html.find(">Sent<").expect("the Sent counter");
    let after = &html[sent_label..];
    let value_at = after.find("rx-stat__value").expect("the counter's value");
    let value = &after[value_at..];
    let number: String = value
        .chars()
        .skip_while(|c| *c != '>')
        .skip(1)
        .take_while(|c| *c != '<')
        .collect();
    assert_eq!(number.trim(), "0", "{after}");
    assert_eq!(html.matches(">Sent<").count(), 1, "{html}");
}

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

/// `mark_contact_invalid` alone is a 422 for a non-marketing type and
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

#[renox::test]
async fn the_models_match_the_schema() {
    boot().await.app.assert_models_match_schema().await;
}

#[renox::test]
async fn every_view_compiles() {
    boot().await.app.assert_views_compile();
}
