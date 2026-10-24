//! The invoice actions an AI assistant may run (BL-AGT-08, decision 0006
//! point 5): *create draft* and *issue*, each previewed first and confirmed
//! by the user. Nothing else of an invoice is offered (no write-off, credit,
//! void or approval decision).
//!
//! - `invoicing.invoice.issue` is the `issue` transition with the input the
//!   screen gathers ([`crate::issue::gather`]): the same permission
//!   (`invoices.issue`), guards and approval policy. The preview states that
//!   the number is assigned on issue (the series is gapless and only taken in
//!   the issue's transaction, so no number is reserved), the total, the VAT
//!   and the due date. An issue over the user's limit (an `invoicing.issue`
//!   policy whose amount the total is above) submits an approval request.
//! - `invoicing.invoice.create_draft` drafts an invoice to a party through
//!   the kernel's invoicer, as the modules do; it needs `invoices.create`.

use renox::prelude::*;
use renox::serde_json::Value;
use wl_kernel::agent::{ActionKind, AgentAction, Change, Confirmed, Money, Outcome, Planned};
use wl_kernel::invoicing::{DraftLine, INVOICE_SUBJECT_TYPE, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::{PartyId, Subject};

use crate::invoice::Invoice;
use crate::issue::{ISSUE, gather};
use crate::permissions;

/// What the preview says of the number: the series is gapless, so it is
/// taken only by the issue itself.
pub const NUMBER_ON_ISSUE: &str =
    "Assigned when issued: the next number of the invoice series (none is reserved before)";

/// The key of the issue action.
pub const ISSUE_ACTION: &str = "invoicing.invoice.issue";

/// The key of the create-draft action.
pub const CREATE_DRAFT_ACTION: &str = "invoicing.invoice.create_draft";

/// Registers the invoice actions.
pub(crate) fn register() {
    AgentAction::transition::<Invoice>(INVOICE_SUBJECT_TYPE, ISSUE)
        .label("Issue the invoice")
        .description(
            "Issues a draft invoice: it gets its number, its issue date and its due date, and is \
             sent to the customer. Over the user's limit it is sent for approval instead.",
        )
        .input(|state, invoice, _args| {
            Box::pin(async move {
                let input = gather(&state, &invoice).await?;
                renox::serde_json::to_value(input)
                    .map_err(|e| Error::from(renox::anyhow::anyhow!(e)))
            })
        })
        .describe(|state, invoice, _args| {
            Box::pin(async move {
                let party = wl_kernel::parties::directory(&state)
                    .billing_identity(&state.db, invoice.party())
                    .await?
                    .map(|p| p.legal_name_en)
                    .unwrap_or_else(|| format!("party #{}", invoice.party_id));
                let due = invoice.due_date.map_or_else(
                    || "The payment terms after the issue date".to_owned(),
                    |d| d.to_string(),
                );
                Ok(
                    Change::new(format!("Issue invoice draft #{} to {party}", invoice.id))
                        .line("Number", NUMBER_ON_ISSUE)
                        .line("Customer", &party)
                        .line("Issue date", wl_kernel::documents::today())
                        .line("Due date", due)
                        .amount("Net", invoice.net_total, &invoice.currency)
                        .amount("VAT", invoice.vat_total, &invoice.currency)
                        .amount("Total", invoice.gross_total, &invoice.currency),
                )
            })
        })
        .done(|_state, invoice| {
            Box::pin(async move {
                let number = invoice.number.clone().unwrap_or_default();
                Ok((
                    format!("Issued invoice {number}."),
                    json!({
                        "number": number,
                        "status": invoice.status.to_string(),
                        "total": Money::new(invoice.gross_total, &invoice.currency),
                        "vat": Money::new(invoice.vat_total, &invoice.currency),
                    }),
                ))
            })
        })
        .register();

    AgentAction::custom(INVOICE_SUBJECT_TYPE, "create_draft", ActionKind::Create)
        .label("Draft an invoice")
        .description(
            "Drafts an invoice to a customer for the record it bills (e.g. property.lease:9), \
             with its lines. The draft is not sent: issue it with invoicing.invoice.issue.",
        )
        .arguments(json!({
            "type": "object",
            "required": ["party", "source", "lines"],
            "properties": {
                "party": { "type": "integer", "description": "The customer's party id" },
                "source": { "type": "string", "description": "What the invoice bills, e.g. property.lease:9" },
                "due_date": { "type": "string", "description": "YYYY-MM-DD" },
                "lines": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["description", "amount"],
                        "properties": {
                            "description": { "type": "string" },
                            "amount": { "type": "integer", "description": "Fils before VAT (AED 1.00 = 100)" },
                            "vat": { "type": "string", "enum": ["standard", "zero_rated", "exempt", "out_of_scope", "reverse_charge"] }
                        }
                    }
                }
            }
        }))
        .preview(|call, _record, args| {
            Box::pin(async move {
                let draft = DraftArgs::read(&args)?;
                ensure_may_create(&call.user)?;
                let party = party_name(&call.state, draft.party).await?;
                let net: i64 = draft.lines.iter().map(|l| l.unit_price).sum();
                let vat: i64 = draft
                    .lines
                    .iter()
                    .map(|l| l.unit_price * l.vat_category.rate_bp() / 10_000)
                    .sum();
                let mut change = Change::new(format!("Draft an invoice to {party}"))
                    .line("Customer", &party)
                    .line("Bills", draft.source.words())
                    .line("Status", "draft (not sent)");
                for line in &draft.lines {
                    change = change.line(
                        "Line",
                        format!(
                            "{}: {} ({})",
                            line.description,
                            Money::new(line.unit_price, "AED").text,
                            line.vat_category.as_str()
                        ),
                    );
                }
                let change = change
                    .amount("Net", net, "AED")
                    .amount("VAT (estimated)", vat, "AED")
                    .amount("Total (estimated)", net + vat, "AED");
                Ok(Planned::new(change))
            })
        })
        .confirm(|call, _record, args, _fingerprint| {
            Box::pin(async move {
                let draft = DraftArgs::read(&args)?;
                ensure_may_create(&call.user)?;
                party_name(&call.state, draft.party).await?;
                let mut request = InvoiceDraft::new(PartyId(draft.party), draft.source.clone());
                for line in draft.lines {
                    request = request.line(line);
                }
                if let Some(due) = draft.due_date {
                    request = request.due_date(due);
                }
                let Some(id) = invoicer(&call.state).draft(&call.state.db, request).await? else {
                    return Err(Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "Invoicing is not enabled in this app.".to_owned(),
                    ));
                };
                let subject = Subject::new(INVOICE_SUBJECT_TYPE, id.0);
                Ok(Confirmed {
                    outcome: Outcome::Done,
                    action: CREATE_DRAFT_ACTION.to_owned(),
                    record: Some(subject.to_string()),
                    message: format!("Drafted invoice #{}; it is not issued yet.", id.0),
                    url: wl_kernel::agent::actions::url_of(INVOICE_SUBJECT_TYPE, id.0),
                    data: json!({ "invoice": id.0 }),
                })
            })
        })
        .register();
}

