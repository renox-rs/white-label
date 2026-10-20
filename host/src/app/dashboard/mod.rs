//! The page people land on after logging in: a welcome card, the property
//! figures (units available and leased, leases ending within 90 days,
//! cheques due and bounced, overdue invoices, open tickets, the person's own
//! tasks, compliance flags), what waits for the person's decision, and what
//! they did lately (the activity log). Every figure links to its page, and a
//! tile is left out when the person lacks the permission of its page or the
//! client's `client.toml` does not enable the module. The template is
//! `resources/views/dashboard/show.html`, made of the app's page patterns
//! (`resources/views/components/`).

use std::sync::Arc;

use renox::audit;
use renox::nav::NavLink;
use renox::prelude::*;
use serde::Serialize;
use wl_invoicing::Invoice;
use wl_kernel::Manifest;
use wl_kernel::approvals;
use wl_kernel::cheques::ChequeStatus;
use wl_kernel::documents::today;
use wl_kernel::invoicing::InvoiceStatus;
use wl_payments::Cheque;
use wl_ticketing::ticket::{Ticket, TicketStatus};

pub struct Dashboard;

impl Module for Dashboard {
    fn name(&self) -> &'static str {
        "dashboard"
    }

    /// The sidebar link (`<rx-sidebar menu="app"/>` in the layout): the
    /// signed-in pages only.
    fn register(&self, app: &mut Registry) {
        // The invoice's page, for the requests about it (modules register
        // their own subjects' pages the same way).
        approvals::register_subject_link("invoicing.invoice", |id| format!("/invoices/{id}"));
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

/// One figure of the dashboard: what it counts, its number, the page that
/// lists it, and how it is drawn.
#[derive(Serialize, Debug, Clone)]
pub struct Tile {
    /// The text key of the label.
    pub label: &'static str,
    /// The figure, formatted.
    pub value: String,
    /// The page that lists what is counted.
    pub url: String,
    /// The icon.
    pub icon: &'static str,
    /// The colour of the icon's disc (`clay`, `sand`, or none for the accent).
    pub tone: Option<&'static str>,
    /// How many records the figure adds up, when the figure is an amount.
    pub count: Option<u64>,
}

/// What the home page of a person shows of the business.
#[derive(Serialize, Debug, Clone, Default)]
pub struct Figures {
    /// The tiles the person may open, in the order shown.
    pub tiles: Vec<Tile>,
    /// The leases that end soonest, with the link to each.
    pub expiring: Vec<property::dashboard::ExpiringLease>,
    /// The person's open tasks (tickets assigned to them), newest first.
    pub tasks: Vec<TaskRow>,
}

/// An open task of the person.
#[derive(Serialize, Debug, Clone)]
pub struct TaskRow {
    /// The ticket's id (`/tickets/{id}`).
    pub id: i64,
    /// The ticket's number.
    pub number: String,
    /// What it is about.
    pub title: String,
}

/// The tickets still being worked on.
const OPEN_TICKETS: [TicketStatus; 4] = [
    TicketStatus::Open,
    TicketStatus::Assigned,
    TicketStatus::InProgress,
    TicketStatus::OnHold,
];

/// The figures `user` may see: each module's counts are read through its own
/// models and only when its page is open to the person (`require_permission`
/// of the route) and the module is enabled for the client.
///
/// # Errors
///
/// The database's errors.
pub async fn figures(state: &AppState, user: &AuthUser) -> Result<Figures> {
    let manifest = state.provided::<Arc<Manifest>>();
    let on = |name: &str| manifest.as_ref().is_some_and(|m| m.is_enabled(name));
    let mut out = Figures::default();
    let tile = |label, value: String, url: &str, icon, tone| Tile {
        label,
        value,
        url: url.to_owned(),
        icon,
        tone,
        count: None,
    };

    // The vertical's own: units, leases, compliance (per building).
    let property = property::dashboard::figures(state, user, today()).await?;
    if let Some(n) = property.units_available {
        out.tiles.push(tile(
            "dashboard.tiles.units_available",
            number(n),
            "/property",
            "building-2",
            None,
        ));
    }
    if let Some(n) = property.units_leased {
        out.tiles.push(tile(
            "dashboard.tiles.units_leased",
            number(n),
            "/property",
            "house",
            Some("clay"),
        ));
    }
    if let Some(n) = property.leases_expiring {
        out.tiles.push(tile(
            "dashboard.tiles.leases_expiring",
            number(n),
            "/property/leases",
            "calendar-days",
            Some("sand"),
        ));
    }
    out.expiring = property.expiring;

    if on("payments") && user.has_permission(wl_payments::permissions::CHEQUES_VIEW) {
        let count = |status: ChequeStatus| {
            Cheque::query()
                .where_(Cheque::STATUS.eq(status))
                .count(&state.db)
        };
        let due = count(ChequeStatus::DueForDeposit).await?;
        let bounced = count(ChequeStatus::Bounced).await?;
        out.tiles.push(tile(
            "dashboard.tiles.cheques_due",
            number(due),
            "/cheques?in.status=due_for_deposit",
            "banknote",
            None,
        ));
        out.tiles.push(tile(
            "dashboard.tiles.cheques_bounced",
            number(bounced),
            "/cheques?in.status=bounced",
            "triangle-alert",
            Some("clay"),
        ));
    }

    if on("invoicing") && user.has_permission(wl_invoicing::permissions::VIEW) {
        let overdue = || {
            Invoice::query()
                .where_(Invoice::STATUS.eq(InvoiceStatus::Overdue))
                .where_(Invoice::CURRENCY.eq("AED"))
        };
        let count = overdue().count(&state.db).await?;
        let total: i64 = overdue().sum::<i64, _>(&state.db, "balance").await?;
        let mut card = tile(
            "dashboard.tiles.invoices_overdue",
            property::money_text(total),
            "/invoices?in.status=overdue",
            "receipt",
            Some("clay"),
        );
        card.count = Some(count);
        out.tiles.push(card);
    }

    if on("ticketing") && user.has_permission(wl_ticketing::permissions::VIEW) {
        let open = wl_ticketing::visibility::visible_to(Ticket::query(), user)
            .where_(Ticket::STATUS.is_in(OPEN_TICKETS))
            .count(&state.db)
            .await?;
        out.tiles.push(tile(
            "dashboard.tiles.tickets_open",
            number(open),
            "/tickets?in.status=open&in.status=assigned&in.status=in_progress&in.status=on_hold",
            "clipboard-list",
            None,
        ));
        let mine = wl_ticketing::visibility::visible_to(Ticket::query(), user)
            .where_(Ticket::STATUS.is_in(OPEN_TICKETS))
            .where_(Ticket::ASSIGNEE_USER_ID.eq(user.id))
            .order_by_desc(Ticket::ID);
        let tasks = mine.clone().limit(5).get(&state.db).await?;
        let total = mine.count(&state.db).await?;
        out.tiles.push(tile(
            "dashboard.tiles.my_tasks",
            number(total),
            "/tickets?in.status=open&in.status=assigned&in.status=in_progress&in.status=on_hold",
            "circle-check",
            Some("sand"),
        ));
        out.tasks = tasks
            .into_iter()
            .map(|t| TaskRow {
                id: t.id,
                number: t.number,
                title: t.title,
            })
            .collect();
    }

    if let Some(n) = property.buildings_flagged {
        out.tiles.push(tile(
            "dashboard.tiles.compliance_flags",
            number(n),
            "/property/compliance",
            "shield",
            Some("clay"),
        ));
    }
    Ok(out)
}

fn number(n: u64) -> String {
    n.to_string()
}

async fn show(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    let figures = figures(&state, &user).await?;
    let activity = audit::for_user(&state.db, user.id, 8).await?;
    // What this person must fix and resubmit (wl-kernel approvals).
    let needs_action: Vec<_> = approvals::needs_action_items(&state.db, &user, &lang)
        .await?
        .into_iter()
        .map(|item| {
            let url = approvals::subject_url(&item.subject_type, item.subject_id);
            context! { title => item.title, comment => item.comment, url, returned_at => item.returned_at }
        })
        .collect();
    // What this person may decide now (wl-kernel approvals).
    let awaiting: Vec<_> = approvals::awaiting_items(&state.db, &user, &lang)
        .await?
        .into_iter()
        .map(|item| context! { title => item.title, url => item.url })
        .collect();
    let awaiting_count = awaiting.len();
    Ok(view(
        "dashboard/show.html",
        context! {
            activity,
            needs_action,
            awaiting,
            awaiting_count,
            tiles => figures.tiles,
            expiring => figures.expiring,
            tasks => figures.tasks,
        },
    ))
}
