//! Revising an issued purchase order (wl-inventory §4 *Purchase order*
//! `revise`, `STK.ACL.01`, `STK.ACL.03`, issue #227). Quantities, prices or
//! the expected date change after the order was sent; a holder of
//! `purchase_orders.issue` runs `revise` with a reason:
//!
//! - the guards: the order is `issued` or `partially_received`; a reason; the
//!   currency is unchanged; no line goes below what it has accepted; a later
//!   expected date says who asked for it (*us* or *the supplier*); something
//!   changed; a higher total or a later date passes
//!   `BlockGuard::check(supplier, extend)` (`PTY.ACL.03`);
//! - a higher total re-checks `STK.ACL.03` on the new total: above a limit
//!   (default AED 10,000 and 50,000) the user's `revise` becomes an approval
//!   request for as many steps as limits the new total is above, and the
//!   revision in force stays so until the last approval; a total that is not
//!   higher, or higher but below the first limit, applies at once. The
//!   requester cannot approve (the kernel's segregation of duties);
//! - the effect, in the transaction: the revision being replaced is kept
//!   read-only in [`PurchaseOrderRevision`] (its lines, total, date and
//!   document), the lines take the new quantity and price, the revision
//!   number goes up by one, and the document is rendered again, marked
//!   "Revision N". Committed cost ([`crate::purchase_order::committed_value`])
//!   follows the lines. Open receipts read the lines when they are posted, so
//!   they use the new ones;
//! - after the commit ([`crate::events`]): `PurchaseOrderRevised` and the
//!   notice to the supplier with the link to the document.
//!
//! Decisions:
//!
//! - A transition has one target state, so the spec's one `revise` from two
//!   states is two transitions, `revise` (`issued`) and `revise_part`
//!   (`partially_received`), each with its `_approved` twin. An order that
//!   left the state of the request while it waited (goods came in and it is
//!   `partially_received`, or it is `received`) has the request failed by the
//!   kernel ("this can't be done from the state ..."); the purchaser asks
//!   again.
//! - At the final approval [`recheck`] reads the order again: the revision
//!   must still be the one the request was made on, and no line may be below
//!   what it has accepted by then; otherwise the request is `failed`.
//! - A revision changes the quantity and the price of the lines the order has
//!   and the expected date; lines are not added or removed (a line goes to
//!   quantity 0 to drop it), and the currency cannot change.
//! - The limits are compared on the total in the order's currency. Converting
//!   a foreign currency to AED (`FIN.CUR.01`) is not done here.
//! - One revision waits at a time: while a request for the order is pending,
//!   a second `revise` is refused.
//! - The document is plain ASCII text rendered as a PDF like the return note,
//!   kept on the order and on the revision, and served by the document route.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{
    Machine, Outcome, StateMachine, Transition, TransitionCtx, TransitionError,
};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::block_guard::{ActionKind, GuardOutcome, guard};

use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel};

use crate::item::{Item, quantity_text, value_of};
use crate::permissions;
use crate::purchase_order::{
    PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus, lines_in, total_of,
};
use crate::return_note;
use crate::texts::{english, said};

/// The user's `revise` transition's name, for an `issued` order.
pub const REVISE: &str = "revise";

/// The user's `revise_part` transition's name, for a `partially_received`
/// order.
pub const REVISE_PART: &str = "revise_part";

/// The system's `revise_approved` transition's name: the kernel runs it when
/// the approval of a revision of an `issued` order is given.
pub const REVISE_APPROVED: &str = "revise_approved";

/// The system's `revise_part_approved` transition's name.
pub const REVISE_PART_APPROVED: &str = "revise_part_approved";

/// The four transitions of a revision.
pub const TRANSITIONS: [&str; 4] = [REVISE, REVISE_PART, REVISE_APPROVED, REVISE_PART_APPROVED];

/// The approval policy's action key of `revise` (`STK.ACL.03`).
pub const ACTION: &str = "inventory.purchase_order_revise";

