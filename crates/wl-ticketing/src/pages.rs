//! The back-office pages (wl-ticketing §6, DESIGN-SYSTEM):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /tickets` | `wl_ticketing.index` | `tickets.view` (a confidential ticket only for `tickets.confidential.view` and its requester) |
//! | `GET /tickets/{id}` | `wl_ticketing.show` | `tickets.view` (a confidential ticket: 404 without `tickets.confidential.view`, unless the requester; each view is audited) |
//! | `GET /tickets/new` | `wl_ticketing.create` | `tickets.create` |
//! | `POST /tickets` | `wl_ticketing.store` | `tickets.create` |
//! | `POST /tickets/{id}/charges` | `wl_ticketing.charge_add` | `tickets.view` and the assignee before resolve, or `tickets.assign` (fields `description`, `quantity`, `unit_price` in AED, `vat_category`) |
//! | `DELETE /tickets/{id}/charges/{line}` | `wl_ticketing.charge_remove` | the same |
//! | `POST /tickets/{id}/assign` | `wl_ticketing.assign` | `tickets.assign` |
//! | `POST /tickets/{id}/hold` | `wl_ticketing.hold` | `tickets.view` and the assignee |
//! | `POST /tickets/{id}/cancel` | `wl_ticketing.cancel` | `tickets.assign` |
//! | `POST /tickets/{id}/new-time` | `wl_ticketing.new_time` | `tickets.assign` (field `visit_at`: a new visit time with the same vendor after a no-show) |
//! | `POST /tickets/{id}/actions/{name}` | `wl_ticketing.action` | `tickets.view` and the action's own rule |
//!
//! Every POST calls the ticket services with the time of the request
//! (`renox::db::now()`), never the generic transition route, because the SLA
//! effects need the input the services work out. A refusal of a guard answers
//! 422 with its message under the field it belongs to. Every template is
//! compiled in and can be replaced by the app (`resources/views/wl-ticketing/…`).

use std::collections::{BTreeMap, HashMap};

use renox::chrono::NaiveDateTime;
use renox::db::DateTime;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{self, TransitionError};
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::Subject;
use wl_kernel::calendar::calendar;
use wl_kernel::notify::fill;
use wl_kernel::parties::directory;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::tickets::{Assignee, OpenTicket, Requester, TicketChannel};

use crate::charge::{self, ChargeInput};
use crate::assign::{AssignKind, AssignRequest, Assignment, assign, reasons_of};
use crate::comments::{Comment, CommentVisibility};
use crate::config::{Category, HoldReason};
use crate::contract::{ServiceContract, ServiceContracts};
use crate::no_show::{self, NoShow};
use crate::open::Tickets;
use crate::permissions;
use crate::priority::Priority;
use crate::texts;
use crate::ticket::{Ticket, TicketStatus};
use crate::visibility;
use crate::work;

/// The texts a template reads as `tx['fields.number']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_ticketing.` prefix) in a language.
type Text<'a> = &'a dyn Fn(&str) -> String;

/// The package texts in `lang`.
pub(crate) fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_ticketing.{key}"))
}

/// The module's routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .get("/tickets/new", new)
        .name("wl_ticketing.create")
        .post("/tickets", store)
        .name("wl_ticketing.store")
        .require_permission(permissions::CREATE)
        .merge(
            Routes::new()
                .get("/tickets", index)
                .name("wl_ticketing.index")
                .get("/tickets/{id}", show)
                .name("wl_ticketing.show")
                .post("/tickets/{id}/hold", store_hold)
                .name("wl_ticketing.hold")
                .post("/tickets/{id}/actions/{name}", store_action)
                .name("wl_ticketing.action")
                .post("/tickets/{id}/charges", store_charge)
                .name("wl_ticketing.charge_add")
                .delete("/tickets/{id}/charges/{line}", destroy_charge)
                .name("wl_ticketing.charge_remove")
                .require_permission(permissions::VIEW),
        )
        .merge(
            Routes::new()
                .post("/tickets/{id}/assign", store_assign)
                .name("wl_ticketing.assign")
                .post("/tickets/{id}/cancel", store_cancel)
                .name("wl_ticketing.cancel")
                .post("/tickets/{id}/new-time", store_new_time)
                .name("wl_ticketing.new_time")
                .require_permission(permissions::ASSIGN),
        )
}

/// The badge colour of a status, as the machine's tones give it.
fn status_kind(status: TicketStatus) -> &'static str {
    match status {
        TicketStatus::Assigned | TicketStatus::InProgress => "info",
        TicketStatus::OnHold => "warning",
        TicketStatus::Resolved | TicketStatus::Closed => "success",
        TicketStatus::Open | TicketStatus::Cancelled => "neutral",
    }
}

