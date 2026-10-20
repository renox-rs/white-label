//! BL-TKT-02 (#147): a chargeable ticket is invoiced at `resolve` or at
//! close, as its category says: charge lines, the amount the requester is
//! shown, one draft per ticket, the auto-close (`WRK.RUL.02`).

mod support;

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::tickets::{Assignee, OpenTicket};
use wl_ticketing::auto_close;
use wl_ticketing::charge;
use wl_ticketing::config::InvoiceOn;
use wl_ticketing::{
    Category, Comment, CommentVisibility, Priority, Ticket, TicketStatus, Tickets,
    permissions as perm,
};

use support::{Booted, COOLAIR, boot, dubai};

/// The people of one repair job.
struct Job {
    ticket: Ticket,
    coordinator: User,
    technician: User,
    tenant: User,
}

async fn repair_category(t: &Booted, invoice_on: InvoiceOn, chargeable: bool) {
    Category::create(
        t.db(),
        Category {
            key: "repair".into(),
            name_en: "Repair".into(),
            default_priority: Priority::Normal,
            sla_policy_id: wl_ticketing::SlaPolicy::by_key(t.db(), "standard")
                .await
                .unwrap()
                .map(|p| p.id),
            chargeable,
            invoice_on,
            active: true,
            ..Category::default()
        },
    )
    .await
    .unwrap();
}

/// A repair assigned and started: the tenant asked for it (a user who pays
/// through CoolAir's party when `bill`), the technician works it.
async fn job(t: &Booted, bill: bool) -> Job {
    let coordinator = t.coordinator().await;
    let technician = t.technician("Tim").await;
    let tenant = t.user("Tara", &[perm::VIEW]).await;
    let mut request = OpenTicket::new(Subject::new("property.unit", 12), "repair", "AC capacitor")
        .requester_user(tenant.id);
    if bill {
        request = request.bill_to(COOLAIR);
    }
    let opened = Tickets::open_at(t.state(), request, dubai("2026-11-03", 8, 0))
        .await
        .unwrap();
    let ticket = t.ticket(opened.id.0).await;
    let tech_id = technician.id.to_string();
    t.assign(
        &coordinator,
        &ticket,
        "2026-11-03 09:00",
        &[("user", tech_id.as_str())],
    )
    .await
    .assert_ok();
    t.run(&technician, &ticket, "start", "2026-11-03 10:00", &[])
        .await
        .assert_ok();
    let ticket = t.reload(&ticket).await;
    assert_eq!(ticket.status, TicketStatus::InProgress);
    assert_eq!(ticket.assignee(), Some(Assignee::user(technician.id)));
    Job {
        ticket,
        coordinator,
        technician,
        tenant,
    }
}

/// The technician adds the line "Replace AC capacitor", 1 × AED 333.33,
/// standard VAT.
async fn add_capacitor(t: &Booted, job: &Job) {
    t.app.acting_as(&job.technician);
    t.app
        .htmx()
        .post(
            &format!("/tickets/{}/charges", job.ticket.id),
            &[
                ("description", "Replace AC capacitor"),
                ("quantity", "1"),
                ("unit_price", "333.33"),
                ("vat_category", "standard"),
            ],
        )
        .await
        .assert_ok();
}

fn requester_comments(comments: &[Comment]) -> Vec<&Comment> {
    comments
        .iter()
        .filter(|c| c.visibility == CommentVisibility::Requester)
        .collect()
}

