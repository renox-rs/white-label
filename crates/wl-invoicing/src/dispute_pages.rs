//! The dispute screens on the invoice page (wl-invoicing §6, #67 A16):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /invoices/{id}/dispute` | `wl_invoicing.dispute` | `invoices.dispute` |
//! | `POST /invoices/{id}/dispute/resolve` | `wl_invoicing.resolve_dispute` | `invoices.dispute` (and `invoices.credit` for `credited`) |
//!
//! The POST routes call [`crate::dispute::raise`] and
//! [`crate::dispute_resolve::resolve`], never the generic transition route. A
//! refusal of a guard answers 422 with its message under the field it
//! belongs to; a missing permission answers 403. [`invoice_dispute`] gives
//! the invoice page the "Dispute" and "Resolve dispute" sheets, the dispute
//! panel (reason, amount, lines, who and when, the ticket link, the pending
//! credit and its approval card) and the lines to tag as disputed.

use std::collections::HashMap;

use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalCard, ApprovalRequest};
use wl_kernel::invoicing::{DisputeBy, DisputeOutcome, DisputeRequest, InvoiceId};

use crate::allocate::amount_text;
use crate::credit_note::CreditNote;
use crate::credit_note_pages::parse_amount;
use crate::dispute::{DISPUTABLE, Dispute, DisputeVia};
use crate::dispute_resolve::{ResolveInput, resolve};
use crate::invoice::Invoice;
use crate::invoicer::lines_of;
use crate::permissions;
use crate::texts;

/// The module's dispute routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .post("/invoices/{id}/dispute", store_dispute)
        .name("wl_invoicing.dispute")
        .post("/invoices/{id}/dispute/resolve", store_resolve)
        .name("wl_invoicing.resolve_dispute")
        .require_permission(permissions::DISPUTE)
}

/// The package text `key` (without the `wl_invoicing.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_invoicing.{key}"))
}

/// A 422 with `message` under `field`, keeping the input.
fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// A refusal of the service as the response: a 422 message goes under the
/// field `field_of` picks, anything else (403, 404) passes through.
fn refusal<F: Serialize>(error: Error, form: &F, field_of: fn(&str) -> &'static str) -> Error {
    match error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => {
            invalid(field_of(&message), message, form)
        }
        other => other,
    }
}

/// The field a refusal of a dispute belongs under.
fn dispute_field(message: &str) -> &'static str {
    if message.contains("reason") {
        "reason"
    } else {
        "amount"
    }
}

/// The field a refusal of a resolution belongs under.
fn resolve_field(message: &str) -> &'static str {
    if message.contains("note") {
        "note"
    } else if message.contains("amount") {
        "agreed_amount"
    } else {
        "outcome"
    }
}

/// The positions of the ticked line checkboxes, `line_<position>`.
fn ticked_lines(form: &HashMap<String, String>) -> Vec<i32> {
    let mut positions: Vec<i32> = form
        .iter()
        .filter(|(_, value)| !value.is_empty() && *value != "0" && *value != "false")
        .filter_map(|(key, _)| key.strip_prefix("line_")?.parse().ok())
        .collect();
    positions.sort_unstable();
    positions
}

