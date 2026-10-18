//! An invoice dispute (wl-invoicing §5.10, #67): the [`Dispute`] record and
//! the [`raise`] service.
//!
//! The dispute is a flag, not a state: `raise` sets the invoice's `disputed`
//! column and `dispute_id`, and never writes the status, due date, balance,
//! lines or totals. It runs in this order:
//!
//! 1. Who: a user needs `invoices.dispute`; a party only when the client
//!    turned on [`InvoicingConfig::portal_disputes`] and it is the invoice's
//!    party; the system needs nothing.
//! 2. The guards ([`plan`]): the invoice is `issued`, `partially_paid` or
//!    `overdue`; the reason is not blank; every ticked line is on the
//!    invoice; the amount (by default the gross sum of the ticked lines) is
//!    above 0 and within the balance.
//! 3. One transaction: the dispute row is inserted (the partial unique index
//!    allows one open dispute per invoice), then `UPDATE … SET disputed = 1
//!    WHERE id = ? AND disputed = 0`. Zero rows means "already disputed", so
//!    the second of two raises at once is refused and rolled back.
//! 4. After the commit: the `billing_dispute` ticket ([`TaskCreator`]),
//!    assigned to the invoice owner when that user still holds
//!    `invoices.issue`, else unassigned with a notice to every holder of
//!    `invoices.issue` in the invoice's scope; the party's
//!    acknowledgement; and `InvoiceDisputed`.
//!
//! The ticket is opened after the commit, not inside the transaction: the
//! ticket service writes through its own connection, which a held write
//! transaction would block.

use renox::auth::permissions::users_with_permission_in;
use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::{
    DisputeBy, DisputeOutcome, DisputeRequest, DisputedLine, InvoiceDisputed, InvoiceStatus,
};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest, fill};
use wl_kernel::scopes;
use wl_kernel::tickets::{self, Assignee, OpenTicket, TicketChannel, TicketId};
use wl_kernel::{PartyId, Subject};

use crate::allocate::amount_text;
use crate::config_of;
use crate::invoice::Invoice;
use crate::invoicer::lines_of;
use crate::line::InvoiceLine;
use crate::permissions;
use crate::texts::english;

/// The notification type that tells the party its dispute was received.
pub const ACKNOWLEDGED: &str = "invoicing.dispute_acknowledged";

/// The staff notice that a dispute's ticket has no owner.
pub const UNASSIGNED: &str = "invoicing.dispute_unassigned";

/// The ticket category of a dispute.
pub const TICKET_CATEGORY: &str = "billing_dispute";

/// The subject type of a dispute: `invoicing.dispute:7`.
pub const SUBJECT_TYPE: &str = "invoicing.dispute";

/// The module that sends the notices.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the types belong to.
const GROUP: &str = "invoices";

/// The states an invoice can be disputed in.
pub(crate) const DISPUTABLE: [InvoiceStatus; 3] = [
    InvoiceStatus::Issued,
    InvoiceStatus::PartiallyPaid,
    InvoiceStatus::Overdue,
];

/// How a dispute came in.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DisputeVia {
    /// A staff user, in the back office.
    #[default]
    Staff,
    /// The party, from the portal.
    Portal,
    /// The system, e.g. a delivery source.
    System,
}

impl DisputeVia {
    /// The ticket channel it maps to.
    pub fn channel(self) -> TicketChannel {
        match self {
            DisputeVia::Staff => TicketChannel::Staff,
            DisputeVia::Portal => TicketChannel::Portal,
            DisputeVia::System => TicketChannel::System,
        }
    }
}

