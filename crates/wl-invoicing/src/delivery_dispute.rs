//! Flag and resolve a dispute from a disputed delivery (wl-invoicing §5.10
//! step 5, §6 *Consumes: wl-delivery*, issue #69). `wl-delivery` puts a
//! delivery under review and emits events; this module reacts, without
//! depending on that crate:
//!
//! - `DeliveryDisputed`: for each **issued** invoice (`issued`,
//!   `partially_paid`, `overdue`) with lines whose origin is a line of the
//!   delivery, the system raises a dispute (reason *delivery disputed*) for
//!   the invoice's amount of those lines, capped at its balance, with the
//!   delivery's ticket as the dispute ticket and the delivery as its source.
//!   An invoice that is already disputed is not flagged again: the delivery's
//!   ticket is linked to the open dispute with a comment. A **draft** that
//!   bills the delivery raises nothing: the receivables role gets the task
//!   *delivery disputed before issue*, and the system does not issue it
//!   while the review is open ([`held_by_review`]). A paid, credited or
//!   written-off invoice is left alone.
//! - `DeliveryDisputeClosed` (*upheld*): the disputes the system raised for
//!   that delivery are resolved `upheld`; a dispute the delivery was only
//!   linked to stays open. The draft tasks close.
//! - `DeliveryReversed` and `DeliveryLost`: the flag stays and the invoice
//!   owner gets a task to resolve it, usually `credited` for the undelivered
//!   lines. A reversal ends the review (a draft may be issued again); a loss
//!   keeps it, because nothing is billed for a lost delivery.
//!
//! A refusal of the dispute guards (for example a balance of 0) is not an
//! error of the event: the invoice is then left as it is.

use std::collections::BTreeMap;

use renox::auth::permissions::users_with_permission_in;
use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::{
    DeliveryDisputeClosed, DeliveryDisputed, DeliveryLost, DeliveryReversed, ReviewOutcome,
};
use wl_kernel::invoicing::{DisputeBy, DisputeRequest, InvoiceStatus};
use wl_kernel::notify::{self, NotifyRequest, fill};
use wl_kernel::scopes;
use wl_kernel::tickets::{self, TaskComment, TicketId};
use wl_kernel::Subject;

use crate::dispute::{self, DISPUTABLE, Dispute};
use crate::dispute_resolve::{self, ResolveInput};
use crate::invoice::Invoice;
use crate::line::InvoiceLine;
use crate::owner::{self, OwnerRole};
use crate::permissions;
use crate::task::{self, InvoicingTask, TaskKind, TaskStatus};
use crate::texts::english;

/// A delivery that went under review, as this module keeps it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_delivery_reviews", index(delivery_type, delivery_id))]
pub struct DeliveryReview {
    /// The primary key.
    pub id: i64,
    /// The subject type of the delivery.
    pub delivery_type: String,
    /// The delivery's id.
    pub delivery_id: i64,
    /// The delivery's ticket; `None` when ticketing is off.
    pub ticket_id: Option<i64>,
    /// The source lines the delivery carries, as `type:id`.
    pub origins: Json<Vec<String>>,
    /// Whether the review holds the system's issue of drafts that bill the
    /// delivery.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Which users a task is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Audience {
    /// The invoice owner, else the receivables role.
    Owner,
    /// Every holder of `invoices.issue` in the invoice's scope.
    Receivables,
}

/// A 422 is a refusal of a guard, not a failure.
fn is_refusal(error: &Error) -> bool {
    matches!(error, Error::Status(code, _) if *code == StatusCode::UNPROCESSABLE_ENTITY)
}

/// The invoice lines whose origin is one of `origins`.
async fn lines_billing(db: &Db, origins: &[Subject]) -> Result<Vec<InvoiceLine>> {
    let mut found = Vec::new();
    for origin in origins {
        let mut rows = InvoiceLine::query()
            .where_(InvoiceLine::ORIGIN_TYPE.eq(origin.subject_type.clone()))
            .where_(InvoiceLine::ORIGIN_ID.eq(origin.subject_id))
            .get(db)
            .await?;
        found.append(&mut rows);
    }
    Ok(found)
}

