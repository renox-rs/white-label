//! Charge lines and the invoice draft of a chargeable ticket (wl-ticketing
//! §3, §4.1, issue #147).
//!
//! A chargeable ticket carries **charge lines** (description, quantity, unit
//! price, VAT category). They are both the amount the requester is shown and
//! the lines the invoice draft carries, so showing the amount needs no
//! `Invoicer` call: [`total`] adds up the lines with VAT as the draft will.
//!
//! The assignee enters them before `resolve` (a chargeable ticket cannot be
//! resolved without one); a holder of `tickets.assign` may change them until
//! the ticket is closed.
//!
//! Each category says when its chargeable tickets are invoiced
//! ([`InvoiceOn`]): the draft is made at `resolve`, or at close (`confirm`,
//! `close` or the auto-close, `WRK.RUL.02`); in the second case the requester
//! only sees the amount at `resolve`, as a comment. **One draft per ticket**:
//! the draft's id is kept on the ticket, and a later `resolve` (after a
//! `reopen`) or a change of the lines replaces the lines of that draft
//! ([`Invoicer::update_draft`]) instead of drafting a second one; the draft
//! made at close is issued at once by the system when the invoicing module is
//! set to issue the ticket source automatically (`auto_issue_source`; 01
//! Property sets it, BL-PRP-26 item 7a). An issued
//! invoice is never rewritten. The party billed is the one the ticket names
//! when it is opened (`OpenTicket::bill_to`, for a requester who is a user),
//! else the requester's party; a ticket without one gets no draft and an
//! internal comment says so.
//!
//! The draft is made after the transition commits (a listener on
//! [`Transitioned`]), never inside it.

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceId, VatCategory, invoicer};
use wl_kernel::tickets::Assignee;

use crate::comments::{Comment, CommentVisibility};
use crate::config::{Category, InvoiceOn};
use crate::permissions;
use crate::texts;
use crate::ticket::{Ticket, TicketStatus};
use crate::work::{AUTO_CONFIRM, CLOSE, CONFIRM, RESOLVE};

/// One line of what a chargeable ticket bills.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_ticketing_charge_lines", index(ticket_id))]
pub struct ChargeLine {
    /// The primary key.
    pub id: i64,
    /// The ticket.
    #[model(references = "wl_ticketing_tickets")]
    pub ticket_id: i64,
    /// The place in the list, from 1.
    pub position: i64,
    /// What is billed.
    pub description: String,
    /// The quantity in thousandths (1000 = 1).
    pub quantity_milli: i64,
    /// The price of one unit, in fils, before VAT.
    pub unit_price: i64,
    /// The VAT treatment.
    pub vat_category: VatCategory,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ChargeLine {
    /// The line as the invoice draft carries it, billing `origin`.
    pub fn draft_line(&self, origin: Option<Subject>) -> DraftLine {
        let line = DraftLine::new(&self.description, self.unit_price, self.vat_category)
            .quantity_milli(self.quantity_milli);
        match origin {
            Some(origin) => line.origin(origin),
            None => line,
        }
    }

    /// The line with VAT, in fils.
    pub fn gross(&self) -> i64 {
        self.draft_line(None).gross()
    }
}

/// The charge lines of ticket `ticket_id`, in order.
///
/// # Errors
///
/// The database's errors.
pub async fn lines_of(db: &Db, ticket_id: i64) -> Result<Vec<ChargeLine>> {
    ChargeLine::query()
        .where_(ChargeLine::TICKET_ID.eq(ticket_id))
        .order_by(ChargeLine::POSITION)
        .order_by(ChargeLine::ID)
        .get(db)
        .await
}

/// The total of `lines` with VAT at each line's category, in fils: the
/// amount the requester is shown and the draft will carry.
pub fn total(lines: &[ChargeLine]) -> i64 {
    lines.iter().map(ChargeLine::gross).sum()
}

/// `fils` as `AED 1,234.50`.
pub fn money(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let fils = fils.unsigned_abs();
    let whole = (fils / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}AED {grouped}.{:02}", fils % 100)
}

/// A charge line to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChargeInput {
    /// What is billed.
    pub description: String,
    /// The quantity in thousandths (1000 = 1).
    pub quantity_milli: i64,
    /// The price of one unit, in fils, before VAT.
    pub unit_price: i64,
    /// The VAT treatment.
    pub vat_category: VatCategory,
}

/// A 422 with the English text of `key`.
fn refused(key: &str) -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        texts::english(key).to_owned(),
    )
}

/// Why `input` cannot be a charge line, if it cannot.
pub(crate) fn input_problem(input: &ChargeInput) -> Option<&'static str> {
    if input.description.trim().is_empty() {
        Some("wl_ticketing.charge.no_description")
    } else if input.quantity_milli <= 0 {
        Some("wl_ticketing.charge.bad_quantity")
    } else if input.unit_price < 0 {
        Some("wl_ticketing.charge.bad_price")
    } else {
        None
    }
}