fn invalid(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

fn ensure_may_create(user: &AuthUser) -> Result<()> {
    if user.has_permission(permissions::CREATE) {
        Ok(())
    } else {
        Err(Error::Status(
            StatusCode::FORBIDDEN,
            "You may not do this: draft an invoice.".to_owned(),
        ))
    }
}

/// The party's legal name; 422 when it is not found.
async fn party_name(state: &AppState, party: i64) -> Result<String> {
    wl_kernel::parties::directory(state)
        .billing_identity(&state.db, PartyId(party))
        .await?
        .map(|p| p.legal_name_en)
        .ok_or_else(|| invalid("The customer was not found."))
}

/// The arguments of `create_draft`.
struct DraftArgs {
    party: i64,
    source: Subject,
    due_date: Option<renox::chrono::NaiveDate>,
    lines: Vec<DraftLine>,
}

impl DraftArgs {
    fn read(args: &Value) -> Result<DraftArgs> {
        let party = args
            .get("party")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid("`party` (the customer's id) is missing."))?;
        let source: Subject = args
            .get("source")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| invalid("`source` is not like property.lease:9."))?;
        let due_date = match args.get("due_date").and_then(Value::as_str) {
            Some(text) => Some(
                text.parse()
                    .map_err(|_| invalid("`due_date` is not a date like 2026-11-30."))?,
            ),
            None => None,
        };
        let mut lines = Vec::new();
        for line in args
            .get("lines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let description = line
                .get("description")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .ok_or_else(|| invalid("A line has no description."))?;
            let amount = line
                .get("amount")
                .and_then(Value::as_i64)
                .filter(|a| *a > 0)
                .ok_or_else(|| invalid("A line's amount must be a positive number of fils."))?;
            let vat = match line.get("vat").and_then(Value::as_str) {
                None => VatCategory::Standard,
                Some(text) => *VatCategory::ALL
                    .iter()
                    .find(|c| c.as_str() == text)
                    .ok_or_else(|| invalid("A line's `vat` is not a VAT category."))?,
            };
            lines.push(DraftLine::new(description, amount, vat));
        }
        if lines.is_empty() {
            return Err(invalid("An invoice needs at least one line."));
        }
        Ok(DraftArgs {
            party,
            source,
            due_date,
            lines,
        })
    }
}
