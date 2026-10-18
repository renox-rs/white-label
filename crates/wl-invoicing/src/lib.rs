#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod ageing;
mod ageing_pages;
pub mod allocate;
pub mod allocation;
pub mod bad_debt_relief;
pub mod config;
pub mod credit;
pub mod credit_note;
pub mod credit_note_line;
mod credit_note_machine;
pub mod credit_note_movement;
mod credit_note_pages;
pub mod credit_notes;
pub mod dispute;
pub mod dispute_credit;
mod dispute_pages;
pub mod dispute_resolve;
pub mod escalation;
pub mod escalation_log;
mod events;
pub mod invoice;
pub mod invoicer;
pub mod issue;
pub mod line;
mod machine;
pub mod overdue;
pub mod owner;
pub mod pages;
pub mod policy;
pub mod recover;
pub mod recovery;
pub mod refund;
pub mod reminders;
pub mod restore;
pub mod seed;
mod texts;
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
pub use config::{InvoicingConfig, ReasonProblem, ReliefMonths, SettingError, WriteOffReason};
pub use credit_note::{CreditNote, CreditNoteKind};
pub use credit_note_line::CreditNoteLine;
pub use credit_note_movement::{CreditNoteMovement, MovementKind};
pub use dispute::{Dispute, DisputeVia};
pub use escalation_log::EscalationLog;
pub use invoice::{Invoice, InvoiceKind, PartySnapshot};
pub use invoicer::Invoices;
pub use line::InvoiceLine;
pub use recovery::Recovery;
pub use reminders::ReminderLog;
pub use vat::{CategoryTotal, LineAmounts, VatTotals};
pub use vat_adjustment::{AdjustmentKind, AdjustmentStatus, VatAdjustment};
pub use vat_period::TaxPeriod;
pub use vat_summary::{VatSummary, VatSummaryStatus};
pub use write_off_notice::SendWriteOffNotice;

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
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
        "wl-invoicing/_adjustment_sheet.html",
        include_str!("../views/wl-invoicing/_adjustment_sheet.html"),
    ),
];

/// The module's settings in `state`: those `register` provided, else the
/// §7 defaults.
pub(crate) fn config_of(state: &AppState) -> Arc<InvoicingConfig> {
    state
        .provided::<InvoicingConfig>()
        .unwrap_or_else(|| Arc::new(InvoicingConfig::default()))
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
}

/// The invoicing module. The host adds it with
/// `.module(wl_invoicing::WlInvoicing::new())` when `client.toml` enables
/// `invoicing`.
#[derive(Default)]
pub struct WlInvoicing {
    config: InvoicingConfig,
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

    /// The upper day edges of the ageing buckets (default `[30, 60, 90]`:
    /// 0-30, 31-60, 61-90, 90+). Sorted; zero and repeats are dropped.
    pub fn ageing_buckets(mut self, edges: &[u32]) -> Self {
        let mut edges: Vec<u32> = edges.iter().copied().filter(|e| *e > 0).collect();
        edges.sort_unstable();
        edges.dedup();
        self.config.ageing_buckets = edges;
        self
    }

    /// The module's settings.
    pub fn config(&self) -> &InvoicingConfig {
        &self.config
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
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Invoice>()
            .model::<InvoiceLine>()
            .model::<CreditNote>()
            .model::<CreditNoteLine>()
            .model::<CreditNoteMovement>()
            .model::<Allocation>()
            .model::<Recovery>()
            .model::<Dispute>()
            .model::<EscalationLog>()
            .model::<ReminderLog>()
            .model::<VatAdjustment>()
            .model::<VatSummary>()
            .view_data::<pages::IndexPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<vat_pages::IndexPage>()
            .view_data::<vat_pages::ShowPage>()
            .view_data::<credit_note_pages::IndexPage>()
            .view_data::<credit_note_pages::ShowPage>()
            .view_data::<ageing_pages::IndexPage>();
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
        // The kernel runs `write_off_approved` by name; `FIN.ACL.15`.
        policy::register(&self.config);
        // The settings the services read (`config_of`).
        app.provide(self.config.clone());
        // `InvoiceIssued`, `InvoicePaid` and `InvoiceOverdue` after a commit.
        events::register(app);
        // The customer notice and `InvoiceWrittenOff` after a write-off.
        write_off::register(app);
        // `db:seed`: the demo invoices, as drafts.
        app.seeder(|state: AppState| async move { seed::run(&state).await.map(|_| ()) });
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
                .icon("receipt-text")
                .group("nav.app")
                .order(40)
                .permission(permissions::VIEW),
        );
        // The VAT summaries, next to the invoices (`nav.vat_summaries`).
        app.nav(
            "app",
            NavLink::route("wl_invoicing.vat_summaries", "nav.vat_summaries")
                .icon("landmark")
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
        // `invoicing.dispute_resolved`, and the listener that resolves the
        // ticket and tells the party after `InvoiceDisputeResolved`.
        dispute_resolve::register_types();
        dispute_resolve::register(app);
        // `invoicing.dispute_escalation`: the notice of `FIN.RUL.15`.
        escalation::register_types();
        // `invoicing.credit_note_restored`: the credit is available again.
        restore::register_types();
        // A failed refund paid out of a credit note gives its amount back.
        restore::register(app);
        // Other modules draft and read invoices through the kernel (§6).
        wl_kernel::invoicing::provide_invoicer(app, Invoices);
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
            .dispute_escalation_role("cfo");
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
    }
}