/// `(value, label)` pairs for a select, labelled with `prefix.value`.
pub(crate) fn options<'a>(
    t: Text<'_>,
    prefix: &str,
    values: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, String)> {
    values
        .into_iter()
        .map(|value| (value.to_owned(), t(&format!("{prefix}.{value}"))))
        .collect()
}

/// The tickets grid: search on the number and title, filters by status,
/// priority, category and assignee.
pub fn grid(lang: &Lang) -> Grid {
    grid_with(&texts_in(lang))
}

/// [`grid`] with the texts of `t`.
fn grid_with(t: Text<'_>) -> Grid {
    let statuses = options(t, "status", TicketStatus::ALL.iter().map(|s| s.as_str()));
    let priorities = options(t, "priority", Priority::ALL.iter().map(|p| p.as_str()));
    Grid::new("wl-ticketing")
        .title(&t("title"))
        .column(
            Column::text("number", &t("fields.number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/tickets/{id}"),
        )
        .column(
            Column::text("title", &t("fields.title"))
                .mobile()
                .searchable(),
        )
        .column(
            Column::text("subject", &t("fields.subject"))
                .computed("{T}.\"subject_type\" || ' #' || {T}.\"subject_id\""),
        )
        .column(Column::related(
            "category",
            &t("fields.category"),
            "wl_ticketing_categories",
            "category_id",
            "name_en",
        ))
        .column(
            Column::select("priority", &t("fields.priority"), priorities)
                .badges(&[
                    ("urgent", "danger"),
                    ("high", "warning"),
                    ("normal", "info"),
                    ("low", "neutral"),
                ])
                .mobile(),
        )
        .column(
            // A staff user, else a vendor party.
            Column::text("assignee", &t("fields.assignee"))
                .computed(
                    "COALESCE((SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = {T}.\"assignee_user_id\"), \
                     (SELECT \"wl_parties_parties\".\"display_name\" FROM \"wl_parties_parties\" WHERE \"wl_parties_parties\".\"id\" = {T}.\"assignee_party_id\"))",
                )
                .searchable(),
        )
        .column(Column::datetime("visit_at", &t("fields.visit_at")).hidden())
        .column(Column::datetime(
            "resolution_due",
            &t("fields.resolution_due"),
        ))
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("open", "neutral"),
                    ("assigned", "info"),
                    ("in_progress", "info"),
                    ("on_hold", "warning"),
                    ("resolved", "success"),
                    ("closed", "success"),
                    ("cancelled", "neutral"),
                ])
                .mobile(),
        )
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .sort_by("-created_at")
        .row_url("/tickets/{id}")
        .cards_on_mobile()
        .empty_state(&t("empty"), Some(&t("empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-ticketing/index.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    can_create: bool,
    tickets: GridPage<Ticket>,
}

/// `GET /tickets`: the grid.
async fn index(lang: Lang, user: AuthUser, request: GridRequest) -> Result<View> {
    let tickets = grid(&lang)
        .page(visibility::visible_to(Ticket::query(), &user), &request)
        .await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        can_create: user.has_permission(permissions::CREATE),
        tickets,
    }))
}

/// What the new-ticket page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-ticketing/new.html")]
pub(crate) struct NewPage {
    tx: Texts,
    /// The active categories, `(key, name)`.
    categories: Vec<(String, String)>,
}

/// `GET /tickets/new`: the staff create form.
async fn new(State(state): State<AppState>, lang: Lang) -> Result<View> {
    let categories = Category::query()
        .where_(Category::ACTIVE.eq(true))
        .order_by(Category::NAME_EN)
        .get(&state.db)
        .await?
        .iter()
        .map(|c| (c.key.clone(), c.name("en").to_owned()))
        .collect();
    Ok(View::typed(NewPage {
        tx: texts::all(&lang),
        categories,
    }))
}

/// The form that creates a ticket.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct TicketForm {
    /// The type of the record it is about, e.g. `property.unit`.
    #[validate(required, max = 100)]
    pub subject_type: String,
    /// The id of that record.
    pub subject_id: i64,
    /// The category key.
    #[validate(required, max = 64)]
    pub category: String,
    /// The title.
    #[validate(required, max = 200)]
    pub title: String,
    /// The description.
    #[validate(max = 5000)]
    pub description: Option<String>,
    /// The code of the party who asked for it, if one did.
    #[validate(max = 32)]
    pub requester_code: Option<String>,
}

/// A validation error on `field`, with the form's input filled in again.
pub(crate) fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// The party with `code`, if the directory finds it.
pub(crate) async fn party_by_code(state: &AppState, code: &str) -> Result<Option<PartyId>> {
    Ok(directory(state)
        .find_by_code(&state.db, code.trim())
        .await?
        .map(|party| party.id))
}

