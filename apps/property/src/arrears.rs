//! Escalating arrears to a formal demand and a legal case (`PRP.RUL.10`,
//! 01 Property §4.1 row "Arrears grow after the last reminder", §7.2
//! *Statutory notices*; BL-PRP-14).
//!
//! The daily run ([`run`], 02:00 Asia/Dubai, also the command [`TASK`]) works
//! per lease (all its versions together):
//!
//! 1. **Send a formal payment demand.** N days after an invoice's due date
//!    ([`DEMAND_DAYS`], 30), when its balance is above 0, it is not disputed
//!    (nor in the grace period after an upheld dispute) and the lease carries
//!    no legal case flag, the property manager gets **one** task per lease that
//!    lists every invoice of the lease overdue by N days or more. An invoice
//!    reaching N days while the task is open (the demand not sent yet) is added
//!    to it; one that is paid, disputed, credited or written off meanwhile is
//!    removed, and the task closes when nothing is left.
//! 2. **Sending** the demand ([`send_demand`]) is the property manager's, from
//!    the task: a statutory notice to the tenant
//!    ([`crate::notices::PAYMENT_DEMAND`]). It closes the task and freezes the
//!    list: the demand and its clock cover only the invoices it lists, and an
//!    invoice reaching N days afterwards opens a new demand (invoices on a sent
//!    demand are not listed again). The platform never sends a demand by
//!    itself.
//! 3. **The M-day clock** ([`LEGAL_DAYS`], 30) starts at the proof-of-delivery
//!    date of the demand ([`delivered_on`]), never at sending. M days after it,
//!    when any listed invoice still has a balance (partly paid counts), the
//!    property manager gets a second task: *open a legal case or agree a
//!    payment plan*. A demand whose message gave up with no proof records no
//!    date and makes no second task.
//! 4. **A payment plan** ([`record_plan`]) is a note with the agreed dates on
//!    the lease. The invoices are not changed; the rule stays quiet for the
//!    lease until the latest agreed date, and its second task is closed and
//!    made again after that date if the invoices are still unpaid. A legal
//!    case flag ([`crate::legal_case`]) set from the second task stops the
//!    rule for the lease.
//!
//! N and M are `⚠ open: lawyer (L-22)`: they are constants here, and the
//! defaults are built.
//!
//! Decided where the spec is silent:
//!
//! - A payment plan quiets the whole lease, not only the invoices overdue when
//!   it was agreed.
//! - An invoice's age is counted from the due date of the invoice, else of its
//!   charge.
//! - A lease that ended (terminated, renewed, amended) is escalated too: the
//!   arrears stay on their invoices.

use std::collections::BTreeSet;

use renox::chrono::{Duration, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, InvoiceSummary, invoicer};
use wl_kernel::notify::{NoticeStatus, StatutoryNoticeProofRecorded, notifier};
use wl_kernel::role_tasks::RoleTask;
use wl_kernel::tickets::{TaskComment, task_creator};

use crate::building::{clean, refused};
use crate::charge::{ChargeStatus, LeaseCharge};
use crate::lease::Lease;
use crate::legal_case::has_open;
use crate::notices::{self, PAYMENT_DEMAND};
use crate::permissions::{self, PROPERTY_MANAGER_ROLE};
use crate::texts::english;

/// The scheduled task and the command.
pub const TASK: &str = "property:arrears";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";

/// The rule.
pub const RULE: &str = "PRP.RUL.10";

/// N: the days after an invoice's due date before it is listed on a demand.
pub const DEMAND_DAYS: i64 = 30;

/// M: the days after the demand's proof of delivery before the legal-case
/// task.
pub const LEGAL_DAYS: i64 = 30;

/// The subject type of a demand.
pub const DEMAND_SUBJECT_TYPE: &str = "property.arrears_demand";