/// Whether `user` may change the charge lines of `ticket`: a holder of
/// `tickets.assign` until the ticket is closed or cancelled; its assignee
/// (holding `tickets.work`) until it is resolved.
pub fn may_change(ticket: &Ticket, user: &AuthUser) -> bool {
    let live = !matches!(
        ticket.status,
        TicketStatus::Closed | TicketStatus::Cancelled
    );
    if user.has_permission(permissions::ASSIGN) {
        return live;
    }
    let before_resolve = matches!(
        ticket.status,
        TicketStatus::Assigned | TicketStatus::InProgress | TicketStatus::OnHold
    );
    before_resolve
        && user.has_permission(permissions::WORK)
        && ticket.assignee() == Some(Assignee::User { id: user.id })
}

/// The chargeable ticket `id` that `user` may change the lines of.
async fn changeable(state: &AppState, user: &AuthUser, id: i64) -> Result<Ticket> {
    let ticket = Ticket::find_or_404(&state.db, id).await?;
    if !ticket.chargeable {
        return Err(refused("wl_ticketing.charge.not_chargeable"));
    }
    if !may_change(&ticket, user) {
        return Err(Error::Forbidden);
    }
    Ok(ticket)
}

/// Adds `input` as the last charge line of ticket `id`, by `user`; the draft
/// invoice, when there is one, follows.
///
/// # Errors
///
/// 422 for a ticket that is not chargeable or a line without a description,
/// with a quantity of 0 or less, or a negative price; `Forbidden` unless
/// [`may_change`]; `NotFound` for an unknown ticket.
pub async fn add_line(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    input: ChargeInput,
) -> Result<ChargeLine> {
    let ticket = changeable(state, user, id).await?;
    if let Some(problem) = input_problem(&input) {
        return Err(refused(problem));
    }
    let position = lines_of(&state.db, ticket.id)
        .await?
        .last()
        .map_or(1, |last| last.position + 1);
    let line = ChargeLine::create(
        &state.db,
        ChargeLine {
            ticket_id: ticket.id,
            position,
            description: input.description.trim().to_owned(),
            quantity_milli: input.quantity_milli,
            unit_price: input.unit_price,
            vat_category: input.vat_category,
            ..ChargeLine::default()
        },
    )
    .await?;
    refresh_draft(state, ticket.id).await?;
    Ok(line)
}

/// Removes charge line `line` of ticket `id`, by `user`; the draft invoice,
/// when there is one, follows.
///
/// # Errors
///
/// As [`add_line`]; `NotFound` for a line of another ticket.
pub async fn remove_line(state: &AppState, user: &AuthUser, id: i64, line: i64) -> Result {
    let ticket = changeable(state, user, id).await?;
    let mut row = ChargeLine::find(&state.db, line)
        .await?
        .filter(|row| row.ticket_id == ticket.id)
        .ok_or(Error::NotFound)?;
    row.delete(&state.db).await?;
    refresh_draft(state, ticket.id).await
}

/// The draft of `ticket` with its charge `lines`, billing its requester's
/// party; `None` when there is no party to bill.
fn draft_of(ticket: &Ticket, lines: &[ChargeLine]) -> Option<InvoiceDraft> {
    let party = ticket.bill_party()?;
    let origin = ticket.ticket_id().subject();
    let mut draft = InvoiceDraft::new(party, origin.clone()).scope(ticket.subject());
    for line in lines {
        draft = draft.line(line.draft_line(Some(origin.clone())));
    }
    Some(draft)
}

/// Replaces the lines of the draft invoice of ticket `id`, if it has one
/// that is still a draft.
async fn refresh_draft(state: &AppState, id: i64) -> Result {
    let ticket = Ticket::find_or_404(&state.db, id).await?;
    let Some(invoice) = ticket.invoice_id else {
        return Ok(());
    };
    let lines = lines_of(&state.db, ticket.id).await?;
    if let Some(draft) = draft_of(&ticket, &lines) {
        invoicer(state)
            .update_draft(&state.db, InvoiceId(invoice), draft)
            .await?;
    }
    Ok(())
}

/// Makes the invoice draft of ticket `id` from its charge lines, or replaces
/// the lines of the one it has: **one draft per ticket**. Returns its id;
/// `None` when the ticket is not chargeable or has no line, when invoicing is
/// off, or when there is no party to bill (then an internal comment says so).
///
/// # Errors
///
/// `NotFound` for an unknown ticket; the database's and the invoicer's errors.
pub async fn ensure_draft(state: &AppState, id: i64) -> Result<Option<i64>> {
    let ticket = Ticket::find_or_404(&state.db, id).await?;
    let lines = lines_of(&state.db, ticket.id).await?;
    if !ticket.chargeable || lines.is_empty() {
        return Ok(None);
    }
    let Some(draft) = draft_of(&ticket, &lines) else {
        Comment::create(
            &state.db,
            Comment {
                ticket_id: ticket.id,
                visibility: CommentVisibility::Internal,
                text: texts::english("wl_ticketing.charge.no_party").to_owned(),
                ..Comment::default()
            },
        )
        .await?;
        return Ok(None);
    };
    if let Some(existing) = ticket.invoice_id {
        invoicer(state)
            .update_draft(&state.db, InvoiceId(existing), draft)
            .await?;
        return Ok(Some(existing));
    }
    let Some(made) = invoicer(state).draft(&state.db, draft).await? else {
        return Ok(None);
    };
    Ticket::query()
        .where_(Ticket::ID.eq(ticket.id))
        .where_(Ticket::INVOICE_ID.is_null())
        .update(&state.db, &[("invoice_id", &Some(made.0))])
        .await?;
    Ok(Some(made.0))
}