/// The notification type of the notice to the supplier.
pub const NOTICE: &str = "inventory.purchase_order_revised";

/// The notification type of the notice to the supplier: transactional, by
/// e-mail, with the link to the document of the revision.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "inventory")
        .group("purchase_orders")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.revise.subject")),
            english("wl_inventory.revise.body"),
        )
}

/// Registers the notification type with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
}

/// Who asked for a later expected date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestedBy {
    /// We did.
    Us,
    /// The supplier did.
    Supplier,
}

impl RequestedBy {
    /// The stored key: `us` or `supplier`.
    pub fn as_str(self) -> &'static str {
        match self {
            RequestedBy::Us => "us",
            RequestedBy::Supplier => "supplier",
        }
    }

    /// The asker called `text`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "us" => Some(RequestedBy::Us),
            "supplier" => Some(RequestedBy::Supplier),
            _ => None,
        }
    }
}

/// An earlier revision of a purchase order, kept read-only: what the order
/// was before the revision that replaced it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_purchase_order_revisions",
    unique(purchase_order_id, revision)
)]
pub struct PurchaseOrderRevision {
    /// The primary key.
    pub id: i64,
    /// The order.
    pub purchase_order_id: i64,
    /// The revision number this row keeps.
    pub revision: i64,
    /// Why the revision was replaced.
    pub reason: String,
    /// The total of the revision, in minor units of the order's currency.
    pub total: i64,
    /// The expected date of the revision.
    pub expected_date: Option<NaiveDate>,
    /// Who asked for that date, if a revision set a later one.
    pub date_requested_by: Option<String>,
    /// The lines of the revision, as JSON ([`RevisionLine`]).
    pub lines: String,
    /// The PDF document of the revision.
    pub document: Option<String>,
    /// The user who replaced it.
    pub revised_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PurchaseOrderRevision {
    /// The lines the revision had.
    pub fn lines(&self) -> Vec<RevisionLine> {
        renox::serde_json::from_str(&self.lines).unwrap_or_default()
    }
}

/// One line of a kept revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionLine {
    /// The order line.
    pub po_line_id: i64,
    /// Its position.
    pub position: i64,
    /// The item.
    pub item_id: i64,
    /// What was ordered, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// The price of one base unit, in fils.
    pub unit_price: i64,
    /// What was accepted when the revision was replaced.
    pub accepted_qty_milli: i64,
}

impl RevisionLine {
    /// `line` as it is now.
    fn of(line: &PurchaseOrderLine) -> Self {
        RevisionLine {
            po_line_id: line.id,
            position: line.position,
            item_id: line.item_id,
            quantity_milli: line.quantity_milli,
            unit_price: line.unit_price,
            accepted_qty_milli: line.accepted_qty_milli,
        }
    }
}

/// The earlier revisions of order `purchase_order_id`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn revisions_of(db: &Db, purchase_order_id: i64) -> Result<Vec<PurchaseOrderRevision>> {
    PurchaseOrderRevision::query()
        .where_(PurchaseOrderRevision::PURCHASE_ORDER_ID.eq(purchase_order_id))
        .order_by(PurchaseOrderRevision::REVISION)
        .get(db)
        .await
}

/// The new quantity and price of one line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineChange {
    /// The order line.
    pub po_line_id: i64,
    /// The quantity it will have, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// The price of one base unit it will have, in fils.
    pub unit_price: i64,
}

