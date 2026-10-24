//! A ticket entered late (BL-TKT-13, wl-kernel §5.13 points 1–4): written on
//! paper during a power or internet cut and typed in later.
//!
//! wl-ticketing opts in to the kernel's late-entry check for the ticket
//! (business-time field `reported_at`, parent subject: the ticket's subject,
//! so rule runs on the unit fired in between are listed for review).
//! [`open_reported`] opens a staff ticket at the time the problem was
//! reported:
//!
//! - a *Reported at* in the future is refused;
//! - within M minutes of now: an ordinary ticket, not flagged;
//! - more than M minutes before: the kernel asks for the reason (and an
//!   optional outage reference) and records the late entry; the ticket is
//!   flagged late entry and its SLA runs from the reported time, never from
//!   the entry time. The entry appears in the next `PLT.RUL.06` digest of the
//!   managers of the subject's scope;
//! - beyond N hours the kernel's `PLT.ACL.10` approval applies. Decided: the
//!   ticket is opened at once all the same (a tenant without cooling cannot
//!   wait for an approval) and is linked to the entry, which records it when
//!   the approval is granted.
//!
//! Portal and system tickets are reported when they are created: never
//! checked, never flagged. *Reported at* is not editable on an existing
//! ticket, so it cannot change without this check.

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use wl_kernel::late_entries::{self, Entered, EntrySpec, LateEntry, LateEntryDef, NewRecord};
use wl_kernel::machines::BoxFuture;
use wl_kernel::tickets::{OpenTicket, TICKET_SUBJECT_TYPE, TicketChannel, TicketRef};

use crate::config::Category;
use crate::open::Tickets;
use crate::ticket::Ticket;

/// When a staff ticket was reported, with the late reason and outage
/// reference the form gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reported {
    /// When the problem was reported (the business time).
    pub at: DateTime,
    /// Why it is entered late (required when it is).
    pub reason: Option<String>,
    /// The outage it refers to, if any.
    pub outage_reference: Option<String>,
}

impl Reported {
    /// Reported at `at`, with no reason yet.
    pub fn at(at: DateTime) -> Self {
        Reported {
            at,
            reason: None,
            outage_reference: None,
        }
    }

    /// Entered late because of `reason`.
    pub fn reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.trim().to_owned()).filter(|r| !r.is_empty());
        self
    }

    /// During the outage `reference`.
    pub fn outage(mut self, reference: &str) -> Self {
        self.outage_reference = Some(reference.trim().to_owned()).filter(|r| !r.is_empty());
        self
    }
}

/// The kernel's create handler: the ticket is opened by [`open_reported`]
/// after the entry is assessed, so here it is only found again (an entry
/// granted after approval records the ticket opened with it).
fn existing_ticket<'a>(tx: &'a mut Transaction, new: &'a NewRecord) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        let Some(entry) = new.late_entry_id else {
            return Ok(0);
        };
        Ok(Ticket::query()
            .where_(Ticket::LATE_ENTRY_ID.eq(entry))
            .first(&mut *tx)
            .await?
            .map_or(0, |ticket| ticket.id))
    })
}

/// Registers the ticket's late-entry definition (at start-up).
pub(crate) fn register() {
    LateEntryDef::of(TICKET_SUBJECT_TYPE, existing_ticket)
        .business_time("reported_at")
        .parent("subject")
        .register();
}

fn unprocessable(message: &str) -> Error {
    abort(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

/// User `user_id` opens `request` reported at `reported` (see the module
/// documentation).
///
/// # Errors
///
/// 422 for a *Reported at* in the future, a late entry without a reason, an
/// unknown category or a blank title (nothing is stored then), and as
/// [`Tickets::open`].
pub async fn open_reported(
    state: &AppState,
    user_id: i64,
    request: OpenTicket,
    reported: Reported,
) -> Result<TicketRef> {
    let now = renox::db::now();
    if reported.at > now {
        return Err(unprocessable(
            "The time it was reported cannot be in the future.",
        ));
    }
    if matches!(
        request.channel,
        TicketChannel::Portal | TicketChannel::System
    ) {
        return Tickets::open(state, request).await;
    }
    // Checked before the entry is stored, so a refusal leaves nothing.
    match Category::by_key(&state.db, request.category_key.trim()).await? {
        Some(category) if category.active => {}
        _ => {
            return Err(unprocessable(&format!(
                "`{}` is not a ticket category.",
                request.category_key
            )));
        }
    }
    if request.title.trim().is_empty() {
        return Err(unprocessable("A ticket needs a title."));
    }
    let scope = wl_kernel::scopes::scope_of(&state.db, &request.subject).await?;
    let mut spec = EntrySpec::new(TICKET_SUBJECT_TYPE, user_id, reported.at)
        .parent(request.subject.clone())
        .scope(scope);
    if let Some(reason) = &reported.reason {
        spec = spec.reason(reason);
    }
    if let Some(reference) = &reported.outage_reference {
        spec = spec.outage_reference(reference);
    }
    let entered = late_entries::enter(state, spec).await?;
    let opened = Tickets::open_at(state, request, reported.at).await?;
    let entry_id = match &entered {
        Entered::OnTime { .. } => None,
        Entered::Recorded { entry, .. } | Entered::Pending { entry, .. } => Some(entry.id),
    };
    let late = entry_id.is_some();
    let reason = reported.reason.clone().filter(|_| late);
    let outage = reported.outage_reference.clone().filter(|_| late);
    let at = Some(reported.at);
    Ticket::query()
        .where_(Ticket::ID.eq(opened.id.0))
        .update(
            &state.db,
            &[
                ("reported_at", &at),
                ("late_entry", &late),
                ("late_reason", &reason),
                ("outage_reference", &outage),
                ("late_entry_id", &entry_id),
            ],
        )
        .await?;
    if let Some(id) = entry_id {
        // The entry names the ticket, for the digest and the review.
        let record = Some(opened.id.0);
        LateEntry::query()
            .where_(LateEntry::ID.eq(id))
            .update(&state.db, &[("record_id", &record)])
            .await?;
    }
    Ok(opened)
}