/// `POST /tickets`: opens the ticket as staff, then shows it.
async fn store(
    State(state): State<AppState>,
    lang: Lang,
    Valid(form): Valid<TicketForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let mut request = OpenTicket::new(
        Subject::new(form.subject_type.trim(), form.subject_id),
        form.category.trim(),
        &form.title,
    )
    .channel(TicketChannel::Staff);
    if let Some(description) = &form.description {
        request = request.description(description);
    }
    if let Some(code) = form
        .requester_code
        .as_deref()
        .filter(|c| !c.trim().is_empty())
    {
        match party_by_code(&state, code).await? {
            Some(id) => request = request.requester(Requester::Party { id }),
            None => {
                return Err(invalid("requester_code", t("assign.unknown_party"), &form));
            }
        }
    }
    let ticket = Tickets::open(&state, request).await?;
    Ok((
        Toast::success(t("created")),
        Redirect::route("wl_ticketing.show", &[&ticket.id.0])?,
    )
        .into_response())
}

/// One row of the reassignment history.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct AssignmentRow {
    id: i64,
    /// "CoolAir to Imran", "Assigned to Imran" or the visit change.
    summary: String,
    reason: Option<String>,
    reason_text: Option<String>,
    visit: Option<String>,
    by: Option<String>,
    at: Option<String>,
}

/// One comment on the ticket.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct CommentRow {
    id: i64,
    /// The author's name, or "System".
    author: String,
    /// Who sees it.
    visibility: String,
    text: String,
    at: Option<String>,
}

/// The ticket as the card shows it.
#[derive(Serialize, Debug, Clone, Default)]
pub(crate) struct Card {
    category: String,
    priority: String,
    channel: String,
    subject: String,
    requester: Option<String>,
    assignee: Option<String>,
    visit: Option<String>,
    due: Option<String>,
    sla_started: Option<String>,
    response_due: Option<String>,
    resolution_due: Option<String>,
    responded: Option<String>,
    resolved: Option<String>,
    hold_reason: Option<String>,
    cancel_reason: Option<String>,
    created: Option<String>,
}

/// One button for a lifecycle action that asks for nothing more.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ActionRow {
    name: &'static str,
    label: String,
}

/// What the ticket page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-ticketing/show.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    ticket: Ticket,
    /// The number.
    title: String,
    status_label: String,
    status_kind: &'static str,
    card: Card,
    /// The actions with no input of their own: start, resume, resolve…
    actions: Vec<ActionRow>,
    can_assign: bool,
    /// Whether the assign sheet is a reassignment (the ticket is `assigned`):
    /// only then does it ask for the reason.
    reassigning: bool,
    can_hold: bool,
    can_cancel: bool,
    /// The staff users the assign sheet offers, `(id, name)`.
    users: Vec<(i64, String)>,
    /// The priorities, `(value, label)`.
    priorities: Vec<(String, String)>,
    /// The reassignment reasons, `(code, name)`.
    reasons: Vec<(String, String)>,
    /// The hold reasons, `(key, name)`.
    hold_reasons: Vec<(String, String)>,
    /// The priority the assign sheet starts on.
    priority_selected: &'static str,
    assignments: Vec<AssignmentRow>,
    comments: Vec<CommentRow>,
    timeline: Vec<Value>,
    /// Whether the ticket is chargeable, so the charge section shows.
    chargeable: bool,
    /// The charge lines, in order.
    charges: Vec<ChargeRow>,
    /// The total with VAT, `AED 350.00`; the amount the requester is shown.
    charge_total: String,
    /// Whether the add and remove actions show.
    can_charge: bool,
    /// The VAT categories, `(value, label)`.
    vat_options: Vec<(String, String)>,
    /// The draft invoice made for the ticket, when there is one.
    invoice_id: Option<i64>,
    /// The code of the vendor the ticket's service contract proposes, which
    /// the assign sheet starts with (an unassigned ticket only).
    proposed_party_code: String,
    /// The service contract the ticket is handled under, when it has one.
    contract: Option<ContractLink>,
    /// The vendor no-show the coordinator has not answered yet, as a line.
    no_show: Option<String>,
    /// Whether "New time" is offered: there is such a no-show.
    can_new_time: bool,
    /// The reassignment reason the assign sheet starts on.
    reason_selected: String,
}

/// The contract a ticket is handled under, as the ticket page links to it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ContractLink {
    id: i64,
    number: String,
}

/// One charge line as the ticket page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ChargeRow {
    id: i64,
    description: String,
    /// `1` or `2.5`.
    quantity: String,
    /// `AED 333.33`, before VAT.
    unit_price: String,
    /// The VAT category's label.
    vat: String,
    /// The line with VAT.
    total: String,
}

