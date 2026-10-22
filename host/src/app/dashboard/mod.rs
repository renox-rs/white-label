//! The page people land on after logging in: a welcome card, the figures of
//! the client's business, what waits for the person's decision, and what they
//! did lately (the activity log). Every figure links to its page, and a tile
//! is left out when the person lacks the permission of its page or the
//! client's `client.toml` does not enable the module. The template is
//! `resources/views/dashboard/show.html`, made of the app's page patterns
//! (`resources/views/components/`).
//!
//! The figures depend on the client's vertical app (`[client] app`):
//! [`property_tiles`] (units, leases, compliance), [`construction_tiles`]
//! (projects, IPAs, guarantees, variations, absence, permits, orders, budget),
//! [`manpower_tiles`] (contracts, requests, documents, deployments, absence,
//! timesheets, payroll) and, for any other app, [`generic_tiles`] (the shared modules' figures
//! only: no Property table is read). The decisions and the activity are the same for all.

mod activity_label;
mod construction_tiles;
mod generic_tiles;
mod manpower_tiles;
mod property_tiles;

use std::sync::Arc;

use renox::audit;
use renox::nav::NavLink;
use renox::prelude::*;
use serde::Serialize;
use wl_invoicing::Invoice;
use wl_kernel::approvals;
use wl_kernel::cheques::ChequeStatus;
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::{ClientConfig, Manifest};
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

impl Tile {
    /// A tile with the accent colour and no record count.
    fn new(label: &'static str, value: String, url: &str, icon: &'static str) -> Self {
        Tile {
            label,
            value,
            url: url.to_owned(),
            icon,
            tone: None,
            count: None,
        }
    }

    /// With the colour of the icon's disc.
    fn toned(mut self, tone: &'static str) -> Self {
        self.tone = Some(tone);
        self
    }
}

