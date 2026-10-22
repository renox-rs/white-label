//! A recalled ingredient lot traced to orders and customers (06 F&B §4.4 row
//! *Supplier or authority recalls an ingredient lot*, §7.2 *Recall owner*, §9
//! *Recalls*; BL-FNB-13).
//!
//! The recall is wl-inventory's: the head chef or the owner opens it, every
//! lot in scope is held at every outlet and in transit, and its trace follows
//! the transfers, the production batches (the batches made from the lot are
//! held too, [`crate::batches`]) and the orders. Three things are the
//! kitchen's:
//!
//! - [`OrderParties`], the app's `TracePartyResolver`: an order
//!   (`fnb.order:N`, the source of the recipe consumption of
//!   [`crate::consumption`]) is its customer, or, for an aggregator's order and
//!   a sale at the counter, nobody known ([`TraceParty::WalkIn`]): those are
//!   counted per outlet and day.
//! - [`trace`], the list the head chef and the owner read: one line per
//!   customer of a direct order with their **possible** orders (sales consume
//!   ingredients by recipe without recording the lot, so an order is linked to
//!   a lot only as *possible*, never *confirmed*), and one line per outlet and
//!   day for the sales without a known customer, with their channels.
//! - [`notify`], the owner's or the head chef's: one notice per customer by SMS
//!   or e-mail (English and Arabic) listing all their possible orders, through
//!   the kernel's notifier; it tells the recall which lines it told
//!   (`LotTrace::set_notice`). A customer whose account is closed has no
//!   contact: the lines are waived with that reason.
//! - [`decide_public`], the owner's decision on a public notice and on telling
//!   the aggregators. The task for it opens when a trace finds sales without a
//!   known customer (`RecallTraced`); recording the decision closes the task and
//!   either marks those lines told (a public notice) or waives them with the
//!   decision. The aggregators' customers are never messaged directly: we have
//!   no contact data for them.
//!
//! Decisions: the notice to a customer is the kitchen's own text (no product,
//! no lot): it says a supplier recalled an ingredient and lists the orders; the
//! authority's notification is open item AU-20 and is not built: the recall
//! report of wl-inventory is exported, and the setting
//! [`authority_notice_task`](crate::settings::FnbSettings::authority_notice_task)
//! gives the owner a task to do it by hand when the client wants one.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_date;
use wl_kernel::inventory::{
    RECALL_SUBJECT_TYPE, RecallTraced, TraceFuture, TraceMark, TraceParty, TracePartyResolver,
    TracedHolder, lot_trace, provide_trace_party_resolver,
};
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest, Urgency,
};
use wl_kernel::parties::{PartyStatus, directory};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::sales::order_desk;
use wl_kernel::tickets::task_creator;
use wl_kernel::{ClientConfig, PartyId, Subject};

use crate::catalogue::Outlet;
use crate::order::{FnbOrder, SUBJECT_TYPE};
use crate::permissions;
use crate::placing::refused;
use crate::settings::settings;
use crate::texts::english;

/// The notification type of the notice to a customer.
pub const NOTICE_TYPE: &str = "fnb.recall_notice";

/// The subject type of a notice sent to a customer.
pub const NOTICE_SUBJECT_TYPE: &str = "fnb.recall_notice";

/// What the owner decides about the sales without a known customer.
pub mod public_decision {
    /// A public notice is published and the aggregators are told.
    pub const NOTICE: &str = "public_notice";
    /// No public notice: the lines are waived with the decision.
    pub const NONE: &str = "no_public_notice";
    /// Both.
    pub const ALL: [&str; 2] = [NOTICE, NONE];
}

