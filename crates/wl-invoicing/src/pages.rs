//! The back-office pages (wl-invoicing §6, DESIGN-SYSTEM):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /invoices` | `wl_invoicing.index` | `invoices.view` |
//! | `GET /invoices/{id}` | `wl_invoicing.show` | `invoices.view` |
//! | `POST /invoices/{id}/issue` | `wl_invoicing.issue` | `invoices.issue` |
//! | `POST /invoices/{id}/write-off` | `wl_invoicing.write_off` | `invoices.write_off` |
//! | `POST /invoices/{id}/transitions/{name}` | `wl_invoicing.transitions.*` | `invoices.view` and the transition's permission |
//!
//! `issue` and `write_off` have their own routes because their guards read
//! the input the module gathers (the issuing facts; the balance and the
//! reason); the kernel's transition route ([`wl_kernel::machines::routes`])
//! serves `<rx-transition-actions>` for the others, and both announce an
//! approval request a run created. Every template is compiled in and can be
//! replaced by the app (`resources/views/wl-invoicing/…`).

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{self, Available, StateMachine, TransitionError};
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::IssuerSnapshot;
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};
use wl_kernel::notify::fill;

use crate::allocate::amount_text;
use crate::allocation::Allocation;
use crate::config::{InvoicingConfig, ReasonProblem};
use crate::config_of;
use crate::invoice::{Invoice, PartySnapshot};
use crate::invoicer::{lines_of, totals_of};
use crate::issue::issue;
use crate::line::InvoiceLine;
use crate::permissions;
use crate::texts::{self, english};
use crate::vat_adjustment::VatAdjustment;
use crate::write_off::{offered, write_off};
use crate::write_off_notice::recipient;