/// AC1: a category that invoices on close: at `resolve` the tenant is shown
/// AED 350.00 and no draft exists; at `confirm` one draft exists with the
/// line and a total of AED 350.00, billed to the tenant's party.
#[renox::test]
async fn tkt02_a_close_category_shows_the_amount_at_resolve_and_drafts_at_confirm() {
    let t = boot().await;
    repair_category(&t, InvoiceOn::Close, true).await;
    let job = job(&t, true).await;
    assert!(job.ticket.chargeable);
    add_capacitor(&t, &job).await;

    let lines = charge::lines_of(t.db(), job.ticket.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(charge::total(&lines), 35_000);

    t.run(
        &job.technician,
        &job.ticket,
        "resolve",
        "2026-11-03 12:00",
        &[],
    )
    .await
    .assert_ok();
    assert!(t.invoices.all().is_empty(), "no draft at resolve");
    let comments = Comment::of_ticket(t.db(), job.ticket.id).await.unwrap();
    let shown = requester_comments(&comments);
    assert_eq!(shown.len(), 1);
    assert!(shown[0].text.contains("AED 350.00"), "{}", shown[0].text);
    assert!(t.reload(&job.ticket).await.invoice_id.is_none());

    t.run(&job.tenant, &job.ticket, "confirm", "2026-11-03 15:00", &[])
        .await
        .assert_ok();
    let drafts = t.invoices.all();
    assert_eq!(drafts.len(), 1, "one draft at confirm");
    let draft = &drafts[0].draft;
    assert_eq!(draft.party, COOLAIR);
    assert_eq!(draft.source, job.ticket.ticket_id().subject());
    assert_eq!(draft.scope, Some(Subject::new("property.unit", 12)));
    assert_eq!(draft.lines.len(), 1);
    assert_eq!(draft.lines[0].description, "Replace AC capacitor");
    assert_eq!(draft.lines[0].unit_price, 33_333);
    assert_eq!(draft.lines[0].quantity_milli, 1_000);
    assert_eq!(draft.lines[0].vat_category, VatCategory::Standard);
    assert_eq!(draft.lines.iter().map(|l| l.gross()).sum::<i64>(), 35_000);
    let after = t.reload(&job.ticket).await;
    assert_eq!(after.status, TicketStatus::Closed);
    assert_eq!(after.invoice_id, Some(drafts[0].id));
}

/// AC2: resolving a chargeable ticket with no charge line is refused; a
/// ticket whose category is not chargeable resolves freely.
#[renox::test]
async fn tkt02_resolving_without_a_charge_line_is_refused() {
    let t = boot().await;
    repair_category(&t, InvoiceOn::Close, true).await;
    let job = job(&t, true).await;
    t.run(
        &job.technician,
        &job.ticket,
        "resolve",
        "2026-11-03 12:00",
        &[],
    )
    .await
    .assert_status(422)
    .assert_see("Add the charge before resolving.");
    assert_eq!(t.reload(&job.ticket).await.status, TicketStatus::InProgress);

    add_capacitor(&t, &job).await;
    t.run(
        &job.technician,
        &job.ticket,
        "resolve",
        "2026-11-03 12:00",
        &[],
    )
    .await
    .assert_ok();

    let plain = boot().await;
    repair_category(&plain, InvoiceOn::Resolve, false).await;
    let free = self::job(&plain, true).await;
    assert!(!free.ticket.chargeable);
    plain
        .run(
            &free.technician,
            &free.ticket,
            "resolve",
            "2026-11-03 12:00",
            &[],
        )
        .await
        .assert_ok();
    assert!(plain.invoices.all().is_empty());
}

/// AC3: if the tenant does nothing, `WRK.RUL.02` closes the ticket after
/// three days and the draft is created then; not before.
#[renox::test]
async fn tkt02_the_auto_close_confirms_after_three_days_and_the_draft_follows() {
    let t = boot().await;
    repair_category(&t, InvoiceOn::Close, true).await;
    let job = job(&t, true).await;
    add_capacitor(&t, &job).await;
    t.run(
        &job.technician,
        &job.ticket,
        "resolve",
        "2026-11-03 12:00",
        &[],
    )
    .await
    .assert_ok();

    let early = auto_close::run(t.state(), dubai("2026-11-06", 11, 0))
        .await
        .unwrap();
    assert_eq!(early, 0);
    assert_eq!(t.reload(&job.ticket).await.status, TicketStatus::Resolved);
    assert!(t.invoices.all().is_empty());

    let closed = auto_close::run(t.state(), dubai("2026-11-06", 12, 0))
        .await
        .unwrap();
    assert_eq!(closed, 1);
    assert_eq!(t.reload(&job.ticket).await.status, TicketStatus::Closed);
    assert_eq!(t.invoices.all().len(), 1);
    assert_eq!(t.closed.all().len(), 1, "TicketClosed follows");

    // Once closed it is not closed again.
    assert_eq!(
        auto_close::run(t.state(), dubai("2026-11-09", 12, 0))
            .await
            .unwrap(),
        0
    );
    assert_eq!(t.invoices.all().len(), 1);
}

/// AC4: a category that invoices on resolve (the default) drafts at
/// `resolve`; a `reopen` and a second `resolve` make no second draft, and the
/// draft carries the lines as they are then.
#[renox::test]
async fn tkt02_a_resolve_category_drafts_once_even_after_a_reopen() {
    let t = boot().await;
    repair_category(&t, InvoiceOn::Resolve, true).await;
    let job = job(&t, true).await;
    add_capacitor(&t, &job).await;
    t.run(
        &job.technician,
        &job.ticket,
        "resolve",
        "2026-11-03 12:00",
        &[],
    )
    .await
    .assert_ok();
    let drafts = t.invoices.all();
    assert_eq!(drafts.len(), 1, "drafted at resolve");
    assert_eq!(drafts[0].draft.lines.len(), 1);
    assert!(
        requester_comments(&Comment::of_ticket(t.db(), job.ticket.id).await.unwrap()).is_empty(),
        "the amount is shown only for a category that invoices on close"
    );

    t.run(&job.tenant, &job.ticket, "reopen", "2026-11-04 09:00", &[])
        .await
        .assert_ok();
    t.app.acting_as(&job.technician);
    t.app
        .htmx()
        .post(
            &format!("/tickets/{}/charges", job.ticket.id),
            &[
                ("description", "Call-out"),
                ("unit_price", "50"),
                ("vat_category", "exempt"),
            ],
        )
        .await
        .assert_ok();
    // The new line is already in the draft, and the next resolve adds none.
    assert_eq!(t.invoices.all()[0].draft.lines.len(), 2);
    t.run(
        &job.technician,
        &job.ticket,
        "resolve",
        "2026-11-04 11:00",
        &[],
    )
    .await
    .assert_ok();
    let drafts = t.invoices.all();
    assert_eq!(drafts.len(), 1, "no second draft");
    assert_eq!(drafts[0].draft.lines.len(), 2);
    assert_eq!(drafts[0].draft.lines[1].description, "Call-out");
    assert_eq!(drafts[0].draft.lines[1].quantity_milli, 1_000);

    // An issued invoice is not rewritten.
    t.invoices.issue(drafts[0].id);
    t.app.acting_as(&job.coordinator);
    t.app
        .htmx()
        .post(
            &format!("/tickets/{}/charges", job.ticket.id),
            &[("description", "Extra"), ("unit_price", "10")],
        )
        .await
        .assert_ok();
    assert_eq!(t.invoices.all()[0].draft.lines.len(), 2);
    assert_eq!(
        charge::lines_of(t.db(), job.ticket.id).await.unwrap().len(),
        3
    );
}

/// The coordinator changes the charge lines after resolve, before close; the
/// assignee cannot, and the total follows.
#[renox::test]
async fn tkt02_the_coordinator_changes_the_charge_after_resolve() {
    let t = boot().await;
    repair_category(&t, InvoiceOn::Close, true).await;
    let job = job(&t, true).await;
    add_capacitor(&t, &job).await;
    t.run(
        &job.technician,
        &job.ticket,
        "resolve",
        "2026-11-03 12:00",
        &[],
    )
    .await
    .assert_ok();
    let page = format!("/tickets/{}/charges", job.ticket.id);

    t.app.acting_as(&job.technician);
    t.app
        .htmx()
        .post(&page, &[("description", "Late"), ("unit_price", "1")])
        .await
        .assert_forbidden();

    t.app.acting_as(&job.coordinator);
    t.app
        .htmx()
        .post(
            &page,
            &[
                ("description", "Gas top-up"),
                ("unit_price", "50"),
                ("vat_category", "exempt"),
            ],
        )
        .await
        .assert_ok();
    t.app
        .get(&format!("/tickets/{}", job.ticket.id))
        .await
        .assert_ok()
        .assert_see("AED 400.00");
    let lines = charge::lines_of(t.db(), job.ticket.id).await.unwrap();
    assert_eq!(charge::total(&lines), 40_000);

    // A bad number is refused under its field.
    t.app
        .htmx()
        .post(&page, &[("description", "x"), ("unit_price", "ten")])
        .await
        .assert_status(422);

    // Closing drafts what the lines are then; a closed ticket's charge is
    // fixed.
    t.run(
        &job.coordinator,
        &job.ticket,
        "close",
        "2026-11-03 16:00",
        &[],
    )
    .await
    .assert_ok();
    let drafts = t.invoices.all();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].draft.lines.len(), 2);
    assert_eq!(
        drafts[0].draft.lines.iter().map(|l| l.gross()).sum::<i64>(),
        40_000
    );
    t.app
        .htmx()
        .post(&page, &[("description", "Late"), ("unit_price", "1")])
        .await
        .assert_forbidden();
}