/// A dispute of an invoice (wl-invoicing §3). Money is fils of the invoice's
/// currency. Past disputes stay as rows; at most one per invoice is open
/// (`outcome` empty).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_disputes", index(invoice_id))]
pub struct Dispute {
    /// The primary key.
    pub id: i64,
    /// The invoice.
    pub invoice_id: i64,
    /// Why it is disputed.
    pub reason: String,
    /// The disputed amount, fixed when it was raised.
    pub amount: i64,
    /// The disputed line positions; empty when not tied to lines.
    pub lines: Json<Vec<i32>>,
    /// The `billing_dispute` ticket; `None` when ticketing is off.
    pub ticket_id: Option<i64>,
    /// The staff user who raised it.
    pub raised_by_user_id: Option<i64>,
    /// The party that raised it.
    pub raised_by_party_id: Option<i64>,
    /// How it came in.
    pub raised_via: DisputeVia,
    /// When it was raised.
    pub raised_at: DateTime,
    /// How it was settled; `None` while open.
    pub outcome: Option<DisputeOutcome>,
    /// The note given with the resolution.
    pub resolution_note: Option<String>,
    /// The amount agreed on a `credited` resolution.
    pub agreed_amount: Option<i64>,
    /// The credit note of a `credited` resolution.
    pub credit_note_id: Option<i64>,
    /// The approval request of the credit, while one is pending.
    pub approval_request_id: Option<i64>,
    /// The user who resolved it.
    pub resolved_by_user_id: Option<i64>,
    /// When it was resolved.
    pub resolved_at: Option<DateTime>,
    /// The last day chasing stays paused after `upheld`.
    pub grace_until: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Dispute {
    /// The dispute as a [`Subject`]: `invoicing.dispute:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// Whether it is still open.
    pub fn is_open(&self) -> bool {
        self.outcome.is_none()
    }

    /// The disputed line positions.
    pub fn positions(&self) -> &[i32] {
        &self.lines.0
    }

    /// Who raised it, in the kernel's terms.
    pub fn raised_by(&self) -> DisputeBy {
        match (self.raised_by_user_id, self.raised_by_party_id) {
            (Some(id), _) => DisputeBy::User { id },
            (None, Some(id)) => DisputeBy::Party { id: PartyId(id) },
            (None, None) => DisputeBy::System,
        }
    }
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// What the guards settled: the amount and the sorted, distinct positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The disputed amount in fils.
    pub amount: i64,
    /// The disputed line positions, ascending.
    pub positions: Vec<i32>,
}

/// The guards of a dispute (A2) on an invoice in `status` with `balance`
/// and lines `(position, gross amount)`. Returns the amount and positions,
/// or the 422 message.
pub fn plan(
    status: InvoiceStatus,
    balance: i64,
    lines: &[(i32, i64)],
    reason: &str,
    amount: Option<i64>,
    ticked: &[i32],
) -> Result<Plan, String> {
    if !DISPUTABLE.contains(&status) {
        return Err("Only an issued, partly paid or overdue invoice can be disputed.".to_owned());
    }
    if reason.trim().is_empty() {
        return Err("Give a reason for the dispute.".to_owned());
    }
    let mut positions = ticked.to_vec();
    positions.sort_unstable();
    positions.dedup();
    let mut ticked_gross = 0;
    for position in &positions {
        match lines.iter().find(|(p, _)| p == position) {
            Some((_, gross)) => ticked_gross += gross,
            None => return Err(format!("line {position} is not on this invoice")),
        }
    }
    let amount = match amount {
        Some(amount) => amount,
        None if positions.is_empty() => {
            return Err("Give the disputed amount or tick the disputed lines.".to_owned());
        }
        None => ticked_gross,
    };
    if amount <= 0 {
        return Err("The disputed amount must be above 0.".to_owned());
    }
    if amount > balance {
        return Err("The disputed amount is above the invoice's balance.".to_owned());
    }
    Ok(Plan { amount, positions })
}

/// The ticket description: the reason, the amount and the disputed lines.
pub fn ticket_description(
    currency: &str,
    reason: &str,
    amount: i64,
    lines: &[(&InvoiceLine, i64)],
) -> String {
    let mut text = format!(
        "Reason: {}\nDisputed amount: {}",
        reason.trim(),
        amount_text(currency, amount)
    );
    if !lines.is_empty() {
        text.push_str("\nDisputed lines:");
        for (line, gross) in lines {
            text.push_str(&format!(
                "\n{}. {} ({})",
                line.position,
                line.description,
                amount_text(currency, *gross)
            ));
        }
    }
    text
}

/// Whether `user` holds `permission` in `scope`, outside a request.
pub(crate) async fn holds(
    db: &Db,
    user: i64,
    permission: &str,
    scope: &renox::auth::permissions::Scope,
) -> Result<bool> {
    Ok(users_with_permission_in(db, permission, scope)
        .await?
        .iter()
        .any(|u| u.id == user))
}

/// Who may raise, and how it came in. A 403 when not allowed.
async fn authorize(state: &AppState, invoice: &Invoice, by: DisputeBy) -> Result<DisputeVia> {
    match by {
        DisputeBy::System => Ok(DisputeVia::System),
        DisputeBy::User { id } => {
            let scope = scopes::scope_of(&state.db, &invoice.subject()).await?;
            if holds(&state.db, id, permissions::DISPUTE, &scope).await? {
                Ok(DisputeVia::Staff)
            } else {
                Err(Error::Forbidden)
            }
        }
        DisputeBy::Party { id } => {
            if config_of(state).portal_disputes && id == invoice.party() {
                Ok(DisputeVia::Portal)
            } else {
                Err(Error::Forbidden)
            }
        }
    }
}

/// The owner to assign the ticket to: `owner_id` when that user still exists
/// and still holds `invoices.issue` in the invoice's scope (A5).
async fn owner_of(state: &AppState, invoice: &Invoice) -> Result<Option<i64>> {
    let Some(owner) = invoice.owner_id else {
        return Ok(None);
    };
    if User::find(&state.db, owner).await?.is_none() {
        return Ok(None);
    }
    let scope = scopes::scope_of(&state.db, &invoice.subject()).await?;
    Ok(holds(&state.db, owner, permissions::ISSUE, &scope)
        .await?
        .then_some(owner))
}

/// The template variables of the dispute notices.
fn data(invoice: &Invoice, dispute: &Dispute, ticket_number: Option<&str>) -> Value {
    let party_name = invoice
        .party_snapshot
        .as_ref()
        .map(|snapshot| snapshot.0.legal_name_en.clone())
        .unwrap_or_default();
    let ticket = ticket_number
        .map(|number| {
            fill(
                english("wl_invoicing.dispute.ticket_text"),
                &json!({ "ticket": number }),
            )
        })
        .unwrap_or_default();
    json!({
        "number": invoice.number.clone().unwrap_or_default(),
        "party_name": party_name,
        "amount": amount_text(&invoice.currency, dispute.amount),
        "reason": dispute.reason,
        "ticket": ticket,
        "link": format!("/invoices/{}", invoice.id),
    })
}

/// The type `key` with its English e-mail text under `texts`
/// (`wl_invoicing.dispute.{texts}.subject` and `.body`).
pub(crate) fn dispute_type(key: &str, texts: &str, category: Category) -> NotificationTypeDef {
    NotificationTypeDef::new(key, MODULE)
        .group(GROUP)
        .category(category)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english(&format!("wl_invoicing.dispute.{texts}.subject"))),
            english(&format!("wl_invoicing.dispute.{texts}.body")),
        )
}

