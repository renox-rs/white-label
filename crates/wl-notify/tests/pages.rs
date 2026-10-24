//! The back-office pages of wl-notify: the acceptance checks AC4 and AC9 of
//! #55, and the module's models and views against the schema and routes.

mod support;

use wl_notify::permissions as perm;
use wl_notify::{MessageStatus, SendError};

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
            .find(&format!("data-grid-copy=\"{key}\""))
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
    assert!(receipt.contains("None"), "{receipt}");

    // #530: each type reads by its name, a row opens the type's form (the
    // row link names it by its number), and the grid searches and pages.
    assert!(html.contains(">Reminder<"), "the type in words");
    let reminder = wl_notify::NotificationType::by_key(t.db(), REMINDER)
        .await
        .unwrap()
        .expect("the reminder type");
    assert!(html.contains(&format!("/notification-types/{}", reminder.id)));
    t.app
        .get(&format!("/notification-types/{}", reminder.id))
        .await
        .assert_ok()
        .assert_view("wl-notify/type_edit.html")
        .assert_see("Reminder");
    t.app
        .get("/notification-types?search=reminder")
        .await
        .assert_ok()
        .assert_see(REMINDER)
        .assert_dont_see(support::RECEIPT);
    t.app
        .get("/notification-types?per_page=1")
        .await
        .assert_ok()
        .assert_see("1–1 of");
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
    // Only the counter's label says "Sent", and it counts nothing; no row
    // reads "Sent" or "Delivered" (the filter's choices do, so look at the rows).
    assert!(html.contains("0 · Sent"), "{html}");
    let rows = &html[html.find("<tbody").expect("the rows")..];
    assert!(!rows.contains(">Delivered<"), "{rows}");
    assert!(!rows.contains(">Sent<"), "{rows}");
}

/// #530: the log pages through every message, so a message older than the
/// 200 newest is on a later page and found by a search; it opens newest
/// first and reads the type in words.
#[renox::test]
async fn the_log_reaches_a_message_older_than_the_200_newest() {
    let t = boot().await;
    for n in 0..230 {
        wl_notify::Message::create(
            t.db(),
            wl_notify::Message {
                type_key: REMINDER.into(),
                subject_type: "invoicing.invoice".into(),
                subject_id: n + 1,
                address: Some(format!("m{n:03}@example.com")),
                channel: wl_kernel::notify::NotifyChannel::Email,
                rendered_body: "Reminder".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    let viewer = t.user("Vera", &[perm::LOG_VIEW]).await;
    t.app.acting_as(&viewer);
    // Newest first: the first page holds the newest 25, not the oldest.
    t.app
        .get("/notification-log")
        .await
        .assert_ok()
        .assert_see("m229@example.com")
        .assert_see("of 230")
        .assert_dont_see("m000@example.com");
    // 230 messages at 25 a page: the oldest is on page 10.
    t.app
        .get("/notification-log?page=10")
        .await
        .assert_ok()
        .assert_see("m000@example.com")
        .assert_dont_see("m229@example.com");
    // Another page size, and a search by recipient.
    t.app
        .get("/notification-log?per_page=100&page=3")
        .await
        .assert_ok()
        .assert_see("m000@example.com");
    t.app
        .get("/notification-log?search=m000%40example")
        .await
        .assert_ok()
        .assert_see("m000@example.com")
        .assert_see("of 1")
        .assert_dont_see("m001@example.com");
    // Sorted oldest first, the oldest leads the first page.
    t.app
        .get("/notification-log?sort=id")
        .await
        .assert_ok()
        .assert_see("m000@example.com")
        .assert_dont_see("m229@example.com");
}

#[renox::test]
async fn the_models_match_the_schema() {
    boot().await.app.assert_models_match_schema().await;
}

#[renox::test]
async fn every_view_compiles() {
    boot().await.app.assert_views_compile();
}
