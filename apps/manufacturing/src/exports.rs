//! Export evidence and the 5% VAT correction (07 Manufacturing §2 *VAT*, §4.1
//! last row of "When it goes wrong", §7.2 `MFG.RUL.08`, §9 *VAT*; BL-MFG-12,
//! #393).
//!
//! An export is zero-rated only if the goods leave the UAE within 90 days and
//! the exit is evidenced. The rule follows every dispatch that left and whose
//! invoice lines (the dispatch or its lines are their origin) carry the VAT
//! category *zero-rated*:
//!
//! - **Day 60** ([`WARN_DAYS`]) without evidence: the accountant is told.
//! - **Day 90** ([`FLAG_DAYS`]) without evidence: the dispatch is flagged
//!   *5% VAT due* and the accountant gets one task naming the dispatch, the
//!   zero-rated invoices with their amounts and the customer's contract note
//!   ([`ContractNote`]; whether the VAT is recharged to the customer depends
//!   on the contract and is the accountant's decision). The credit note and
//!   the standard-rated invoice are made with the normal invoicing tools: the
//!   rule changes and credits nothing, and an invoice that is paid does not
//!   stop it.
//! - **Evidence** (a customs exit certificate, a bill of lading or an airway
//!   bill attached to the dispatch, [`EVIDENCE`]) stops the rule for the
//!   dispatch. Evidence attached after the flag keeps the flag and tells the
//!   accountant *evidence received after correction*; nothing is reversed
//!   (`⚠ open: accountant A-12`).
//!
//! [`run`] is the rule for one day. It is idempotent: a [`ExportWatch`] row
//! per dispatch holds the day each step fired, so a second run the same day
//! creates nothing, and a late run (the scheduler was down) fires the steps
//! whose day has passed, 60 and then 90 in the same run. The day count is in
//! calendar days from the day the dispatch left (Asia/Dubai); an invoice
//! issued after the dispatch does not move the clock.
//!
//! **Compliance-critical** (`PLT.RUL.04`): [`run_daily`] runs the rule as a
//! kernel rule run (the retries, the owner alert for a run that ends `failed`
//! and the check for a run that never happened are the kernel's, BL-KRN-08);
//! the host marks the rule critical with its time ([`RUN_AT`]).
//!
//! Decisions: the days N and M are the defaults of the spec as constants
//! (the module has no settings page yet); the accountants are the holders of
//! the `accountant` role, else whom the kernel picks; the rule counts a
//! dispatch that is `dispatched`, `delivered` or `lost`.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::automation::{RuleRunSpec, RunOutcome, run_rule};
use wl_kernel::compliance::{occurrence_key, schedule_subject};
use wl_kernel::documents::{Validity, guard};
use wl_kernel::inventory::{DispatchFacts, dispatch_directory};
use wl_kernel::invoicing::{VatCategory, invoicer};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};
use wl_kernel::{PartyId, Subject, role_tasks};

use crate::notices;
use crate::permissions;
use crate::texts::english;

/// The rule.
pub const RULE: &str = "MFG.RUL.08";
/// The time the rule runs, Asia/Dubai.
pub const RUN_AT: &str = "08:00";
/// The name of the scheduled task.
pub const TASK: &str = "manufacturing:export-evidence";
/// Days after the dispatch left when the accountant is warned (N).
pub const WARN_DAYS: i64 = 60;
/// Days after the dispatch left when the zero rate no longer holds (M).
pub const FLAG_DAYS: i64 = 90;

/// A customs exit certificate.
pub const CUSTOMS_EXIT: &str = "customs_exit_certificate";
/// A bill of lading.
pub const BILL_OF_LADING: &str = "bill_of_lading";
/// An airway bill.
pub const AIRWAY_BILL: &str = "airway_bill";
/// The document kinds that evidence an exit.
pub const EVIDENCE: [&str; 3] = [CUSTOMS_EXIT, BILL_OF_LADING, AIRWAY_BILL];

