#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod ageing;
mod agent;
mod ageing_pages;
pub mod allocate;
pub mod allocation;
pub mod bad_debt_relief;
pub mod bill_allocation;
mod bill_events;
pub mod bill_line;
pub mod bill_machine;
pub mod bill_notices;
mod bill_pages;
pub mod bill_pay;
pub mod bill_reopen_notices;
pub mod bill_reversal;
pub mod bill_submit;
pub mod bill_unallocate;
pub mod bill_unallocation;
pub mod budget;
pub mod budget_actual;
pub mod budget_category;
mod budget_category_pages;
pub mod budget_contract;
pub mod budget_lines;
pub mod budget_machine;
mod budget_notices;
mod budget_pages;
pub mod budget_reminder;
pub mod budget_report;
mod budget_report_pages;
pub mod budget_variance;
pub mod cash_alert;
pub mod cash_balance;
pub mod cash_forecast;
pub mod cash_forecast_load;
mod cash_forecast_pages;
pub mod config;
pub mod credit;
pub mod credit_note;
pub mod credit_note_line;
mod credit_note_machine;
pub mod credit_note_movement;
mod credit_note_pages;
pub mod credit_notes;
pub mod debit_note_contract;
mod debit_note_events;
pub mod debit_note_line;
mod debit_note_machine;
pub mod debit_note_movement;
pub mod debit_note_settle;
pub mod debit_note_vat;
pub mod debit_notes;
pub mod delivery_billing;
pub mod delivery_dispute;
pub mod dispute;
pub mod dispute_credit;
mod dispute_pages;
pub mod dispute_resolve;
mod drawing_pages;
pub mod drawings;
pub mod drawings_machine;
pub mod duplicates;
pub mod einvoice;
pub mod einvoice_adapter;
mod einvoice_pages;
pub mod einvoice_payload;
pub mod einvoice_send;
mod einvoice_tasks;
pub mod escalation;
pub mod escalation_log;
mod events;
pub mod invoice;
pub mod invoicer;
pub mod issue;
pub mod line;
mod machine;
mod open_search;
pub mod overdue;
pub mod owner;
pub mod owner_drawings;
pub mod pages;
pub mod payables;
pub mod payer_agreement;
pub mod payer_bill;
pub mod payer_bill_machine;
mod payer_bill_pages;
pub mod payer_closing;
pub mod payer_contract;
pub mod payer_credit;
pub mod payer_credit_refund;
pub mod payer_issue;
mod payer_notices;
pub mod payer_policy;
pub mod payer_recoupment;
pub mod payer_rejection;
pub mod payer_remittance;
pub mod payer_split;
pub mod payer_tax;
mod payer_texts;
pub mod period_locks;
pub mod policy;
pub mod portal;
pub mod rate_card;
mod rate_card_contract;
mod rate_card_pages;
mod rate_card_screens;
mod rate_card_texts;
pub mod rate_cards;
pub mod rate_pricing;
pub mod recover;
pub mod recovery;
pub mod refund;
pub mod relations;
pub mod search;
pub mod reminders;
pub mod reopen_notices;
mod report_table;
pub mod restore;
pub mod reversal;
pub mod seed;
pub mod subscription;
pub mod subscription_machine;
mod subscription_pages;
pub mod subscriptions;
pub mod supplier_bill;
pub mod supplier_debit_note;
pub mod task;
mod task_pages;
mod texts;
pub mod unallocate;
pub mod unallocation;
pub mod usage;
pub mod usage_adjustment;
pub mod usage_billing;
pub mod vat;
pub mod vat_adjustment;
pub mod vat_pages;
pub mod vat_period;
pub mod vat_summary;
pub mod write_off;
pub mod write_off_notice;

