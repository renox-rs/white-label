//! The back-office routes of the vertical (07 Manufacturing §7.1):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /manufacturing/work-orders` | `manufacturing.work_orders` | a permission of the vertical for the plant or the line |
//! | `POST /manufacturing/work-orders` | `manufacturing.work_order_create` | `work_orders.create` for the plant |
//! | `GET /manufacturing/work-orders/{id}` | `manufacturing.work_order` | as the list |
//! | `POST /manufacturing/work-orders/{id}/release` | `manufacturing.release` | `work_orders.release` |
//! | `POST /manufacturing/work-orders/{id}/start` | `manufacturing.start` | `work_orders.report` for the plant or the line |
//! | `POST /manufacturing/work-orders/{id}/hold` | `manufacturing.hold` | `work_orders.report` |
//! | `POST /manufacturing/work-orders/{id}/resume` | `manufacturing.resume` | `work_orders.report` |
//! | `POST /manufacturing/work-orders/{id}/complete` | `manufacturing.complete` | `work_orders.report` |
//! | `POST /manufacturing/work-orders/{id}/close` | `manufacturing.close` | `work_orders.close` |
//! | `POST /manufacturing/work-orders/{id}/cancel` | `manufacturing.cancel` | `work_orders.release` |
//! | `POST /manufacturing/work-orders/{id}/unlink` | `manufacturing.unlink` | `work_orders.release` (BL-MFG-02) |
//! | `POST /manufacturing/work-orders/{id}/abort` | `manufacturing.abort` | `work_orders.release`; the production manager approves (BL-MFG-01) |
//! | `POST /manufacturing/work-orders/{id}/issue` | `manufacturing.issue` | `work_orders.issue` |
//! | `POST /manufacturing/work-orders/{id}/report` | `manufacturing.report` | `work_orders.report` |
//! | `POST /manufacturing/reports/{id}/void` | `manufacturing.report_void` | `work_orders.report` on the line; the line's supervisor or the production manager (`work_orders.void_approve`) approves (BL-MFG-04) |
//! | `GET /portal/manufacturing/invoices` | `manufacturing.portal_invoices` | a portal grant for the buyer's company (BL-MFG-13) |
//! | `POST /portal/manufacturing/invoices/{id}/dispute` | `manufacturing.portal_dispute` | as above; reason, text and, for quality, the ticked lines |
//! | `GET /manufacturing/ncrs` | `manufacturing.ncrs` | a permission of the NCRs for a plant |
//! | `POST /manufacturing/ncrs` | `manufacturing.ncr_create` | `ncrs.raise` for the plant |
//! | `GET /manufacturing/ncrs/{id}` | `manufacturing.ncr` | as the list |
//! | `POST /manufacturing/ncrs/{id}/disposition` | `manufacturing.ncr_disposition` | `ncrs.disposition`; a scrap above AED 2,000 waits for `stock.approve`, *use as is* for `ncrs.approve` (BL-MFG-08) |
//! | `POST /manufacturing/ncrs/{id}/close` | `manufacturing.ncr_close` | `ncrs.disposition` |
//! | `POST /manufacturing/ncrs/{id}/reopen` | `manufacturing.ncr_reopen` | `ncrs.disposition` |
//! | `POST /manufacturing/ncrs/{id}/cancel` | `manufacturing.ncr_cancel` | `ncrs.disposition` |
//! | `POST /manufacturing/ncrs/{id}/cause` | `manufacturing.ncr_cause` | `ncrs.raise` (BL-MFG-15) |
//! | `GET /manufacturing/conformity` | `manufacturing.conformity` | a permission of the vertical (BL-MFG-11) |
//! | `POST /manufacturing/conformity` | `manufacturing.conformity_flag` | `conformity.manage`; the item by its code, regulated, export only, the accepted kinds |
//! | `GET /portal/manufacturing/orders` | `manufacturing.portal_orders` | a portal grant for the buyer's company (BL-MFG-06) |
//! | `POST /portal/manufacturing/orders` | `manufacturing.portal_orders_upload` | as above; PO number, call-off number, note; a duplicate is refused naming the existing order's number and status |
//! | `GET /manufacturing/exports` | `manufacturing.exports` | `exports.manage` (BL-MFG-12) |
//! | `POST /manufacturing/exports/notes` | `manufacturing.exports_note` | `exports.manage`; the customer's contract note |
//! | `GET /manufacturing/recalls` | `manufacturing.recalls` | `recalls.view` or `recalls.manage` (BL-MFG-10) |
//! | `GET /manufacturing/recalls/{id}` | `manufacturing.recall` | as the list: the customers a recall reached |
//!
//! The lists and the record page answer a page when the request names
//! `text/html` and JSON otherwise ([`wl_kernel::negotiate`]); the actions
//! answer 303 back to the page of the work order with a toast
//! ([`crate::screens::done`]) and a refusal as its 422 with the words the
//! sheet shows.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;