/// `POST /invoices/{id}/dispute`: flags the invoice as disputed and reloads
/// its page.
async fn store_dispute(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(invoice): Found<Invoice>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").cloned().unwrap_or_default();
    let amount =
        match form
            .get("amount")
            .map(|a| a.trim())
            .filter(|a| !a.is_empty())
        {
            None => None,
            Some(typed) => Some(parse_amount(typed).ok_or_else(|| {
                invalid("amount", text(&lang, "credit_note.errors.amount"), &form)
            })?),
        };
    let mut request = DisputeRequest::new(
        InvoiceId(invoice.id),
        reason,
        DisputeBy::User { id: user.id },
    )
    .lines(ticked_lines(&form));
    request.amount = amount;
    crate::dispute::raise(&state, request)
        .await
        .map_err(|e| refusal(e, &form, dispute_field))?;
    Ok((
        Toast::success(text(&lang, "dispute.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The resolve sheet's form.
#[derive(Deserialize, Serialize, Debug)]
pub struct ResolveForm {
    /// `upheld` or `credited`.
    pub outcome: String,
    /// The resolution note.
    #[serde(default)]
    pub note: String,
    /// The agreed amount of a `credited` resolution.
    #[serde(default)]
    pub agreed_amount: String,
}

/// `POST /invoices/{id}/dispute/resolve`: settles the open dispute and
/// reloads the page. `credited` without `invoices.credit` answers 403.
async fn store_resolve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(invoice): Found<Invoice>,
    Form(form): Form<ResolveForm>,
) -> Result<Response> {
    let outcome = match form.outcome.as_str() {
        "upheld" => DisputeOutcome::Upheld,
        "credited" => DisputeOutcome::Credited,
        _ => {
            return Err(invalid(
                "outcome",
                text(&lang, "dispute.errors.outcome"),
                &form,
            ));
        }
    };
    let agreed_amount = match form.agreed_amount.trim() {
        "" => None,
        typed => Some(parse_amount(typed).ok_or_else(|| {
            invalid(
                "agreed_amount",
                text(&lang, "credit_note.errors.amount"),
                &form,
            )
        })?),
    };
    if outcome == DisputeOutcome::Credited && agreed_amount.is_none() {
        return Err(invalid(
            "agreed_amount",
            text(&lang, "dispute.errors.agreed_amount"),
            &form,
        ));
    }
    let input = ResolveInput {
        outcome,
        note: form.note.clone(),
        agreed_amount,
        adjustment_timesheet: None,
    };
    let resolution = resolve(&state, invoice.id, input, DisputeBy::User { id: user.id })
        .await
        .map_err(|e| refusal(e, &form, resolve_field))?;
    let key = if resolution.is_resolved() {
        "dispute.resolve.done"
    } else {
        "dispute.resolve.sent"
    };
    Ok((Toast::success(text(&lang, key)), renox::HxRefresh).into_response())
}

/// One line the dispute sheet offers as a checkbox.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct DisputeLineRow {
    position: i64,
    description: String,
    gross: String,
}

/// A link with its label: the ticket, or the pending credit note.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Link {
    label: String,
    link: String,
}

/// The dispute panel of an invoice with an open dispute.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct DisputePanel {
    reason: String,
    amount: String,
    lines: Vec<String>,
    raised_by: String,
    raised_at: DateTime,
    ticket: Option<Link>,
    /// Why there is no ticket link (ticketing is off).
    ticket_note: Option<String>,
    /// The credit note waiting for its approval.
    pending_credit: Option<Link>,
}

/// The dispute parts of the invoice page.
#[derive(Serialize, Debug, Clone, Default)]
pub(crate) struct InvoiceDispute {
    /// Whether to offer the "Dispute" sheet.
    pub can_dispute: bool,
    /// Whether to offer the "Resolve dispute" sheet.
    pub can_resolve: bool,
    /// Whether `credited` is offered in it.
    pub can_credit: bool,
    /// The open dispute.
    pub panel: Option<DisputePanel>,
    /// The lines the dispute sheet offers.
    pub lines: Vec<DisputeLineRow>,
    /// The positions of the disputed lines (for the "Disputed" tags).
    pub positions: Vec<i64>,
    /// The dispute sheet's description, with the balance.
    pub description: String,
    /// The outcomes the resolve sheet offers, `(code, label)`.
    pub outcomes: Vec<(String, String)>,
}

/// The ticket number of `id`, when the ticketing table can say.
async fn ticket_number(db: &Db, id: i64) -> Option<String> {
    renox::db::sql("SELECT number FROM wl_ticketing_tickets WHERE id = ?")
        .bind(id)
        .scalar_optional::<String>(db)
        .await
        .ok()
        .flatten()
}

/// The text key part of a way a dispute came in.
fn via_key(via: DisputeVia) -> &'static str {
    match via {
        DisputeVia::Staff => "staff",
        DisputeVia::Portal => "portal",
        DisputeVia::System => "system",
    }
}

/// Who raised `dispute`, as text.
async fn raised_by_text(db: &Db, lang: &Lang, invoice: &Invoice, dispute: &Dispute) -> String {
    let via = text(
        lang,
        &format!("dispute.via.{}", via_key(dispute.raised_via)),
    );
    let name = match dispute.raised_by() {
        DisputeBy::User { id } => User::find(db, id).await.ok().flatten().map(|u| u.name),
        DisputeBy::Party { .. } => invoice
            .party_snapshot
            .as_ref()
            .map(|s| s.0.legal_name_en.clone()),
        DisputeBy::System => None,
    };
    match name {
        Some(name) => format!("{name} ({via})"),
        None => via,
    }
}