/// A quantity in thousandths as text: `1`, `2.5`, `0.125`.
fn quantity_text(milli: i64) -> String {
    let (whole, part) = (milli / 1000, milli % 1000);
    if part == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{part:03}").trim_end_matches('0').to_owned()
    }
}

/// `text` as a number scaled by 10^`decimals` (`333.33` at 2 is 33333):
/// digits with at most `decimals` after the point; `None` for anything else.
fn parse_scaled(text: &str, decimals: usize) -> Option<i64> {
    let text = text.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() && fraction.is_empty()
        || !digits(whole)
        || !digits(fraction)
        || fraction.len() > decimals
    {
        return None;
    }
    let mut padded = fraction.to_owned();
    while padded.len() < decimals {
        padded.push('0');
    }
    let scale = 10_i64.checked_pow(u32::try_from(decimals).ok()?)?;
    let whole: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let fraction: i64 = if padded.is_empty() { 0 } else { padded.parse().ok()? };
    whole.checked_mul(scale)?.checked_add(fraction)
}

/// An instant as Asia/Dubai wall-clock time.
pub(crate) fn when(state: &AppState, at: Option<DateTime>) -> Option<String> {
    at.map(|at| {
        calendar(state)
            .local(at)
            .format("%d %b %Y %H:%M")
            .to_string()
    })
}

/// The name of an assignee: the user's name or the party's display name.
pub(crate) async fn assignee_name(state: &AppState, assignee: Assignee) -> Result<String> {
    Ok(match assignee {
        Assignee::User { id } => User::find(&state.db, id)
            .await?
            .map_or_else(|| format!("#{id}"), |u| u.name),
        Assignee::Party { id } => directory(state)
            .display_name(&state.db, id, "en")
            .await?
            .unwrap_or_else(|| format!("#{}", id.0)),
    })
}

/// The name of the requester.
async fn requester_name(state: &AppState, requester: Requester) -> Result<String> {
    assignee_name(
        state,
        match requester {
            Requester::User { id } => Assignee::User { id },
            Requester::Party { id } => Assignee::Party { id },
        },
    )
    .await
}

/// The lifecycle actions `user` is offered on `ticket`, as the services will
/// accept them: the status, then who may run it. The service still checks.
fn offered(ticket: &Ticket, user: &AuthUser) -> Offered {
    let assigner = user.has_permission(permissions::ASSIGN);
    let works = user.has_permission(permissions::WORK)
        && (assigner || ticket.assignee() == Some(Assignee::User { id: user.id }));
    let requester = ticket.requester_user_id == Some(user.id);
    let status = ticket.status;
    Offered {
        assign: assigner && matches!(status, TicketStatus::Open | TicketStatus::Assigned),
        cancel: assigner
            && matches!(
                status,
                TicketStatus::Open | TicketStatus::Assigned | TicketStatus::OnHold
            ),
        hold: works && status == TicketStatus::InProgress,
        start: works && status == TicketStatus::Assigned,
        resume: works && status == TicketStatus::OnHold,
        resolve: works && status == TicketStatus::InProgress,
        confirm: requester && status == TicketStatus::Resolved,
        close: assigner && status == TicketStatus::Resolved,
        reopen: (requester || assigner) && status == TicketStatus::Resolved,
    }
}

/// What [`offered`] says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Offered {
    assign: bool,
    cancel: bool,
    hold: bool,
    start: bool,
    resume: bool,
    resolve: bool,
    confirm: bool,
    close: bool,
    reopen: bool,
}

/// The simple actions of `offered`, in the order the buttons show.
fn simple_actions(t: Text<'_>, offered: Offered) -> Vec<ActionRow> {
    [
        ("start", offered.start),
        ("resume", offered.resume),
        ("resolve", offered.resolve),
        ("confirm", offered.confirm),
        ("close", offered.close),
        ("reopen", offered.reopen),
    ]
    .into_iter()
    .filter(|(_, shown)| *shown)
    .map(|(name, _)| ActionRow {
        name,
        label: t(&format!("action.{name}")),
    })
    .collect()
}