use std::sync::Arc;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use allocation::Allocation;
pub use bill_allocation::BillAllocation;
pub use bill_line::{BillLine, BillLineDraft};
pub use bill_machine::{approve as approve_supplier_bill, void as void_supplier_bill};
pub use bill_submit::submit as submit_supplier_bill;
pub use bill_unallocation::BillUnallocation;
pub use budget::{BudgetStatus, CompanyBudget, CompanyBudgetLine, StartChoice};
pub use budget_category::{BudgetCategory, BudgetKind, BudgetMapping, MappingKind};
pub use budget_contract::Budgets;
pub use budget_reminder::BudgetReminder;
pub use budget_variance::BudgetWarning;
pub use config::{
    AdjustmentPlus, DuplicateKey, EInvoiceEntity, InputVatTiming, InvoicingConfig, ReasonProblem,
    ReliefMonths, SettingError, WriteOffReason,
};
pub use credit_note::{CreditNote, CreditNoteKind};
pub use credit_note_line::CreditNoteLine;
pub use credit_note_movement::{CreditNoteMovement, MovementKind};
pub use debit_note_line::{DebitNoteLine, PricedAt};
pub use debit_note_movement::{DebitMovementKind, DebitNoteMovement};
pub use dispute::{Dispute, DisputeVia};
pub use drawings::BillRecoding;
pub use drawings_machine::{DrawingApproval, DrawingStatus};
pub use einvoice::{EInvoiceTransmission, TransmissionStatus};
pub use einvoice_adapter::{EInvoiceAdapter, EInvoiceScope};
pub use escalation_log::EscalationLog;
pub use invoice::{Invoice, InvoiceKind, PartySnapshot};
pub use invoicer::Invoices;
pub use line::InvoiceLine;
pub use payer_agreement::{PayerAgreement, VatRecipient};
pub use payer_bill::{PayerBill, PayerBillLine};
pub use payer_contract::PayerBilling;
pub use recovery::Recovery;
pub use reminders::ReminderLog;
pub use subscription::{Subscription, SubscriptionCycle, SubscriptionPause};
pub use subscriptions::{Canceller, SubscriptionBook};
pub use supplier_bill::{BillDraft, SupplierBill, SupplierBillStatus};
pub use supplier_debit_note::{DebitNoteOrigin, DebitNoteReason, SupplierDebitNote};
pub use task::{InvoicingTask, TaskKind, TaskStatus};
pub use unallocation::{Unallocation, UnallocationOutcome};
pub use usage::{UsageDraft, UsageRecord};
pub use usage_adjustment::{AdjustmentOutcome, UsageAdjustment};
pub use vat::{CategoryTotal, LineAmounts, VatTotals};
pub use vat_adjustment::{AdjustmentKind, AdjustmentStatus, VatAdjustment};
pub use vat_period::TaxPeriod;
pub use vat_summary::{VatSummary, VatSummaryStatus};
pub use write_off_notice::SendWriteOffNotice;

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-invoicing/rate_card.html",
        include_str!("../views/wl-invoicing/rate_card.html"),
    ),
    (
        "wl-invoicing/rate_card_version.html",
        include_str!("../views/wl-invoicing/rate_card_version.html"),
    ),
    (
        "wl-invoicing/einvoices.html",
        include_str!("../views/wl-invoicing/einvoices.html"),
    ),
    (
        write_off_notice::TEMPLATE,
        include_str!("../views/wl-invoicing/write_off_notice.html"),
    ),
    (
        "wl-invoicing/index.html",
        include_str!("../views/wl-invoicing/index.html"),
    ),
    (
        "wl-invoicing/show.html",
        include_str!("../views/wl-invoicing/show.html"),
    ),
    (
        "wl-invoicing/_write_off_sheet.html",
        include_str!("../views/wl-invoicing/_write_off_sheet.html"),
    ),
    (
        "wl-invoicing/credit_notes.html",
        include_str!("../views/wl-invoicing/credit_notes.html"),
    ),
    (
        "wl-invoicing/credit_note.html",
        include_str!("../views/wl-invoicing/credit_note.html"),
    ),
    (
        "wl-invoicing/_credit_sheet.html",
        include_str!("../views/wl-invoicing/_credit_sheet.html"),
    ),
    (
        "wl-invoicing/_credit_note_form.html",
        include_str!("../views/wl-invoicing/_credit_note_form.html"),
    ),
    (
        "wl-invoicing/_dispute_sheet.html",
        include_str!("../views/wl-invoicing/_dispute_sheet.html"),
    ),
    (
        "wl-invoicing/_resolve_dispute_sheet.html",
        include_str!("../views/wl-invoicing/_resolve_dispute_sheet.html"),
    ),
    (
        "wl-invoicing/ageing.html",
        include_str!("../views/wl-invoicing/ageing.html"),
    ),
    (
        "wl-invoicing/vat_summaries.html",
        include_str!("../views/wl-invoicing/vat_summaries.html"),
    ),
    (
        "wl-invoicing/vat_summary.html",
        include_str!("../views/wl-invoicing/vat_summary.html"),
    ),
    (
        "wl-invoicing/tasks.html",
        include_str!("../views/wl-invoicing/tasks.html"),
    ),
    (
        "wl-invoicing/_adjustment_sheet.html",
        include_str!("../views/wl-invoicing/_adjustment_sheet.html"),
    ),
    (
        "wl-invoicing/bills_index.html",
        include_str!("../views/wl-invoicing/bills_index.html"),
    ),
    (
        "wl-invoicing/bill_form.html",
        include_str!("../views/wl-invoicing/bill_form.html"),
    ),
    (
        "wl-invoicing/bill_show.html",
        include_str!("../views/wl-invoicing/bill_show.html"),
    ),
    (
        "wl-invoicing/_bill_reason_sheet.html",
        include_str!("../views/wl-invoicing/_bill_reason_sheet.html"),
    ),
    (
        "wl-invoicing/budgets.html",
        include_str!("../views/wl-invoicing/budgets.html"),
    ),
    (
        "wl-invoicing/budget.html",
        include_str!("../views/wl-invoicing/budget.html"),
    ),
    (
        "wl-invoicing/_budget_sheets.html",
        include_str!("../views/wl-invoicing/_budget_sheets.html"),
    ),
    (
        "wl-invoicing/budget_form.html",
        include_str!("../views/wl-invoicing/budget_form.html"),
    ),
    (
        "wl-invoicing/budget_categories.html",
        include_str!("../views/wl-invoicing/budget_categories.html"),
    ),
    (
        "wl-invoicing/budget_category_form.html",
        include_str!("../views/wl-invoicing/budget_category_form.html"),
    ),
    (
        "wl-invoicing/budget_report.html",
        include_str!("../views/wl-invoicing/budget_report.html"),
    ),
    (
        "wl-invoicing/cash_forecast.html",
        include_str!("../views/wl-invoicing/cash_forecast.html"),
    ),
    (
        "wl-invoicing/cash_forecast_cell.html",
        include_str!("../views/wl-invoicing/cash_forecast_cell.html"),
    ),
    (
        "wl-invoicing/subscriptions.html",
        include_str!("../views/wl-invoicing/subscriptions.html"),
    ),
    (
        "wl-invoicing/subscription.html",
        include_str!("../views/wl-invoicing/subscription.html"),
    ),
    (
        "wl-invoicing/payer_bills.html",
        include_str!("../views/wl-invoicing/payer_bills.html"),
    ),
    (
        "wl-invoicing/payer_bill.html",
        include_str!("../views/wl-invoicing/payer_bill.html"),
    ),
    (
        "wl-invoicing/portal_invoices.html",
        include_str!("../views/wl-invoicing/portal_invoices.html"),
    ),
    (
        "wl-invoicing/portal_statement.html",
        include_str!("../views/wl-invoicing/portal_statement.html"),
    ),
];

/// The module's settings in `state`: those `register` provided, else the
/// §7 defaults.
pub(crate) fn config_of(state: &AppState) -> Arc<InvoicingConfig> {
    state
        .provided::<InvoicingConfig>()
        .unwrap_or_else(|| Arc::new(InvoicingConfig::default()))
}

/// Whether `user` holds `permission` globally or in any scope. Lists use it
/// to decide who gets the export menu of their grid.
pub(crate) fn may_export(user: &AuthUser, permission: &str) -> bool {
    user.has_permission(permission) || user.has_permission_anywhere(permission)
}

/// The module's permissions (wl-invoicing §6).
pub mod permissions {
    /// See invoices and credit notes.
    pub const VIEW: &str = "invoices.view";
    /// Create, change and discard draft invoices.
    pub const CREATE: &str = "invoices.create";
    /// Issue an invoice.
    pub const ISSUE: &str = "invoices.issue";
    /// Credit an invoice and issue credit notes.
    pub const CREDIT: &str = "invoices.credit";
    /// Request a bad-debt write-off (`FIN.ACL.15`).
    pub const WRITE_OFF: &str = "invoices.write_off";
    /// Raise and resolve an invoice dispute.
    pub const DISPUTE: &str = "invoices.dispute";
    /// See receivables: balances, ageing and reminders.
    pub const RECEIVABLES_VIEW: &str = "receivables.view";
    /// See VAT summaries and their adjustments.
    pub const VAT_VIEW: &str = "vat.view";
    /// Prepare a VAT summary and decide its adjustments.
    pub const VAT_PREPARE: &str = "vat.prepare";
    /// Review a VAT summary, or send it back (`FIN.ACL.10`).
    pub const VAT_REVIEW: &str = "vat.review";
    /// Lock a reviewed VAT summary.
    pub const VAT_LOCK: &str = "vat.lock";
    /// See supplier bills.
    pub const SUPPLIER_BILLS_VIEW: &str = "supplier_bills.view";
    /// Enter, change and delete draft supplier bills, and submit them.
    pub const SUPPLIER_BILLS_CREATE: &str = "supplier_bills.create";
    /// Approve a supplier bill for payment, or void it (`FIN.ACL.03`).
    pub const SUPPLIER_BILLS_APPROVE: &str = "supplier_bills.approve";
    /// See the company budgets and the budget categories.
    pub const BUDGETS_VIEW: &str = "budgets.view";
    /// Prepare, submit and discard company budget versions.
    pub const BUDGETS_MANAGE: &str = "budgets.manage";
    /// Approve or return a submitted company budget (`PLT.APR.01`).
    pub const BUDGETS_APPROVE: &str = "budgets.approve";
    /// Add and change budget categories and their mapping.
    pub const BUDGET_CATEGORIES_MANAGE: &str = "budget_categories.manage";
    /// Approve owner drawings (`FIN.ACL.21`) and see their lines, payees and
    /// notes (`FIN.ACL.22`); others see the drawings totals.
    pub const DRAWINGS_APPROVE: &str = "drawings.approve";
    /// Open the cash-flow forecast of the user's entities (`FIN.CFL.01`).
    pub const CASHFLOW_VIEW: &str = "cashflow.view";
    /// Enter and void opening cash balances.
    pub const CASHFLOW_MANAGE: &str = "cashflow.manage";
    /// See the e-invoice board: the transmissions per entity and state.
    pub const EINVOICES_VIEW: &str = "einvoices.view";
    /// Resubmit a rejected e-invoice transmission after its cause was fixed.
    pub const EINVOICES_MANAGE: &str = "einvoices.manage";
    /// See the subscriptions and their cycles.
    pub const SUBSCRIPTIONS_VIEW: &str = "subscriptions.view";
    /// Pause, resume and cancel a subscription in the back office.
    pub const SUBSCRIPTIONS_MANAGE: &str = "subscriptions.manage";
    /// See payer bills, their lines, remittances and the payer credit
    /// balance.
    pub const PAYER_BILLS_VIEW: &str = "payer_bills.view";
    /// Issue a payer bill, send it again, record its technical rejection and
    /// void one created in error.
    pub const PAYER_BILLS_ISSUE: &str = "payer_bills.issue";
    /// Record the payer's remittances and recoupments (`FIN.ACL.16`).
    pub const PAYER_BILLS_RECONCILE: &str = "payer_bills.reconcile";
    /// Write off the open amount of a payer bill, or move it to the customer
    /// (`FIN.ACL.14`).
    pub const PAYER_BILLS_WRITE_OFF: &str = "payer_bills.write_off";
    /// See the rate cards, their versions and lines (`FIN.RAT.01`).
    pub const RATE_CARDS_VIEW: &str = "rate_cards.view";
    /// Make rate cards, new versions and their lines, and record the
    /// client's signed acceptance (`MPW.ACL.16`).
    pub const RATE_CARDS_MANAGE: &str = "rate_cards.manage";
    /// Approve a rate card version dated in the past after the client
    /// accepted it (`MPW.ACL.16`, the MD).
    pub const RATE_CARDS_APPROVE: &str = "rate_cards.approve";
}