/// What the invoice page shows of the dispute of `invoice`, for `user`, and
/// the approval cards of the credit note a resolution waits on.
pub(crate) async fn invoice_dispute(
    db: &Db,
    lang: &Lang,
    user: &AuthUser,
    invoice: &Invoice,
) -> Result<(InvoiceDispute, Vec<ApprovalCard>)> {
    let may_dispute = user.has_permission_anywhere(permissions::DISPUTE);
    let open = match invoice.dispute_id {
        Some(id) if invoice.disputed => Dispute::find(db, id).await?.filter(Dispute::is_open),
        _ => None,
    };
    let lines = lines_of(db, invoice.id).await?;
    let money = |fils: i64| amount_text(&invoice.currency, fils);
    let can_dispute = may_dispute && !invoice.disputed && DISPUTABLE.contains(&invoice.status);
    let offered = if can_dispute {
        lines
            .iter()
            .map(|line| DisputeLineRow {
                position: line.position,
                description: line.description.clone(),
                gross: money(line.gross_amount),
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut cards = Vec::new();
    let mut waiting = false;
    let mut panel = None;
    let mut positions = Vec::new();
    if let Some(dispute) = &open {
        positions = dispute.positions().iter().map(|p| i64::from(*p)).collect();
        let disputed_lines = lines
            .iter()
            .filter(|l| positions.contains(&l.position))
            .map(|l| {
                format!(
                    "{}. {} ({})",
                    l.position,
                    l.description,
                    money(l.gross_amount)
                )
            })
            .collect();
        let (ticket, ticket_note) = match dispute.ticket_id {
            Some(id) => {
                let label = ticket_number(db, id)
                    .await
                    .unwrap_or_else(|| format!("#{id}"));
                (
                    Some(Link {
                        label,
                        link: format!("/tickets/{id}"),
                    }),
                    None,
                )
            }
            None => (None, Some(text(lang, "dispute.no_ticket"))),
        };
        let mut pending_credit = None;
        if let Some(request_id) = dispute.approval_request_id
            && ApprovalRequest::find(db, request_id)
                .await?
                .is_some_and(|r| r.is_pending())
        {
            waiting = true;
            let note = match dispute.credit_note_id {
                Some(id) => CreditNote::find(db, id).await?,
                None => None,
            };
            if let Some(note) = note {
                cards = approvals::cards_for(db, &note.subject(), user, lang).await?;
                pending_credit = Some(Link {
                    label: note
                        .number
                        .clone()
                        .unwrap_or_else(|| text(lang, "credit_note.draft_number")),
                    link: format!("/credit-notes/{}", note.id),
                });
            }
        }
        panel = Some(DisputePanel {
            reason: dispute.reason.clone(),
            amount: money(dispute.amount),
            lines: disputed_lines,
            raised_by: raised_by_text(db, lang, invoice, dispute).await,
            raised_at: dispute.raised_at,
            ticket,
            ticket_note,
            pending_credit,
        });
    }
    let description = wl_kernel::notify::fill(
        &text(lang, "dispute.description"),
        &renox::serde_json::json!({ "balance": money(invoice.balance) }),
    );
    let may_credit = user.has_permission_anywhere(permissions::CREDIT);
    let mut outcomes = vec![("upheld".to_owned(), text(lang, "dispute.resolve.upheld"))];
    if may_credit {
        outcomes.push((
            "credited".to_owned(),
            text(lang, "dispute.resolve.credited"),
        ));
    }
    Ok((
        InvoiceDispute {
            can_dispute,
            can_resolve: may_dispute && panel.is_some() && !waiting,
            can_credit: may_credit,
            panel,
            lines: offered,
            positions,
            description,
            outcomes,
        },
        cards,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticked_lines_are_read_from_the_checkboxes_in_order() {
        let form: HashMap<String, String> = [
            ("line_3", "1"),
            ("line_1", "on"),
            ("line_2", "0"),
            ("reason", "x"),
            ("line_x", "1"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
        assert_eq!(ticked_lines(&form), [1, 3]);
    }

    #[test]
    fn refusals_land_under_their_fields() {
        assert_eq!(dispute_field("Give a reason for the dispute."), "reason");
        assert_eq!(dispute_field("line 9 is not on this invoice"), "amount");
        assert_eq!(resolve_field("Give a note for the resolution."), "note");
        assert_eq!(
            resolve_field("The agreed amount is too high"),
            "agreed_amount"
        );
        assert_eq!(resolve_field("The invoice has no open dispute."), "outcome");
    }
}