use crate::abort::{self, AbortAsk, ReturnLine};
use crate::db::refused;
use crate::issue::{self, IssueAsk};
use crate::ncr::{Cause, Disposition, Origin};
use crate::ncrs::{self, DispositionAsk, NewNcr};
use crate::report::{self, ReportAsk};
use crate::screens::{self, done, parse_quantity, work_order_page};
use crate::texts::{english, t};
use crate::unlink;
use crate::work_orders::{self, NewWorkOrder};

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get(
            "/manufacturing/work-orders",
            screens::work_orders_page::show,
        )
        .name("manufacturing.work_orders")
        .post("/manufacturing/work-orders", create)
        .name("manufacturing.work_order_create")
        .get(
            "/manufacturing/work-orders/{id}",
            screens::work_order_page::show,
        )
        .name("manufacturing.work_order")
        .post("/manufacturing/work-orders/{id}/release", release)
        .name("manufacturing.release")
        .post("/manufacturing/work-orders/{id}/start", start)
        .name("manufacturing.start")
        .post("/manufacturing/work-orders/{id}/hold", hold)
        .name("manufacturing.hold")
        .post("/manufacturing/work-orders/{id}/resume", resume)
        .name("manufacturing.resume")
        .post("/manufacturing/work-orders/{id}/complete", complete)
        .name("manufacturing.complete")
        .post("/manufacturing/work-orders/{id}/close", close)
        .name("manufacturing.close")
        .post("/manufacturing/work-orders/{id}/cancel", cancel)
        .name("manufacturing.cancel")
        .post("/manufacturing/work-orders/{id}/unlink", unlink_order)
        .name("manufacturing.unlink")
        .post("/manufacturing/work-orders/{id}/abort", abort_order)
        .name("manufacturing.abort")
        .post("/manufacturing/work-orders/{id}/issue", issue_material)
        .name("manufacturing.issue")
        .post("/manufacturing/work-orders/{id}/report", report_output)
        .name("manufacturing.report")
        .post("/manufacturing/reports/{id}/void", void_report)
        .name("manufacturing.report_void")
        .get("/portal/manufacturing/invoices", crate::portal::show)
        .name("manufacturing.portal_invoices")
        .post(
            "/portal/manufacturing/invoices/{id}/dispute",
            crate::portal::dispute,
        )
        .name("manufacturing.portal_dispute")
        .get("/manufacturing/ncrs", screens::ncrs_page::show)
        .name("manufacturing.ncrs")
        .post("/manufacturing/ncrs", raise_ncr)
        .name("manufacturing.ncr_create")
        .get("/manufacturing/ncrs/{id}", screens::ncr_page::show)
        .name("manufacturing.ncr")
        .post("/manufacturing/ncrs/{id}/disposition", disposition_ncr)
        .name("manufacturing.ncr_disposition")
        .post("/manufacturing/ncrs/{id}/close", close_ncr)
        .name("manufacturing.ncr_close")
        .post("/manufacturing/ncrs/{id}/reopen", reopen_ncr)
        .name("manufacturing.ncr_reopen")
        .post("/manufacturing/ncrs/{id}/cancel", cancel_ncr)
        .name("manufacturing.ncr_cancel")
        .post("/manufacturing/ncrs/{id}/cause", cause_ncr)
        .name("manufacturing.ncr_cause")
        .get("/manufacturing/conformity", screens::conformity_page::show)
        .name("manufacturing.conformity")
        .post("/manufacturing/conformity", screens::conformity_page::flag)
        .name("manufacturing.conformity_flag")
        .get("/portal/manufacturing/orders", crate::po_upload::show)
        .name("manufacturing.portal_orders")
        .post("/portal/manufacturing/orders", crate::po_upload::post)
        .name("manufacturing.portal_orders_upload")
        .get("/manufacturing/exports", screens::exports_page::show)
        .name("manufacturing.exports")
        .post("/manufacturing/exports/notes", screens::exports_page::note)
        .name("manufacturing.exports_note")
        .get("/manufacturing/recalls", screens::recall_page::list)
        .name("manufacturing.recalls")
        .get("/manufacturing/recalls/{id}", screens::recall_page::show)
        .name("manufacturing.recall")
}