/// The invoicing module. The host adds it with
/// `.module(wl_invoicing::WlInvoicing::new())` when `client.toml` enables
/// `invoicing`.
#[derive(Default)]
pub struct WlInvoicing {
    config: InvoicingConfig,
    /// The ASP adapters by name (`FIN.INV.01`).
    adapters: Vec<(String, Arc<dyn EInvoiceAdapter>)>,
    /// The e-invoicing scope; the default puts company recipients in.
    scope: Option<Arc<dyn EInvoiceScope>>,
}

impl WlInvoicing {
    /// The module with the §7 defaults ([`InvoicingConfig::default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// Days from the issue date to the due date when a draft has none.
    pub fn payment_terms_days(mut self, days: u32) -> Self {
        self.config.payment_terms_days = days;
        self
    }

    /// The reminder steps (`FIN.RUL.01`), in days relative to the due date
    /// (negative before it).
    pub fn reminder_steps(mut self, days: &[i32]) -> Self {
        let mut steps = days.to_vec();
        steps.sort_unstable();
        steps.dedup();
        self.config.reminder_steps = steps;
        self
    }

    /// Adds a write-off reason code, or changes whether an existing one
    /// needs a text.
    pub fn write_off_reason(mut self, code: &str, text_required: bool) -> Self {
        let reasons = &mut self.config.write_off_reasons;
        match reasons.iter_mut().find(|r| r.code == code) {
            Some(reason) => reason.text_required = text_required,
            None => reasons.push(WriteOffReason::new(code, text_required)),
        }
        self
    }

    /// Whether the customer write-off notice is sent on `write_off`.
    pub fn write_off_notice(mut self, send: bool) -> Self {
        self.config.write_off_notice = send;
        self
    }

    /// The roles whose holders approve a write-off by default (`FIN.ACL.15`).
    pub fn write_off_approvers(mut self, roles: &[&str]) -> Self {
        self.config.write_off_approvers = roles.iter().map(|r| (*r).to_owned()).collect();
        self
    }

    /// The tax period (the FTA stagger) of the legal entity with this code.
    /// Entities without one are quarterly on calendar quarters.
    pub fn tax_period(mut self, entity_code: &str, period: TaxPeriod) -> Self {
        let periods = &mut self.config.tax_periods;
        match periods.iter_mut().find(|(code, _)| code == entity_code) {
            Some(entry) => entry.1 = period,
            None => periods.push((entity_code.to_owned(), period)),
        }
        self
    }

    /// The months after the date of supply before bad-debt relief is
    /// proposed (`FIN.RUL.14`); [`ReliefMonths`] is never below 6.
    pub fn bad_debt_relief_months(mut self, months: ReliefMonths) -> Self {
        self.config.bad_debt_relief_months = months;
        self
    }

    /// When the `invoicing:bad-debt-relief` task runs, `HH:MM` in
    /// Asia/Dubai.
    pub fn bad_debt_relief_run_at(mut self, time: &str) -> Self {
        self.config.bad_debt_relief_run_at = time.to_owned();
        self
    }

    /// Turns the bad-debt relief proposal on or off; off, the task is not
    /// scheduled.
    pub fn bad_debt_relief(mut self, on: bool) -> Self {
        self.config.bad_debt_relief = on;
        self
    }

    /// The role told when relief is proposed (default `accountant`).
    pub fn relief_notify_role(mut self, role: &str) -> Self {
        self.config.relief_notify_role = role.to_owned();
        self
    }

    /// Whether the party may dispute an invoice from the portal (default
    /// off).
    pub fn portal_disputes(mut self, on: bool) -> Self {
        self.config.portal_disputes = on;
        self
    }

    /// Days chasing stays paused after an upheld dispute (default 7).
    pub fn dispute_grace_days(mut self, days: u32) -> Self {
        self.config.dispute_grace_days = days;
        self
    }

    /// Whole days between two escalation steps of an open dispute
    /// (`FIN.RUL.15`, default 14). 0 is stored as 1.
    pub fn dispute_escalation_days(mut self, days: u32) -> Self {
        self.config.dispute_escalation_days = days.max(1);
        self
    }

    /// The role told from the second escalation step on (default
    /// `finance_manager`).
    pub fn dispute_escalation_role(mut self, role: &str) -> Self {
        self.config.dispute_escalation_role = role.to_owned();
        self
    }

    /// The role whose holders get the ticket of a dispute (the first holder
    /// is the assignee, every holder is told) instead of the invoice's owner
    /// or the holders of `invoices.issue` (default none; 04: the engagement
    /// manager).
    pub fn dispute_ticket_role(mut self, role: &str) -> Self {
        self.config.dispute_ticket_role = Some(role.to_owned());
        self
    }

    /// The upper day edges of the ageing buckets (default `[30, 60, 90]`:
    /// 0-30, 31-60, 61-90, 90+). Sorted; zero and repeats are dropped.
    pub fn ageing_buckets(mut self, edges: &[u32]) -> Self {
        let mut edges: Vec<u32> = edges.iter().copied().filter(|e| *e > 0).collect();
        edges.sort_unstable();
        edges.dedup();
        self.config.ageing_buckets = edges;
        self
    }

    /// Whether a submitted supplier bill is checked for a duplicate
    /// (`FIN.ACL.03`, default on). Off, no bill is held.
    pub fn duplicate_check(mut self, on: bool) -> Self {
        self.config.duplicate_check = on;
        self
    }

    /// The key sets the duplicate check compares, each on its own (default
    /// `[SupplierInvoiceNumber]`). Repeats are dropped.
    pub fn duplicate_keys(mut self, keys: &[DuplicateKey]) -> Self {
        let mut kept: Vec<DuplicateKey> = Vec::new();
        for key in keys {
            if !kept.contains(key) {
                kept.push(*key);
            }
        }
        self.config.duplicate_keys = kept;
        self
    }