/// The texts a template reads as `tx['fields.number']`.
type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_invoicing.` prefix) in a language.
type Text<'a> = &'a dyn Fn(&str) -> String;

/// The package texts in `lang`.
fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_invoicing.{key}"))
}

/// The module's routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/invoices", index)
                .name("wl_invoicing.index")
                .get("/invoices/{id}", show)
                .name("wl_invoicing.show")
                .group(
                    "/invoices",
                    "wl_invoicing.transitions.",
                    wl_kernel::machines::routes::<Invoice>(),
                )
                .require_permission(permissions::VIEW),
        )
        .merge(
            Routes::new()
                .post("/invoices/{id}/issue", store_issue)
                .name("wl_invoicing.issue")
                .require_permission(permissions::ISSUE),
        )
        .merge(
            Routes::new()
                .post("/invoices/{id}/write-off", store_write_off)
                .name("wl_invoicing.write_off")
                .require_permission(permissions::WRITE_OFF),
        )
}

/// The badge colour of a status, as the machine's tones give it.
fn status_kind(status: InvoiceStatus) -> &'static str {
    match status {
        InvoiceStatus::Issued | InvoiceStatus::PartiallyPaid => "info",
        InvoiceStatus::Overdue => "error",
        InvoiceStatus::Paid => "success",
        InvoiceStatus::Draft | InvoiceStatus::WrittenOff | InvoiceStatus::Credited => "neutral",
    }
}

/// The invoices grid: search on the number and the party, filters by
/// status and issuing entity, money in each invoice's currency.
pub fn grid(lang: &Lang) -> Grid {
    grid_with(&texts_in(lang))
}

/// [`grid`] with the texts of `t`.
fn grid_with(t: Text<'_>) -> Grid {
    let statuses: Vec<(String, String)> = InvoiceStatus::ALL
        .iter()
        .map(|s| (s.as_str().to_owned(), t(&format!("status.{}", s.as_str()))))
        .collect();
    Grid::new("wl-invoicing")
        .title(&t("title"))
        .column(
            Column::text("number", &t("fields.number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/invoices/{id}"),
        )
        .column(
            // The party as billed (the snapshot taken at `issue`).
            Column::text("party_name", &t("fields.party"))
                .computed("json_extract({T}.\"party_snapshot\", '$.legal_name_en')")
                .mobile()
                .searchable(),
        )
        .column(Column::related(
            "entity",
            &t("fields.entity"),
            "wl_legal_entities",
            "legal_entity_id",
            "code",
        ))
        .column(Column::date("issue_date", &t("fields.issue_date")))
        .column(Column::date("due_date", &t("fields.due_date")))
        .column(Column::money("gross_total", &t("fields.gross_total")).currency_from("currency"))
        .column(
            Column::money("balance", &t("fields.balance"))
                .currency_from("currency")
                .mobile(),
        )
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("draft", "neutral"),
                    ("issued", "info"),
                    ("partially_paid", "info"),
                    ("overdue", "danger"),
                    ("paid", "success"),
                    ("written_off", "neutral"),
                    ("credited", "neutral"),
                ])
                .mobile(),
        )
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .sort_by("-created_at")
        .row_url("/invoices/{id}")
        .cards_on_mobile()
        .empty_state(&t("empty"), Some(&t("empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/index.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    invoices: GridPage<Invoice>,
}

/// `GET /invoices`: the grid.
async fn index(lang: Lang, request: GridRequest) -> Result<View> {
    let invoices = grid(&lang).page(Invoice::query(), &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        invoices,
    }))
}

/// One line as the invoice page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineRow {
    id: i64,
    description: String,
    quantity: String,
    unit_price: String,
    discount: String,
    net: String,
    category: String,
    vat: String,
    gross: String,
}

/// One VAT category's totals.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct VatRow {
    category: String,
    net: String,
    vat: String,
}

/// One allocation.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct AllocationRow {
    id: i64,
    source: String,
    amount: String,
    allocated_at: DateTime,
}

/// The write-off section of a written-off invoice.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct WriteOffRow {
    amount: String,
    reason: String,
    reason_text: Option<String>,
    date: Option<renox::chrono::NaiveDate>,
    notice_on: Option<renox::chrono::NaiveDate>,
    /// Why no notice date is shown: off, no billing e-mail, or being sent.
    notice_note: Option<String>,
    relief: Option<String>,
    relief_status: Option<String>,
}

/// A quantity in thousandths as `2.5`.
fn quantity_text(milli: i64) -> String {
    let whole = milli / 1000;
    let rest = (milli % 1000).abs();
    if rest == 0 {
        return whole.to_string();
    }
    let decimals = format!("{rest:03}");
    format!("{whole}.{}", decimals.trim_end_matches('0'))
}

/// The label of a VAT category.
fn category_label(t: Text<'_>, category: VatCategory) -> String {
    t(&format!("vat_category.{}", category.as_str()))
}

/// The label of a write-off reason code.
fn reason_label(t: Text<'_>, code: &str) -> String {
    t(&format!("write_off_reason.{code}"))
}

/// What the invoice page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/show.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    invoice: Invoice,
    /// The number, or "Draft".
    title: String,
    status_label: String,
    status_kind: &'static str,
    /// Whether an approval request about it is pending (the badge).
    pending: bool,
    issuer: Option<IssuerSnapshot>,
    party: Option<PartySnapshot>,
    /// Net, VAT, gross, allocated and balance, formatted.
    amounts: BTreeMap<&'static str, String>,
    lines: Vec<LineRow>,
    vat: Vec<VatRow>,
    allocations: Vec<AllocationRow>,
    write_off: Option<WriteOffRow>,
    /// The approval cards (`wl-kernel/approval_card.html`).
    approvals: Vec<ApprovalCard>,
    /// The invoice's history merged with its approval requests'.
    timeline: Vec<Value>,
    /// The other transitions this user may run (`<rx-transition-actions>`).
    actions: Vec<Available>,
    can_issue: bool,
    can_write_off: bool,
    /// The write-off reasons, `(code, label)`.
    reasons: Vec<(String, String)>,
    /// The sheet's description, with the balance.
    write_off_description: String,
}

/// The write-off section of `invoice` with its bad-debt `relief`, once it
/// is written off.
fn write_off_row(
    config: &InvoicingConfig,
    t: Text<'_>,
    invoice: &Invoice,
    relief: Option<&VatAdjustment>,
) -> Option<WriteOffRow> {
    let amount = invoice.write_off_amount?;
    let notice_note = match invoice.write_off_notice_on {
        Some(_) => None,
        None => Some(match recipient(config, invoice) {
            Ok(_) => t("write_off_notice.waiting"),
            Err(why) => t(why.text_key().trim_start_matches("wl_invoicing.")),
        }),
    };
    Some(WriteOffRow {
        amount: amount_text(&invoice.currency, amount),
        reason: invoice
            .write_off_reason_code
            .as_deref()
            .map(|code| reason_label(t, code))
            .unwrap_or_default(),
        reason_text: invoice.write_off_reason_text.clone(),
        date: invoice.write_off_date,
        notice_on: invoice.write_off_notice_on,
        notice_note,
        relief: relief.map(|r| amount_text("AED", r.amount)),
        relief_status: relief.map(|r| t(&format!("adjustment_status.{}", r.status.as_str()))),
    })
}

/// `GET /invoices/{id}`: the snapshots, lines, VAT, allocations, the
/// write-off, the approval card, the history and the actions.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(invoice): Found<Invoice>,
) -> Result<View> {
    let t = texts_in(&lang);
    let db = state.db.clone();
    let currency = invoice.currency.clone();
    let money = |fils: i64| amount_text(&currency, fils);
    let lines = lines_of(&db, invoice.id)
        .await?
        .iter()
        .map(|line: &InvoiceLine| LineRow {
            id: line.id,
            description: line.description.clone(),
            quantity: quantity_text(line.quantity_milli),
            unit_price: money(line.unit_price),
            discount: money(line.discount),
            net: money(line.net_amount),
            category: category_label(&t, line.vat_category),
            vat: money(line.vat_amount),
            gross: money(line.gross_amount),
        })
        .collect();
    let vat = totals_of(&db, invoice.id)
        .await?
        .categories
        .iter()
        .map(|c| VatRow {
            category: category_label(&t, c.category),
            net: money(c.net),
            vat: money(c.vat),
        })
        .collect();
    let allocations = Allocation::query()
        .where_(Allocation::INVOICE_ID.eq(invoice.id))
        .order_by(Allocation::ID)
        .get(&db)
        .await?
        .into_iter()
        .map(|a| AllocationRow {
            id: a.id,
            source: a.source().to_string(),
            amount: money(a.amount),
            allocated_at: a.allocated_at,
        })
        .collect();
    let amounts = BTreeMap::from([
        ("net_total", money(invoice.net_total)),
        ("vat_total", money(invoice.vat_total)),
        ("gross_total", money(invoice.gross_total)),
        ("allocated_total", money(invoice.allocated_total)),
        ("balance", money(invoice.balance)),
    ]);
    let subject = invoice.subject();
    let pending = !approvals::pending_for(&db, &subject).await?.is_empty();
    let approval_cards = approvals::cards_for(&db, &subject, &user, &lang).await?;
    let history = state_machine::history(&db, Invoice::TABLE, invoice.id).await?;
    let mut timeline =
        state_machine::timeline(&history, Some(&state_machine::info_of::<Invoice>()));
    timeline.extend(approvals::timeline_for(&db, &subject, &lang).await?);
    timeline.sort_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()));
    // `issue` and `write_off` have their own buttons.
    let actions = invoice
        .available(&user, &db)
        .await?
        .into_iter()
        .filter(|a| a.name != crate::issue::ISSUE && a.name != crate::write_off::WRITE_OFF)
        .collect();
    let config = config_of(&state);
    let can_issue =
        invoice.status == InvoiceStatus::Draft && user.has_permission(permissions::ISSUE);
    let can_write_off = !pending && offered(&config, &user, &invoice).await;
    let reasons = if can_write_off {
        config
            .write_off_reasons
            .iter()
            .map(|r| (r.code.clone(), reason_label(&t, &r.code)))
            .collect()
    } else {
        Vec::new()
    };
    let write_off_description = fill(
        &t("write_off.description"),
        &json!({ "balance": money(invoice.balance) }),
    );
    let relief = VatAdjustment::relief_of(&db, &invoice).await?;
    let write_off = write_off_row(&config, &t, &invoice, relief.as_ref());
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: invoice.number.clone().unwrap_or_else(|| t("draft_number")),
        status_label: t(&format!("status.{}", invoice.status.as_str())),
        status_kind: status_kind(invoice.status),
        pending,
        issuer: invoice.issuer_snapshot.as_ref().map(|s| s.0.clone()),
        party: invoice.party_snapshot.as_ref().map(|s| s.0.clone()),
        amounts,
        lines,
        vat,
        allocations,
        write_off,
        approvals: approval_cards,
        timeline,
        actions,
        can_issue,
        can_write_off,
        reasons,
        write_off_description,
        invoice,
    }))
}

/// `POST /invoices/{id}/issue`: issues the draft and reloads its page. A
/// refusal answers 422 with the guard's message.
async fn store_issue(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(invoice): Found<Invoice>,
) -> Result<Response> {
    issue(&state, &user, &invoice)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.issue.done")),
        Redirect::route("wl_invoicing.show", &[&invoice.id])?,
    )
        .into_response())
}

/// The write-off sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct WriteOffForm {
    /// The reason code.
    #[validate(required, max = 64)]
    pub reason_code: String,
    /// The text given with the reason (required for `other`).
    #[validate(max = 2000)]
    pub reason_text: Option<String>,
}

/// The form field a write-off refusal belongs under.
fn refused_field(message: &str) -> &'static str {
    if message == english(ReasonProblem::TextRequired.text_key()) {
        "reason_text"
    } else {
        "reason_code"
    }
}

/// `POST /invoices/{id}/write-off`: writes off the open balance, or (above
/// the `FIN.ACL.15` limit) sends it for approval, and reloads the page. The
/// guards' refusals (no balance, disputed, an unknown reason, `other`
/// without a text) answer 422 with the message under the field.
async fn store_write_off(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(invoice): Found<Invoice>,
    Valid(form): Valid<WriteOffForm>,
) -> Result<Response> {
    match write_off(
        &state,
        &user,
        &invoice,
        &form.reason_code,
        form.reason_text.as_deref(),
    )
    .await
    {
        Ok(outcome) => {
            let key = if outcome.is_pending() {
                "wl_invoicing.write_off.sent"
            } else {
                "wl_invoicing.write_off.done"
            };
            Ok((Toast::success(texts::t(&lang, key)), renox::HxRefresh).into_response())
        }
        Err(TransitionError::Guard(message)) => {
            let mut errors = Errors::new();
            errors.add(refused_field(&message), message);
            Err(Error::Validation(
                ValidationError::new(errors).with_input(&form),
            ))
        }
        Err(other) => Err(other.into_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english_text(key: &str) -> String {
        texts::english(&format!("wl_invoicing.{key}")).to_owned()
    }

    #[test]
    fn the_grid_lists_the_invoice_columns_and_statuses() {
        let grid = grid_with(&english_text);
        let keys: Vec<&str> = grid.columns().iter().map(Column::key).collect();
        assert_eq!(
            keys,
            [
                "number",
                "party_name",
                "entity",
                "issue_date",
                "due_date",
                "gross_total",
                "balance",
                "status",
                "created_at"
            ]
        );
        let status = grid
            .columns()
            .iter()
            .find(|c| c.key() == "status")
            .expect("status column");
        assert!(
            status
                .options()
                .iter()
                .any(|(value, label)| value == "written_off" && label == "Written off")
        );
        assert_eq!(status.options().len(), InvoiceStatus::ALL.len());
    }

    #[test]
    fn quantities_read_without_trailing_zeros() {
        assert_eq!(quantity_text(1_000), "1");
        assert_eq!(quantity_text(2_500), "2.5");
        assert_eq!(quantity_text(1_125), "1.125");
    }

    #[test]
    fn a_missing_text_is_shown_under_the_text_field() {
        assert_eq!(
            refused_field(english(ReasonProblem::TextRequired.text_key())),
            "reason_text"
        );
        assert_eq!(refused_field("The invoice is disputed."), "reason_code");
    }

    #[test]
    fn every_status_and_category_has_a_label() {
        for status in InvoiceStatus::ALL {
            let key = format!("status.{}", status.as_str());
            assert_ne!(english_text(&key), format!("wl_invoicing.{key}"));
        }
        for category in VatCategory::ALL {
            let key = format!("vat_category.{}", category.as_str());
            assert_ne!(english_text(&key), format!("wl_invoicing.{key}"));
        }
    }
}