/// A ticket with nobody to bill gets no draft, and an internal comment says
/// so; a ticket that is not chargeable refuses a charge line.
#[renox::test]
async fn tkt02_no_party_means_no_draft_and_a_note() {
    let t = boot().await;
    repair_category(&t, InvoiceOn::Close, true).await;
    let job = job(&t, false).await;
    add_capacitor(&t, &job).await;
    t.run(
        &job.technician,
        &job.ticket,
        "resolve",
        "2026-11-03 12:00",
        &[],
    )
    .await
    .assert_ok();
    t.run(&job.tenant, &job.ticket, "confirm", "2026-11-03 15:00", &[])
        .await
        .assert_ok();
    assert!(t.invoices.all().is_empty());
    let comments = Comment::of_ticket(t.db(), job.ticket.id).await.unwrap();
    assert!(comments.iter().any(
        |c| c.visibility == CommentVisibility::Internal && c.text.contains("no party to bill")
    ));

    let general = t.open("general", None).await;
    assert!(!general.chargeable);
    let tech = t.technician("Tess").await;
    let id = tech.id.to_string();
    t.assign(
        &job.coordinator,
        &general,
        "2026-11-03 09:00",
        &[("user", id.as_str())],
    )
    .await
    .assert_ok();
    t.app.acting_as(&job.coordinator);
    t.app
        .htmx()
        .post(
            &format!("/tickets/{}/charges", general.id),
            &[("description", "x"), ("unit_price", "1")],
        )
        .await
        .assert_status(422);
}

/// The category setting: a manager makes a category chargeable and picks
/// when it invoices; tickets opened afterwards follow.
#[renox::test]
async fn tkt02_a_manager_sets_charging_per_category() {
    let t = boot().await;
    let manager = t.user("Mina", &[perm::VIEW, perm::MANAGE]).await;
    t.app.acting_as(&manager);
    t.app
        .htmx()
        .post(
            "/ticket-categories/general/charging",
            &[("chargeable", "1"), ("invoice_on", "close")],
        )
        .await
        .assert_ok();
    let general = Category::by_key(t.db(), "general").await.unwrap().unwrap();
    assert!(general.chargeable);
    assert_eq!(general.invoice_on, InvoiceOn::Close);
    assert!(t.open("general", None).await.chargeable);

    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .htmx()
        .post(
            "/ticket-categories/general/charging",
            &[("chargeable", "0"), ("invoice_on", "resolve")],
        )
        .await
        .assert_forbidden();
}