/// A 303 to `path`.
pub(crate) fn back(path: &str) -> Response {
    renox::axum::response::Redirect::to(path).into_response()
}

/// The text of a form field, empty when absent.
fn field(form: &BTreeMap<String, String>, name: &str) -> String {
    form.get(name)
        .map(|v| v.trim().to_owned())
        .unwrap_or_default()
}

/// A whole number field; `None` when empty, a 422 when it is not a number.
fn number(lang: &Lang, form: &BTreeMap<String, String>, name: &str) -> Result<Option<i64>> {
    match field(form, name).as_str() {
        "" => Ok(None),
        value => value
            .parse::<i64>()
            .map(Some)
            .map_err(|_| refused(t(lang, "manufacturing.bad_input"))),
    }
}

/// A quantity field in thousandths; `None` when empty, a 422 when it is not a
/// quantity.
fn quantity(lang: &Lang, form: &BTreeMap<String, String>, name: &str) -> Result<Option<i64>> {
    match field(form, name).as_str() {
        "" => Ok(None),
        value => parse_quantity(value)
            .map(Some)
            .ok_or_else(|| refused(t(lang, "manufacturing.bad_input"))),
    }
}

/// A date field; `None` when empty, a 422 when it is not a date.
fn date(lang: &Lang, form: &BTreeMap<String, String>, name: &str) -> Result<Option<NaiveDate>> {
    match field(form, name).as_str() {
        "" => Ok(None),
        value => value
            .parse::<NaiveDate>()
            .map(Some)
            .map_err(|_| refused(t(lang, "manufacturing.bad_input"))),
    }
}

/// `POST /manufacturing/work-orders`: `plant_id`, `line_id`, `item_id`,
/// `bom_id`, `quantity`, `planned_start`, `planned_end`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let plant_id = number(&lang, &form, "plant_id")?.unwrap_or_default();
    let item_id = number(&lang, &form, "item_id")?.unwrap_or_default();
    let quantity_milli = quantity(&lang, &form, "quantity")?.unwrap_or_default();
    let order = work_orders::create(
        &state,
        &user,
        NewWorkOrder {
            plant_id,
            line_id: number(&lang, &form, "line_id")?,
            item_id,
            bom_id: number(&lang, &form, "bom_id")?,
            quantity_milli,
            planned_start: date(&lang, &form, "planned_start")?,
            planned_end: date(&lang, &form, "planned_end")?,
            sales_order: None,
        },
    )
    .await?;
    Ok(done(
        &lang,
        &work_order_page::url(order.id),
        "manufacturing.screen.done.created",
    ))
}

macro_rules! simple_action {
    ($name:ident, $service:path, $key:literal) => {
        async fn $name(
            State(state): State<AppState>,
            user: AuthUser,
            lang: Lang,
            Path(id): Path<i64>,
        ) -> Result<Response> {
            $service(&state, &user, id).await?;
            Ok(done(&lang, &work_order_page::url(id), $key))
        }
    };
}

simple_action!(
    release,
    work_orders::release,
    "manufacturing.screen.done.released"
);
simple_action!(
    start,
    work_orders::start,
    "manufacturing.screen.done.started"
);
simple_action!(
    resume,
    work_orders::resume,
    "manufacturing.screen.done.resumed"
);
simple_action!(
    complete,
    work_orders::complete,
    "manufacturing.screen.done.completed"
);
simple_action!(
    close,
    work_orders::close,
    "manufacturing.screen.done.closed"
);

