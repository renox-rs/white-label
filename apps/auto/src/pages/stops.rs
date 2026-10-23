//! The pages of a job card that ends before delivery (`AUT.ACL.21`,
//! `AUT.ACL.17`; BL-AUT-01): cancelling it after the work started, releasing
//! the vehicle, saying the customer is on account, and naming the purchase
//! order of a special-order part. The job card page (`pages::job_cards`)
//! shows the forms; these are the routes they post to.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /auto/job-cards/{id}/cancel` | `auto.job_card_cancel` | `job_cards.advise` |
//! | `POST /auto/job-cards/{id}/release` | `auto.job_card_release` | `job_cards.advise` |
//! | `POST /auto/job-cards/{id}/on-account` | `auto.job_card_on_account` | `job_cards.approve` |
//! | `POST /auto/job-cards/{id}/parts/{line}/order` | `auto.job_card_part_order` | `job_cards.advise` |
//!
//! The cancel form carries, per approved labour line, `work_<line>` and
//! `hours_<line>`, and per approved part line `fitted_<line>`; a refusal comes
//! back in words inside the form.

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::dubai_text;

use super::{
    invalid, parse_milli, parse_money, parse_optional_date, parse_optional_id, text, ticked,
};
use crate::parts_wait::PartsWait;
use crate::substitute::{self, Substitute};
use crate::estimate::Estimate;
use crate::job_card::{JobCard, JobCardStatus};
use crate::job_stops::{self, StopRequest};
use crate::labour_line::{LineStatus, WorkStatus, hours_text};
use crate::money_text;
use crate::release::{self, AS_IS, Release, SAFE};
use crate::stop::{
    CLAIM_WITHDRAWN, CUSTOMER_STOPPED, INSURER_NOTICE, REFUSED_TO_SIGN, SIGNED, WRITTEN_WITHDRAWAL,
};
use crate::permissions;
use crate::texts;

/// The routes of the stops.
pub fn routes() -> Routes {
    Routes::new()
        .post("/auto/job-cards/{id}/cancel", cancel)
        .name("auto.job_card_cancel")
        .post("/auto/job-cards/{id}/release", release_vehicle)
        .name("auto.job_card_release")
        .post("/auto/job-cards/{id}/on-account", on_account)
        .name("auto.job_card_on_account")
        .post("/auto/job-cards/{id}/parts/{line}/order", order)
        .name("auto.job_card_part_order")
        .post("/auto/job-cards/{id}/parts/{line}/expected", expected)
        .name("auto.job_card_part_expected")
        .post("/auto/job-cards/{id}/substitute", substitute_record)
        .name("auto.job_card_substitute")
        .post("/auto/job-cards/{id}/substitute/refuse", substitute_refuse)
        .name("auto.job_card_substitute_refuse")
        .post("/auto/job-cards/{id}/substitute/return", substitute_return)
        .name("auto.job_card_substitute_return")
        .require_auth()
}

/// A labour line in the cancel form.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct StopLabourRow {
    /// The line.
    pub id: i64,
    /// What the field asks, naming the line.
    pub label: String,
    /// `not_done`, `part_done` or `done`, as recorded.
    pub work: String,
    /// The hours worked, as recorded.
    pub hours: String,
}

/// A part line in the cancel form.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct StopPartRow {
    /// The line.
    pub id: i64,
    /// What the field asks, naming the part.
    pub label: String,
    /// The quantity fitted, as recorded.
    pub fitted: String,
}

/// A special-order part and its purchase order.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct OrderRow {
    /// The line.
    pub id: i64,
    /// The part.
    pub description: String,
    /// The purchase order number, or a dash.
    pub order: String,
    /// The number as a form field.
    pub raw: String,
}

/// What the job card page shows of the stops.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct StopView {
    /// The approved labour lines, for the cancel form.
    pub labour: Vec<StopLabourRow>,
    /// The approved part lines, for the cancel form.
    pub parts: Vec<StopPartRow>,
    /// The value of the work done so far, before VAT.
    pub value: String,
    /// The reasons a cancel can have.
    pub code_options: Vec<[String; 2]>,
    /// The ways the customer can acknowledge.
    pub ack_options: Vec<[String; 2]>,
    /// How the job stopped, when it did: label and value.
    pub summary: Vec<[String; 2]>,
    /// The special-order parts and their purchase orders.
    pub orders: Vec<OrderRow>,
    /// Whether the vehicle can be released now.
    pub can_release: bool,
    /// The conditions a vehicle is released in.
    pub release_options: Vec<[String; 2]>,
    /// Whether a holder of `job_cards.approve` can say the customer is on
    /// account.
    pub can_on_account: bool,
    /// Whether the customer is on account.
    pub on_account: bool,
    /// Whether the message asking the customer to collect the vehicle gave up.
    pub collect_failed: bool,
}