/// Whether an open delivery review holds the system's issue of a draft whose
/// lines are `lines`: one of the lines bills a line of a delivery under
/// review.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn held_by_review(db: &Db, lines: &[InvoiceLine]) -> Result<bool> {
    let billed: Vec<String> = lines
        .iter()
        .filter_map(InvoiceLine::origin)
        .map(|origin| origin.to_string())
        .collect();
    if billed.is_empty() {
        return Ok(false);
    }
    let reviews = DeliveryReview::query()
        .where_(DeliveryReview::ACTIVE.eq(true))
        .get(db)
        .await?;
    Ok(reviews
        .iter()
        .any(|review| review.origins.0.iter().any(|o| billed.contains(o))))
}

/// Stores the review of the delivery of `event`, or keeps the one that is
/// open (a repeated event).
async fn record_review(state: &AppState, event: &DeliveryDisputed) -> Result {
    let delivery = event.delivery.subject();
    let open = DeliveryReview::query()
        .where_(DeliveryReview::DELIVERY_TYPE.eq(delivery.subject_type.clone()))
        .where_(DeliveryReview::DELIVERY_ID.eq(delivery.subject_id))
        .where_(DeliveryReview::ACTIVE.eq(true))
        .first(&state.db)
        .await?;
    if open.is_some() {
        return Ok(());
    }
    let mut review = DeliveryReview {
        delivery_type: delivery.subject_type.clone(),
        delivery_id: delivery.subject_id,
        ticket_id: event.ticket.map(|t| t.0),
        origins: Json(event.lines.iter().map(ToString::to_string).collect()),
        active: true,
        ..DeliveryReview::default()
    };
    review.insert(&state.db).await
}

/// Ends the open reviews of `delivery`.
async fn end_reviews(state: &AppState, delivery: &Subject) -> Result {
    let open = DeliveryReview::query()
        .where_(DeliveryReview::DELIVERY_TYPE.eq(delivery.subject_type.clone()))
        .where_(DeliveryReview::DELIVERY_ID.eq(delivery.subject_id))
        .where_(DeliveryReview::ACTIVE.eq(true))
        .get(&state.db)
        .await?;
    for mut review in open {
        review.active = false;
        review.save_only(&state.db, &["active"]).await?;
    }
    Ok(())
}

/// The disputes still open that were raised for `delivery`.
async fn open_disputes_of(db: &Db, delivery: &Subject) -> Result<Vec<Dispute>> {
    Dispute::query()
        .where_(Dispute::SOURCE_TYPE.eq(delivery.subject_type.clone()))
        .where_(Dispute::SOURCE_ID.eq(delivery.subject_id))
        .where_(Dispute::OUTCOME.is_null())
        .order_by(Dispute::ID)
        .get(db)
        .await
}

/// The users a task of `audience` for `invoice` goes to.
async fn audience_of(state: &AppState, invoice: &Invoice, audience: Audience) -> Result<(OwnerRole, Vec<i64>)> {
    match audience {
        Audience::Owner => {
            let (role, users) = owner::recipients(&state.db, invoice).await?;
            Ok((role, users.iter().map(|u| u.id).collect()))
        }
        Audience::Receivables => {
            let scope = scopes::scope_of(&state.db, &invoice.subject()).await?;
            let mut users = users_with_permission_in(&state.db, permissions::ISSUE, &scope).await?;
            users.sort_by_key(|u| u.id);
            users.dedup_by_key(|u| u.id);
            Ok((OwnerRole::Receivables, users.iter().map(|u| u.id).collect()))
        }
    }
}

/// The number the tasks and notices call the invoice by.
fn invoice_name(invoice: &Invoice) -> String {
    invoice
        .number
        .clone()
        .unwrap_or_else(|| format!("draft {}", invoice.id))
}