/// `POST /manufacturing/work-orders/{id}/hold`: `reason`.
async fn hold(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    work_orders::hold(&state, &user, id, &field(&form, "reason")).await?;
    Ok(done(
        &lang,
        &work_order_page::url(id),
        "manufacturing.screen.done.held",
    ))
}

/// `POST /manufacturing/work-orders/{id}/cancel`: `reason`.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    work_orders::cancel(&state, &user, id, &field(&form, "reason")).await?;
    Ok(done(
        &lang,
        &work_order_page::url(id),
        "manufacturing.screen.done.cancelled",
    ))
}

/// `POST /manufacturing/work-orders/{id}/unlink`: `reason`.
async fn unlink_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    unlink::unlink(&state, &user, id, &field(&form, "reason")).await?;
    Ok(done(
        &lang,
        &work_order_page::url(id),
        "manufacturing.screen.done.unlinked",
    ))
}

/// `POST /manufacturing/work-orders/{id}/abort`: `reason` and one
/// `return_<item>_<lot>` quantity per material that goes back (lot 0 is none).
async fn abort_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let mut returns = Vec::new();
    for (name, value) in &form {
        let Some(rest) = name.strip_prefix("return_") else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        let (item, lot) = rest
            .split_once('_')
            .and_then(|(item, lot)| Some((item.parse::<i64>().ok()?, lot.parse::<i64>().ok()?)))
            .ok_or_else(|| refused(t(&lang, "manufacturing.bad_input")))?;
        let quantity_milli =
            parse_quantity(value).ok_or_else(|| refused(t(&lang, "manufacturing.bad_input")))?;
        if quantity_milli == 0 {
            continue;
        }
        returns.push(ReturnLine {
            item_id: item,
            lot_id: (lot != 0).then_some(lot),
            quantity_milli,
        });
    }
    abort::request(
        &state,
        &user,
        id,
        AbortAsk {
            reason: field(&form, "reason"),
            returns,
        },
    )
    .await?;
    Ok(done(
        &lang,
        &work_order_page::url(id),
        "manufacturing.screen.done.abort_requested",
    ))
}

/// `POST /manufacturing/work-orders/{id}/issue`: `item_id`, `quantity`, `lot`
/// (the number printed on the pack) or `lot_id`.
async fn issue_material(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let item_id = number(&lang, &form, "item_id")?
        .ok_or_else(|| refused(english("manufacturing.issue.item", &[])))?;
    let quantity_milli = quantity(&lang, &form, "quantity")?.unwrap_or_default();
    issue::issue(
        &state,
        &user,
        id,
        IssueAsk {
            item_id,
            quantity_milli,
            lot_id: number(&lang, &form, "lot_id")?,
            lot_number: Some(field(&form, "lot")),
        },
    )
    .await?;
    Ok(done(
        &lang,
        &work_order_page::url(id),
        "manufacturing.screen.done.issued",
    ))
}

/// `POST /manufacturing/work-orders/{id}/report`: `good`, `scrap`, `note`.
async fn report_output(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    report::record(
        &state,
        &user,
        id,
        ReportAsk {
            good_milli: quantity(&lang, &form, "good")?.unwrap_or_default(),
            scrap_milli: quantity(&lang, &form, "scrap")?.unwrap_or_default(),
            note: Some(field(&form, "note")),
        },
    )
    .await?;
    Ok(done(
        &lang,
        &work_order_page::url(id),
        "manufacturing.screen.done.reported",
    ))
}

/// `POST /manufacturing/reports/{id}/void`: `reason`. The void waits for the
/// approval of the line's supervisor or the production manager.
async fn void_report(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let found = report::ProductionReport::find_or_404(&state.db, id).await?;
    let outcome = report::void(&state, &user, id, &field(&form, "reason")).await?;
    let key = match outcome {
        wl_kernel::voids::VoidOutcome::Pending(_) => "manufacturing.screen.done.void_requested",
        wl_kernel::voids::VoidOutcome::Voided(_) => "manufacturing.screen.done.voided",
    };
    Ok(done(&lang, &work_order_page::url(found.work_order_id), key))
}