/// What the job card page shows of the stops of `card` for `user`.
pub(crate) fn view_of(
    lang: &Lang,
    user: &AuthUser,
    card: &JobCard,
    estimate: &Estimate,
) -> StopView {
    let advises = user.has_permission(permissions::JOB_CARDS_ADVISE);
    let labour = estimate
        .labour
        .iter()
        .filter(|line| line.approval == LineStatus::Approved)
        .map(|line| StopLabourRow {
            id: line.id,
            label: texts::t_with(
                lang,
                "auto.stop.work",
                &[("description", &line.description)],
            ),
            work: line.work.as_str().to_owned(),
            hours: if line.hours_done_milli > 0 {
                hours_text(line.hours_done_milli)
            } else {
                String::new()
            },
        })
        .collect();
    let parts = estimate
        .parts
        .iter()
        .filter(|line| line.approval == LineStatus::Approved)
        .map(|line| StopPartRow {
            id: line.id,
            label: texts::t_with(
                lang,
                "auto.stop.fitted",
                &[
                    ("description", &line.description),
                    ("issued", &hours_text(line.issued_milli)),
                ],
            ),
            fitted: hours_text(line.fitted_milli),
        })
        .collect();
    let orders = estimate
        .parts
        .iter()
        .filter(|line| line.special_order && line.approval != LineStatus::Removed)
        .map(|line| OrderRow {
            id: line.id,
            description: line.description.clone(),
            order: line
                .purchase_order_id
                .map_or_else(|| text(lang, "none"), |id| format!("#{id}")),
            raw: line
                .purchase_order_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
        })
        .collect();
    let closed = matches!(
        card.status,
        JobCardStatus::Declined | JobCardStatus::Cancelled
    );
    let mut summary = Vec::new();
    if closed {
        let none = || text(lang, "none");
        summary.push([
            text(lang, "stop.entry.reason"),
            card.closed_reason.clone().unwrap_or_else(none),
        ]);
        if let Some(method) = &card.ack_method {
            let who = card
                .ack_name
                .clone()
                .or_else(|| card.ack_witness.clone())
                .or_else(|| card.ack_document.clone())
                .unwrap_or_else(none);
            summary.push([
                text(lang, "stop.entry.ack"),
                format!("{} · {who}", text(lang, &format!("stop.ack.{method}"))),
            ]);
        }
        if card.status == JobCardStatus::Cancelled {
            summary.push([
                text(lang, "stop.entry.work_value"),
                money_text(card.work_done_value),
            ]);
        }
        summary.push([
            text(lang, "stop.entry.billed"),
            money_text(card.billed_net),
        ]);
        if card.bill_lowered {
            summary.push([text(lang, "stop.entry.lowered"), text(lang, "jobs.yes")]);
        }
        let handed: Vec<String> = estimate
            .parts
            .iter()
            .filter(|line| line.handed_over_at.is_some())
            .map(|line| line.description.clone())
            .collect();
        if !handed.is_empty() {
            summary.push([text(lang, "stop.entry.handed_over"), handed.join(", ")]);
        }
        if let (Some(at), Some(to)) = (card.released_at, &card.released_to) {
            let condition = card
                .release_condition
                .as_deref()
                .unwrap_or(SAFE)
                .to_owned();
            summary.push([
                text(lang, "stop.entry.released"),
                texts::t_with(
                    lang,
                    "auto.release.value",
                    &[
                        ("to", to),
                        ("condition", &text(lang, &format!("release.{condition}"))),
                        ("at", &dubai_text(at)),
                    ],
                ),
            ]);
        }
    }
    // The reminders to collect the vehicle and its storage (`AUT.RUL.18`).
    if let Some(first) = card.collect_first_on {
        summary.push([text(lang, "storage.first"), first.to_string()]);
    }
    if let Some(second) = card.collect_second_on {
        summary.push([text(lang, "storage.second"), second.to_string()]);
    }
    if let Some(manager) = card.collect_manager_on {
        summary.push([text(lang, "storage.manager"), manager.to_string()]);
    }
    if card.storage_days > 0 {
        summary.push([text(lang, "storage.days"), card.storage_days.to_string()]);
    }
    if let Some(invoice) = card.storage_invoice_id {
        summary.push([text(lang, "storage.invoice"), format!("#{invoice}")]);
    }
    StopView {
        labour,
        parts,
        value: money_text(crate::stop::value_of(card, estimate)),
        code_options: [CUSTOMER_STOPPED, CLAIM_WITHDRAWN]
            .into_iter()
            .map(|code| [code.to_owned(), text(lang, &format!("stop.code.{code}"))])
            .collect(),
        ack_options: [SIGNED, REFUSED_TO_SIGN, INSURER_NOTICE, WRITTEN_WITHDRAWAL]
            .into_iter()
            .map(|method| [method.to_owned(), text(lang, &format!("stop.ack.{method}"))])
            .collect(),
        summary,
        orders,
        can_release: advises && closed && card.released_at.is_none(),
        release_options: [SAFE, AS_IS]
            .into_iter()
            .map(|condition| {
                [
                    condition.to_owned(),
                    text(lang, &format!("release.{condition}")),
                ]
            })
            .collect(),
        can_on_account: user.has_permission(permissions::JOB_CARDS_APPROVE)
            && card.released_at.is_none(),
        on_account: card.on_account,
        collect_failed: card.collect_message_failed_at.is_some(),
    }
}

