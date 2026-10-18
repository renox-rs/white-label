//! The page people land on after logging in: a welcome card, a few figures
//! over a period (`?period=`, `renox::chart::Period`), new sign-ups as a
//! chart, a summary with totals, and what the signed-in person did lately
//! (the activity log). The template is `resources/views/dashboard/show.html`,
//! made of the app's page patterns (`resources/views/components/`).

use renox::audit;
use renox::chart::{Period, Trend};
use renox::nav::NavLink;
use renox::prelude::*;
use wl_kernel::approvals;

pub struct Dashboard;

impl Module for Dashboard {
    fn name(&self) -> &'static str {
        "dashboard"
    }

    /// The sidebar link (`<rx-sidebar menu="app"/>` in the layout): the
    /// signed-in pages only.
    fn register(&self, app: &mut Registry) {
        app.nav(
            "app",
            NavLink::route("dashboard", "nav.dashboard")
                .icon("layout-dashboard")
                .order(0)
                .when(|_| true),
        );
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/dashboard", show)
            .name("dashboard")
            .require_verified()
    }
}

/// A page for the record a returned request is about, when the host knows
/// one (the kernel does not know the modules' routes).
fn subject_url(subject_type: &str, id: i64) -> Option<String> {
    match subject_type {
        "invoicing.invoice" => Some(format!("/invoices/{id}")),
        _ => None,
    }
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    period: Period,
) -> Result<View> {
    let signups = Trend::of(User::query(), "created_at")
        .over(period)
        .count(&state)
        .await?;
    let signups_before = Trend::of(User::query(), "created_at")
        .over(period.previous())
        .count(&state)
        .await?;
    let users = User::query().count(&state.db).await?;
    let verified = User::query()
        .where_not_null("email_verified_at")
        .count(&state.db)
        .await?;
    let unread = user.unread_notification_count(&state.db).await?;
    let activity = audit::for_user(&state.db, user.id, 8).await?;
    // What this person must fix and resubmit (wl-kernel approvals).
    let needs_action: Vec<_> = approvals::needs_action_items(&state.db, &user, &lang)
        .await?
        .into_iter()
        .map(|item| {
            let url = subject_url(&item.subject_type, item.subject_id);
            context! { title => item.title, comment => item.comment, url, returned_at => item.returned_at }
        })
        .collect();
    Ok(view(
        "dashboard/show.html",
        context! {
            period,
            users,
            verified,
            unread,
            activity,
            needs_action,
            signups,
            signups_total => signups.total(),
            signups_change => signups.change_from(&signups_before),
            signups_before => signups_before.total(),
        },
    ))
}