/// A notice sent to one customer of a recall: all their possible orders in
/// one message.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_recall_notices", index(recall_id), index(party_id))]
pub struct RecallNotice {
    /// The primary key.
    pub id: i64,
    /// The recall (`inventory.recall`).
    pub recall_id: i64,
    /// The customer told.
    pub party_id: i64,
    /// The orders the notice lists, comma separated.
    pub order_numbers: String,
    /// How many recall lines it covers.
    pub line_count: i64,
    /// The user who sent it.
    pub sent_by: Option<i64>,
    /// When it was sent.
    pub sent_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RecallNotice {
    /// The notice as a [`Subject`]: `fnb.recall_notice:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(NOTICE_SUBJECT_TYPE, self.id)
    }

    /// The notices of `recall_id`, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_recall(db: &Db, recall_id: i64) -> Result<Vec<RecallNotice>> {
        RecallNotice::query()
            .where_(RecallNotice::RECALL_ID.eq(recall_id))
            .order_by(RecallNotice::ID)
            .get(db)
            .await
    }
}

/// The owner's decision on a public notice and the aggregators, one per
/// recall.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_recall_decisions", unique(recall_id))]
pub struct RecallDecision {
    /// The primary key.
    pub id: i64,
    /// The recall (`inventory.recall`).
    pub recall_id: i64,
    /// `public_notice` or `no_public_notice`, one of [`public_decision`].
    pub decision: String,
    /// Whether the aggregators were told.
    pub aggregators_told: bool,
    /// What the owner wrote.
    pub note: String,
    /// The user who decided.
    pub decided_by: i64,
    /// When.
    pub decided_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RecallDecision {
    /// The decision of `recall_id`, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_recall(db: &Db, recall_id: i64) -> Result<Option<RecallDecision>> {
        RecallDecision::query()
            .where_(RecallDecision::RECALL_ID.eq(recall_id))
            .first(db)
            .await
    }
}

/// Whether the customer of `order` is unknown: an aggregator's order, or a sale
/// through a channel without a customer (the counter).
async fn is_walk_in(state: &AppState, order: &FnbOrder) -> Result<bool> {
    if settings(state)
        .walk_in_channels
        .iter()
        .any(|channel| *channel == order.channel)
    {
        return Ok(true);
    }
    Ok(order_desk(state)
        .find(state, order.sales_order())
        .await?
        .is_some_and(|view| view.aggregator))
}

/// The app's `TracePartyResolver`: an order is its customer, or nobody known
/// for an aggregator's order and a counter sale.
#[derive(Debug, Clone, Copy, Default)]
pub struct OrderParties;

impl TracePartyResolver for OrderParties {
    fn party<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> TraceFuture<'a, TraceParty> {
        Box::pin(async move {
            if subject.subject_type != SUBJECT_TYPE {
                return Ok(TraceParty::Unknown);
            }
            let Some(order) = FnbOrder::find(&state.db, subject.subject_id).await? else {
                return Ok(TraceParty::Unknown);
            };
            if is_walk_in(state, &order).await? {
                Ok(TraceParty::WalkIn)
            } else {
                Ok(TraceParty::Party(PartyId(order.party_id)))
            }
        })
    }
}

/// One possible order of a customer in the trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracedOrder {
    /// The recall line, `inventory.recall_line:7`.
    pub line: Subject,
    /// The order.
    pub order_id: i64,
    /// Its number.
    pub number: String,
    /// The outlet's name.
    pub outlet: String,
    /// The day of the order (Asia/Dubai).
    pub day: Option<NaiveDate>,
    /// How sure the trace is: never confirmed for a recipe.
    pub mark: TraceMark,
    /// `pending`, `sent` or `waived`.
    pub notice: String,
}

/// A direct-order customer with their possible orders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerGroup {
    /// The customer.
    pub party: i64,
    /// Their name.
    pub name: String,
    /// Their possible orders, oldest first.
    pub orders: Vec<TracedOrder>,
    /// Whether the account is closed: there is no contact to tell.
    pub no_contact: bool,
}

impl CustomerGroup {
    /// The lines still to tell.
    pub fn pending(&self) -> Vec<&TracedOrder> {
        self.orders
            .iter()
            .filter(|order| order.notice == "pending")
            .collect()
    }