/// What the home page of a person shows of the business.
#[derive(Serialize, Debug, Clone, Default)]
pub struct Figures {
    /// The text key of the welcome card's line.
    pub lead: &'static str,
    /// The tiles the person may open, in the order shown.
    pub tiles: Vec<Tile>,
    /// The leases that end soonest, with the link to each (Property).
    pub expiring: Vec<::property::dashboard::ExpiringLease>,
    /// The person's open tasks (tickets assigned to them), newest first.
    pub tasks: Vec<TaskRow>,
    /// Whether the person may open tiles that all count nothing (Manpower):
    /// the page then says nothing needs their action.
    pub quiet: bool,
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

/// The page of the open tickets.
const OPEN_TICKETS_URL: &str =
    "/tickets?in.status=open&in.status=assigned&in.status=in_progress&in.status=on_hold";

/// The vertical app of the client (`[client] app`), `property` when the
/// configuration is not provided.
fn app_of(state: &AppState) -> String {
    state
        .provided::<Arc<ClientConfig>>()
        .map_or_else(|| "property".to_owned(), |config| config.client.app.clone())
}

/// Whether the client's `client.toml` enables the module `name`.
fn module_on(state: &AppState, name: &str) -> bool {
    state
        .provided::<Arc<Manifest>>()
        .as_ref()
        .is_some_and(|manifest| manifest.is_enabled(name))
}

/// The figures `user` may see: each module's counts are read through its own
/// models and only when its page is open to the person (`require_permission`
/// of the route) and the module is enabled for the client. Which figures there
/// are depends on the client's vertical app.
///
/// # Errors
///
/// The database's errors.
pub async fn figures(state: &AppState, user: &AuthUser) -> Result<Figures> {
    match app_of(state).as_str() {
        "property" => property_tiles::figures(state, user).await,
        "construction" => construction_tiles::figures(state, user).await,
        "manpower" => manpower_tiles::figures(state, user).await,
        _ => generic_tiles::figures(state, user).await,
    }
}

/// The cheques tiles of the payments module: the bounced ones and, with
/// `due`, those due for deposit.
async fn cheque_tiles(
    state: &AppState,
    user: &AuthUser,
    due: bool,
    out: &mut Vec<Tile>,
) -> Result<()> {
    if !(module_on(state, "payments")
        && user.has_permission(wl_payments::permissions::CHEQUES_VIEW))
    {
        return Ok(());
    }
    let count = |status: ChequeStatus| {
        Cheque::query()
            .where_(Cheque::STATUS.eq(status))
            .count(&state.db)
    };
    if due {
        let due = count(ChequeStatus::DueForDeposit).await?;
        out.push(Tile::new(
            "dashboard.tiles.cheques_due",
            number(due),
            "/cheques?in.status=due_for_deposit",
            "banknote",
        ));
    }
    let bounced = count(ChequeStatus::Bounced).await?;
    out.push(
        Tile::new(
            "dashboard.tiles.cheques_bounced",
            number(bounced),
            "/cheques?in.status=bounced",
            "triangle-alert",
        )
        .toned("clay"),
    );
    Ok(())
}

/// The overdue invoices tile of the invoicing module: their balance, and how
/// many they are.
async fn invoice_tile(state: &AppState, user: &AuthUser, out: &mut Vec<Tile>) -> Result<()> {
    if !(module_on(state, "invoicing") && user.has_permission(wl_invoicing::permissions::VIEW)) {
        return Ok(());
    }
    let overdue = || {
        Invoice::query()
            .where_(Invoice::STATUS.eq(InvoiceStatus::Overdue))
            .where_(Invoice::CURRENCY.eq("AED"))
    };
    let count = overdue().count(&state.db).await?;
    let total: i64 = overdue().sum::<i64, _>(&state.db, "balance").await?;
    let mut card = Tile::new(
        "dashboard.tiles.invoices_overdue",
        ::property::money_text(total),
        "/invoices?in.status=overdue",
        "receipt",
    )
    .toned("clay");
    card.count = Some(count);
    out.push(card);
    Ok(())
}

/// The tickets of the ticketing module: with `open`, the tile of the open
/// ones; always the tile and the list of the person's own tasks.
async fn ticket_tiles(
    state: &AppState,
    user: &AuthUser,
    open: bool,
    out: &mut Figures,
) -> Result<()> {
    if !(module_on(state, "ticketing") && user.has_permission(wl_ticketing::permissions::VIEW)) {
        return Ok(());
    }
    if open {
        let open = wl_ticketing::visibility::visible_to(Ticket::query(), user)
            .where_(Ticket::STATUS.is_in(OPEN_TICKETS))
            .count(&state.db)
            .await?;
        out.tiles.push(Tile::new(
            "dashboard.tiles.tickets_open",
            number(open),
            OPEN_TICKETS_URL,
            "clipboard-list",
        ));
    }
    let mine = wl_ticketing::visibility::visible_to(Ticket::query(), user)
        .where_(Ticket::STATUS.is_in(OPEN_TICKETS))
        .where_(Ticket::ASSIGNEE_USER_ID.eq(user.id))
        .order_by_desc(Ticket::ID);
    let tasks = mine.clone().limit(5).get(&state.db).await?;
    let total = mine.count(&state.db).await?;
    out.tiles.push(
        Tile::new(
            "dashboard.tiles.my_tasks",
            number(total),
            OPEN_TICKETS_URL,
            "circle-check",
        )
        .toned("sand"),
    );
    out.tasks = tasks
        .into_iter()
        .map(|t| TaskRow {
            id: t.id,
            number: t.number,
            title: t.title,
        })
        .collect();
    Ok(())
}

fn number(n: u64) -> String {
    n.to_string()
}

async fn show(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    let figures = figures(&state, &user).await?;
    // The person's last actions, each with its words (the log keeps a key).
    let activity: Vec<_> = audit::for_user(&state.db, user.id, 8)
        .await?
        .into_iter()
        .map(|entry| {
            let label = activity_label::label(&lang, &entry.action);
            context! { action => entry.action, label, created_at => entry.created_at, ip => entry.ip }
        })
        .collect();
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
    // The timesheets waiting for a client approver (Manpower portal).
    let portal_waiting = manpower_tiles::timesheets_waiting(&state, &user).await?;
    Ok(view(
        "dashboard/show.html",
        context! {
            activity,
            needs_action,
            awaiting,
            awaiting_count,
            portal_waiting,
            portal_url => manpower_tiles::TIMESHEETS_URL,
            lead => figures.lead,
            tiles => figures.tiles,
            expiring => figures.expiring,
            tasks => figures.tasks,
            quiet => figures.quiet,
        },
    ))
}