/// A formal payment demand: the invoices of a lease overdue by N days.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_arrears_demands", index(root_id, closed_on))]
pub struct ArrearsDemand {
    /// The primary key.
    pub id: i64,
    /// The id of version 1: the lease all versions belong to.
    pub root_id: i64,
    /// The version in force when the demand was made.
    #[model(references = "property_leases")]
    pub lease_id: i64,
    /// The day it was made.
    pub created_on: NaiveDate,
    /// The day the statutory notice was sent.
    pub sent_on: Option<NaiveDate>,
    /// Who sent it.
    pub sent_by: Option<i64>,
    /// The proof-of-delivery date: the M-day clock starts here.
    pub delivered_on: Option<NaiveDate>,
    /// The day the task to open a legal case or agree a payment plan was made.
    pub legal_task_on: Option<NaiveDate>,
    /// The day it needed nothing more.
    pub closed_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// An invoice listed on a demand.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "property_arrears_demand_invoices",
    index(demand_id),
    index(invoice_id)
)]
pub struct DemandInvoice {
    /// The primary key.
    pub id: i64,
    /// The demand.
    #[model(references = "property_arrears_demands")]
    pub demand_id: i64,
    /// The invoice.
    pub invoice_id: i64,
    /// Its number, as the notice names it.
    pub number: Option<String>,
    /// What was open on it when it was listed, in fils.
    pub open_amount: i64,
    /// Its due date.
    pub due_date: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A payment plan agreed with the tenant, kept as a note on the lease.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_payment_plans", index(root_id))]
pub struct PaymentPlan {
    /// The primary key.
    pub id: i64,
    /// The id of version 1: the lease all versions belong to.
    pub root_id: i64,
    /// The version the note was made on.
    #[model(references = "property_leases")]
    pub lease_id: i64,
    /// The agreed dates, as text.
    pub note: String,
    /// The latest agreed date.
    pub latest_on: NaiveDate,
    /// The day it was agreed.
    pub agreed_on: NaiveDate,
    /// Who recorded it.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ArrearsDemand {
    /// The demand as a kernel subject: what the statutory notice is about.
    pub fn subject(&self) -> Subject {
        Subject::new(DEMAND_SUBJECT_TYPE, self.id)
    }

    /// Whether the statutory notice was sent.
    pub fn is_sent(&self) -> bool {
        self.sent_on.is_some()
    }

    /// Whether the demand is waiting to be sent: not sent, not closed.
    pub fn is_open(&self) -> bool {
        self.sent_on.is_none() && self.closed_on.is_none()
    }

    /// The demands of the lease `root_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_root(db: &Db, root_id: i64) -> Result<Vec<ArrearsDemand>> {
        ArrearsDemand::query()
            .where_(ArrearsDemand::ROOT_ID.eq(root_id))
            .order_by(ArrearsDemand::ID)
            .get(db)
            .await
    }

    /// The demand of the lease `root_id` that is waiting to be sent.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open_of(db: &Db, root_id: i64) -> Result<Option<ArrearsDemand>> {
        ArrearsDemand::query()
            .where_(ArrearsDemand::ROOT_ID.eq(root_id))
            .where_(ArrearsDemand::SENT_ON.is_null())
            .where_(ArrearsDemand::CLOSED_ON.is_null())
            .order_by(ArrearsDemand::ID)
            .first(db)
            .await
    }

    /// The invoices listed on the demand, oldest due date first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn invoices(&self, db: &Db) -> Result<Vec<DemandInvoice>> {
        DemandInvoice::query()
            .where_(DemandInvoice::DEMAND_ID.eq(self.id))
            .order_by(DemandInvoice::DUE_DATE)
            .order_by(DemandInvoice::ID)
            .get(db)
            .await
    }
}

impl PaymentPlan {
    /// The latest agreed date of the plans of the lease `root_id`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn latest_of(db: &Db, root_id: i64) -> Result<Option<NaiveDate>> {
        Ok(PaymentPlan::query()
            .where_(PaymentPlan::ROOT_ID.eq(root_id))
            .get(db)
            .await?
            .iter()
            .map(|plan| plan.latest_on)
            .max())
    }
}

/// The key the task to send the demand `id` is found by while open.
pub fn demand_key(id: i64) -> String {
    format!("property.arrears_demand:{id}")
}