    /// `pending` while a line waits, else `sent` when one was told, else
    /// `waived`.
    pub fn notice(&self) -> &'static str {
        if self.orders.iter().any(|o| o.notice == "pending") {
            "pending"
        } else if self.orders.iter().any(|o| o.notice == "sent") {
            "sent"
        } else {
            "waived"
        }
    }
}

/// The sales of one outlet on one day without a known customer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownGroup {
    /// The outlet.
    pub outlet_id: Option<i64>,
    /// The outlet's name.
    pub outlet: String,
    /// The day (Asia/Dubai).
    pub day: NaiveDate,
    /// The sales counted.
    pub sales: i64,
    /// The channels of the outlet's sales that day, `aggregator_a (2)`.
    pub channels: String,
    /// The recall lines of the group.
    pub lines: Vec<Subject>,
    /// `pending`, `sent` or `waived`.
    pub notice: String,
}

/// What a trace shows.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    /// The customers of direct orders.
    pub customers: Vec<CustomerGroup>,
    /// The sales without a known customer, per outlet and day.
    pub unknown: Vec<UnknownGroup>,
    /// The owner's decision on the public notice, once taken.
    pub decision: Option<RecallDecision>,
    /// The notices sent to customers.
    pub notices: Vec<RecallNotice>,
}

fn recall_subject(recall_id: i64) -> Subject {
    Subject::new(RECALL_SUBJECT_TYPE, recall_id)
}

/// The outlet whose stock location is `location_id`, with its name.
async fn outlet_of(db: &Db, location_id: Option<i64>) -> Result<(Option<i64>, String)> {
    let Some(location_id) = location_id else {
        return Ok((None, String::new()));
    };
    Ok(
        match Outlet::query()
            .where_(Outlet::LOCATION_ID.eq(Some(location_id)))
            .first(db)
            .await?
        {
            Some(outlet) => (Some(outlet.id), outlet.name),
            None => (None, format!("#{location_id}")),
        },
    )
}

/// The channels of the sales without a known customer at `outlet_id` on `day`:
/// `aggregator_a (2), pos (1)`.
async fn channels_of(state: &AppState, outlet_id: Option<i64>, day: NaiveDate) -> Result<String> {
    let Some(outlet_id) = outlet_id else {
        return Ok(String::new());
    };
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for order in FnbOrder::query()
        .where_(FnbOrder::OUTLET_ID.eq(Some(outlet_id)))
        .get(&state.db)
        .await?
    {
        if order.created_at.map(dubai_date) == Some(day) && is_walk_in(state, &order).await? {
            *counts.entry(order.channel.clone()).or_insert(0) += 1;
        }
    }
    Ok(counts
        .into_iter()
        .map(|(channel, count)| format!("{channel} ({count})"))
        .collect::<Vec<_>>()
        .join(", "))
}