/// Where one export dispatch stands.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manufacturing_export_watches", unique(dispatch_id))]
pub struct ExportWatch {
    /// The primary key.
    pub id: i64,
    /// The dispatch, `inventory.dispatch:3`.
    pub dispatch_id: i64,
    /// Its number.
    pub number: String,
    /// The customer.
    pub customer_id: Option<i64>,
    /// The day it left (Asia/Dubai).
    pub dispatched_on: NaiveDate,
    /// The day the accountant was warned.
    pub warned_on: Option<NaiveDate>,
    /// The day the dispatch was flagged *5% VAT due*.
    pub flagged_on: Option<NaiveDate>,
    /// The day the evidence was first seen.
    pub evidence_on: Option<NaiveDate>,
    /// Whether the accountant was told that evidence came after the flag.
    pub late_told: bool,
    /// The accountant's task to correct the invoice.
    pub task_id: Option<i64>,
    /// The zero-rated invoices at the flag, in words (number and amount).
    pub invoices: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ExportWatch {
    /// Whether the dispatch is flagged *5% VAT due*.
    pub fn vat_due(&self) -> bool {
        self.flagged_on.is_some()
    }
}

/// The customer's note on how the VAT is treated when the zero rate is lost
/// (recharged to the customer or not): shown on the accountant's task.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manufacturing_contract_notes", unique(party_id))]
pub struct ContractNote {
    /// The primary key.
    pub id: i64,
    /// The customer.
    pub party_id: i64,
    /// The note.
    pub note: String,
    /// The user who wrote it.
    pub set_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Keeps the note of `party` (empty removes it).
///
/// # Errors
///
/// `Forbidden` without `conformity.manage` is not the right here: the
/// accountant and the production manager hold `exports.manage`.
pub async fn set_note(state: &AppState, by: &AuthUser, party: i64, note: &str) -> Result {
    if !by.has_permission(permissions::EXPORTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let note = note.trim().to_owned();
    let found = ContractNote::query()
        .where_(ContractNote::PARTY_ID.eq(party))
        .first(&state.db)
        .await?;
    match found {
        Some(existing) => {
            ContractNote::query()
                .where_(ContractNote::ID.eq(existing.id))
                .update(
                    &state.db,
                    &[
                        ("note", &note),
                        ("set_by", &Some(by.id)),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
        }
        None => {
            ContractNote::create(
                &state.db,
                ContractNote {
                    party_id: party,
                    note,
                    set_by: Some(by.id),
                    ..ContractNote::default()
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// A zero-rated invoice of a dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportInvoice {
    /// The invoice number, else its id.
    pub number: String,
    /// The zero-rated amount billing the dispatch, in fils.
    pub amount: i64,
}

/// The zero-rated invoices that bill `facts` (the dispatch or its lines):
/// issued ones, with the amount of their zero-rated lines of the dispatch.
///
/// # Errors
///
/// The invoicing's errors.
pub async fn zero_rated(state: &AppState, facts: &DispatchFacts) -> Result<Vec<ExportInvoice>> {
    let invoicing = invoicer(state);
    let mut origins: Vec<Subject> = vec![facts.dispatch.clone()];
    origins.extend(facts.lines.iter().map(|line| line.line.clone()));
    let mut ids = Vec::new();
    for origin in &origins {
        for id in invoicing.billing(&state.db, origin).await? {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let mut found = Vec::new();
    for id in ids {
        let amount: i64 = invoicing
            .lines_of(&state.db, id)
            .await?
            .iter()
            .filter(|line| line.vat_category == VatCategory::ZeroRated)
            .filter(|line| line.origin.as_ref().is_some_and(|o| origins.contains(o)))
            .map(|line| line.gross)
            .sum();
        if amount > 0 {
            let number = match invoicing.find(&state.db, id).await? {
                Some(invoice) => invoice.number.unwrap_or_else(|| id.to_string()),
                None => id.to_string(),
            };
            found.push(ExportInvoice { number, amount });
        }
    }
    Ok(found)
}

/// Whether an exit evidence is attached to the dispatch `dispatch` on `day`.
///
/// # Errors
///
/// The document guard's errors.
pub async fn has_evidence(state: &AppState, dispatch: &Subject, day: NaiveDate) -> Result<bool> {
    for kind in EVIDENCE {
        if matches!(
            guard(state)
                .is_valid(&state.db, dispatch, kind, day)
                .await?,
            Validity::Valid { .. }
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The accountants: the holders of the role, else whom the kernel picks.
async fn accountants(state: &AppState) -> Result<Vec<i64>> {
    let mut users: Vec<i64> =
        renox::auth::permissions::users_with_role(&state.db, permissions::ACCOUNTANT_ROLE)
            .await?
            .iter()
            .map(|user| user.id)
            .collect();
    if users.is_empty()
        && let Some(user) = role_tasks::assignee(&state.db, permissions::ACCOUNTANT_ROLE).await?
    {
        users.push(user);
    }
    Ok(users)
}

/// The source key of the task of dispatch `id`.
pub fn task_key(id: i64) -> String {
    format!("manufacturing.export_vat:{id}")
}

/// An amount in fils as `AED 40,000.00`.
fn amount_text(fils: i64) -> String {
    crate::screens::aed(fils)
}

/// The invoices in words: `INV-50 (AED 40,000.00); ...`.
fn invoices_text(invoices: &[ExportInvoice]) -> String {
    invoices
        .iter()
        .map(|invoice| format!("{} ({})", invoice.number, amount_text(invoice.amount)))
        .collect::<Vec<_>>()
        .join("; ")
}

/// What one run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Done {
    /// Dispatches whose accountant was warned.
    pub warned: usize,
    /// Dispatches flagged *5% VAT due*.
    pub flagged: usize,
    /// Dispatches whose late evidence the accountant was told.
    pub late: usize,
}

/// `MFG.RUL.08` for `day` with N = `warn` and M = `flag` days.
///
/// # Errors
///
/// The database's, the invoicing's, the documents' and the ticketing's errors.
pub async fn run(state: &AppState, day: NaiveDate, warn: i64, flag: i64) -> Result<Done> {
    let mut done = Done::default();
    for facts in dispatch_directory(state).left_by(state, day).await? {
        let Some(left) = facts.dispatched_on else {
            continue;
        };
        let known = ExportWatch::query()
            .where_(ExportWatch::DISPATCH_ID.eq(facts.dispatch.subject_id))
            .first(&state.db)
            .await?;
        let invoices = zero_rated(state, &facts).await?;
        if invoices.is_empty() && known.is_none() {
            continue;
        }
        let mut watch = match known {
            Some(watch) => watch,
            None => {
                ExportWatch::create(
                    &state.db,
                    ExportWatch {
                        dispatch_id: facts.dispatch.subject_id,
                        number: facts.number.clone(),
                        customer_id: facts.recipient.map(|party| party.0),
                        dispatched_on: left,
                        ..ExportWatch::default()
                    },
                )
                .await?
            }
        };
        let days = (day - left).num_days();
        if has_evidence(state, &facts.dispatch, day).await? {
            if watch.evidence_on.is_none() {
                set(state, &watch, &[("evidence_on", &Some(day))]).await?;
                watch.evidence_on = Some(day);
            }
            if watch.vat_due() && !watch.late_told {
                tell(
                    state,
                    &watch,
                    &english(
                        "manufacturing.export.late_title",
                        &[("dispatch", &watch.number)],
                    ),
                    &english(
                        "manufacturing.export.late_body",
                        &[("dispatch", &watch.number), ("day", &day)],
                    ),
                )
                .await?;
                set(state, &watch, &[("late_told", &true)]).await?;
                done.late += 1;
            }
            continue;
        }
        if days >= warn && watch.warned_on.is_none() {
            tell(
                state,
                &watch,
                &english(
                    "manufacturing.export.warn_title",
                    &[("dispatch", &watch.number), ("days", &warn)],
                ),
                &english(
                    "manufacturing.export.warn_body",
                    &[
                        ("dispatch", &watch.number),
                        ("invoices", &invoices_text(&invoices)),
                        ("left", &left),
                        ("flag", &flag),
                    ],
                ),
            )
            .await?;
            set(state, &watch, &[("warned_on", &Some(day))]).await?;
            watch.warned_on = Some(day);
            done.warned += 1;
        }
        if days >= flag && watch.flagged_on.is_none() {
            let words = invoices_text(&invoices);
            let task = open_task(state, &watch, &words).await?;
            set(
                state,
                &watch,
                &[
                    ("flagged_on", &Some(day)),
                    ("task_id", &task),
                    ("invoices", &words),
                ],
            )
            .await?;
            done.flagged += 1;
        }
    }
    Ok(done)
}

/// Sets columns of `watch`.
async fn set(
    state: &AppState,
    watch: &ExportWatch,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let stamp = renox::db::now();
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    all.push(("updated_at", &stamp));
    ExportWatch::query()
        .where_(ExportWatch::ID.eq(watch.id))
        .update(&state.db, &all)
        .await?;
    Ok(())
}

/// Tells the accountants.
async fn tell(state: &AppState, watch: &ExportWatch, title: &str, body: &str) -> Result {
    let users = accountants(state).await?;
    notices::tell(
        state,
        Subject::new(
            wl_kernel::inventory::DISPATCH_SUBJECT_TYPE,
            watch.dispatch_id,
        ),
        &watch.number,
        &users,
        title,
        body,
    )
    .await?;
    Ok(())
}

/// The accountant's task to correct the invoice; one per dispatch.
async fn open_task(state: &AppState, watch: &ExportWatch, invoices: &str) -> Result<Option<i64>> {
    let creator = task_creator(state);
    let key = task_key(watch.dispatch_id);
    if let Some(open) = creator.find_open(state, &key).await? {
        return Ok(Some(open.id.0));
    }
    let note = match watch.customer_id {
        Some(party) => ContractNote::query()
            .where_(ContractNote::PARTY_ID.eq(party))
            .first(&state.db)
            .await?
            .map(|found| found.note),
        None => None,
    }
    .filter(|note| !note.is_empty())
    .unwrap_or_else(|| english("manufacturing.export.no_note", &[]));
    let title = english(
        "manufacturing.export.task_title",
        &[("dispatch", &watch.number)],
    );
    let body = english(
        "manufacturing.export.task_body",
        &[
            ("dispatch", &watch.number),
            ("invoices", &invoices),
            ("days", &FLAG_DAYS),
            ("note", &note),
        ],
    );
    let mut ticket = OpenTicket::new(
        Subject::new(
            wl_kernel::inventory::DISPATCH_SUBJECT_TYPE,
            watch.dispatch_id,
        ),
        TASK_CATEGORY,
        &title,
    )
    .description(&body)
    .source_key(&key);
    if let Some(user) = role_tasks::assignee(&state.db, permissions::ACCOUNTANT_ROLE).await? {
        ticket = ticket.assign_to(Assignee::user(user));
    }
    let opened = creator.open(state, ticket).await?;
    // The accountant is told with the task: the title and the body.
    tell(state, watch, &title, &body).await?;
    Ok(opened.map(|opened| opened.id.0))
}

/// `MFG.RUL.08` for `day` as a kernel rule run, once for the day: a run that
/// fails is retried with the kernel's backoff and, after the last attempt,
/// alerts the owner role and the company admin and opens the owner's task;
/// a rerun that succeeds closes the task (BL-KRN-08).
///
/// # Errors
///
/// The run's bookkeeping errors; the rule's own are the run's retries.
pub async fn run_daily(state: &AppState, day: NaiveDate) -> Result<RunOutcome> {
    let spec =
        RuleRunSpec::new(RULE, schedule_subject(), occurrence_key(RULE, day)).compliance_critical();
    run_rule(state, spec, move |state| async move {
        run(&state, day, WARN_DAYS, FLAG_DAYS).await.map(|_| ())
    })
    .await
}

/// The customer of a dispatch as a party, for the page.
pub fn customer_of(watch: &ExportWatch) -> Option<PartyId> {
    watch.customer_id.map(PartyId)
}