/// Tells the requester the amount of a ticket that is invoiced on close, as
/// a comment they see: no invoice exists yet.
async fn tell_amount(state: &AppState, ticket: &Ticket) -> Result {
    let lines = lines_of(&state.db, ticket.id).await?;
    if lines.is_empty() {
        return Ok(());
    }
    let text = texts::english("wl_ticketing.charge.amount_shown")
        .replace(":amount", &money(total(&lines)));
    Comment::create(
        &state.db,
        Comment {
            ticket_id: ticket.id,
            visibility: CommentVisibility::Requester,
            text,
            ..Comment::default()
        },
    )
    .await?;
    Ok(())
}

/// Registers the listener that, after a transition commits, makes the draft
/// invoice of a chargeable ticket: at `resolve` for a category that invoices
/// on resolve (the amount is shown instead for one that invoices on close),
/// at `confirm`, `close` or the auto-close for one that invoices on close.
pub(crate) fn register_events(app: &mut Registry) {
    app.listen(|event: Transitioned<Ticket>, state: AppState| async move {
        let name = event.transition.as_str();
        let resolving = name == RESOLVE;
        if !resolving && ![CONFIRM, CLOSE, AUTO_CONFIRM].contains(&name) {
            return Ok(());
        }
        let ticket = Ticket::find_or_404(&state.db, event.key).await?;
        if !ticket.chargeable {
            return Ok(());
        }
        let invoice_on = Category::find(&state.db, ticket.category_id)
            .await?
            .map_or(InvoiceOn::Resolve, |category| category.invoice_on);
        match (resolving, invoice_on) {
            (true, InvoiceOn::Resolve) => {
                // Drafted only: the lines may still change after a `reopen`.
                ensure_draft(&state, ticket.id).await?;
            }
            (false, InvoiceOn::Close) => {
                if let Some(id) = ensure_draft(&state, ticket.id).await? {
                    // "Issued at once" once the ticket is closed: the system
                    // issues the draft when the invoicing module is set to
                    // issue the ticket source automatically (01 Property turns
                    // it on, `FIN.INV.01`, BL-PRP-26 item 7a). Otherwise, or
                    // when a guard of `issue` refuses (e.g. no billing
                    // identity), the draft stays for a user to review and
                    // issue.
                    let _ = invoicer(&state).issue(&state, InvoiceId(id), None).await;
                }
            }
            (true, InvoiceOn::Close) => tell_amount(&state, &ticket).await?,
            (false, InvoiceOn::Resolve) => {}
        }
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(price: i64, quantity_milli: i64, vat_category: VatCategory) -> ChargeLine {
        ChargeLine {
            description: "Replace AC capacitor".into(),
            quantity_milli,
            unit_price: price,
            vat_category,
            ..ChargeLine::default()
        }
    }

    #[test]
    fn the_total_has_vat_at_each_lines_category() {
        let lines = [
            line(33_333, 1_000, VatCategory::Standard),
            line(10_000, 2_000, VatCategory::Exempt),
        ];
        assert_eq!(lines[0].gross(), 35_000);
        assert_eq!(total(&lines), 55_000);
        assert_eq!(total(&[]), 0);
    }

    #[test]
    fn money_groups_thousands_and_keeps_two_decimals() {
        assert_eq!(money(35_000), "AED 350.00");
        assert_eq!(money(120_005), "AED 1,200.05");
        assert_eq!(money(123_456_789), "AED 1,234,567.89");
        assert_eq!(money(5), "AED 0.05");
        assert_eq!(money(-250), "-AED 2.50");
    }

    #[test]
    fn a_line_needs_a_description_a_quantity_and_a_price() {
        let ok = ChargeInput {
            description: "Call-out".into(),
            quantity_milli: 1_000,
            unit_price: 0,
            vat_category: VatCategory::Standard,
        };
        assert_eq!(input_problem(&ok), None);
        let blank = ChargeInput {
            description: "  ".into(),
            ..ok.clone()
        };
        assert!(input_problem(&blank).is_some());
        let none = ChargeInput {
            quantity_milli: 0,
            ..ok.clone()
        };
        assert!(input_problem(&none).is_some());
        let negative = ChargeInput {
            unit_price: -1,
            ..ok
        };
        assert!(input_problem(&negative).is_some());
    }
}