/// The trace of recall `recall_id`: the customers with their possible orders
/// and the groups without a known customer. Orders appear once, whatever
/// number of lots linked them.
///
/// # Errors
///
/// Database errors and the stock and party modules'.
pub async fn trace(state: &AppState, recall_id: i64) -> Result<Trace> {
    let holders = lot_trace(state)
        .holders(state, &recall_subject(recall_id))
        .await?;
    let mut customers: BTreeMap<i64, CustomerGroup> = BTreeMap::new();
    let mut unknown: BTreeMap<(Option<i64>, NaiveDate), UnknownGroup> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for holder in &holders {
        match &holder.subject {
            None => {
                // A count of walk-in sales of one lot at one outlet on one day.
                let (outlet_id, outlet) =
                    outlet_of(&state.db, holder.location.as_ref().map(|l| l.subject_id)).await?;
                let Some(day) = holder.on else { continue };
                let group = unknown.entry((outlet_id, day)).or_insert_with(|| UnknownGroup {
                    outlet_id,
                    outlet,
                    day,
                    sales: 0,
                    channels: String::new(),
                    lines: Vec::new(),
                    notice: holder.notice.clone(),
                });
                group.sales = group.sales.max(holder.sales);
                group.lines.push(holder.line.clone());
                group.notice = merged_notice(&group.notice, &holder.notice);
            }
            Some(subject) if subject.subject_type == SUBJECT_TYPE => {
                let Some(party) = holder.party else { continue };
                if !seen.insert(holder.line.clone()) {
                    continue;
                }
                let Some(order) = FnbOrder::find(&state.db, subject.subject_id).await? else {
                    continue;
                };
                let (_, outlet) = outlet_of(
                    &state.db,
                    holder.location.as_ref().map(|l| l.subject_id),
                )
                .await?;
                if !customers.contains_key(&party.0) {
                    let name = directory(state)
                        .display_name(&state.db, party, "en")
                        .await?
                        .unwrap_or_default();
                    let status = directory(state).status(&state.db, party).await?;
                    customers.insert(
                        party.0,
                        CustomerGroup {
                            party: party.0,
                            name,
                            orders: Vec::new(),
                            no_contact: !matches!(
                                status,
                                Some(PartyStatus::Active | PartyStatus::Blocked)
                            ),
                        },
                    );
                }
                let Some(group) = customers.get_mut(&party.0) else {
                    continue;
                };
                group.orders.push(TracedOrder {
                    line: holder.line.clone(),
                    order_id: order.id,
                    number: order.number.clone(),
                    outlet,
                    day: holder.on.or_else(|| order.created_at.map(dubai_date)),
                    mark: holder.mark,
                    notice: holder.notice.clone(),
                });
            }
            Some(_) => {}
        }
    }
    let mut groups: Vec<UnknownGroup> = unknown.into_values().collect();
    for group in &mut groups {
        group.channels = channels_of(state, group.outlet_id, group.day).await?;
    }
    groups.sort_by(|a, b| (a.day, &a.outlet).cmp(&(b.day, &b.outlet)));
    let mut people: Vec<CustomerGroup> = customers.into_values().collect();
    for person in &mut people {
        person.orders.sort_by_key(|order| (order.day, order.order_id));
    }
    Ok(Trace {
        customers: people,
        unknown: groups,
        decision: RecallDecision::of_recall(&state.db, recall_id).await?,
        notices: RecallNotice::of_recall(&state.db, recall_id).await?,
    })
}

/// `pending` wins over `sent`, which wins over `waived`.
fn merged_notice(a: &str, b: &str) -> String {
    for state in ["pending", "sent"] {
        if a == state || b == state {
            return state.to_owned();
        }
    }
    "waived".to_owned()
}