/// The cause named by a form field.
fn cause_of(lang: &Lang, text: &str) -> Result<Cause> {
    match text {
        "supplier" => Ok(Cause::Supplier),
        "internal_process" => Ok(Cause::InternalProcess),
        "unknown" => Ok(Cause::Unknown),
        _ => Err(refused(t(lang, "manufacturing.bad_input"))),
    }
}

/// `POST /manufacturing/ncrs`: `plant_id`, `lot_id` or `item_id`, `quantity`,
/// `defect`, `cause` (empty: the default of the lot).
async fn raise_ncr(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let lot = number(&lang, &form, "lot_id")?
        .map(|id| Subject::new(wl_kernel::inventory::LOT_SUBJECT_TYPE, id));
    let item = number(&lang, &form, "item_id")?
        .map(|id| Subject::new(wl_kernel::inventory::ITEM_SUBJECT_TYPE, id));
    let cause = match field(&form, "cause").as_str() {
        "" => None,
        text => Some(cause_of(&lang, text)?),
    };
    let ncr = ncrs::raise(
        &state,
        &user,
        NewNcr {
            plant_id: number(&lang, &form, "plant_id")?.unwrap_or_default(),
            lot,
            item,
            quantity_milli: quantity(&lang, &form, "quantity")?.unwrap_or_default(),
            defect: field(&form, "defect"),
            cause,
            origin: Origin::Incoming,
            source: None,
        },
    )
    .await?;
    Ok(done(
        &lang,
        &screens::ncr_page::url(ncr.id),
        "manufacturing.screen.done.ncr_raised",
    ))
}

/// `POST /manufacturing/ncrs/{id}/disposition`: `disposition` (`rework`,
/// `scrap`, `use_as_is`, `return_to_supplier`), `outcome`, `note`, `line_id`,
/// `bom_id`.
async fn disposition_ncr(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let disposition = match field(&form, "disposition").as_str() {
        "rework" => Disposition::Rework,
        "scrap" => Disposition::Scrap,
        "use_as_is" => Disposition::UseAsIs,
        "return_to_supplier" => Disposition::ReturnToSupplier,
        _ => return Err(refused(english("manufacturing.ncr.choose", &[]))),
    };
    let made = ncrs::disposition(
        &state,
        &user,
        id,
        DispositionAsk {
            disposition,
            outcome: Some(field(&form, "outcome")),
            note: Some(field(&form, "note")),
            line_id: number(&lang, &form, "line_id")?,
            bom_id: number(&lang, &form, "bom_id")?,
        },
    )
    .await?;
    let key = if made.approval_request.is_some() {
        "manufacturing.screen.done.ncr_waiting"
    } else {
        "manufacturing.screen.done.ncr_dispositioned"
    };
    Ok(done(&lang, &screens::ncr_page::url(id), key))
}

/// `POST /manufacturing/ncrs/{id}/close`.
async fn close_ncr(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    ncrs::close(&state, &user, id).await?;
    Ok(done(
        &lang,
        &screens::ncr_page::url(id),
        "manufacturing.screen.done.ncr_closed",
    ))
}

/// `POST /manufacturing/ncrs/{id}/reopen`: `reason`.
async fn reopen_ncr(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    ncrs::reopen(&state, &user, id, &field(&form, "reason")).await?;
    Ok(done(
        &lang,
        &screens::ncr_page::url(id),
        "manufacturing.screen.done.ncr_reopened",
    ))
}

/// `POST /manufacturing/ncrs/{id}/cancel`: `reason`.
async fn cancel_ncr(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    ncrs::cancel(&state, &user, id, &field(&form, "reason")).await?;
    Ok(done(
        &lang,
        &screens::ncr_page::url(id),
        "manufacturing.screen.done.ncr_cancelled",
    ))
}

/// `POST /manufacturing/ncrs/{id}/cause`: `cause`, `reason`.
async fn cause_ncr(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let cause = cause_of(&lang, &field(&form, "cause"))?;
    ncrs::change_cause(&state, &user, id, cause, &field(&form, "reason")).await?;
    Ok(done(
        &lang,
        &screens::ncr_page::url(id),
        "manufacturing.screen.done.ncr_cause",
    ))
}