/// The key the task to open a legal case or agree a payment plan for the
/// demand `id` is found by while open.
pub fn legal_key(id: i64) -> String {
    format!("property.arrears_legal:{id}")
}

/// Whether an invoice in `status` can still be paid.
fn is_open(status: InvoiceStatus) -> bool {
    matches!(
        status,
        InvoiceStatus::Issued | InvoiceStatus::PartiallyPaid | InvoiceStatus::Overdue
    )
}

/// Whether an invoice due on `due` has been overdue for `days` days or more
/// on `day`.
pub fn overdue_by(due: NaiveDate, day: NaiveDate, days: i64) -> bool {
    due + Duration::days(days) <= day
}

/// An invoice of the lease that the rule looks at.
#[derive(Debug, Clone)]
struct Seen {
    id: i64,
    summary: InvoiceSummary,
    due: NaiveDate,
}

/// The invoices of every version of the lease `root_id` (one per charge that
/// was invoiced), with their summary.
async fn invoices_of(state: &AppState, root_id: i64) -> Result<Vec<Seen>> {
    let mut seen: Vec<Seen> = Vec::new();
    for version in Lease::versions_of(&state.db, root_id).await? {
        for charge in version.charges(&state.db).await? {
            let Some(id) = charge.invoice_id else {
                continue;
            };
            if charge.status != ChargeStatus::Invoiced || seen.iter().any(|s| s.id == id) {
                continue;
            }
            if let Some(summary) = invoicer(state).find(&state.db, InvoiceId(id)).await? {
                let due = summary.due_date.unwrap_or(charge.due_date);
                seen.push(Seen { id, summary, due });
            }
        }
    }
    // The damages invoice of a deposit applied to the deductions follows the
    // flow like a rent invoice (BL-PRP-17).
    if let Some(settlement) = crate::deposit_forfeit::DepositSettlement::of_root(&state.db, root_id).await?
        && let Some(id) = settlement.excess_invoice_id
        && !seen.iter().any(|s| s.id == id)
        && let Some(summary) = invoicer(state).find(&state.db, InvoiceId(id)).await?
    {
        let due = summary.due_date.unwrap_or(settlement.applied_on);
        seen.push(Seen { id, summary, due });
    }
    Ok(seen)
}

/// Whether `seen` is overdue by N days on `day`, is open with a balance, and
/// is not disputed.
fn eligible(seen: &Seen, day: NaiveDate) -> bool {
    is_open(seen.summary.status)
        && seen.summary.balance > 0
        && !seen.summary.chasing_paused(day)
        && overdue_by(seen.due, day, DEMAND_DAYS)
}

/// The invoice ids listed on a demand that was sent.
async fn on_sent_demands(db: &Db, root_id: i64) -> Result<BTreeSet<i64>> {
    let mut ids = BTreeSet::new();
    for demand in ArrearsDemand::of_root(db, root_id).await? {
        if demand.is_sent() {
            ids.extend(demand.invoices(db).await?.iter().map(|i| i.invoice_id));
        }
    }
    Ok(ids)
}

/// A line of the task and the notice: `INV-0001, due 2027-03-01, AED 12,000.00`.
fn describe(number: Option<&str>, id: i64, due: NaiveDate, open: i64) -> String {
    format!(
        "{}, due {due}, {}",
        number.map_or_else(|| format!("invoice {id}"), str::to_owned),
        crate::money_text(open)
    )
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Demands made (a task to send each).
    pub demands: usize,
    /// Invoices added to a demand that was waiting.
    pub added: usize,
    /// Invoices taken off a demand that was waiting.
    pub removed: usize,
    /// Demands closed because nothing was left on them.
    pub closed: usize,
    /// Tasks to open a legal case or agree a payment plan.
    pub legal_tasks: usize,
    /// Leases that failed; the run moves on.
    pub failed: usize,
}

impl Report {
    fn add(&mut self, other: Report) {
        self.demands += other.demands;
        self.added += other.added;
        self.removed += other.removed;
        self.closed += other.closed;
        self.legal_tasks += other.legal_tasks;
        self.failed += other.failed;
    }
}