/// What a `revise` run carries to its guards and effect: what the user asks
/// for, and what [`gather`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviseInput {
    /// Why the order is revised.
    pub reason: String,
    /// The lines that change.
    pub lines: Vec<LineChange>,
    /// The new expected date, if it changes.
    pub expected_date: Option<NaiveDate>,
    /// Who asked for a later date: `us` or `supplier`.
    pub date_requested_by: Option<String>,
    /// The currency the revision claims, if given; it must be the order's.
    pub currency: Option<String>,
    /// The user who revises (who asked, when it waited for approval).
    pub by: Option<i64>,
    /// The revision number the request was made on.
    pub base_revision: i64,
    /// The total in force, in minor units.
    pub old_total: i64,
    /// The total of the new revision.
    pub new_total: i64,
    /// The amount the approval policy's condition reads: the new total when
    /// it is higher than the old one, else 0 (`STK.ACL.03`).
    pub amount: i64,
    /// Whether the new total is higher.
    pub raises_total: bool,
    /// Whether the new expected date is later than the one in force.
    pub later_date: bool,
    /// The first thing wrong, `None` when nothing is.
    pub problem: Option<String>,
}

impl ReviseInput {
    /// A revision for `reason`, with no change yet.
    pub fn new(reason: &str) -> Self {
        ReviseInput {
            reason: reason.trim().to_owned(),
            ..ReviseInput::default()
        }
    }

    /// Line `po_line_id` gets `quantity_milli` and `unit_price`.
    pub fn line(mut self, po_line_id: i64, quantity_milli: i64, unit_price: i64) -> Self {
        self.lines.push(LineChange {
            po_line_id,
            quantity_milli,
            unit_price,
        });
        self
    }

    /// The expected date becomes `date`, asked for by `by`.
    pub fn expected(mut self, date: NaiveDate, by: RequestedBy) -> Self {
        self.expected_date = Some(date);
        self.date_requested_by = Some(by.as_str().to_owned());
        self
    }
}

/// Keeps `text` as the problem unless one is already there.
fn first(problem: &mut Option<String>, text: String) {
    if problem.is_none() {
        *problem = Some(text);
    }
}

/// Whether `change` is not what `line` already is.
fn differs(line: &PurchaseOrderLine, change: &LineChange) -> bool {
    line.quantity_milli != change.quantity_milli || line.unit_price != change.unit_price
}

/// What `asked` finds in order `order` read in `tx`: the totals, the amount
/// for the approval policy and the first problem.
///
/// # Errors
///
/// A database error.
pub async fn gather(
    tx: &mut Transaction,
    order: &PurchaseOrder,
    asked: &ReviseInput,
    by: Option<i64>,
) -> Result<ReviseInput> {
    let lines = lines_in(tx, order.id).await?;
    let mut input = asked.clone();
    input.by = by;
    input.base_revision = order.revision;
    input.old_total = total_of(&lines);
    input.problem = None;
    let mut problem: Option<String> = None;
    if !order.is_open_for_receiving() {
        first(
            &mut problem,
            said(
                "wl_inventory.revise.not_open",
                &[("status", &format!("{:?}", order.status))],
            ),
        );
    }
    if input.reason.trim().is_empty() {
        first(&mut problem, said("wl_inventory.revise.reason_needed", &[]));
    }
    if let Some(currency) = &input.currency
        && !currency.trim().eq_ignore_ascii_case(&order.currency)
    {
        first(&mut problem, said("wl_inventory.revise.currency", &[]));
    }
    let mut revised = lines.clone();
    let mut changed = false;
    for change in &input.lines {
        let Some(line) = revised.iter_mut().find(|l| l.id == change.po_line_id) else {
            first(
                &mut problem,
                said(
                    "wl_inventory.revise.unknown_line",
                    &[("line", &change.po_line_id.to_string())],
                ),
            );
            continue;
        };
        let position = line.position.to_string();
        if change.quantity_milli < 0 || change.unit_price < 0 {
            first(
                &mut problem,
                said("wl_inventory.revise.negative", &[("position", &position)]),
            );
        } else if change.quantity_milli < line.accepted_qty_milli {
            first(
                &mut problem,
                said(
                    "wl_inventory.revise.below_accepted",
                    &[
                        ("position", &position),
                        ("accepted", &quantity_text(line.accepted_qty_milli)),
                    ],
                ),
            );
        }
        changed |= differs(line, change);
        line.quantity_milli = change.quantity_milli;
        line.unit_price = change.unit_price;
    }
    input.new_total = total_of(&revised);
    input.raises_total = input.new_total > input.old_total;
    input.amount = if input.raises_total {
        input.new_total
    } else {
        0
    };
    input.later_date = input
        .expected_date
        .is_some_and(|date| order.expected_date.is_none_or(|current| date > current));
    if input.later_date
        && input
            .date_requested_by
            .as_deref()
            .and_then(RequestedBy::parse)
            .is_none()
    {
        first(&mut problem, said("wl_inventory.revise.date_asker", &[]));
    }
    changed |= input
        .expected_date
        .is_some_and(|date| Some(date) != order.expected_date);
    if !changed {
        first(&mut problem, said("wl_inventory.revise.nothing", &[]));
    }
    input.problem = problem;
    Ok(input)
}