    /// The role told when a bill is held as a possible duplicate (default
    /// `accountant`).
    pub fn duplicate_notify_role(mut self, role: &str) -> Self {
        self.config.duplicate_notify_role = role.to_owned();
        self
    }

    /// The month a fiscal year of the company budget starts in, 1 to 12
    /// (default 1, January). Another number is stored as January.
    pub fn budget_fiscal_start_month(mut self, month: u32) -> Self {
        self.config.budget_fiscal_start_month = if (1..=12).contains(&month) { month } else { 1 };
        self
    }

    /// Whether a new budget version may change the months that had ended
    /// when it was created (default off: those lines are locked).
    pub fn budget_revise_past_months(mut self, on: bool) -> Self {
        self.config.budget_revise_past_months = on;
        self
    }

    /// Days before a fiscal year starts that the accountant and the finance
    /// manager are asked for its budget (`FIN.RUL.26`, default 30).
    pub fn budget_reminder_days(mut self, days: u32) -> Self {
        self.config.budget_reminder_days = days;
        self
    }

    /// The roles `FIN.RUL.26` tells (default `accountant`,
    /// `finance_manager`).
    pub fn budget_reminder_roles(mut self, roles: &[&str]) -> Self {
        self.config.budget_reminder_roles = roles.iter().map(|r| (*r).to_owned()).collect();
        self
    }

    /// The share of a month's budget line at which `FIN.RUL.24` warns, in
    /// percent (default 100).
    pub fn budget_variance_percent(mut self, percent: u32) -> Self {
        self.config.budget_variance_percent = percent.max(1);
        self
    }

    /// The role `FIN.RUL.24` tells besides the category's owner role
    /// (default `finance_manager`).
    pub fn budget_variance_role(mut self, role: &str) -> Self {
        self.config.budget_variance_role = role.to_owned();
        self
    }

    /// How many weeks the cash-flow forecast looks ahead (default 13, at
    /// least 1).
    pub fn cash_forecast_weeks(mut self, weeks: u32) -> Self {
        self.config.cash_forecast_weeks = weeks.max(1);
        self
    }

    /// The day a forecast week starts on (default Monday).
    pub fn cash_week_start(mut self, day: renox::chrono::Weekday) -> Self {
        self.config.cash_week_start = day;
        self
    }

    /// Whether the VAT payable is counted in the forecast (default on).
    pub fn cash_vat_counted(mut self, on: bool) -> Self {
        self.config.cash_vat_counted = on;
        self
    }

    /// Whether supplier bills not yet approved are counted (default on).
    pub fn cash_unapproved_bills_counted(mut self, on: bool) -> Self {
        self.config.cash_unapproved_bills_counted = on;
        self
    }

    /// Whether the disputed part of an invoice is counted as a receipt
    /// (default off).
    pub fn cash_disputed_counted(mut self, on: bool) -> Self {
        self.config.cash_disputed_counted = on;
        self
    }

    /// The days after which a bank balance is stale (default 7).
    pub fn cash_stale_days(mut self, days: i64) -> Self {
        self.config.cash_stale_days = days.max(1);
        self
    }

    /// Whether the forecast moves each receipt by its party's average days
    /// late (default off).
    pub fn cash_days_late(mut self, on: bool) -> Self {
        self.config.cash_days_late = on;
        self
    }

    /// Turns e-invoicing on for the legal entity with `entity_code` from
    /// `go_live` (Asia/Dubai) through the ASP adapter named `adapter`
    /// (install it with [`WlInvoicing::asp_adapter`]). From that date every
    /// invoice and credit note to a recipient in scope is sent to the ASP
    /// and a simplified tax invoice can no longer be issued.
    pub fn einvoicing(
        mut self,
        entity_code: &str,
        go_live: renox::chrono::NaiveDate,
        adapter: &str,
    ) -> Self {
        let entry = EInvoiceEntity {
            entity_code: entity_code.to_owned(),
            go_live,
            adapter: Some(adapter.to_owned()).filter(|a| !a.trim().is_empty()),
        };
        match self
            .config
            .einvoice
            .iter_mut()
            .find(|e| e.entity_code == entity_code)
        {
            Some(existing) => *existing = entry,
            None => self.config.einvoice.push(entry),
        }
        self
    }

    /// Installs the ASP adapter `adapter` under `name`.
    pub fn asp_adapter(mut self, name: &str, adapter: impl EInvoiceAdapter) -> Self {
        self.adapters.retain(|(n, _)| n != name);
        self.adapters.push((name.to_owned(), Arc::new(adapter)));
        self
    }

    /// Replaces the e-invoicing scope (default: company recipients).
    pub fn einvoice_scope(mut self, scope: impl EInvoiceScope) -> Self {
        self.scope = Some(Arc::new(scope));
        self
    }

    /// The minutes after `issue` at which a transmission is tried again
    /// (default 5, 30, 120, 360), then every 6 hours until it is sent.
    pub fn einvoice_retries(mut self, minutes: &[i64]) -> Self {
        let mut steps: Vec<i64> = minutes.iter().copied().filter(|m| *m > 0).collect();
        steps.sort_unstable();
        steps.dedup();
        self.config.einvoice_retry_minutes = steps;
        self
    }

    /// The hours after `issue` at which a transmission still not sent raises
    /// a task (`FIN.RUL.16`, default 24).
    pub fn einvoice_task_after_hours(mut self, hours: i64) -> Self {
        self.config.einvoice_task_hours = hours.max(1);
        self
    }

    /// The projected balance the legal entity with `entity_code` wants to
    /// keep, in AED fils (default 0): `FIN.RUL.23` warns below it.
    pub fn cash_threshold(mut self, entity_code: &str, fils: i64) -> Self {
        let thresholds = &mut self.config.cash_thresholds;
        match thresholds.iter_mut().find(|(code, _)| code == entity_code) {
            Some(entry) => entry.1 = fils,
            None => thresholds.push((entity_code.to_owned(), fils)),
        }
        self
    }

    /// When the daily cash check runs, `HH:MM` in Asia/Dubai (default
    /// `06:00`).
    pub fn cash_alert_time(mut self, time: &str) -> Self {
        self.config.cash_alert_time = time.to_owned();
        self
    }

    /// The roles `FIN.RUL.23` tells (default `finance_manager`, `admin`).
    pub fn cash_alert_roles(mut self, roles: &[&str]) -> Self {
        self.config.cash_alert_roles = roles.iter().map(|r| (*r).to_owned()).collect();
        self
    }

    /// Whether the income budget fill counts in the balance `FIN.RUL.23`
    /// checks (default off).
    pub fn cash_income_fill_counted(mut self, on: bool) -> Self {
        self.config.cash_income_fill_counted = on;
        self
    }

    /// Lets the system issue the drafts of this source type (`<module>.<entity>`,
    /// e.g. `ticketing.ticket`) without a user, through `Invoicer::issue` with
    /// no user (default none: drafts are reviewed first).
    pub fn auto_issue_source(mut self, source_type: &str) -> Self {
        if !self
            .config
            .auto_issue_sources
            .iter()
            .any(|s| s == source_type)
        {
            self.config.auto_issue_sources.push(source_type.to_owned());
        }
        self
    }