fn require(user: &AuthUser, permission: &str) -> Result {
    if user.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// The company's name for the notice.
fn company(state: &AppState) -> String {
    state
        .provided::<Arc<ClientConfig>>()
        .map(|config| config.client.name.clone())
        .unwrap_or_default()
}

/// Tells the customers of recall `recall_id`: one notice for each customer with
/// lines still to tell, listing all their possible orders. A customer whose
/// account is closed has no contact: their lines are waived with that reason.
/// Returns the notices made.
///
/// # Errors
///
/// `Forbidden` without `fnb_recalls.decide`; 422 when no customer is left to
/// tell; the notifier's errors (a notice that failed is removed and its lines
/// stay `pending`).
pub async fn notify(state: &AppState, by: &AuthUser, recall_id: i64) -> Result<Vec<RecallNotice>> {
    require(by, permissions::RECALL_TRACE_DECIDE)?;
    let found = trace(state, recall_id).await?;
    let tracer = lot_trace(state);
    let mut made = Vec::new();
    let mut handled = 0;
    for group in found.customers.iter().filter(|g| !g.pending().is_empty()) {
        handled += 1;
        if group.no_contact {
            for order in group.pending() {
                tracer
                    .waive(
                        state,
                        &order.line,
                        &english("fnb.recall.waive_no_contact", &[]),
                        Some(by.id),
                    )
                    .await?;
            }
            continue;
        }
        let pending = group.pending();
        let numbers: Vec<String> = pending.iter().map(|o| o.number.clone()).collect();
        let days: BTreeSet<String> = pending
            .iter()
            .filter_map(|o| o.day.map(|d| d.to_string()))
            .collect();
        let mut notice = RecallNotice {
            recall_id,
            party_id: group.party,
            order_numbers: numbers.join(", "),
            line_count: i64::try_from(pending.len()).unwrap_or(0),
            sent_by: Some(by.id),
            sent_at: Some(renox::db::now()),
            ..RecallNotice::default()
        };
        notice.insert(&state.db).await?;
        let request = NotifyRequest::new(NOTICE_TYPE, notice.subject())
            .subject_label(&english("fnb.recall.label", &[]))
            .to_party(PartyId(group.party))
            .data(json!({
                "company": company(state),
                "orders": notice.order_numbers,
                "days": days.into_iter().collect::<Vec<_>>().join(", "),
            }));
        if let Err(error) = notify::notifier(state).notify(state, request).await {
            RecallNotice::query()
                .where_(RecallNotice::ID.eq(notice.id))
                .delete(&state.db)
                .await?;
            return Err(error);
        }
        for order in pending {
            tracer.set_notice(state, &order.line, true, Some(by.id)).await?;
        }
        renox::audit::record(
            &state.db,
            renox::audit::Entry::new("fnb.recall_notice_sent")
                .user(by.id)
                .subject(NOTICE_SUBJECT_TYPE, notice.id)
                .data(json!({ "recall": recall_id, "orders": notice.line_count })),
        )
        .await?;
        made.push(notice);
    }
    if handled == 0 {
        return Err(refused(english("fnb.recall.problem.nobody_to_tell", &[])));
    }
    Ok(made)
}

/// The key the owner's task about the public notice of a recall is found by.
pub fn public_task_key(recall_id: i64) -> String {
    format!("fnb.recall_public:{recall_id}")
}

/// The key of the owner's task to notify the authority (open item AU-20).
pub fn authority_task_key(recall_id: i64) -> String {
    format!("fnb.recall_authority:{recall_id}")
}

/// The owner decides whether a public notice is published and the aggregators
/// are told. Recording it closes the owner's task and settles the lines of the
/// sales without a known customer: a public notice marks them told, no public
/// notice waives them with the decision.
///
/// # Errors
///
/// `Forbidden` without `fnb_recalls.decide`; 422 for an unknown decision, for a
/// decision already taken or for *no public notice* without a note.
pub async fn decide_public(
    state: &AppState,
    by: &AuthUser,
    recall_id: i64,
    decision: &str,
    aggregators_told: bool,
    note: &str,
) -> Result<RecallDecision> {
    require(by, permissions::RECALL_TRACE_DECIDE)?;
    if !public_decision::ALL.contains(&decision) {
        return Err(refused(english("fnb.recall.problem.decision", &[])));
    }
    if RecallDecision::of_recall(&state.db, recall_id).await?.is_some() {
        return Err(refused(english("fnb.recall.problem.decided", &[])));
    }
    let note = note.trim().to_owned();
    if decision == public_decision::NONE && note.is_empty() {
        return Err(refused(english("fnb.recall.problem.note", &[])));
    }
    let tracer = lot_trace(state);
    let found = trace(state, recall_id).await?;
    for group in &found.unknown {
        for line in &group.lines {
            if decision == public_decision::NOTICE {
                tracer.set_notice(state, line, true, Some(by.id)).await?;
            } else {
                let reason = english("fnb.recall.waive_no_public_notice", &[("note", &note)]);
                tracer.waive(state, line, &reason, Some(by.id)).await?;
            }
        }
    }
    let mut row = RecallDecision {
        recall_id,
        decision: decision.to_owned(),
        aggregators_told: decision == public_decision::NOTICE && aggregators_told,
        note: note.clone(),
        decided_by: by.id,
        decided_at: renox::db::now(),
        ..RecallDecision::default()
    };
    row.insert(&state.db).await?;
    let tasks = task_creator(state);
    if let Some(open) = tasks.find_open(state, &public_task_key(recall_id)).await? {
        let closing = english(
            "fnb.recall.task_closed",
            &[("decision", &english(&format!("fnb.screen.recall.decision.{decision}"), &[]))],
        );
        tasks.complete(state, open.id, &closing).await?;
    }
    Ok(row)
}

/// `RecallTraced`: when the trace found sales without a known customer, the
/// owner gets the task to decide the public notice and tell the aggregators;
/// with the setting on, also the task to notify the authority (AU-20). One
/// task of each kind per recall, however often the trace runs again.
///
/// # Errors
///
/// Database errors and the ticket module's.
pub(crate) async fn on_traced(state: &AppState, event: RecallTraced) -> Result {
    let recall_id = event.recall.subject_id;
    let link = format!("/recalls/{recall_id}/orders");
    let unknown = event
        .lines
        .iter()
        .any(|line: &TracedHolder| line.subject.is_none() && line.notice == "pending");
    if unknown && RecallDecision::of_recall(&state.db, recall_id).await?.is_none() {
        let task = RoleTask::new(
            event.recall.clone(),
            permissions::OPERATIONS_DIRECTOR_ROLE,
            &english("fnb.recall.task_title", &[("number", &event.number)]),
            &public_task_key(recall_id),
        )
        .description(&english(
            "fnb.recall.task_body",
            &[("number", &event.number), ("link", &link)],
        ));
        role_tasks::open(state, task).await?;
    }
    if settings(state).authority_notice_task {
        let task = RoleTask::new(
            event.recall.clone(),
            permissions::OPERATIONS_DIRECTOR_ROLE,
            &english("fnb.recall.authority_title", &[("number", &event.number)]),
            &authority_task_key(recall_id),
        )
        .description(&english("fnb.recall.authority_body", &[("number", &event.number)]));
        role_tasks::open(state, task).await?;
    }
    Ok(())
}

/// The notification type of the notice to a customer: urgent, in English and
/// Arabic; it names the orders and the kitchen, never the product.
pub fn notice_types() -> Vec<NotificationTypeDef> {
    let subject = english("fnb.recall.subject", &[]);
    let body = english("fnb.recall.body", &[]);
    vec![
        // No group of the module's (06 §7.2): urgent, and a notice that gives
        // up gives the recall owner (the operations director) a task.
        NotificationTypeDef::new(NOTICE_TYPE, "fnb")
            .group("recalls")
            .category(Category::Transactional)
            .urgency(Urgency::Urgent)
            .give_up(&[GiveUpReaction::StaffTask])
            .staff_role(permissions::OPERATIONS_DIRECTOR_ROLE)
            .channels(&[NotifyChannel::Sms, NotifyChannel::Whatsapp, NotifyChannel::Email])
            .template(NotifyChannel::Email, "en", Some(&subject), &body)
            .template(
                NotifyChannel::Email,
                "ar",
                Some(SUBJECT_AR),
                BODY_AR,
            ),
    ]
}

/// The Arabic subject of the notice.
const SUBJECT_AR: &str = "إشعار سلامة من {{ company }}";

/// The Arabic body of the notice.
const BODY_AR: &str = "{{ company }}: قام أحد الموردين بسحب مكوّن استُخدم في وجباتنا. قد تتأثر طلباتكم {{ orders }}. يرجى عدم تناول أي طعام متبقٍ منها والتواصل معنا.";

/// Registers the resolver, the notification type and the listener of the
/// trace.
pub(crate) fn register(app: &mut Registry) {
    provide_trace_party_resolver(app, OrderParties);
    for def in notice_types() {
        notify::register_type(def);
    }
    app.listen(|event: RecallTraced, state: AppState| async move {
        on_traced(&state, event).await
    });
}
