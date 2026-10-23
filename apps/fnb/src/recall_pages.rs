//! The recall trace screen (06 F&B §4.4 recall row, §7.2 *Recall owner*;
//! BL-FNB-13): the page the stock module's recall page links to for the
//! customers. It lists one line per direct-order customer with their possible
//! orders and one line per outlet and day for the sales without a known
//! customer (aggregators and the counter), the buttons that tell the customers
//! and record the owner's decision on a public notice, and the link to the
//! recall report that can be exported. The authority's notification is open
//! item AU-20 and is not built.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /recalls/{id}/orders` | `fnb.recall_trace` | `fnb_recalls.view` |
//! | `POST /recalls/{id}/orders/notify` | `fnb.recall_notify` | `fnb_recalls.decide` |
//! | `POST /recalls/{id}/orders/public-notice` | `fnb.recall_public` | `fnb_recalls.decide`, fields `decision`, `aggregators`, `note` |
//!
//! A line is never marked *confirmed*: the lots of a recipe are not recorded.

use renox::prelude::*;
use serde::Serialize;
use wl_kernel::inventory::TraceMark;

use crate::pages::{Fields, Texts, field, tx};
use crate::permissions;
use crate::recall::{self, public_decision};
use crate::settings::settings;
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[(
    "fnb/recall_trace.html",
    include_str!("../views/fnb/recall_trace.html"),
)];

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/recalls/{id}/orders", show)
        .name("fnb.recall_trace")
        .post("/recalls/{id}/orders/notify", notify)
        .name("fnb.recall_notify")
        .post("/recalls/{id}/orders/public-notice", public_notice)
        .name("fnb.recall_public")
        .require_auth()
}

fn notice_kind(notice: &str) -> &'static str {
    match notice {
        "sent" => "success",
        "waived" => "info",
        _ => "warning",
    }
}

fn notice_word(lang: &Lang, notice: &str) -> String {
    t(lang, &format!("fnb.screen.recall.notice.{notice}"))
}

/// One possible order of a customer.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct OrderRow {
    number: String,
    url: String,
    outlet: String,
    day: String,
    mark: String,
}

/// One customer of the trace.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct CustomerRow {
    name: String,
    count: usize,
    orders: Vec<OrderRow>,
    status: String,
    status_kind: &'static str,
    no_contact: bool,
}

/// The sales of one outlet on one day without a known customer.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct GroupRow {
    outlet: String,
    day: String,
    sales: i64,
    channels: String,
    status: String,
    status_kind: &'static str,
}

/// The owner's decision.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct DecisionRow {
    decision: String,
    aggregators_told: bool,
    note: String,
    decided_at: String,
}

/// A notice sent to a customer.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct NoticeRow {
    party: String,
    orders: String,
    sent_at: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/recall_trace.html")]
pub(crate) struct RecallTracePage {
    tx: Texts,
    recall_id: i64,
    customers: Vec<CustomerRow>,
    groups: Vec<GroupRow>,
    decision: Option<DecisionRow>,
    notices: Vec<NoticeRow>,
    decisions: Vec<[String; 2]>,
    notes: Vec<String>,
    can_decide: bool,
    can_notify: bool,
    can_decide_public: bool,
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::RECALL_TRACE_VIEW) {
        return Err(Error::Forbidden);
    }
    let found = recall::trace(&state, id).await?;
    let customers: Vec<CustomerRow> = found
        .customers
        .iter()
        .map(|group| CustomerRow {
            name: group.name.clone(),
            count: group.orders.len(),
            orders: group
                .orders
                .iter()
                .map(|order| OrderRow {
                    number: order.number.clone(),
                    url: format!("/orders/{}", order.order_id),
                    outlet: order.outlet.clone(),
                    day: order.day.map(|d| d.to_string()).unwrap_or_default(),
                    mark: t(
                        &lang,
                        match order.mark {
                            TraceMark::Confirmed => "fnb.screen.recall.mark.confirmed",
                            TraceMark::Uncertain => "fnb.screen.recall.mark.uncertain",
                            TraceMark::Possible => "fnb.screen.recall.mark.possible",
                        },
                    ),
                })
                .collect(),
            status: notice_word(&lang, group.notice()),
            status_kind: notice_kind(group.notice()),
            no_contact: group.no_contact,
        })
        .collect();
    let groups = found
        .unknown
        .iter()
        .map(|group| GroupRow {
            outlet: group.outlet.clone(),
            day: group.day.to_string(),
            sales: group.sales,
            channels: group.channels.clone(),
            status: notice_word(&lang, &group.notice),
            status_kind: notice_kind(&group.notice),
        })
        .collect();
    let decision = found.decision.as_ref().map(|d| DecisionRow {
        decision: t(&lang, &format!("fnb.screen.recall.decision.{}", d.decision)),
        aggregators_told: d.aggregators_told,
        note: d.note.clone(),
        decided_at: wl_kernel::documents::dubai_text(d.decided_at),
    });
    let notices = found
        .notices
        .iter()
        .map(|n| NoticeRow {
            party: found
                .customers
                .iter()
                .find(|c| c.party == n.party_id)
                .map(|c| c.name.clone())
                .unwrap_or_default(),
            orders: n.order_numbers.clone(),
            sent_at: n
                .sent_at
                .map(wl_kernel::documents::dubai_text)
                .unwrap_or_default(),
        })
        .collect();
    let decisions = public_decision::ALL
        .iter()
        .map(|d| {
            [
                (*d).to_owned(),
                t(&lang, &format!("fnb.screen.recall.decision.{d}")),
            ]
        })
        .collect();
    let mut notes = vec![t(&lang, "fnb.screen.recall.note_possible")];
    if settings(&state).authority_notice_task {
        notes.push(t(&lang, "fnb.screen.recall.note_authority_task"));
    } else {
        notes.push(t(&lang, "fnb.screen.recall.note_authority"));
    }
    let decide = user.has_permission_anywhere(permissions::RECALL_TRACE_DECIDE);
    Ok(View::typed(RecallTracePage {
        tx: tx(&lang),
        recall_id: id,
        can_notify: decide && found.customers.iter().any(|c| !c.pending().is_empty()),
        can_decide_public: decide && found.decision.is_none(),
        can_decide: decide,
        customers,
        groups,
        decision,
        notices,
        decisions,
        notes,
    })
    .into_response())
}

/// `POST /recalls/{id}/orders/notify`: one notice for each customer.
async fn notify(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    recall::notify(&state, &user, id).await?;
    Ok((
        Toast::success(t(&lang, "fnb.recall.notified")),
        Redirect::route("fnb.recall_trace", &[&id])?,
    )
        .into_response())
}

/// `POST /recalls/{id}/orders/public-notice`: the owner's decision.
async fn public_notice(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let told = form
        .get("aggregators")
        .is_some_and(|v| !v.trim().is_empty() && v != "0" && v != "false");
    recall::decide_public(
        &state,
        &user,
        id,
        &field(&form, "decision"),
        told,
        &field(&form, "note"),
    )
    .await?;
    Ok((
        Toast::success(t(&lang, "fnb.recall.decided")),
        Redirect::route("fnb.recall_trace", &[&id])?,
    )
        .into_response())
}