/// Opens the task `kind` for `invoice` about `delivery`, once per
/// (kind, delivery, invoice), and tells its audience. Returns whether a task
/// was opened.
async fn open_task(
    state: &AppState,
    kind: TaskKind,
    invoice: &Invoice,
    delivery: &Subject,
    amount: i64,
    audience: Audience,
) -> Result<bool> {
    let message = fill(
        english(kind.text_key()),
        &json!({
            "delivery": delivery.to_string(),
            "number": invoice_name(invoice),
        }),
    );
    let (role, users) = audience_of(state, invoice, audience).await?;
    let mut task = InvoicingTask {
        kind,
        invoice_id: Some(invoice.id),
        source_type: delivery.subject_type.clone(),
        source_id: delivery.subject_id,
        amount,
        key: format!("{}:{}", delivery.subject_id, invoice.id),
        message: message.clone(),
        role: role.as_str().to_owned(),
        status: TaskStatus::Open,
        ..InvoicingTask::default()
    };
    match task.insert(&state.db).await {
        Ok(()) => {}
        Err(error) if error.is_unique_violation() => return Ok(false),
        Err(error) => return Err(error),
    }
    if !users.is_empty() {
        let party_name = invoice
            .party_snapshot
            .as_ref()
            .map(|snapshot| snapshot.0.legal_name_en.clone())
            .unwrap_or_default();
        let mut notice = NotifyRequest::new(task::REVERSAL_TASK, invoice.subject()).data(json!({
            "number": invoice_name(invoice),
            "party_name": party_name,
            "message": message,
            "link": format!("/invoices/{}", invoice.id),
        }));
        for id in users {
            notice = notice.to_user(id);
        }
        notify::notifier(state).notify(state, notice).await?;
    }
    Ok(true)
}

/// Links the delivery's ticket to the open dispute of `invoice` with a
/// comment on the dispute's ticket.
async fn link_to_open_dispute(
    state: &AppState,
    invoice: &Invoice,
    event: &DeliveryDisputed,
) -> Result {
    let Some(id) = invoice.dispute_id else {
        return Ok(());
    };
    let Some(dispute) = Dispute::find(&state.db, id).await?.filter(Dispute::is_open) else {
        return Ok(());
    };
    let Some(ticket) = dispute.ticket_id else {
        return Ok(());
    };
    let text = fill(
        english("wl_invoicing.delivery.linked"),
        &json!({
            "delivery": event.delivery.subject().to_string(),
            "reason": event.reason,
            "ticket": event.ticket.map(|t| t.0.to_string()).unwrap_or_else(|| "-".to_owned()),
        }),
    );
    tickets::task_creator(state)
        .comment(state, TicketId(ticket), TaskComment::internal(&text))
        .await
}