/// What the page shows of the substitute vehicle of a job card.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct SubstituteView {
    /// Whether one was asked for (the warranty task is open or closed).
    pub shown: bool,
    /// Where it stands, in words.
    pub status: String,
    /// The plate lent, or a dash.
    pub plate: String,
    /// The days lent: from, to.
    pub period: String,
    /// Whether the advisor can record the car or the refusal.
    pub can_record: bool,
    /// Whether the advisor can record its return.
    pub can_return: bool,
    /// Today, as the default day of the forms.
    pub today: String,
}

/// The wait for parts of `card`: its start and its legal supply date, as label
/// and value; empty when it never waited.
pub(crate) async fn wait_rows(
    state: &AppState,
    lang: &Lang,
    card: &JobCard,
) -> Result<Vec<[String; 2]>> {
    let mut rows = Vec::new();
    if let (Some(started), Some(legal)) = (card.parts_wait_started_at, card.legal_supply_date) {
        rows.push([text(lang, "parts.wait_started"), dubai_text(started)]);
        rows.push([text(lang, "parts.legal_date"), legal.to_string()]);
        if let Some(wait) = PartsWait::open_of(&state.db, card.id).await? {
            let source = text(lang, &format!("supply.source.{}", wait.source));
            rows.push([text(lang, "parts.legal_source"), source]);
        }
    }
    Ok(rows)
}

/// The substitute vehicle of `card` for `user`.
pub(crate) async fn substitute_view(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
    card: &JobCard,
) -> Result<SubstituteView> {
    let advises = user.has_permission(permissions::JOB_CARDS_ADVISE) && card.status.is_open();
    let row = Substitute::of_job_card(&state.db, card.id).await?;
    let none = || text(lang, "none");
    let view = match row {
        None => SubstituteView {
            shown: false,
            status: String::new(),
            plate: none(),
            period: none(),
            can_record: false,
            can_return: false,
            today: substitute::default_day().to_string(),
        },
        Some(row) => {
            let status = if row.lent() && row.to_date.is_some() {
                "returned"
            } else if row.lent() {
                "lent"
            } else if row.refused() {
                "refused"
            } else {
                "requested"
            };
            SubstituteView {
                shown: true,
                status: text(lang, &format!("substitute.status.{status}")),
                plate: row.plate.clone().unwrap_or_else(none),
                period: match (row.from_date, row.to_date) {
                    (Some(from), Some(to)) => format!("{from} → {to}"),
                    (Some(from), None) => format!("{from} →"),
                    _ => row.refused_reason.clone().unwrap_or_else(none),
                },
                can_record: advises && row.requested_only(),
                can_return: advises && row.lent() && row.to_date.is_none(),
                today: substitute::default_day().to_string(),
            }
        }
    };
    Ok(view)
}