/// The history rows of `ticket`, oldest first.
async fn assignment_rows(
    state: &AppState,
    lang: &Lang,
    ticket_id: i64,
) -> Result<Vec<AssignmentRow>> {
    let t = texts_in(lang);
    let reasons = reasons_of(state);
    let mut names: HashMap<i64, String> = HashMap::new();
    let mut rows = Vec::new();
    for row in Assignment::history(&state.db, ticket_id).await? {
        let to = match row.assignee() {
            Some(a) => assignee_name(state, a).await?,
            None => String::new(),
        };
        let from = match row.previous_assignee() {
            Some(a) => assignee_name(state, a).await?,
            None => String::new(),
        };
        let key = match row.kind {
            AssignKind::First => "assignment.first",
            AssignKind::Reassign => "assignment.reassign",
            AssignKind::VisitChange => "assignment.visit_change",
        };
        let summary = fill(&t(key), &json!({ "from": from, "to": to }));
        let reason = row.reason_code.as_deref().map(|code| {
            reasons
                .get(code)
                .map_or_else(|| code.to_owned(), |r| r.name.clone())
        });
        let by = match row.by_user_id {
            Some(id) => {
                if let std::collections::hash_map::Entry::Vacant(slot) = names.entry(id) {
                    let name = User::find(&state.db, id)
                        .await?
                        .map_or_else(|| format!("#{id}"), |u| u.name);
                    slot.insert(name);
                }
                names.get(&id).cloned()
            }
            None => None,
        };
        rows.push(AssignmentRow {
            id: row.id,
            summary,
            reason,
            reason_text: row.reason_text.clone(),
            visit: when(state, row.visit_at),
            by,
            at: when(state, Some(row.at)),
        });
    }
    Ok(rows)
}

/// The comments of `ticket_id`, oldest first. Internal ones are for staff
/// (`tickets.work` or `tickets.assign`), so the requester sees only theirs.
async fn comment_rows(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
    ticket_id: i64,
) -> Result<Vec<CommentRow>> {
    let t = texts_in(lang);
    let staff = user.has_permission(permissions::WORK) || user.has_permission(permissions::ASSIGN);
    let mut names: HashMap<i64, String> = HashMap::new();
    let mut rows = Vec::new();
    for comment in Comment::of_ticket(&state.db, ticket_id).await? {
        if comment.visibility == CommentVisibility::Internal && !staff {
            continue;
        }
        let author = match comment.author_user_id {
            Some(id) => {
                if let std::collections::hash_map::Entry::Vacant(slot) = names.entry(id) {
                    let name = User::find(&state.db, id)
                        .await?
                        .map_or_else(|| format!("#{id}"), |u| u.name);
                    slot.insert(name);
                }
                names.get(&id).cloned().unwrap_or_default()
            }
            None => t("comments.system"),
        };
        rows.push(CommentRow {
            id: comment.id,
            author,
            visibility: t(&format!("comments.{}", comment.visibility.as_str())),
            text: comment.text,
            at: when(state, comment.created_at),
        });
    }
    Ok(rows)
}