/// `DeliveryDisputed`: flags the issued invoices, tasks the drafts.
async fn on_disputed(state: &AppState, event: &DeliveryDisputed) -> Result {
    let delivery = event.delivery.subject();
    record_review(state, event).await?;
    let mut by_invoice: BTreeMap<i64, Vec<InvoiceLine>> = BTreeMap::new();
    for line in lines_billing(&state.db, &event.lines).await? {
        by_invoice.entry(line.invoice_id).or_default().push(line);
    }
    for (invoice_id, lines) in by_invoice {
        let Some(invoice) = Invoice::find(&state.db, invoice_id).await? else {
            continue;
        };
        if invoice.status == InvoiceStatus::Draft {
            open_task(
                state,
                TaskKind::DeliveryDisputedBeforeIssue,
                &invoice,
                &delivery,
                0,
                Audience::Receivables,
            )
            .await?;
            continue;
        }
        if !DISPUTABLE.contains(&invoice.status) {
            continue;
        }
        if invoice.disputed {
            link_to_open_dispute(state, &invoice, event).await?;
            continue;
        }
        let gross: i64 = lines.iter().map(|line| line.gross_amount).sum();
        let positions: Vec<i32> = lines
            .iter()
            .filter_map(|line| i32::try_from(line.position).ok())
            .collect();
        let mut request = DisputeRequest::new(
            invoice.invoice_id(),
            english("wl_invoicing.delivery.reason"),
            DisputeBy::System,
        )
        .amount(gross.min(invoice.balance))
        .lines(positions)
        .source(delivery.clone());
        if let Some(ticket) = event.ticket {
            request = request.ticket(ticket);
        }
        match dispute::raise(state, request).await {
            Ok(_) => {}
            Err(error) if is_refusal(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// `DeliveryDisputeClosed`: resolves `upheld` the disputes the system raised
/// for the delivery, and closes the draft tasks.
async fn on_closed(state: &AppState, event: &DeliveryDisputeClosed) -> Result {
    let delivery = event.delivery.subject();
    match event.outcome {
        ReviewOutcome::Upheld => {}
    }
    end_reviews(state, &delivery).await?;
    let open = InvoicingTask::query()
        .where_(InvoicingTask::KIND.eq(TaskKind::DeliveryDisputedBeforeIssue))
        .where_(InvoicingTask::SOURCE_TYPE.eq(delivery.subject_type.clone()))
        .where_(InvoicingTask::SOURCE_ID.eq(delivery.subject_id))
        .where_(InvoicingTask::STATUS.eq(TaskStatus::Open))
        .get(&state.db)
        .await?;
    for mut task in open {
        task.status = TaskStatus::Done;
        task.done_at = Some(renox::db::now());
        task.save_only(&state.db, &["status", "done_at"]).await?;
    }
    for dispute in open_disputes_of(&state.db, &delivery).await? {
        let note = english("wl_invoicing.delivery.upheld_note");
        match dispute_resolve::resolve(
            state,
            dispute.invoice_id,
            ResolveInput::upheld(note),
            DisputeBy::System,
        )
        .await
        {
            Ok(_) => {}
            Err(error) if is_refusal(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// The owner's task for each dispute the delivery raised and that is still
/// open.
async fn task_for_disputes(state: &AppState, delivery: &Subject, kind: TaskKind) -> Result {
    for dispute in open_disputes_of(&state.db, delivery).await? {
        let Some(invoice) = Invoice::find(&state.db, dispute.invoice_id).await? else {
            continue;
        };
        open_task(state, kind, &invoice, delivery, dispute.amount, Audience::Owner).await?;
    }
    Ok(())
}

/// `DeliveryReversed`: the flag stays, the owner gets a task, and the review
/// ends.
async fn on_reversed(state: &AppState, event: &DeliveryReversed) -> Result {
    let delivery = event.delivery.subject();
    end_reviews(state, &delivery).await?;
    task_for_disputes(state, &delivery, TaskKind::DeliveryReversed).await
}

/// `DeliveryLost`: the flag stays and the owner gets a task; the review stays
/// open, since nothing is billed for a lost delivery.
async fn on_lost(state: &AppState, event: &DeliveryLost) -> Result {
    let delivery = event.delivery.subject();
    task_for_disputes(state, &delivery, TaskKind::DeliveryLost).await
}

/// Registers the listeners on the delivery events.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: DeliveryDisputed, state: AppState| async move {
        on_disputed(&state, &event).await
    });
    app.listen(|event: DeliveryDisputeClosed, state: AppState| async move {
        on_closed(&state, &event).await
    });
    app.listen(|event: DeliveryReversed, state: AppState| async move {
        on_reversed(&state, &event).await
    });
    app.listen(|event: DeliveryLost, state: AppState| async move {
        on_lost(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_422_is_a_refusal_and_other_errors_are_not() {
        assert!(is_refusal(&Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "x".into()
        )));
        assert!(!is_refusal(&Error::Forbidden));
    }

    #[test]
    fn a_draft_is_named_by_its_id() {
        let draft = Invoice {
            id: 4,
            ..Invoice::default()
        };
        assert_eq!(invoice_name(&draft), "draft 4");
        let issued = Invoice {
            number: Some("INV-1".into()),
            ..Invoice::default()
        };
        assert_eq!(invoice_name(&issued), "INV-1");
    }
}