/// Brings the demand waiting to be sent for lease `root_id` up to date on
/// `day`: it is made when there is an invoice to list, its list follows the
/// invoices that qualify, and it closes when none is left.
async fn sync_open(state: &AppState, lease: &Lease, day: NaiveDate) -> Result<Report> {
    let mut report = Report::default();
    let root_id = lease.root_id;
    let sent = on_sent_demands(&state.db, root_id).await?;
    let seen = invoices_of(state, root_id).await?;
    let wanted: Vec<&Seen> = seen
        .iter()
        .filter(|s| !sent.contains(&s.id) && eligible(s, day))
        .collect();
    let existing = ArrearsDemand::open_of(&state.db, root_id).await?;

    let demand = match existing {
        Some(demand) => demand,
        None if wanted.is_empty() => return Ok(report),
        None => {
            let demand = ArrearsDemand::create(
                &state.db,
                ArrearsDemand {
                    root_id,
                    lease_id: lease.id,
                    created_on: day,
                    ..Default::default()
                },
            )
            .await?;
            report.demands += 1;
            demand
        }
    };

    let listed = demand.invoices(&state.db).await?;
    let wanted_ids: BTreeSet<i64> = wanted.iter().map(|s| s.id).collect();
    let stale: Vec<i64> = listed
        .iter()
        .filter(|row| !wanted_ids.contains(&row.invoice_id))
        .map(|row| row.id)
        .collect();
    if !stale.is_empty() {
        report.removed += stale.len();
        DemandInvoice::query()
            .where_(DemandInvoice::ID.is_in(stale))
            .delete(&state.db)
            .await?;
    }
    let mut added = Vec::new();
    for seen in &wanted {
        if listed.iter().any(|row| row.invoice_id == seen.id) {
            continue;
        }
        DemandInvoice::create(
            &state.db,
            DemandInvoice {
                demand_id: demand.id,
                invoice_id: seen.id,
                number: seen.summary.number.clone(),
                open_amount: seen.summary.balance,
                due_date: seen.due,
                ..Default::default()
            },
        )
        .await?;
        added.push(describe(
            seen.summary.number.as_deref(),
            seen.id,
            seen.due,
            seen.summary.balance,
        ));
    }
    report.added += added.len();

    let creator = task_creator(state);
    if wanted.is_empty() {
        // Nothing is left: the task closes and so does the demand.
        if let Some(ticket) = creator.find_open(state, &demand_key(demand.id)).await? {
            let note = english("property.arrears.nothing_left", &[]);
            creator.complete(state, ticket.id, &note).await?;
        }
        ArrearsDemand::query()
            .where_(ArrearsDemand::ID.eq(demand.id))
            .update(
                &state.db,
                &[
                    ("closed_on", &Some(day)),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        report.closed += 1;
        return Ok(report);
    }

    let lines: Vec<String> = wanted
        .iter()
        .map(|s| describe(s.summary.number.as_deref(), s.id, s.due, s.summary.balance))
        .collect();
    let title = english(
        "property.arrears.demand_task",
        &[("lease", &lease.number())],
    );
    let description = english(
        "property.arrears.demand_task_description",
        &[("days", &DEMAND_DAYS), ("list", &lines.join("; "))],
    );
    let task = RoleTask::new(
        lease.subject(),
        PROPERTY_MANAGER_ROLE,
        &title,
        &demand_key(demand.id),
    )
    .description(&description);
    match creator.find_open(state, &demand_key(demand.id)).await? {
        Some(ticket) if !added.is_empty() => {
            let comment = english(
                "property.arrears.demand_added",
                &[("list", &added.join("; "))],
            );
            creator
                .comment(state, ticket.id, TaskComment::internal(&comment))
                .await?;
        }
        Some(_) => {}
        None => {
            crate::tasks::open(state, &lease.scope(), task).await?;
        }
    }
    Ok(report)
}

/// The day the demand `demand` was delivered, when it was: stored once known,
/// else asked from the notifier (the provider's receipt, or the proof staff
/// recorded when the message gave up). `None` while it is not delivered, so
/// the M-day clock never starts at sending.
///
/// # Errors
///
/// The notifier's and the database's errors.
pub async fn delivered_on(state: &AppState, demand: &ArrearsDemand) -> Result<Option<NaiveDate>> {
    if let Some(day) = demand.delivered_on {
        return Ok(Some(day));
    }
    if !demand.is_sent() {
        return Ok(None);
    }
    let status = notifier(state)
        .notice_status(state, &demand.subject(), PAYMENT_DEMAND)
        .await?;
    let day = match status {
        NoticeStatus::Delivered { at } => dubai_date(at),
        NoticeStatus::ProofRecorded { delivered_on } => delivered_on,
        NoticeStatus::NotDelivered => return Ok(None),
    };
    ArrearsDemand::query()
        .where_(ArrearsDemand::ID.eq(demand.id))
        .where_(ArrearsDemand::DELIVERED_ON.is_null())
        .update(
            &state.db,
            &[
                ("delivered_on", &Some(day)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(Some(day))
}

/// The proof of delivery of a demand was recorded: its date is stored.
///
/// # Errors
///
/// The notifier's and the database's errors.
pub(crate) async fn on_proof(state: &AppState, event: &StatutoryNoticeProofRecorded) -> Result {
    if event.type_key != PAYMENT_DEMAND || event.subject.subject_type != DEMAND_SUBJECT_TYPE {
        return Ok(());
    }
    if let Some(demand) = ArrearsDemand::find(&state.db, event.subject.subject_id).await? {
        delivered_on(state, &demand).await?;
    }
    Ok(())
}

/// The second step for the sent demands of `lease`: M days after the proof of
/// delivery, the property manager gets the task to open a legal case or agree
/// a payment plan, when a listed invoice still has a balance.
async fn escalate_sent(state: &AppState, lease: &Lease, day: NaiveDate) -> Result<Report> {
    let mut report = Report::default();
    for demand in ArrearsDemand::of_root(&state.db, lease.root_id).await? {
        if !demand.is_sent() || demand.legal_task_on.is_some() || demand.closed_on.is_some() {
            continue;
        }
        let Some(delivered) = delivered_on(state, &demand).await? else {
            continue;
        };
        if !overdue_by(delivered, day, LEGAL_DAYS) {
            continue;
        }
        let mut unpaid = Vec::new();
        for row in demand.invoices(&state.db).await? {
            let Some(summary) = invoicer(state)
                .find(&state.db, InvoiceId(row.invoice_id))
                .await?
            else {
                continue;
            };
            if is_open(summary.status) && summary.balance > 0 && !summary.chasing_paused(day) {
                unpaid.push(describe(
                    summary.number.as_deref(),
                    row.invoice_id,
                    row.due_date,
                    summary.balance,
                ));
            }
        }
        let column = if unpaid.is_empty() {
            "closed_on"
        } else {
            "legal_task_on"
        };
        let claimed = ArrearsDemand::query()
            .where_(ArrearsDemand::ID.eq(demand.id))
            .where_(ArrearsDemand::LEGAL_TASK_ON.is_null())
            .where_(ArrearsDemand::CLOSED_ON.is_null())
            .update(
                &state.db,
                &[(column, &Some(day)), ("updated_at", &renox::db::now())],
            )
            .await?;
        if claimed == 0 || unpaid.is_empty() {
            continue;
        }
        let title = english("property.arrears.legal_task", &[("lease", &lease.number())]);
        let description = english(
            "property.arrears.legal_task_description",
            &[("days", &LEGAL_DAYS), ("list", &unpaid.join("; "))],
        );
        let task = RoleTask::new(
            lease.subject(),
            PROPERTY_MANAGER_ROLE,
            &title,
            &legal_key(demand.id),
        )
        .description(&description);
        crate::tasks::open(state, &lease.scope(), task).await?;
        report.legal_tasks += 1;
    }
    Ok(report)
}

/// The rule for one lease on `day`.
async fn run_lease(state: &AppState, root_id: i64, day: NaiveDate) -> Result<Report> {
    let Some(lease) = Lease::versions_of(&state.db, root_id).await?.pop() else {
        return Ok(Report::default());
    };
    // A legal case flag stops the rule; a payment plan quiets it until its
    // latest agreed date.
    if has_open(&state.db, root_id).await? {
        return Ok(Report::default());
    }
    if PaymentPlan::latest_of(&state.db, root_id)
        .await?
        .is_some_and(|latest| day <= latest)
    {
        return Ok(Report::default());
    }
    let mut report = sync_open(state, &lease, day).await?;
    report.add(escalate_sent(state, &lease, day).await?);
    Ok(report)
}

/// `PRP.RUL.10` for `day`: the leases with an invoice overdue by N days or
/// more, and the leases with a demand waiting or sent, are looked at. Every
/// lease is tried; a failure is counted and the run moves on.
///
/// # Errors
///
/// The database's errors.
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let mut roots: BTreeSet<i64> = BTreeSet::new();
    let late = LeaseCharge::query()
        .where_(LeaseCharge::STATUS.eq(ChargeStatus::Invoiced))
        .where_(LeaseCharge::DUE_DATE.lte(day - Duration::days(DEMAND_DAYS)))
        .get(&state.db)
        .await?;
    for charge in late {
        if let Some(lease) = Lease::find(&state.db, charge.lease_id).await? {
            roots.insert(lease.root_id);
        }
    }
    // The damages invoice of a deposit applied to the deductions (BL-PRP-17).
    for settlement in crate::deposit_forfeit::DepositSettlement::query()
        .where_(crate::deposit_forfeit::DepositSettlement::EXCESS_INVOICE_ID.is_not_null())
        .where_(
            crate::deposit_forfeit::DepositSettlement::APPLIED_ON.lte(day - Duration::days(DEMAND_DAYS)),
        )
        .get(&state.db)
        .await?
    {
        roots.insert(settlement.root_id);
    }
    for demand in ArrearsDemand::query()
        .where_(ArrearsDemand::CLOSED_ON.is_null())
        .get(&state.db)
        .await?
    {
        roots.insert(demand.root_id);
    }
    let mut total = Report::default();
    for root in roots {
        match run_lease(state, root, day).await {
            Ok(report) => total.add(report),
            Err(error) => {
                total.failed += 1;
                eprintln!("{TASK}: lease {root}: {error:?}");
            }
        }
    }
    Ok(total)
}

/// Sends the formal payment demand `demand_id` of lease `lease_id` to the
/// tenant as a statutory notice, counting the overdue invoices on `day`, for
/// the user `by`. The list is brought up to date first; sending closes the
/// task and freezes the list. The caller has authorized the user.
///
/// # Errors
///
/// 422 for a demand that is sent or closed already, or when no invoice is
/// left on it.
pub async fn send_demand_as(
    state: &AppState,
    by: Option<i64>,
    lease_id: i64,
    demand_id: i64,
    day: NaiveDate,
) -> Result<ArrearsDemand> {
    let lease = Lease::find(&state.db, lease_id).await?.ok_or(Error::NotFound)?;
    let demand = ArrearsDemand::find(&state.db, demand_id)
        .await?
        .filter(|d| d.root_id == lease.root_id)
        .ok_or(Error::NotFound)?;
    if !demand.is_open() {
        return Err(refused(english("property.arrears.not_open", &[])));
    }
    sync_open(state, &lease, day).await?;
    let demand = ArrearsDemand::find(&state.db, demand_id)
        .await?
        .ok_or(Error::NotFound)?;
    let rows = demand.invoices(&state.db).await?;
    if !demand.is_open() || rows.is_empty() {
        return Err(refused(english("property.arrears.nothing_left", &[])));
    }
    // The statutory notice goes first; the claim follows only if it was sent.
    notices::payment_demand(state, &lease, &demand, &rows, LEGAL_DAYS).await?;
    let claimed = ArrearsDemand::query()
        .where_(ArrearsDemand::ID.eq(demand.id))
        .where_(ArrearsDemand::SENT_ON.is_null())
        .update(
            &state.db,
            &[
                ("sent_on", &Some(day)),
                ("sent_by", &by),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if claimed > 0 {
        let creator = task_creator(state);
        if let Some(ticket) = creator.find_open(state, &demand_key(demand.id)).await? {
            let note = english("property.arrears.demand_sent", &[]);
            creator.complete(state, ticket.id, &note).await?;
        }
    }
    ArrearsDemand::find(&state.db, demand.id)
        .await?
        .ok_or(Error::NotFound)
}

/// `user` sends the formal payment demand `demand_id` of lease `lease_id`
/// today ([`send_demand_as`]).
///
/// # Errors
///
/// 403 without `leases.manage` in the lease's building; as
/// [`send_demand_as`].
pub async fn send_demand(
    state: &AppState,
    user: &AuthUser,
    lease_id: i64,
    demand_id: i64,
) -> Result<ArrearsDemand> {
    let lease = Lease::find(&state.db, lease_id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    send_demand_as(state, Some(user.id), lease_id, demand_id, today()).await
}

/// What a user enters for a payment plan.
#[derive(Debug, Clone)]
pub struct NewPlan {
    /// The agreed dates, as text.
    pub note: String,
    /// The latest agreed date.
    pub latest_on: NaiveDate,
}

/// Records a payment plan agreed with the tenant of lease `lease_id` on
/// `day`, for the user `by`: a note with the agreed dates. The issued invoices
/// are not changed; the rule stays quiet until the latest agreed date; the task
/// to open a legal case or agree a payment plan is closed and made again after
/// that date if the invoices are unpaid. The caller has authorized the user.
///
/// # Errors
///
/// 422 for a blank note or a latest date before `day`.
pub async fn record_plan_as(
    state: &AppState,
    by: Option<i64>,
    lease_id: i64,
    plan: NewPlan,
    day: NaiveDate,
) -> Result<PaymentPlan> {
    let lease = Lease::find(&state.db, lease_id).await?.ok_or(Error::NotFound)?;
    let Some(note) = clean(Some(plan.note)) else {
        return Err(refused(english("property.arrears.plan_note_required", &[])));
    };
    if plan.latest_on < day {
        return Err(refused(english("property.arrears.plan_date_past", &[])));
    }
    let saved = PaymentPlan::create(
        &state.db,
        PaymentPlan {
            root_id: lease.root_id,
            lease_id: lease.id,
            note,
            latest_on: plan.latest_on,
            agreed_on: day,
            recorded_by: by,
            ..Default::default()
        },
    )
    .await?;
    let creator = task_creator(state);
    for demand in ArrearsDemand::of_root(&state.db, lease.root_id).await? {
        if demand.legal_task_on.is_none() || demand.closed_on.is_some() {
            continue;
        }
        if let Some(ticket) = creator.find_open(state, &legal_key(demand.id)).await? {
            let closing = english(
                "property.arrears.plan_agreed",
                &[("until", &plan.latest_on)],
            );
            creator.complete(state, ticket.id, &closing).await?;
        }
        // The rule looks at it again after the latest agreed date.
        ArrearsDemand::query()
            .where_(ArrearsDemand::ID.eq(demand.id))
            .update(
                &state.db,
                &[
                    ("legal_task_on", &None::<NaiveDate>),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    Ok(saved)
}

/// `user` records a payment plan today ([`record_plan_as`]).
///
/// # Errors
///
/// 403 without `leases.manage` in the lease's building; as
/// [`record_plan_as`].
pub async fn record_plan(
    state: &AppState,
    user: &AuthUser,
    lease_id: i64,
    plan: NewPlan,
) -> Result<PaymentPlan> {
    let lease = Lease::find(&state.db, lease_id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    record_plan_as(state, Some(user.id), lease_id, plan, today()).await
}

/// Registers the daily task, its command and the listener of the proof of
/// delivery.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Escalate arrears: formal payment demands and legal case tasks (--date YYYY-MM-DD checks another day)",
        |args, state: AppState| async move {
            let on = match args.value("--date") {
                None => today(),
                Some(text) => text.parse().map_err(|_| {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("--date must be YYYY-MM-DD, got `{text}`"),
                    )
                })?,
            };
            let report = run(&state, on).await?;
            println!(
                "{} demands made, {} invoices added, {} removed, {} closed, {} legal case tasks, {} failed.",
                report.demands,
                report.added,
                report.removed,
                report.closed,
                report.legal_tasks,
                report.failed
            );
            Ok(())
        },
    );
    app.listen(|event: StatutoryNoticeProofRecorded, state: AppState| async move {
        on_proof(&state, &event).await
    });
}

/// The data of the notice: what the demand lists and the demand period.
pub(crate) fn notice_data(
    lease: &Lease,
    rows: &[DemandInvoice],
    legal_days: i64,
) -> renox::serde_json::Value {
    let total: i64 = rows.iter().map(|r| r.open_amount).sum();
    let lines: Vec<String> = rows
        .iter()
        .map(|r| describe(r.number.as_deref(), r.invoice_id, r.due_date, r.open_amount))
        .collect();
    json!({
        "lease": lease.number_with_version(),
        "total": crate::money_text(total),
        "invoices": lines.join("; "),
        "days": legal_days,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    #[test]
    fn an_invoice_due_on_the_first_of_march_is_late_on_the_thirty_first() {
        assert!(!overdue_by(day(3, 1), day(3, 30), DEMAND_DAYS));
        assert!(overdue_by(day(3, 1), day(3, 31), DEMAND_DAYS));
        assert!(overdue_by(day(3, 1), day(5, 1), DEMAND_DAYS));
    }

    #[test]
    fn the_legal_case_task_comes_thirty_days_after_the_proof() {
        assert!(!overdue_by(day(4, 5), day(5, 4), LEGAL_DAYS));
        assert!(overdue_by(day(4, 5), day(5, 5), LEGAL_DAYS));
    }

    #[test]
    fn only_an_open_invoice_with_a_balance_can_be_listed() {
        let summary = |status, balance| InvoiceSummary {
            id: InvoiceId(1),
            number: None,
            status,
            party: wl_kernel::PartyId(7),
            legal_entity_id: None,
            currency: "AED".into(),
            gross_total: 1_200_000,
            balance,
            due_date: Some(day(3, 1)),
            disputed: false,
            chasing_paused_until: None,
        };
        let seen = |status, balance| Seen {
            id: 1,
            summary: summary(status, balance),
            due: day(3, 1),
        };
        assert!(eligible(&seen(InvoiceStatus::Issued, 1_200_000), day(3, 31)));
        assert!(eligible(&seen(InvoiceStatus::PartiallyPaid, 100), day(3, 31)));
        assert!(eligible(&seen(InvoiceStatus::Overdue, 100), day(3, 31)));
        assert!(!eligible(&seen(InvoiceStatus::Paid, 0), day(3, 31)));
        assert!(!eligible(&seen(InvoiceStatus::WrittenOff, 100), day(3, 31)));
        assert!(!eligible(&seen(InvoiceStatus::Credited, 0), day(3, 31)));
        assert!(!eligible(&seen(InvoiceStatus::Issued, 100), day(3, 30)));
        let mut disputed = seen(InvoiceStatus::Issued, 100);
        disputed.summary.disputed = true;
        assert!(!eligible(&disputed, day(3, 31)));
    }

    #[test]
    fn the_tasks_are_found_by_the_demand() {
        assert_eq!(demand_key(4), "property.arrears_demand:4");
        assert_eq!(legal_key(4), "property.arrears_legal:4");
    }

    #[test]
    fn a_demand_is_open_until_it_is_sent_or_closed() {
        let mut demand = ArrearsDemand::default();
        assert!(demand.is_open());
        demand.sent_on = Some(day(4, 3));
        assert!(demand.is_sent());
        assert!(!demand.is_open());
        demand.sent_on = None;
        demand.closed_on = Some(day(4, 4));
        assert!(!demand.is_open());
    }
}