/// `GET /tickets/{id}`: the card with its service level times, the actions
/// for its state, the assign sheet, the reassignment history and the
/// timeline.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(ticket): Found<Ticket>,
) -> Result<View> {
    visibility::ensure_visible(&ticket, &user)?;
    visibility::audit_view(&state.db, &user, &ticket).await?;
    let t = texts_in(&lang);
    let db = state.db.clone();
    let category = Category::find(&db, ticket.category_id).await?;
    let hold_reason = match ticket.hold_reason_id {
        Some(id) => HoldReason::find(&db, id).await?.map(|r| r.name_en),
        None => None,
    };
    let assignee = match ticket.assignee() {
        Some(a) => Some(assignee_name(&state, a).await?),
        None => None,
    };
    let requester = match ticket.requester() {
        Some(r) => Some(requester_name(&state, r).await?),
        None => None,
    };
    let card = Card {
        category: category
            .map(|c| c.name("en").to_owned())
            .unwrap_or_default(),
        priority: t(&format!("priority.{}", ticket.priority.as_str())),
        channel: t(&format!("channel.{}", ticket.channel.as_str())),
        subject: format!("{} #{}", ticket.subject_type, ticket.subject_id),
        requester,
        assignee,
        visit: when(&state, ticket.visit_at),
        due: when(&state, ticket.due_at),
        sla_started: when(&state, ticket.sla_started_at),
        response_due: when(&state, ticket.response_due),
        resolution_due: when(&state, ticket.resolution_due),
        responded: when(&state, ticket.responded_at),
        resolved: when(&state, ticket.resolved_at),
        hold_reason,
        cancel_reason: ticket.cancel_reason.clone(),
        created: when(&state, ticket.created_at),
    };
    let offered = offered(&ticket, &user);
    let users = if offered.assign {
        let mut users: Vec<(i64, String)> = User::query()
            .get(&db)
            .await?
            .into_iter()
            .map(|u| (u.id, u.name))
            .collect();
        users.sort_by(|a, b| a.1.cmp(&b.1));
        users
    } else {
        Vec::new()
    };
    let reasons = reasons_of(&state)
        .all()
        .iter()
        .map(|r| (r.code.clone(), r.name.clone()))
        .collect();
    let hold_reasons = if offered.hold {
        HoldReason::ordered(&db)
            .await?
            .into_iter()
            .map(|r| (r.key, r.name_en))
            .collect()
    } else {
        Vec::new()
    };
    let history = state_machine::history(&db, Ticket::TABLE, ticket.id).await?;
    let timeline = state_machine::timeline(&history, Some(&state_machine::info_of::<Ticket>()));
    let charge_lines = charge::lines_of(&db, ticket.id).await?;
    let charge_total = charge::money(charge::total(&charge_lines));
    let charges = charge_lines
        .iter()
        .map(|line| ChargeRow {
            id: line.id,
            description: line.description.clone(),
            quantity: quantity_text(line.quantity_milli),
            unit_price: charge::money(line.unit_price),
            vat: t(&format!("vat.{}", line.vat_category.as_str())),
            total: charge::money(line.gross()),
        })
        .collect();
    let proposed_party_code = match (offered.assign, ticket.assignee()) {
        (true, None) => match ServiceContracts::proposed_vendor(&state, &ticket, wl_kernel::documents::today()).await? {
            Some(vendor) => directory(&state)
                .find(&db, vendor)
                .await?
                .map(|party| party.code)
                .unwrap_or_default(),
            None => String::new(),
        },
        _ => String::new(),
    };
    let contract = match ticket.contract_id {
        Some(id) => ServiceContract::find(&db, id).await?.map(|c| ContractLink {
            id: c.id,
            number: c.number,
        }),
        None => None,
    };
    let no_show_row = if offered.assign {
        NoShow::current_for(&db, &ticket).await?
    } else {
        None
    };
    let no_show = match &no_show_row {
        Some(row) => {
            let vendor = assignee_name(&state, Assignee::Party { id: row.vendor() }).await?;
            let scheduled = when(&state, Some(row.scheduled_at)).unwrap_or_default();
            Some(fill(
                &t("no_show.banner"),
                &json!({ "vendor": vendor, "scheduled": scheduled }),
            ))
        }
        None => None,
    };
    let reason_selected = if no_show_row.is_some() {
        "vendor_no_show".to_owned()
    } else if ticket.vendor_document_flag {
        crate::vendor_flags::REASSIGN_REASON.to_owned()
    } else {
        String::new()
    };
    let can_charge = ticket.chargeable && charge::may_change(&ticket, &user);
    let vat_options = options(&t, "vat", VatCategory::ALL.iter().map(VatCategory::as_str));
    Ok(View::typed(ShowPage {
        chargeable: ticket.chargeable,
        charges,
        charge_total,
        can_charge,
        vat_options,
        invoice_id: ticket.invoice_id,
        proposed_party_code,
        contract,
        can_new_time: no_show_row.is_some(),
        no_show,
        reason_selected,
        tx: texts::all(&lang),
        title: ticket.number.clone(),
        status_label: t(&format!("status.{}", ticket.status.as_str())),
        status_kind: status_kind(ticket.status),
        card,
        actions: simple_actions(&t, offered),
        can_assign: offered.assign,
        reassigning: ticket.status == TicketStatus::Assigned,
        can_hold: offered.hold,
        can_cancel: offered.cancel,
        users,
        priorities: options(&t, "priority", Priority::ALL.iter().map(|p| p.as_str())),
        reasons,
        hold_reasons,
        priority_selected: ticket.priority.as_str(),
        assignments: assignment_rows(&state, &lang, ticket.id).await?,
        comments: comment_rows(&state, &lang, &user, ticket.id).await?,
        timeline,
        ticket,
    }))
}

/// The assign sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct AssignForm {
    /// The staff user who gets the ticket.
    pub assignee_user_id: Option<i64>,
    /// The code of the vendor party that gets it.
    #[validate(max = 32)]
    pub party_code: Option<String>,
    /// The priority.
    pub priority: Priority,
    /// The visit time, Asia/Dubai wall-clock time.
    pub visit_at: Option<NaiveDateTime>,
    /// The reassignment reason code.
    #[validate(max = 64)]
    pub reason_code: Option<String>,
    /// The explanation given with the reason.
    #[validate(max = 2000)]
    pub reason_text: Option<String>,
}

/// The form field an assign refusal belongs under.
fn assign_field(message: &str) -> &'static str {
    if message.starts_with("Nothing changed") {
        "visit_at"
    } else if message.contains("explanation") {
        "reason_text"
    } else if message.contains("reason") {
        "reason_code"
    } else if message.contains("vendor") {
        "party_code"
    } else if message.contains("assignee") {
        "assignee_user_id"
    } else {
        "visit_at"
    }
}