/// The two types: [`ACKNOWLEDGED`] (the party) and [`UNASSIGNED`] (staff).
pub fn notification_types() -> [NotificationTypeDef; 2] {
    [
        dispute_type(ACKNOWLEDGED, "acknowledged", Category::Transactional),
        dispute_type(UNASSIGNED, "unassigned", Category::Transactional),
    ]
}

/// Registers the types with the kernel registry.
pub(crate) fn register_types() {
    for def in notification_types() {
        notify::register_type(def);
    }
}

/// Flags the invoice of `request` as disputed and returns the dispute's id.
/// See the module docs for the order. Errors: 403 for a user without
/// `invoices.dispute` or a party that may not dispute; 422 for a failed
/// guard or an invoice that is already disputed; 404 for an unknown invoice.
pub async fn raise(state: &AppState, request: DisputeRequest) -> Result<i64> {
    let invoice = Invoice::find_or_404(&state.db, request.invoice.0).await?;
    let via = authorize(state, &invoice, request.by).await?;
    let lines = lines_of(&state.db, invoice.id).await?;
    let grosses: Vec<(i32, i64)> = lines
        .iter()
        .filter_map(|line| Some((i32::try_from(line.position).ok()?, line.gross_amount)))
        .collect();
    let plan = plan(
        invoice.status,
        invoice.balance,
        &grosses,
        &request.reason,
        request.amount,
        &request.lines,
    )
    .map_err(refused)?;
    if invoice.disputed {
        return Err(refused("The invoice is already disputed."));
    }

    let (user_id, party_id) = match request.by {
        DisputeBy::User { id } => (Some(id), None),
        DisputeBy::Party { id } => (None, Some(id.0)),
        DisputeBy::System => (None, None),
    };
    let mut dispute = Dispute {
        invoice_id: invoice.id,
        reason: request.reason.trim().to_owned(),
        amount: plan.amount,
        lines: Json(plan.positions.clone()),
        raised_by_user_id: user_id,
        raised_by_party_id: party_id,
        raised_via: via,
        raised_at: renox::db::now(),
        ..Default::default()
    };
    let already = || refused("The invoice is already disputed.");
    let mut tx = state.db.begin().await?;
    match dispute.insert(&mut tx).await {
        Ok(()) => {}
        Err(error) if error.is_unique_violation() => return Err(already()),
        Err(error) => return Err(error),
    }
    // Compare-and-set: only the first of two raises at once flips the flag.
    let flipped = renox::db::sql(format!(
        "UPDATE {} SET disputed = 1, dispute_id = ?, updated_at = ? \
         WHERE id = ? AND disputed = 0",
        Invoice::TABLE
    ))
    .bind(dispute.id)
    .bind(renox::db::now())
    .bind(invoice.id)
    .execute(&mut tx)
    .await?;
    if flipped == 0 {
        return Err(already());
    }
    tx.commit().await?;

    after_commit(state, &invoice, &mut dispute, &lines, &plan).await?;
    Ok(dispute.id)
}

