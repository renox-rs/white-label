//! The send-window settings on `/notify/send-windows` (#62 part 2, A2, A3).

mod support;

use wl_kernel::notify::NotifyChannel;
use wl_notify::SendWindow;
use wl_notify::permissions as perm;
use wl_notify::window::Window;

use support::boot;

const PATH: &str = "/notify/send-windows";

async fn stored(t: &support::Booted, channel: NotifyChannel) -> Window {
    SendWindow::for_channel(t.db(), channel).await.unwrap()
}

/// The page lists the three channels with the default 07:00-21:00, needs
/// `notifications.channels.manage`, and saving checks the window.
#[renox::test]
async fn the_page_shows_and_saves_the_windows() {
    let t = boot().await;
    let nobody = t.user("Nadia", &[perm::LOG_VIEW]).await;
    t.app.acting_as(&nobody);
    t.app.get(PATH).await.assert_forbidden();
    t.app.htmx().post(PATH, &[]).await.assert_status(403);

    let admin = t.user("Amal", &[perm::CHANNELS_MANAGE]).await;
    t.app.acting_as(&admin);
    t.app
        .get(PATH)
        .await
        .assert_ok()
        .assert_view("wl-notify/send_windows.html")
        .assert_see("Marketing send windows")
        .assert_see("Email")
        .assert_see("SMS")
        .assert_see("WhatsApp")
        .assert_see("07:00")
        .assert_see("21:00");
    assert_eq!(stored(&t, NotifyChannel::Sms).await, Window::default());

    // Narrowing the SMS window is saved; the other channels keep the default.
    t.app
        .htmx()
        .post(
            PATH,
            &[("channel", "sms"), ("start", "09:00"), ("end", "20:00")],
        )
        .await
        .assert_status(303);
    let narrowed = Window {
        start: 540,
        end: 1200,
    };
    assert_eq!(stored(&t, NotifyChannel::Sms).await, narrowed);
    assert_eq!(stored(&t, NotifyChannel::Email).await, Window::default());

    // Widening SMS is refused and changes nothing.
    t.app
        .htmx()
        .post(
            PATH,
            &[("channel", "sms"), ("start", "06:00"), ("end", "22:00")],
        )
        .await
        .assert_status(422)
        .assert_see("SMS marketing may only be sent 07:00-21:00");
    assert_eq!(stored(&t, NotifyChannel::Sms).await, narrowed);

    // Email may be widened.
    t.app
        .htmx()
        .post(
            PATH,
            &[("channel", "email"), ("start", "06:00"), ("end", "23:00")],
        )
        .await
        .assert_status(303);
    let wide = Window {
        start: 360,
        end: 1380,
    };
    assert_eq!(stored(&t, NotifyChannel::Email).await, wide);

    // Start not before end, a bad time and an unknown channel.
    t.app
        .htmx()
        .post(
            PATH,
            &[("channel", "email"), ("start", "10:00"), ("end", "10:00")],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            PATH,
            &[("channel", "email"), ("start", "soon"), ("end", "10:00")],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            PATH,
            &[("channel", "pager"), ("start", "08:00"), ("end", "10:00")],
        )
        .await
        .assert_status(404);
    assert_eq!(stored(&t, NotifyChannel::Email).await, wide);
}