/// `POST /tickets/{id}/assign`: assigns or reassigns, then reloads the
/// page. A refusal (no reason, unknown reason, blocked vendor, nothing
/// changed…) answers 422 with the message under its field.
async fn store_assign(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(ticket): Found<Ticket>,
    Valid(form): Valid<AssignForm>,
) -> Result<Response> {
    visibility::ensure_visible(&ticket, &user)?;
    let t = texts_in(&lang);
    let party = form.party_code.as_deref().filter(|c| !c.trim().is_empty());
    let assignee = match (form.assignee_user_id, party) {
        (Some(_), Some(_)) => {
            return Err(invalid("party_code", t("assign.both"), &form));
        }
        (Some(id), None) => Assignee::User { id },
        (None, Some(code)) => match party_by_code(&state, code).await? {
            Some(id) => Assignee::Party { id },
            None => return Err(invalid("party_code", t("assign.unknown_party"), &form)),
        },
        (None, None) => {
            return Err(invalid("assignee_user_id", t("assign.choose"), &form));
        }
    };
    let request = AssignRequest {
        assignee,
        priority: form.priority,
        visit_at: form.visit_at.map(|at| calendar(&state).from_local(at)),
        reason_code: form.reason_code.clone(),
        reason_text: form.reason_text.clone(),
    };
    match assign(&state, &user, &ticket, request, renox::db::now()).await {
        Ok(_) => Ok((Toast::success(t("assign.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => {
            Err(invalid(assign_field(&message), message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// The new-time sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct NewTimeForm {
    /// The new visit time, Asia/Dubai wall-clock time.
    pub visit_at: NaiveDateTime,
}

/// `POST /tickets/{id}/new-time`: sets a new visit time with the same vendor
/// after a no-show; the requester is told the new time.
async fn store_new_time(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(ticket): Found<Ticket>,
    Valid(form): Valid<NewTimeForm>,
) -> Result<Response> {
    visibility::ensure_visible(&ticket, &user)?;
    let at = calendar(&state).from_local(form.visit_at);
    match no_show::new_time(&state, &user, &ticket, at).await {
        Ok(()) => Ok((
            Toast::success(texts::t(&lang, "wl_ticketing.no_show.new_time_done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(invalid("visit_at", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// The hold sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct HoldForm {
    /// The hold reason key.
    #[validate(required, max = 64)]
    pub hold_reason: String,
}

/// `POST /tickets/{id}/hold`: puts the ticket on hold for the reason.
async fn store_hold(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(ticket): Found<Ticket>,
    Valid(form): Valid<HoldForm>,
) -> Result<Response> {
    visibility::ensure_visible(&ticket, &user)?;
    match work::hold(&state, &user, &ticket, &form.hold_reason, renox::db::now()).await {
        Ok(()) => Ok((
            Toast::success(texts::t(&lang, "wl_ticketing.hold.done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(invalid("hold_reason", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// The cancel sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct CancelForm {
    /// Why the ticket is cancelled.
    #[validate(required, max = 500)]
    pub reason: String,
}

/// `POST /tickets/{id}/cancel`: cancels the ticket for the reason.
async fn store_cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(ticket): Found<Ticket>,
    Valid(form): Valid<CancelForm>,
) -> Result<Response> {
    visibility::ensure_visible(&ticket, &user)?;
    let t = texts_in(&lang);
    match work::cancel(&state, &user, &ticket, form.reason.trim()).await {
        Ok(()) => Ok((Toast::success(t("cancel_ticket.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::ReasonRequired) => {
            Err(invalid("reason", t("cancel_ticket.required"), &form))
        }
        Err(TransitionError::Guard(message)) => Err(invalid("reason", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// The add-charge form: the price in AED and the quantity as typed.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ChargeForm {
    /// What is billed.
    #[validate(required, max = 200)]
    pub description: String,
    /// The quantity, e.g. `1` or `2.5`; 1 when left empty.
    #[validate(max = 20)]
    pub quantity: Option<String>,
    /// The price of one unit in AED before VAT, e.g. `333.33`.
    #[validate(required, max = 20)]
    pub unit_price: String,
    /// The VAT category.
    #[serde(default)]
    pub vat_category: VatCategory,
}

/// `POST /tickets/{id}/charges`: adds a charge line and reloads. A price or
/// quantity that is not a number is a 422 under its field; so is a ticket
/// that is not chargeable. The service checks who may.
async fn store_charge(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(ticket): Found<Ticket>,
    Valid(form): Valid<ChargeForm>,
) -> Result<Response> {
    visibility::ensure_visible(&ticket, &user)?;
    let t = texts_in(&lang);
    let Some(unit_price) = parse_scaled(&form.unit_price, 2) else {
        return Err(invalid("unit_price", t("charge.bad_price_text"), &form));
    };
    let quantity_milli = match form.quantity.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        None => 1000,
        Some(text) => match parse_scaled(text, 3) {
            Some(milli) => milli,
            None => return Err(invalid("quantity", t("charge.bad_quantity_text"), &form)),
        },
    };
    let input = ChargeInput {
        description: form.description.clone(),
        quantity_milli,
        unit_price,
        vat_category: form.vat_category,
    };
    charge::add_line(&state, &user, ticket.id, input).await?;
    Ok((Toast::success(t("charge.added")), renox::HxRefresh).into_response())
}

/// `DELETE /tickets/{id}/charges/{line}`: removes a charge line and goes
/// back to the page.
async fn destroy_charge(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: Back,
    Path((id, line)): Path<(i64, i64)>,
) -> Result<Response> {
    let ticket = Ticket::find_or_404(&state.db, id).await?;
    visibility::ensure_visible(&ticket, &user)?;
    charge::remove_line(&state, &user, id, line).await?;
    Ok((Toast::success(texts::t(&lang, "wl_ticketing.charge.removed")), back).into_response())
}

/// `POST /tickets/{id}/actions/{name}`: start, resume, resolve, confirm,
/// close or reopen, with the time of the request. The service checks who
/// may; a refusal answers 422 or 403 as the transition route does.
async fn store_action(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, name)): Path<(i64, String)>,
) -> Result<Response> {
    let ticket = Ticket::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    visibility::ensure_visible(&ticket, &user)?;
    let at = renox::db::now();
    let done = match name.as_str() {
        "start" => work::start(&state, &user, &ticket, at).await,
        "resume" => work::resume(&state, &user, &ticket, at).await,
        "resolve" => work::resolve(&state, &user, &ticket, at).await,
        "confirm" => work::confirm(&state, &user, &ticket).await,
        "close" => work::close(&state, &user, &ticket).await,
        "reopen" => work::reopen(&state, &user, &ticket, at).await,
        _ => return Err(Error::NotFound),
    };
    done.map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_ticketing.action.done")),
        Redirect::route("wl_ticketing.show", &[&ticket.id])?,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english_text(key: &str) -> String {
        texts::english(&format!("wl_ticketing.{key}")).to_owned()
    }

    #[test]
    fn the_grid_lists_the_ticket_columns_and_statuses() {
        let grid = grid_with(&english_text);
        let keys: Vec<&str> = grid.columns().iter().map(Column::key).collect();
        assert_eq!(
            keys,
            [
                "number",
                "title",
                "subject",
                "category",
                "priority",
                "assignee",
                "visit_at",
                "resolution_due",
                "status",
                "created_at"
            ]
        );
        let status = grid
            .columns()
            .iter()
            .find(|c| c.key() == "status")
            .expect("status column");
        assert_eq!(status.options().len(), TicketStatus::ALL.len());
        assert!(
            status
                .options()
                .iter()
                .any(|(value, label)| value == "on_hold" && label == "On hold")
        );
    }

    #[test]
    fn every_status_priority_and_channel_has_a_label() {
        let missing = |key: String| english_text(&key) == format!("wl_ticketing.{key}");
        for status in TicketStatus::ALL {
            assert!(!missing(format!("status.{}", status.as_str())));
        }
        for priority in Priority::ALL {
            assert!(!missing(format!("priority.{}", priority.as_str())));
        }
        for channel in TicketChannel::ALL {
            assert!(!missing(format!("channel.{}", channel.as_str())));
        }
    }

    #[test]
    fn prices_and_quantities_are_read_as_scaled_numbers() {
        assert_eq!(parse_scaled("333.33", 2), Some(33_333));
        assert_eq!(parse_scaled(" 350 ", 2), Some(35_000));
        assert_eq!(parse_scaled("0.5", 2), Some(50));
        assert_eq!(parse_scaled(".5", 3), Some(500));
        assert_eq!(parse_scaled("2.5", 3), Some(2_500));
        assert_eq!(parse_scaled("1.234", 2), None);
        assert_eq!(parse_scaled("", 2), None);
        assert_eq!(parse_scaled("-1", 2), None);
        assert_eq!(parse_scaled("1,5", 2), None);
        assert_eq!(parse_scaled("abc", 2), None);
        assert_eq!(quantity_text(1_000), "1");
        assert_eq!(quantity_text(2_500), "2.5");
        assert_eq!(quantity_text(125), "0.125");
    }

    #[test]
    fn refusals_are_shown_under_their_field() {
        assert_eq!(
            assign_field("A reassignment needs a reason."),
            "reason_code"
        );
        assert_eq!(
            assign_field("`x` is not a reassignment reason."),
            "reason_code"
        );
        assert_eq!(
            assign_field("This reason needs an explanation."),
            "reason_text"
        );
        assert_eq!(
            assign_field("The vendor is not active, so it cannot be assigned."),
            "party_code"
        );
        assert_eq!(
            assign_field("The assignee is not a user."),
            "assignee_user_id"
        );
        assert_eq!(
            assign_field("Nothing changed: the ticket has this assignee and visit time."),
            "visit_at"
        );
    }
}