/// Opens the ticket, tells the people concerned and emits the event.
async fn after_commit(
    state: &AppState,
    invoice: &Invoice,
    dispute: &mut Dispute,
    lines: &[InvoiceLine],
    plan: &Plan,
) -> Result {
    let disputed: Vec<&InvoiceLine> = lines
        .iter()
        .filter(|line| i32::try_from(line.position).is_ok_and(|p| plan.positions.contains(&p)))
        .collect();
    let described: Vec<(&InvoiceLine, i64)> = disputed
        .iter()
        .map(|line| (*line, line.gross_amount))
        .collect();
    let number = invoice.number.clone().unwrap_or_default();
    let owner = owner_of(state, invoice).await?;
    let mut open = OpenTicket::new(
        invoice.subject(),
        TICKET_CATEGORY,
        &fill(
            english("wl_invoicing.dispute.ticket_title"),
            &json!({ "number": number }),
        ),
    )
    .description(&ticket_description(
        &invoice.currency,
        &dispute.reason,
        dispute.amount,
        &described,
    ))
    .requester_party(invoice.party())
    .channel(dispute.raised_via.channel())
    .source(dispute.subject());
    if let Some(owner) = owner {
        open = open.assign_to(Assignee::user(owner));
    }
    let ticket = tickets::task_creator(state).open(state, open).await?;
    if let Some(ticket) = &ticket {
        dispute.ticket_id = Some(ticket.id.0);
        dispute.save(&state.db).await?;
    }

    let notifier = notify::notifier(state);
    if owner.is_none() {
        let scope = scopes::scope_of(&state.db, &invoice.subject()).await?;
        let mut ids: Vec<i64> = users_with_permission_in(&state.db, permissions::ISSUE, &scope)
            .await?
            .iter()
            .map(|user| user.id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        if !ids.is_empty() {
            let mut notice = NotifyRequest::new(UNASSIGNED, invoice.subject()).data(data(
                invoice,
                dispute,
                ticket.as_ref().map(|t| t.number.as_str()),
            ));
            for id in ids {
                notice = notice.to_user(id);
            }
            notifier.notify(state, notice).await?;
        }
    }
    let ack = NotifyRequest::new(ACKNOWLEDGED, invoice.subject())
        .to_party(invoice.party())
        .data(data(
            invoice,
            dispute,
            ticket.as_ref().map(|t| t.number.as_str()),
        ));
    notifier.notify(state, ack).await?;

    state
        .emit(InvoiceDisputed {
            invoice: invoice.invoice_id(),
            party: invoice.party(),
            source: invoice.source(),
            dispute: dispute.id,
            amount: dispute.amount,
            lines: disputed
                .iter()
                .filter_map(|line| {
                    Some(DisputedLine {
                        position: i32::try_from(line.position).ok()?,
                        amount: line.gross_amount,
                        origin: line.origin(),
                    })
                })
                .collect(),
            ticket: ticket.map(|t| TicketId(t.id.0)),
            raised_by: dispute.raised_by(),
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: [(i32, i64); 3] = [(1, 300_000), (2, 80_000), (3, 40_000)];

    fn plan_of(
        status: InvoiceStatus,
        reason: &str,
        amount: Option<i64>,
        ticked: &[i32],
    ) -> Result<Plan, String> {
        plan(status, 500_000, &LINES, reason, amount, ticked)
    }

    #[test]
    fn ticked_lines_default_the_amount_to_their_gross_sum() {
        let planned = plan_of(InvoiceStatus::Issued, "wrong hours", None, &[3, 2, 2]).unwrap();
        assert_eq!(planned.amount, 120_000);
        assert_eq!(planned.positions, [2, 3]);
        let smaller = plan_of(InvoiceStatus::Overdue, "x", Some(100_000), &[2, 3]).unwrap();
        assert_eq!(smaller.amount, 100_000);
    }

    #[test]
    fn an_amount_without_lines_is_accepted_up_to_the_balance() {
        let planned = plan_of(InvoiceStatus::PartiallyPaid, "x", Some(120_000), &[]).unwrap();
        assert_eq!((planned.amount, planned.positions.len()), (120_000, 0));
        assert!(plan_of(InvoiceStatus::Issued, "x", Some(500_000), &[]).is_ok());
        assert!(
            plan_of(InvoiceStatus::Issued, "x", Some(600_000), &[])
                .unwrap_err()
                .contains("above")
        );
    }

    #[test]
    fn a_line_of_another_invoice_is_refused() {
        let error = plan_of(InvoiceStatus::Issued, "x", None, &[2, 9]).unwrap_err();
        assert_eq!(error, "line 9 is not on this invoice");
    }

    #[test]
    fn only_open_invoices_with_a_reason_and_an_amount_can_be_disputed() {
        for status in [
            InvoiceStatus::Draft,
            InvoiceStatus::Paid,
            InvoiceStatus::WrittenOff,
            InvoiceStatus::Credited,
        ] {
            assert!(plan_of(status, "x", Some(100), &[]).is_err());
        }
        assert!(plan_of(InvoiceStatus::Issued, "  ", Some(100), &[]).is_err());
        assert!(plan_of(InvoiceStatus::Issued, "x", Some(0), &[]).is_err());
        assert!(plan_of(InvoiceStatus::Issued, "x", Some(-5), &[]).is_err());
        assert!(plan_of(InvoiceStatus::Issued, "x", None, &[]).is_err());
    }

    #[test]
    fn the_ticket_describes_the_reason_amount_and_lines() {
        let line = InvoiceLine {
            position: 2,
            description: "Extra hours".into(),
            ..Default::default()
        };
        let text = ticket_description("AED", " wrong hours ", 120_000, &[(&line, 80_000)]);
        assert_eq!(
            text,
            "Reason: wrong hours\nDisputed amount: AED 1,200.00\nDisputed lines:\n2. Extra hours (AED 800.00)"
        );
        let bare = ticket_description("AED", "x", 100, &[]);
        assert!(!bare.contains("Disputed lines"));
    }

    #[test]
    fn a_dispute_knows_who_raised_it() {
        let mut dispute = Dispute {
            id: 7,
            raised_by_user_id: Some(4),
            ..Default::default()
        };
        assert_eq!(dispute.raised_by(), DisputeBy::User { id: 4 });
        assert_eq!(dispute.subject().to_string(), "invoicing.dispute:7");
        assert!(dispute.is_open());
        dispute.raised_by_user_id = None;
        assert_eq!(dispute.raised_by(), DisputeBy::System);
        dispute.raised_by_party_id = Some(9);
        assert_eq!(dispute.raised_by(), DisputeBy::Party { id: PartyId(9) });
        assert_eq!(DisputeVia::Portal.channel(), TicketChannel::Portal);
        assert_eq!(DisputeVia::Staff.channel(), TicketChannel::Staff);
        assert_eq!(DisputeVia::System.channel(), TicketChannel::System);
    }

    #[test]
    fn the_notice_types_are_registered_with_their_texts() {
        let [ack, unassigned] = notification_types();
        assert_eq!(ack.key, ACKNOWLEDGED);
        assert_eq!(unassigned.key, UNASSIGNED);
    }
}