    /// Names a bank account of the legal entity with `entity_code`, so the
    /// forecast shows it (with a warning) before it has a balance.
    pub fn cash_account(mut self, entity_code: &str, account: &str) -> Self {
        self.config
            .cash_accounts
            .push((entity_code.to_owned(), account.to_owned()));
        self
    }

    /// Whether the sole holder of `drawings.approve` of an entity records
    /// their own drawing without a second approver (`FIN.ACL.23`, default
    /// on). Off, such a drawing waits until a second holder exists.
    pub fn drawings_self_approve(mut self, on: bool) -> Self {
        self.config.drawings_self_approve = on;
        self
    }

    /// The role told when a drawing is recorded without a second approver
    /// (default `accountant`).
    pub fn drawings_notify_role(mut self, role: &str) -> Self {
        self.config.drawings_notify_role = role.to_owned();
        self
    }

    /// When a supplier debit note reduces input VAT: on its issue (default)
    /// or on the supplier's tax credit note (§7).
    pub fn debit_note_vat_timing(mut self, timing: InputVatTiming) -> Self {
        self.config.debit_note_vat_timing = timing;
        self
    }

    /// Whether the supplier's tax credit note must be recorded before a
    /// debit note settles (default on, §7).
    pub fn tax_credit_note_required(mut self, required: bool) -> Self {
        self.config.tax_credit_note_required = required;
        self
    }

    /// The role told when a debit note is drafted from a return (default
    /// `accountant`).
    pub fn debit_note_notify_role(mut self, role: &str) -> Self {
        self.config.debit_note_notify_role = role.to_owned();
        self
    }

    /// The role whose holder gets the task of a payer bill the submission
    /// channel rejected (default `billing_coordinator`).
    pub fn payer_bill_task_role(mut self, role: &str) -> Self {
        self.config.payer_bill_task_role = role.to_owned();
        self
    }

    /// The roles whose holders alone approve the write-off of a payer bill
    /// above the payer agreement's limit (`FIN.ACL.14`). By default any
    /// `admin` or holder of `payer_bills.write_off` may. The setting is read
    /// when this module registers its own default policy, so it belongs to the
    /// app that builds the module: no other default is registered over it, and
    /// the order modules register in does not matter. A database that already
    /// holds the policy keeps its row (policies changed in the admin are kept):
    /// change the stored `invoicing.payer_write_off` policy there.
    pub fn payer_write_off_approvers(mut self, roles: &[&str]) -> Self {
        self.config.payer_write_off_approvers = roles.iter().map(|r| (*r).to_owned()).collect();
        self
    }

    /// The write-off limit, in fils, a new payer agreement starts with
    /// (`FIN.ACL.14`; default 0: every write-off is approved; 05: 100000,
    /// AED 1,000). Read by [`PayerAgreement::create`] and [`PayerAgreement::with_defaults`]; an agreement
    /// that exists keeps its own limit.
    pub fn payer_write_off_limit(mut self, fils: i64) -> Self {
        self.config.payer_write_off_limit = fils.max(0);
        self
    }

    /// The owner role of every seeded budget category and the default of a
    /// category made later without one (`FIN.BUD.01`; 05: the accountant).
    /// Owner drawings never get one.
    pub fn budget_category_owner(mut self, role: &str) -> Self {
        self.config.budget_category_owner = Some(role.to_owned());
        self
    }

    /// The days after a payer bill is issued when `FIN.RUL.13` follows up a
    /// bill with no remittance (default 30; 05: 45). A value below 1 is
    /// read as 1.
    pub fn payer_follow_up_days(mut self, days: i64) -> Self {
        self.config.payer_follow_up_days = days.max(1);
        self
    }

    /// What extra hours of an adjustment timesheet on invoiced hours become
    /// (default an extra line on the next usage invoice).
    pub fn adjustment_plus(mut self, plus: AdjustmentPlus) -> Self {
        self.config.adjustment_plus = plus;
        self
    }

    /// The role whose task opens when the credit of an adjustment is refused
    /// (default `coordinator`).
    pub fn adjustment_task_role(mut self, role: &str) -> Self {
        self.config.adjustment_task_role = role.to_owned();
        self
    }

    /// Whether `FIN.RUL.06` suspends a subscription after its last retry
    /// (default on; 04 turns it off).
    pub fn subscription_suspension(mut self, on: bool) -> Self {
        self.config.subscription_suspension = on;
        self
    }

    /// The days after `mark_past_due` at which `FIN.RUL.06` retries or
    /// reminds (default 1, 3, 7). Sorted; zero and repeats are dropped.
    pub fn subscription_retry_days(mut self, days: &[u32]) -> Self {
        let mut steps: Vec<u32> = days.iter().copied().filter(|d| *d > 0).collect();
        steps.sort_unstable();
        steps.dedup();
        self.config.subscription_retry_days = steps;
        self
    }

    /// The days a user may pause a subscription per calendar year (default
    /// 0: no limit). A system pause never counts.
    pub fn subscription_pause_allowance(mut self, days: u32) -> Self {
        self.config.subscription_pause_allowance_days = days;
        self
    }

    /// Whether the party may cancel its subscription (default off).
    pub fn subscription_party_may_cancel(mut self, on: bool) -> Self {
        self.config.subscription_party_may_cancel = on;
        self
    }

    /// Whether a user or party cancel gives a final pro rata invoice or
    /// credit (default off).
    pub fn subscription_pro_rata_on_cancel(mut self, on: bool) -> Self {
        self.config.subscription_pro_rata_on_cancel = on;
        self
    }

    /// The module's settings.
    pub fn config(&self) -> &InvoicingConfig {
        &self.config
    }
}

/// Registers the integration connection of each entity with a go-live date
/// (`asp.<code>`): its health, the screens it affects and the manual
/// fallback. The transmissions are sent by [`einvoice_send`], which reports to
/// it; its own queue is not used, the retry times being the module's.
fn register_asp_connections(config: &InvoicingConfig) {
    use wl_kernel::integrations::{Delivery, IntegrationDef};
    for entry in &config.einvoice {
        IntegrationDef::of(
            &einvoice_send::connection_key(&entry.entity_code),
            "invoicing",
            |_state, _call| Box::pin(async { Ok(Delivery::Sent) }),
        )
        .owner_role("accountant")
        .screens(["invoicing.einvoices", "invoicing.invoice_list"])
        .fallback(
            "Send the e-invoices from the ASP portal by hand until the connection is back.",
            Some("أرسل الفواتير الإلكترونية يدويًا من بوابة مزود الخدمة حتى تعود الخدمة."),
        )
        .register();
    }
}

