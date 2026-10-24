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

#[renox::test]
async fn the_models_match_the_schema() {
    boot().await.app.assert_models_match_schema().await;
}

#[renox::test]
async fn every_view_compiles() {
    boot().await.app.assert_views_compile();
}