fn input(ctx: &TransitionCtx) -> Option<ReviseInput> {
    ctx.input_as::<ReviseInput>().ok()
}

/// The guards and the effect the four transitions share, for an order in
/// `state`.
fn rules(
    transition: Transition<PurchaseOrder>,
    state: PurchaseOrderStatus,
) -> Transition<PurchaseOrder> {
    transition
        .from([state])
        .to(state)
        .guard("revision_allowed", |_order: &PurchaseOrder, ctx| {
            let problem = input(ctx).map_or_else(
                || Some(said("wl_inventory.revise.bad_input", &[])),
                |input| input.problem,
            );
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|order: &PurchaseOrder, ctx, tx| {
            let order = order.clone();
            let run = input(ctx).unwrap_or_default();
            Box::pin(async move { revise_in(tx, order, &run).await })
        })
}

/// The four transitions.
pub(crate) fn transitions() -> Vec<Transition<PurchaseOrder>> {
    use PurchaseOrderStatus::{Issued, PartiallyReceived};
    vec![
        rules(
            Transition::new(REVISE)
                .label("Revise")
                .tone("primary")
                .permission(permissions::PURCHASE_ORDERS_ISSUE),
            Issued,
        ),
        rules(
            Transition::new(REVISE_PART)
                .label("Revise")
                .tone("primary")
                .permission(permissions::PURCHASE_ORDERS_ISSUE),
            PartiallyReceived,
        ),
        rules(
            Transition::new(REVISE_APPROVED)
                .label("Revise (approved)")
                .system(),
            Issued,
        ),
        rules(
            Transition::new(REVISE_PART_APPROVED)
                .label("Revise (approved)")
                .system(),
            PartiallyReceived,
        ),
    ]
}

/// The machine of the order with the revise transitions.
pub(crate) fn add_to(machine: Machine<PurchaseOrder>) -> Machine<PurchaseOrder> {
    transitions().into_iter().fold(machine, Machine::transition)
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// A failure of the JSON of a kept revision.
fn internal(error: renox::serde_json::Error) -> Error {
    Error::Internal(renox::anyhow::Error::new(error))
}

/// The effect of the four transitions, in the run's transaction.
async fn revise_in(tx: &mut Transaction, order: PurchaseOrder, run: &ReviseInput) -> Result {
    if run.base_revision != 0 && run.base_revision != order.revision {
        return Err(refused(said("wl_inventory.revise.moved", &[])));
    }
    // Goods may have been accepted since the guards looked.
    let found = gather(tx, &order, run, run.by).await?;
    if let Some(problem) = found.problem {
        return Err(refused(problem));
    }
    let now = renox::db::now();
    let lines = lines_in(tx, order.id).await?;
    let kept: Vec<RevisionLine> = lines.iter().map(RevisionLine::of).collect();
    let mut earlier = PurchaseOrderRevision {
        purchase_order_id: order.id,
        revision: order.revision,
        reason: run.reason.trim().to_owned(),
        total: total_of(&lines),
        expected_date: order.expected_date,
        date_requested_by: order.date_requested_by.clone(),
        lines: renox::serde_json::to_string(&kept).map_err(internal)?,
        document: order.document.clone(),
        revised_by: run.by,
        ..PurchaseOrderRevision::default()
    };
    earlier.insert(&mut *tx).await?;
    for change in &run.lines {
        PurchaseOrderLine::query()
            .where_(PurchaseOrderLine::ID.eq(change.po_line_id))
            .update(
                &mut *tx,
                &[
                    ("quantity_milli", &change.quantity_milli),
                    ("unit_price", &change.unit_price),
                    ("updated_at", &now),
                ],
            )
            .await?;
    }
    let revision = order.revision + 1;
    let expected = run.expected_date.or(order.expected_date);
    let requested_by = if run.later_date {
        run.date_requested_by.clone()
    } else {
        order.date_requested_by.clone()
    };
    let fresh = lines_in(tx, order.id).await?;
    let mut sheet_lines = Vec::new();
    for line in &fresh {
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        sheet_lines.push(SheetLine {
            item: item.code.clone(),
            quantity_milli: line.quantity_milli,
            unit: item.base_unit.clone(),
            unit_price: line.unit_price,
        });
    }
    let document = render(&Sheet {
        number: order
            .number
            .clone()
            .unwrap_or_else(|| order.subject().to_string()),
        revision,
        supplier: format!("Supplier {}", order.supplier_party_id),
        currency: order.currency.clone(),
        expected: expected.map(|date| date.to_string()),
        reason: Some(run.reason.trim().to_owned()),
        lines: sheet_lines,
    });
    PurchaseOrder::query()
        .where_(PurchaseOrder::ID.eq(order.id))
        .update(
            &mut *tx,
            &[
                ("revision", &revision),
                ("expected_date", &expected),
                ("date_requested_by", &requested_by),
                ("document", &Some(document)),
                ("revision_issued_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// One line of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetLine {
    /// The item's code.
    pub item: String,
    /// The quantity, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// The base unit.
    pub unit: String,
    /// The price of one base unit, in fils.
    pub unit_price: i64,
}

/// What the document says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    /// The order's number.
    pub number: String,
    /// The revision, 1 for the first.
    pub revision: i64,
    /// The supplier, as `Supplier 7`.
    pub supplier: String,
    /// The currency.
    pub currency: String,
    /// The expected date.
    pub expected: Option<String>,
    /// Why this revision was made, for a revision.
    pub reason: Option<String>,
    /// The lines.
    pub lines: Vec<SheetLine>,
}

/// `fils` as `1,200.50`.
pub fn money_text(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let whole = (fils.abs() / 100).to_string();
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}{grouped}.{:02}", fils.abs() % 100)
}

/// The PDF document of `sheet`: marked "Revision N" from the second
/// revision on.
pub fn render(sheet: &Sheet) -> String {
    let title = if sheet.revision > 1 {
        format!(
            "Purchase order {} - Revision {}",
            sheet.number, sheet.revision
        )
    } else {
        format!("Purchase order {}", sheet.number)
    };
    let mut lines = vec![
        (16, title),
        (11, format!("Supplier: {}", sheet.supplier)),
        (11, format!("Currency: {}", sheet.currency)),
    ];
    if let Some(expected) = &sheet.expected {
        lines.push((11, format!("Expected: {expected}")));
    }
    if let Some(reason) = sheet.reason.as_deref().filter(|r| !r.is_empty()) {
        lines.push((11, format!("Reason for the revision: {reason}")));
    }
    lines.push((11, String::new()));
    lines.push((12, "Lines".to_owned()));
    let mut total = 0;
    for (position, line) in (1..).zip(&sheet.lines) {
        let value = value_of(line.quantity_milli, line.unit_price);
        total += value;
        lines.push((
            11,
            format!(
                "{position}. {}: {} {} at {} = {}",
                line.item,
                quantity_text(line.quantity_milli),
                line.unit,
                money_text(line.unit_price),
                money_text(value)
            ),
        ));
    }
    lines.push((11, String::new()));
    lines.push((
        12,
        format!("Total: {} {}", money_text(total), sheet.currency),
    ));
    return_note::document(&lines)
}

/// What [`revise`] did.
#[derive(Debug, Clone)]
pub enum ReviseOutcome {
    /// The new revision is in force.
    Revised(PurchaseOrder),
    /// The new total is above a limit: the order stays as it is and waits for
    /// the approvers (`STK.ACL.03`).
    Waiting(ApprovalRequest),
}

/// `user` revises order `order_id` as `asked`. Refused with the first
/// problem before anything is asked or written; a higher total or a later
/// date is also refused when the supplier is blocked; above a limit it
/// becomes an approval request instead (see the module docs).
///
/// # Errors
///
/// `Forbidden` without `purchase_orders.issue`; `Guard` (422) for a problem
/// of the revision; the neutral *on hold* refusal for a blocked supplier.
pub async fn revise(
    state: &AppState,
    user: &AuthUser,
    order_id: i64,
    asked: &ReviseInput,
) -> std::result::Result<ReviseOutcome, TransitionError> {
    let order = PurchaseOrder::find_or_404(&state.db, order_id).await?;
    let name = match order.status {
        PurchaseOrderStatus::Issued => REVISE,
        PurchaseOrderStatus::PartiallyReceived => REVISE_PART,
        other => {
            return Err(TransitionError::Guard(said(
                "wl_inventory.revise.not_open",
                &[("status", &format!("{other:?}"))],
            )));
        }
    };
    let waiting = ApprovalRequest::pending_about(&state.db, &order.subject()).await?;
    if waiting.iter().any(|request| request.action == ACTION) {
        return Err(TransitionError::Guard(said(
            "wl_inventory.revise.pending",
            &[],
        )));
    }
    // Reads in a transaction of its own, rolled back: nothing is held while
    // the transition opens its own.
    let mut tx = state.db.begin().await?;
    let found = gather(&mut tx, &order, asked, Some(user.id)).await?;
    let blocked = if found.problem.is_none() && (found.raises_total || found.later_date) {
        guard(state)
            .check_in(&mut tx, Some(user), order.supplier(), ActionKind::Extend)
            .await?
    } else {
        GuardOutcome::Allowed
    };
    tx.rollback().await?;
    if let Some(problem) = &found.problem {
        return Err(TransitionError::Guard(problem.clone()));
    }
    if let GuardOutcome::Blocked(refusal) = blocked {
        return Err(TransitionError::Failed(refusal.into_error()));
    }
    let outcome = order
        .transition(name)
        .by(user)
        .input(&found)
        .run(state)
        .await?;
    approvals::announce(state, &outcome).await?;
    match outcome {
        Outcome::Done(_) => {
            let done = PurchaseOrder::find_or_404(&state.db, order.id).await?;
            Ok(ReviseOutcome::Revised(done))
        }
        Outcome::Pending(_) => {
            let request = ApprovalRequest::pending_about(&state.db, &order.subject())
                .await?
                .into_iter()
                .find(|request| request.action == ACTION)
                .ok_or(Error::NotFound)?;
            Ok(ReviseOutcome::Waiting(request))
        }
    }
}

/// The final check of a request for a revision: before the approved
/// transition runs, the order must still be on the revision the request was
/// made on and no line may be below what it has accepted by now. `Some`
/// message fails the request (`approval failed: ...`).
///
/// # Errors
///
/// A database error.
pub async fn recheck(tx: &mut Transaction, request: &ApprovalRequest) -> Result<Option<String>> {
    if request.action != ACTION {
        // The block guard of `issue` is checked again at its final approval;
        // registering this check replaced the kernel's.
        return wl_kernel::block_guard::recheck(tx, request).await;
    }
    let asked: ReviseInput =
        renox::serde_json::from_value(request.payload.0.clone()).unwrap_or_default();
    let order = PurchaseOrder::find_or_404(&mut *tx, request.subject_id).await?;
    if asked.base_revision != order.revision {
        return Ok(Some(said("wl_inventory.revise.moved", &[])));
    }
    Ok(gather(tx, &order, &asked, asked.by).await?.problem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_grouped_in_thousands_with_two_decimals() {
        assert_eq!(money_text(0), "0.00");
        assert_eq!(money_text(5), "0.05");
        assert_eq!(money_text(120_000), "1,200.00");
        assert_eq!(money_text(123_456_789), "1,234,567.89");
        assert_eq!(money_text(-250), "-2.50");
    }

    fn sheet(revision: i64) -> Sheet {
        Sheet {
            number: "PO-2026-0003".into(),
            revision,
            supplier: "Supplier 7".into(),
            currency: "AED".into(),
            expected: Some("2026-11-15".into()),
            reason: Some("price rise".into()),
            lines: vec![SheetLine {
                item: "PIPE".into(),
                quantity_milli: 10_000,
                unit: "kg".into(),
                unit_price: 120_000,
            }],
        }
    }

    #[test]
    fn a_revision_is_marked_in_the_document_and_the_first_issue_is_not() {
        let revised = render(&sheet(2));
        assert!(revised.starts_with("%PDF-1.4\n"));
        assert!(revised.contains("Purchase order PO-2026-0003 - Revision 2"));
        assert!(revised.contains("1. PIPE: 10 kg at 1,200.00 = 12,000.00"));
        assert!(revised.contains("Total: 12,000.00 AED"));
        assert!(revised.contains("Reason for the revision: price rise"));
        let first = render(&sheet(1));
        assert!(!first.contains("Revision"));
    }

    #[test]
    fn the_four_transitions_stay_in_their_state_and_the_twins_are_for_the_system() {
        let machine = PurchaseOrder::machine();
        for (name, from, system) in [
            (REVISE, "issued", false),
            (REVISE_PART, "partially_received", false),
            (REVISE_APPROVED, "issued", true),
            (REVISE_PART_APPROVED, "partially_received", true),
        ] {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.is_system(), system, "{name}");
            assert_eq!(transition.from_states(), [from], "{name}");
            assert_eq!(transition.to_state(), Some(from), "{name}");
            assert_eq!(transition.guard_names(), ["revision_allowed"], "{name}");
        }
    }

    #[test]
    fn who_asked_is_us_or_the_supplier() {
        assert_eq!(RequestedBy::parse(" us "), Some(RequestedBy::Us));
        assert_eq!(RequestedBy::parse("supplier"), Some(RequestedBy::Supplier));
        assert_eq!(RequestedBy::parse("them"), None);
        assert_eq!(RequestedBy::Supplier.as_str(), "supplier");
    }

    #[test]
    fn a_revision_is_built_line_by_line() {
        let date = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        let input = ReviseInput::new("  price rise ")
            .line(11, 10_000, 120_000)
            .expected(date, RequestedBy::Supplier);
        assert_eq!(input.reason, "price rise");
        assert_eq!(input.lines.len(), 1);
        assert_eq!(input.date_requested_by.as_deref(), Some("supplier"));
    }

    #[test]
    fn a_line_differs_by_quantity_or_price() {
        let line = PurchaseOrderLine::new(1, 1, 2, 10_000, 90_000);
        let same = LineChange {
            po_line_id: 0,
            quantity_milli: 10_000,
            unit_price: 90_000,
        };
        assert!(!differs(&line, &same));
        assert!(differs(
            &line,
            &LineChange {
                unit_price: 120_000,
                ..same.clone()
            }
        ));
        assert!(differs(
            &line,
            &LineChange {
                quantity_milli: 9_000,
                ..same
            }
        ));
    }
}