impl Module for WlInvoicing {
    fn name(&self) -> &'static str {
        "wl-invoicing"
    }

    fn routes(&self) -> Routes {
        pages::routes()
            .merge(vat_pages::routes())
            .merge(credit_note_pages::routes())
            .merge(ageing_pages::routes())
            .merge(dispute_pages::routes())
            .merge(task_pages::routes())
            .merge(bill_pages::routes())
            .merge(budget_pages::routes())
            .merge(budget_category_pages::routes())
            .merge(budget_report_pages::routes())
            .merge(cash_forecast_pages::routes())
            .merge(drawing_pages::routes())
            .merge(einvoice_pages::routes())
            .merge(subscription_pages::routes())
            .merge(payer_bill_pages::routes())
            .merge(rate_card_pages::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        // What the invoice's Related panel lists (BL-AGT-03), and the
        // relations of credit notes and parties (BL-AGT-04).
        relations::register();
        // What an AI assistant may do with an invoice, previewed and
        // confirmed by the user (BL-AGT-08).
        agent::register();
        // Invoices and credit notes in the global search (BL-AGT-04).
        search::register(app);
        // An invoice is looked after by the people of its scope unit (the
        // building it was billed for): the approvers of its write-off, credit
        // or dispute are looked for there, not only among the holders of a
        // global role. The scope unit's own scope is the vertical's
        // (`wl_kernel::scopes::register` for its subject type).
        wl_kernel::scopes::register(wl_kernel::invoicing::INVOICE_SUBJECT_TYPE, |db, id| {
            Box::pin(async move {
                let Some(invoice) = Invoice::find(db, id).await? else {
                    return Ok(renox::auth::permissions::Scope::global());
                };
                match (invoice.scope_type, invoice.scope_id) {
                    (Some(kind), Some(unit)) => {
                        wl_kernel::scopes::scope_of(db, &wl_kernel::Subject::new(&kind, unit)).await
                    }
                    _ => Ok(renox::auth::permissions::Scope::global()),
                }
            })
        });
        // Where a page names a payer agreement (a vertical's contract): its
        // name.
        wl_kernel::documents::register_subject_label(
            wl_kernel::invoicing::PAYER_AGREEMENT_SUBJECT_TYPE,
            |db, id| {
                Box::pin(async move {
                    let Some(agreement) = PayerAgreement::find(db, id).await? else {
                        return Ok(None);
                    };
                    Ok(Some(agreement.name))
                })
            },
        );
        // Where a page names an invoice (a cheque's documents): its number.
        wl_kernel::documents::register_subject_label(
            wl_kernel::invoicing::INVOICE_SUBJECT_TYPE,
            |db, id| {
                Box::pin(async move {
                    Ok(Invoice::find(db, id).await?.map(|invoice| {
                        invoice
                            .number
                            .clone()
                            .unwrap_or_else(|| format!("Draft #{}", invoice.id))
                    }))
                })
            },
        );
        // And the other way, for the lists: the invoices billed for the
        // buildings (or other scope units) a user holds a permission in.
        wl_kernel::scopes::register_derived_members(
            wl_kernel::invoicing::INVOICE_SUBJECT_TYPE,
            |db, scopes| {
                Box::pin(async move {
                    let reach = wl_kernel::scopes::Reach::only(scopes.to_vec());
                    let query = reach
                        .apply(db, Invoice::query(), "scope_type", "scope_id")
                        .await?;
                    Ok(query.get(db).await?.into_iter().map(|i| i.id).collect())
                })
            },
        );
        wl_kernel::scopes::register_derived_members(
            wl_kernel::invoicing::CREDIT_NOTE_SUBJECT_TYPE,
            |db, scopes| {
                Box::pin(async move {
                    let reach = wl_kernel::scopes::Reach::only(scopes.to_vec());
                    let invoices = reach
                        .ids(db, wl_kernel::invoicing::INVOICE_SUBJECT_TYPE)
                        .await?
                        .unwrap_or_default();
                    if invoices.is_empty() {
                        return Ok(Vec::new());
                    }
                    Ok(CreditNote::query()
                        .where_in("invoice_id", invoices)
                        .get(db)
                        .await?
                        .into_iter()
                        .map(|note| note.id)
                        .collect())
                })
            },
        );
        app.model::<cash_balance::OpeningCashBalance>();
        app.model::<cash_alert::CashAlert>();
        app.model::<EInvoiceTransmission>();
        app.model::<Invoice>()
            .model::<InvoiceLine>()
            .model::<SupplierBill>()
            .model::<BillLine>()
            .model::<SupplierDebitNote>()
            .model::<DebitNoteLine>()
            .model::<DebitNoteMovement>()
            .model::<duplicates::BillException>()
            .model::<CreditNote>()
            .model::<CreditNoteLine>()
            .model::<CreditNoteMovement>()
            .model::<Allocation>()
            .model::<BillAllocation>()
            .model::<BillUnallocation>()
            .model::<Recovery>()
            .model::<Unallocation>()
            .model::<InvoicingTask>()
            .model::<Dispute>()
            .model::<delivery_dispute::DeliveryReview>()
            .model::<delivery_billing::CodDifference>()
            .model::<EscalationLog>()
            .model::<ReminderLog>()
            .model::<VatAdjustment>()
            .model::<VatSummary>()
            .model::<BudgetCategory>()
            .model::<BudgetMapping>()
            .model::<CompanyBudget>()
            .model::<CompanyBudgetLine>()
            .model::<BudgetReminder>()
            .model::<BudgetWarning>()
            .model::<DrawingApproval>()
            .model::<UsageRecord>()
            .model::<UsageAdjustment>()
            .model::<rate_card::RateCard>()
            .model::<rate_card::RateCardVersion>()
            .model::<rate_card::RateCardLine>()
            .model::<rate_pricing::UsagePricing>()
            .model::<BillRecoding>()
            .model::<Subscription>()
            .model::<SubscriptionPause>()
            .model::<SubscriptionCycle>()
            .model::<PayerAgreement>()
            .model::<PayerBill>()
            .model::<PayerBillLine>()
            .model::<payer_rejection::PayerSubmissionError>()
            .model::<payer_remittance::PayerRemittance>()
            .model::<payer_remittance::PayerAllocation>()
            .model::<payer_recoupment::PayerRecoupment>()
            .model::<payer_credit::PayerCreditMovement>()
            .view_data::<rate_card_screens::CardPage>()
            .view_data::<rate_card_screens::VersionPage>()
            .view_data::<subscription_pages::IndexPage>()
            .view_data::<portal::PortalInvoicesPage>()
            .view_data::<portal::PortalStatementPage>()
            .view_data::<subscription_pages::ShowPage>()
            .view_data::<payer_bill_pages::IndexPage>()
            .view_data::<payer_bill_pages::ShowPage>()
            .view_data::<pages::IndexPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<vat_pages::IndexPage>()
            .view_data::<vat_pages::ShowPage>()
            .view_data::<credit_note_pages::IndexPage>()
            .view_data::<credit_note_pages::ShowPage>()
            .view_data::<ageing_pages::IndexPage>()
            .view_data::<task_pages::IndexPage>()
            .view_data::<bill_pages::IndexPage>()
            .view_data::<bill_pages::ShowPage>()
            .view_data::<bill_pages::FormPage>()
            .view_data::<budget_pages::IndexPage>()
            .view_data::<budget_pages::ShowPage>()
            .view_data::<budget_pages::FormPage>()
            .view_data::<budget_category_pages::IndexPage>()
            .view_data::<budget_category_pages::FormPage>()
            .view_data::<budget_report_pages::IndexPage>()
            .view_data::<cash_forecast_pages::IndexPage>()
            .view_data::<cash_forecast_pages::CellPage>()
            .view_data::<einvoice_pages::IndexPage>();
        // The customer write-off notice (§10), sent from the queue.
        app.job::<SendWriteOffNotice>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        // `rnx state:diagram Invoice` and the admin panel find the machine.
        renox::state_machine::register::<Invoice>();
        renox::state_machine::register::<CreditNote>();
        renox::state_machine::register::<VatSummary>();
        // The supplier bill machine, as the subject `invoicing.supplier_bill`.
        bill_machine::register();
        // The supplier debit note machine, as `invoicing.supplier_debit_note`.
        debit_note_machine::register();
        // The payer bill machine, as `invoicing.payer_bill`, and what follows
        // its committed transitions (tax documents, tasks, events).
        payer_bill_machine::register(&self.config);
        payer_bill_machine::register_events(app);
        // Verticals make and drive payer bills through the kernel (§6).
        wl_kernel::payer_bills::provide_payer_bills(app, PayerBilling);
        // The balance wl-payments reads before a refund of it is asked, and
        // the refund movements that follow its approval or its failure.
        wl_kernel::payer_bills::provide_payer_credits(app, payer_credit_refund::Credits);
        payer_credit_refund::register(app);
        // `FIN.RUL.17`, the cost reversal and the supplier notice of `issue`,
        // and the debit notes other modules draft and read through the kernel.
        debit_note_events::register_types();
        debit_note_events::register(app);
        wl_kernel::invoicing::provide_supplier_debit_notes(app, debit_note_contract::DebitNotes);
        // Which dates are in a filed VAT period, for the modules that post
        // to a past date.
        wl_kernel::invoicing::provide_period_locks(
            app,
            period_locks::VatPeriodLocks::new(self.config.clone()),
        );
        // The kernel runs `write_off_approved` by name; `FIN.ACL.15`.
        policy::register(&self.config);
        // The settings the services read (`config_of`).
        app.provide(self.config.clone());
        // `InvoiceIssued`, `InvoicePaid` and `InvoiceOverdue` after a commit.
        events::register(app);
        // Hours corrected after billing (§5.2 step 5).
        usage_adjustment::register(app);
        wl_kernel::usage::provide_usage_recorder(app, usage_billing::Recorder);
        // The customer notice and `InvoiceWrittenOff` after a write-off.
        write_off::register(app);
        // `db:seed`: the demo invoices, as drafts.
        app.seeder(|state: AppState| async move { seed::run(&state).await.map(|_| ()) });
        // The seeded budget categories (§7), kept as the client changed them.
        let owner = self.config.budget_category_owner.clone();
        app.seeder(move |state: AppState| {
            let owner = owner.clone();
            async move {
                budget_category::install_owned(&state.db, owner.as_deref())
                    .await
                    .map(|_| ())
            }
        });
        let owner = self.config.budget_category_owner.clone();
        app.command(
            "invoicing:install",
            "Store the seeded budget categories and their mapping, where the database lacks them",
            move |_args, state: AppState| {
                let owner = owner.clone();
                async move {
                    let created =
                        budget_category::install_owned(&state.db, owner.as_deref()).await?;
                    println!("{created} budget categories created.");
                    Ok(())
                }
            },
        );
        // In the app group of the staff sidebar; the label is the app's
        // `nav.invoices` text.
        app.nav(
            "app",
            NavLink::route("wl_invoicing.index", "nav.invoices")
                .icon("receipt")
                .group("nav.app")
                .order(40)
                .permission(permissions::VIEW),
        );
        // The credit notes, next to the invoices (`nav.credit_notes`).
        app.nav(
            "app",
            NavLink::route("wl_invoicing.credit_notes", "nav.credit_notes")
                .icon("file-text")
                .group("nav.app")
                .order(40)
                .permission(permissions::VIEW),
        );
        // The VAT summaries, next to the invoices (`nav.vat_summaries`).
        app.nav(
            "app",
            NavLink::route("wl_invoicing.vat_summaries", "nav.vat_summaries")
                .icon("percent")
                .group("nav.app")
                .order(41)
                .permission(permissions::VAT_VIEW),
        );
        // The receivables ageing (`nav.ageing`).
        app.nav(
            "app",
            NavLink::route("wl_invoicing.ageing", "nav.ageing")
                .icon("clock")
                .group("nav.app")
                .order(42)
                .permission(permissions::RECEIVABLES_VIEW),
        );
        // The open reversal tasks (`nav.reversal_tasks`).
        app.nav(
            "app",
            NavLink::route("wl_invoicing.tasks", "nav.reversal_tasks")
                .icon("clipboard-list")
                .group("nav.app")
                .order(43)
                .permission(permissions::RECEIVABLES_VIEW),
        );
        // The company budgets (`nav.budgets`).
        app.nav(
            "app",
            NavLink::route("wl_invoicing.budgets", "nav.budgets")
                .icon("chart-pie")
                .group("nav.app")
                .order(45)
                .permission(permissions::BUDGETS_VIEW),
        );
        // The budget vs actual report (`nav.budget_report`); the page checks
        // the permission against the user's entities.
        app.nav(
            "app",
            NavLink::route("wl_invoicing.budget_report", "nav.budget_report")
                .icon("chart-column")
                .group("nav.app")
                .order(46)
                .permission(permissions::BUDGETS_VIEW),
        );
        // The cash-flow forecast (`nav.cash_forecast`); the page checks the
        // permission against the user's entities.
        app.nav(
            "app",
            NavLink::route("wl_invoicing.cash_forecast", "nav.cash_forecast")
                .icon("trending-up")
                .group("nav.app")
                .order(47)
                .permission(permissions::CASHFLOW_VIEW),
        );
        // The e-invoice board (`nav.einvoices`).
        app.nav(
            "app",
            NavLink::route("wl_invoicing.einvoices", "nav.einvoices")
                .icon("upload")
                .group("nav.app")
                .order(48)
                .permission(permissions::EINVOICES_VIEW),
        );
        // The supplier bills (`nav.supplier_bills`).
        app.nav(
            "app",
            NavLink::route("wl_invoicing.supplier_bills", "nav.supplier_bills")
                .icon("file-text")
                .group("nav.app")
                .order(44)
                .permission(permissions::SUPPLIER_BILLS_VIEW),
        );
        // The payer bills (`nav.payer_bills`).
        app.nav(
            "app",
            NavLink::route("wl_invoicing.payer_bills", "nav.payer_bills")
                .icon("clipboard-list")
                .group("nav.app")
                .order(49)
                .permission(permissions::PAYER_BILLS_VIEW),
        );
        // `FIN.RUL.02`: the daily `invoicing:mark-overdue` task.
        overdue::schedule(app);
        // `FIN.RUL.01`: the reminder types and the daily `invoicing:reminders`.
        reminders::register_types();
        reminders::schedule(app);
        // The daily `invoicing:vat-periods`: each running period's summary.
        vat_summary::schedule(app);
        // `FIN.RUL.14`: the relief notification type and the daily
        // `invoicing:bad-debt-relief` (when the setting is on).
        bad_debt_relief::register_types();
        // `invoicing.bad_debt_recovered`: the accountant notice of a recovery.
        recover::register_types();
        bad_debt_relief::schedule(app, &self.config);
        // `invoicing.dispute_acknowledged` and `invoicing.dispute_unassigned`.
        dispute::register_types();
        // BL-TKT-12: a billing dispute without an owner goes to the
        // receivables role in the invoice's scope.
        wl_kernel::ticket_routing::register_ticket_router(
            dispute::TICKET_CATEGORY,
            dispute::DisputeRouter,
        );
        // `invoicing.dispute_resolved`, and the listener that resolves the
        // ticket and tells the party after `InvoiceDisputeResolved`.
        dispute_resolve::register_types();
        dispute_resolve::register(app);
        // A delivery under review flags its invoices, and its close, reversal
        // or loss resolves them or tasks the owner (#69).
        delivery_dispute::register(app);
        // `DeliveryDelivered` bills the delivered quantity and handles a COD
        // difference (#85).
        delivery_billing::register(app);
        // `invoicing.dispute_escalation`: the notice of `FIN.RUL.15`.
        escalation::register_types();
        escalation::schedule(app);
        // `invoicing.credit_note_restored`: the credit is available again.
        restore::register_types();
        // A failed refund paid out of a credit note gives its amount back.
        restore::register(app);
        // `invoicing.invoice_reopened`: the owner and `receivables` notice.
        reopen_notices::register_types();
        // `invoicing.reversal_task`: the accountant notice of a refused reversal.
        task::register_types();
        // A reversed payment reopens its invoices (`unallocate`).
        reversal::register(app);
        // Other modules draft and read invoices through the kernel (§6).
        wl_kernel::invoicing::provide_invoicer(app, Invoices);
        wl_kernel::invoice_search::provide_invoice_search(app, open_search::OpenInvoices);
        // Other modules read supplier bill lines through the kernel (§6).
        wl_kernel::invoicing::provide_supplier_bills(app, payables::Payables);
        // `SupplierBillApproved` / `SupplierBillVoided` after a commit, and
        // the accountant notice of a held possible duplicate.
        bill_events::register(app);
        bill_notices::register_types();
        // `invoicing.supplier_bill_reopened`: the accountant notice.
        bill_reopen_notices::register_types();
        // A reversed payment reopens its supplier bills (`unallocate`).
        bill_reversal::register(app);
        // The company budget machine, its approval policy and its event
        // (`FIN.BUD.01`), the budgets other modules read through the kernel,
        // and `FIN.RUL.26`'s notice and daily task.
        budget_machine::register(app);
        wl_kernel::budgets::provide_company_budgets(app, Budgets);
        budget_notices::register_types();
        budget_reminder::schedule(app);
        // `FIN.INV.01`: the ASP adapters and the scope, the transmission
        // machine, the connection of each entity with a go-live date, the
        // queue of transmissions and its scheduled run.
        app.provide(einvoice_adapter::Installed {
            adapters: self.adapters.clone(),
            scope: self.scope.clone().unwrap_or_else(|| {
                Arc::new(einvoice_adapter::DefaultScope) as Arc<dyn EInvoiceScope>
            }),
        });
        renox::state_machine::register::<EInvoiceTransmission>();
        register_asp_connections(&self.config);
        // The *Look up* of a company's recipient identifier (wl-parties)
        // asks the entity's ASP adapter.
        wl_kernel::einvoice::provide_recipient_lookup(
            app,
            einvoice_adapter::AdapterLookup::of(&self.config),
        );
        einvoice_send::register(app);
        // `FIN.RUL.23` and `FIN.RUL.25`: the notice type and the daily check
        // of the projected cash balance and of the bank balances.
        cash_alert::register_types();
        cash_alert::schedule(app, &self.config);
        // `FIN.RUL.24`: a bill that posts an actual may reach its budget.
        budget_variance::register(app);
        // `FIN.ACL.21`: the drawing approval machine and its policy, and the
        // final re-check and the notice of a corrected drawing.
        drawings_machine::register();
        drawings::register(app);
        // `FIN.ACL.21` for the payments: wl-payments asks for the approval of
        // a drawing through the kernel, and hears when it ends.
        wl_kernel::drawings::provide_owner_drawings(app, owner_drawings::Drawings);
        owner_drawings::register(app);
        // The Subscription (§4): its machine, the verticals' contract, the
        // block effect, the listeners (reopened and paid cycle invoices,
        // party blocks), the party notices and the daily
        // `invoicing:subscriptions` (billing and `FIN.RUL.06`).
        renox::state_machine::register::<Subscription>();
        // Rate cards (`FIN.RAT.01`, `MPW.ACL.16`): the version machine and its
        // approval policy, the contract the verticals ask, and the repricing
        // of unlocked hours when a version is accepted.
        rate_card::register();
        wl_kernel::rate_cards::provide_rate_cards(app, rate_card_contract::Cards);
        rate_cards::listen(app);
        wl_kernel::subscriptions::provide_subscriptions(app, SubscriptionBook);
        subscriptions::register(app);
        // The subscriptions (`nav.subscriptions`).
        app.nav(
            "app",
            NavLink::route("wl_invoicing.subscriptions", "nav.subscriptions")
                .icon("refresh-cw")
                .group("nav.app")
                .order(44)
                .permission(permissions::SUBSCRIPTIONS_VIEW),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builder_changes_the_defaults() {
        let module = WlInvoicing::new()
            .payment_terms_days(14)
            .reminder_steps(&[7, -3, 7])
            .write_off_reason("deceased", false)
            .write_off_reason("other", false)
            .write_off_notice(false)
            .write_off_approvers(&["admin"])
            .tax_period("PGR", TaxPeriod::Monthly)
            .tax_period("PGR", TaxPeriod::Quarterly { first_month: 2 })
            .bad_debt_relief_months(ReliefMonths::new(9).unwrap())
            .bad_debt_relief_run_at("03:30")
            .bad_debt_relief(false)
            .relief_notify_role("finance")
            .portal_disputes(true)
            .dispute_grace_days(10)
            .dispute_escalation_days(0)
            .dispute_escalation_role("cfo")
            .duplicate_check(false)
            .duplicate_keys(&[
                DuplicateKey::ContractPeriodAmount,
                DuplicateKey::SupplierInvoiceNumber,
                DuplicateKey::ContractPeriodAmount,
            ])
            .duplicate_notify_role("finance")
            .subscription_suspension(false)
            .subscription_retry_days(&[7, 0, 2, 7])
            .subscription_pause_allowance(30)
            .subscription_party_may_cancel(true)
            .subscription_pro_rata_on_cancel(true);
        let config = module.config();
        assert_eq!(config.payment_terms_days, 14);
        assert_eq!(config.reminder_steps, [-3, 7]);
        assert!(config.write_off_reason("deceased").is_some());
        assert!(!config.write_off_reason("other").unwrap().text_required);
        assert!(!config.write_off_notice);
        assert_eq!(config.write_off_approvers, ["admin"]);
        assert_eq!(
            config.tax_period_of("PGR"),
            TaxPeriod::Quarterly { first_month: 2 }
        );
        assert_eq!(config.tax_period_of("PGP"), TaxPeriod::default());
        assert_eq!(config.bad_debt_relief_months.get(), 9);
        assert_eq!(config.bad_debt_relief_run_at, "03:30");
        assert!(!config.bad_debt_relief);
        assert_eq!(config.relief_notify_role, "finance");
        assert!(config.portal_disputes);
        assert_eq!(config.dispute_grace_days, 10);
        assert_eq!(config.dispute_escalation_days, 1);
        assert_eq!(config.dispute_escalation_role, "cfo");
        assert!(!config.duplicate_check);
        assert_eq!(
            config.duplicate_keys,
            [
                DuplicateKey::ContractPeriodAmount,
                DuplicateKey::SupplierInvoiceNumber
            ]
        );
        assert_eq!(config.duplicate_notify_role, "finance");
        assert!(!config.subscription_suspension);
        assert_eq!(config.subscription_retry_days, [2, 7]);
        assert_eq!(config.subscription_pause_allowance_days, 30);
        assert!(config.subscription_party_may_cancel);
        assert!(config.subscription_pro_rata_on_cancel);
    }
}