/// A field of the form, trimmed; `None` when blank.
fn field(form: &BTreeMap<String, String>, name: &str) -> Option<String> {
    form.get(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The cancel the form asks for.
fn request_of(form: &BTreeMap<String, String>) -> std::result::Result<StopRequest, Error> {
    let mut request = StopRequest::new(&field(form, "code").unwrap_or_default());
    request.note = field(form, "note");
    request.ack_method = field(form, "ack").unwrap_or_default();
    request.ack_name = field(form, "ack_name");
    request.ack_signature = field(form, "ack_signature");
    request.ack_witness = field(form, "ack_witness");
    request.ack_document = field(form, "ack_document");
    if let Some(bill) = field(form, "bill") {
        request.bill = Some(parse_money(&bill)?);
    }
    for (name, value) in form {
        if let Some(id) = name.strip_prefix("work_") {
            let line: i64 = id
                .parse()
                .map_err(|_| invalid("That line is not on this job card."))?;
            let work: WorkStatus = value
                .trim()
                .parse()
                .map_err(|_| invalid("Say how much of each labour line was done."))?;
            let hours = match field(form, &format!("hours_{line}")) {
                Some(hours) => parse_milli(&hours)?,
                None => 0,
            };
            request = request.labour(line, work, hours);
        } else if let Some(id) = name.strip_prefix("fitted_") {
            let line: i64 = id
                .parse()
                .map_err(|_| invalid("That line is not on this job card."))?;
            request = request.fitted(line, parse_milli(value)?);
        }
    }
    Ok(request)
}

/// `POST /auto/job-cards/{id}/cancel`.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let request = request_of(&form)?;
    let stopped = job_stops::cancel(&state, &user, id, request).await?;
    let key = if stopped.approval_request.is_some() {
        "stop.waits"
    } else {
        "stop.cancelled"
    };
    Ok((Toast::success(text(&lang, key)), renox::HxRefresh).into_response())
}

/// `POST /auto/job-cards/{id}/release`: `released_to`, `condition`,
/// `signature`.
async fn release_vehicle(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let request = Release {
        released_to: field(&form, "released_to").unwrap_or_default(),
        condition: field(&form, "condition").unwrap_or_default(),
        signature: field(&form, "signature"),
    };
    release::release(&state, &user, id, request).await?;
    Ok((
        Toast::success(text(&lang, "release.released")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /auto/job-cards/{id}/on-account`: `on_account` ticked or not.
async fn on_account(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let on = ticked(form.get("on_account").map_or("", String::as_str));
    release::set_on_account(&state, &user, id, on).await?;
    Ok((
        Toast::success(text(&lang, "release.on_account_saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /auto/job-cards/{id}/parts/{line}/order`: `order`, the purchase order
/// number, blank to clear it.
async fn order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((_id, line)): Path<(i64, i64)>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let number = parse_optional_id(form.get("order").map_or("", String::as_str))?;
    job_stops::link_purchase_order(&state, &user, line, number).await?;
    Ok((
        Toast::success(text(&lang, "stop.order_saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /auto/job-cards/{id}/parts/{line}/expected`: `expected`, a date,
/// blank to clear it.
async fn expected(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((_id, line)): Path<(i64, i64)>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let day = parse_optional_date(form.get("expected").map_or("", String::as_str))?;
    crate::parts::set_expected(&state, &user, line, day).await?;
    Ok((
        Toast::success(text(&lang, "parts.expected_saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /auto/job-cards/{id}/substitute`: `plate`, `from`, `to`.
async fn substitute_record(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let from = parse_optional_date(form.get("from").map_or("", String::as_str))?
        .unwrap_or_else(substitute::default_day);
    let to = parse_optional_date(form.get("to").map_or("", String::as_str))?;
    let plate = field(&form, "plate").unwrap_or_default();
    substitute::record(&state, &user, id, &plate, from, to).await?;
    Ok((
        Toast::success(text(&lang, "substitute.saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /auto/job-cards/{id}/substitute/refuse`: `reason`.
async fn substitute_refuse(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = field(&form, "reason").unwrap_or_default();
    substitute::refuse(&state, &user, id, &reason).await?;
    Ok((
        Toast::success(text(&lang, "substitute.saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /auto/job-cards/{id}/substitute/return`: `to`.
async fn substitute_return(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let to = parse_optional_date(form.get("to").map_or("", String::as_str))?
        .unwrap_or_else(substitute::default_day);
    substitute::return_vehicle(&state, &user, id, to).await?;
    Ok((
        Toast::success(text(&lang, "substitute.saved")),
        renox::HxRefresh,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn the_cancel_form_names_the_work_done_per_line() {
        let request = request_of(&form(&[
            ("code", "customer_stopped"),
            ("ack", "signed"),
            ("ack_name", " Mona Hassan "),
            ("work_4", "part_done"),
            ("hours_4", "1.5"),
            ("work_5", "done"),
            ("fitted_9", "2"),
            ("bill", "300"),
        ]))
        .unwrap();
        assert_eq!(request.code, "customer_stopped");
        assert_eq!(request.ack_name.as_deref(), Some("Mona Hassan"));
        assert_eq!(request.bill, Some(30_000));
        assert_eq!(request.labour.len(), 2);
        assert_eq!(request.labour[0].line, 4);
        assert_eq!(request.labour[0].hours_milli, 1_500);
        assert_eq!(request.parts[0].fitted_milli, 2_000);
    }

    #[test]
    fn a_work_status_that_is_not_one_is_refused_in_words() {
        let refused = request_of(&form(&[("work_4", "mostly")])).unwrap_err();
        assert!(refused.to_string().contains("how much"), "{refused}");
    }
}
