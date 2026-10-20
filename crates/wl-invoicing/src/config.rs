//! The module's settings (wl-invoicing §7). They are client defaults kept in
//! code, set on [`WlInvoicing`](crate::WlInvoicing) with builder calls; a
//! settings page comes later.

use crate::vat_period::TaxPeriod;

/// A write-off reason code (`FIN.ACL.15`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOffReason {
    /// The code stored on the invoice, e.g. `insolvent`.
    pub code: String,
    /// Whether the requester must give a text with it (`other`).
    pub text_required: bool,
}

impl WriteOffReason {
    /// A reason with the given code.
    pub fn new(code: impl Into<String>, text_required: bool) -> Self {
        Self {
            code: code.into(),
            text_required,
        }
    }

    /// The key of its label in the texts: `wl_invoicing.write_off_reason.{code}`.
    pub fn text_key(&self) -> String {
        format!("wl_invoicing.write_off_reason.{}", self.code)
    }
}

/// The least months after the date of supply before bad-debt relief may be
/// proposed (UAE VAT rule: 6 months).
pub const MIN_RELIEF_MONTHS: u32 = 6;

/// A setting that is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingError(pub String);

impl std::fmt::Display for SettingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SettingError {}

/// The months after the date of supply a written-off invoice waits before
/// its bad-debt relief is proposed (`FIN.RUL.14`). Never below 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReliefMonths(u32);

impl ReliefMonths {
    /// `months`, or an error below [`MIN_RELIEF_MONTHS`].
    pub fn new(months: u32) -> Result<Self, SettingError> {
        if months < MIN_RELIEF_MONTHS {
            return Err(SettingError(
                "The months after the date of supply cannot be below 6".to_owned(),
            ));
        }
        Ok(Self(months))
    }

    /// The number of months.
    pub fn get(self) -> u32 {
        self.0
    }
}

impl Default for ReliefMonths {
    fn default() -> Self {
        Self(MIN_RELIEF_MONTHS)
    }
}

/// A key set the supplier bill duplicate check compares (`FIN.ACL.03`, §7).
/// Each enabled key set is checked on its own, always within one supplier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DuplicateKey {
    /// The same supplier invoice number, compared without surrounding
    /// spaces and ignoring case.
    SupplierInvoiceNumber,
    /// The same service contract, billing period, currency and gross total
    /// (`PRP.ACL.22`).
    ContractPeriodAmount,
}

/// The e-invoicing settings of one legal entity (`FIN.INV.01`, §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EInvoiceEntity {
    /// The legal entity's code.
    pub entity_code: String,
    /// The go-live date: e-invoicing applies to documents issued on or after
    /// it (Asia/Dubai).
    pub go_live: renox::chrono::NaiveDate,
    /// The name of the ASP adapter; `None` while none is chosen (documents
    /// are queued and wait).
    pub adapter: Option<String>,
}

/// The settings of the invoicing module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoicingConfig {
    /// Days from the issue date to the due date when a draft has no due date.
    pub payment_terms_days: u32,
    /// Reminder steps (`FIN.RUL.01`), in days relative to the due date:
    /// negative before it, positive after it. Kept in ascending order.
    pub reminder_steps: Vec<i32>,
    /// The reason codes a write-off may give.
    pub write_off_reasons: Vec<WriteOffReason>,
    /// Whether the customer write-off notice is sent on `write_off`.
    pub write_off_notice: bool,
    /// The roles whose holders approve a write-off by default (`FIN.ACL.15`).
    pub write_off_approvers: Vec<String>,
    /// The tax period of entities that have none of their own: quarterly
    /// on calendar quarters.
    pub default_tax_period: TaxPeriod,
    /// The tax period (the FTA stagger) of single entities, by entity code.
    pub tax_periods: Vec<(String, TaxPeriod)>,
    /// Whether bad-debt relief is proposed (`FIN.RUL.14`).
    pub bad_debt_relief: bool,
    /// When the relief task runs, `HH:MM` in Asia/Dubai.
    pub bad_debt_relief_run_at: String,
    /// The months after the date of supply before relief is proposed.
    pub bad_debt_relief_months: ReliefMonths,
    /// The role told when relief is proposed.
    pub relief_notify_role: String,
    /// Whether the party may dispute an invoice from the portal.
    pub portal_disputes: bool,
    /// Days chasing stays paused after an upheld dispute.
    pub dispute_grace_days: u32,
    /// The upper day edges of the ageing buckets, ascending: `[30, 60, 90]`
    /// gives 0-30, 31-60, 61-90 and 90+.
    pub ageing_buckets: Vec<u32>,
    /// Whole days between two escalation steps of an open dispute
    /// (`FIN.RUL.15`). Never below 1.
    pub dispute_escalation_days: u32,
    /// The role told from the second escalation step on.
    pub dispute_escalation_role: String,
    /// Whether a submitted supplier bill is checked for a duplicate
    /// (`FIN.ACL.03`).
    pub duplicate_check: bool,
    /// The key sets the duplicate check compares, each on its own.
    pub duplicate_keys: Vec<DuplicateKey>,
    /// The role told when a bill is held as a possible duplicate.
    pub duplicate_notify_role: String,
    /// The month a fiscal year of the company budget starts in, 1 to 12
    /// (`FIN.BUD.01`). A change applies from the next fiscal year that has
    /// no budget version yet.
    pub budget_fiscal_start_month: u32,
    /// Whether a new budget version may change the months that had ended
    /// when it was created (`FIN.BUD.01`); off, those lines are locked.
    pub budget_revise_past_months: bool,
    /// Days before a fiscal year starts that `FIN.RUL.26` asks for its
    /// budget.
    pub budget_reminder_days: u32,
    /// The roles told by `FIN.RUL.26`.
    pub budget_reminder_roles: Vec<String>,
    /// The share of a month's budget line at which `FIN.RUL.24` warns, in
    /// percent.
    pub budget_variance_percent: u32,
    /// The role `FIN.RUL.24` always tells besides the category's owner role.
    pub budget_variance_role: String,
    /// Whether the sole holder of `drawings.approve` of an entity records
    /// their own drawing without a second approver (`FIN.ACL.23`).
    pub drawings_self_approve: bool,
    /// The role told when a drawing is recorded without a second approver.
    pub drawings_notify_role: String,
    /// How many weeks the cash-flow forecast looks ahead (`FIN.CFL.01`).
    pub cash_forecast_weeks: u32,
    /// The first day of a forecast week.
    pub cash_week_start: renox::chrono::Weekday,
    /// Whether the VAT payable is counted in the forecast.
    pub cash_vat_counted: bool,
    /// Whether supplier bills not yet approved are counted.
    pub cash_unapproved_bills_counted: bool,
    /// Whether the disputed part of an invoice is counted as a receipt.
    pub cash_disputed_counted: bool,
    /// The days after which a bank balance is stale (a warning).
    pub cash_stale_days: i64,
    /// Whether receipts move by each party's average days late
    /// (`adjust by days late`, §5.14 step 6).
    pub cash_days_late: bool,
    /// The projected balance each legal entity (by code) wants to keep, in
    /// AED fils; an entity not listed has 0 (`FIN.RUL.23`).
    pub cash_thresholds: Vec<(String, i64)>,
    /// When the daily cash check (`FIN.RUL.23`, `FIN.RUL.25`) runs, `HH:MM`
    /// in Asia/Dubai.
    pub cash_alert_time: String,
    /// The roles `FIN.RUL.23` tells; a role nobody holds falls back to the
    /// company admin.
    pub cash_alert_roles: Vec<String>,
    /// Whether the income budget fill counts in the balance `FIN.RUL.23`
    /// checks.
    pub cash_income_fill_counted: bool,
    /// The e-invoicing settings of the entities that have a go-live date
    /// (`FIN.INV.01`).
    pub einvoice: Vec<EInvoiceEntity>,
    /// The minutes after `issue` at which a transmission the ASP did not take
    /// is tried again: 5, 30, 120 and 360.
    pub einvoice_retry_minutes: Vec<i64>,
    /// The hours between tries after the last of those, until it is sent: 6.
    pub einvoice_repeat_hours: i64,
    /// The hours after `issue` at which a transmission still `queued` or
    /// `submitted` raises a task (`FIN.RUL.16`): 24.
    pub einvoice_task_hours: i64,
    /// The bank accounts of each legal entity (by code) the forecast shows
    /// even before they have a balance.
    pub cash_accounts: Vec<(String, String)>,
    /// The source types (`<module>.<entity>`, e.g. `ticketing.ticket`) whose
    /// drafts the system may issue without a user (`Invoicer::issue` with no
    /// user). Empty by default: drafts are reviewed first.
    pub auto_issue_sources: Vec<String>,
}

impl Default for InvoicingConfig {
    /// The §7 defaults: 30 days' payment terms; reminders 3 days before and
    /// 1, 7 and 14 days after the due date; the five write-off reasons
    /// (`other` needs a text); the write-off notice on; approvers
    /// `receivables` and `admin`.
    fn default() -> Self {
        Self {
            payment_terms_days: 30,
            reminder_steps: vec![-3, 1, 7, 14],
            write_off_reasons: vec![
                WriteOffReason::new("insolvent", false),
                WriteOffReason::new("untraceable", false),
                WriteOffReason::new("uneconomic_to_pursue", false),
                WriteOffReason::new("settlement_agreed", false),
                WriteOffReason::new("other", true),
            ],
            write_off_notice: true,
            write_off_approvers: vec!["receivables".to_owned(), "admin".to_owned()],
            default_tax_period: TaxPeriod::default(),
            tax_periods: Vec::new(),
            bad_debt_relief: true,
            bad_debt_relief_run_at: "02:00".to_owned(),
            bad_debt_relief_months: ReliefMonths::default(),
            relief_notify_role: "accountant".to_owned(),
            portal_disputes: false,
            dispute_grace_days: 7,
            ageing_buckets: vec![30, 60, 90],
            dispute_escalation_days: 14,
            dispute_escalation_role: "finance_manager".to_owned(),
            duplicate_check: true,
            duplicate_keys: vec![DuplicateKey::SupplierInvoiceNumber],
            duplicate_notify_role: "accountant".to_owned(),
            budget_fiscal_start_month: 1,
            budget_revise_past_months: false,
            budget_reminder_days: 30,
            budget_reminder_roles: vec!["accountant".to_owned(), "finance_manager".to_owned()],
            budget_variance_percent: 100,
            budget_variance_role: "finance_manager".to_owned(),
            drawings_self_approve: true,
            drawings_notify_role: "accountant".to_owned(),
            cash_forecast_weeks: 13,
            cash_week_start: renox::chrono::Weekday::Mon,
            cash_vat_counted: true,
            cash_unapproved_bills_counted: true,
            cash_disputed_counted: false,
            cash_stale_days: 7,
            cash_days_late: false,
            cash_thresholds: Vec::new(),
            cash_alert_time: "06:00".to_owned(),
            cash_alert_roles: vec!["finance_manager".to_owned(), "admin".to_owned()],
            cash_income_fill_counted: false,
            einvoice: Vec::new(),
            einvoice_retry_minutes: vec![5, 30, 120, 360],
            einvoice_repeat_hours: 6,
            einvoice_task_hours: 24,
            cash_accounts: Vec::new(),
            auto_issue_sources: Vec::new(),
        }
    }
}

/// Why a write-off reason is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonProblem {
    /// The code is not one of the configured reasons.
    Unknown,
    /// The reason needs a text and none (or only blanks) was given.
    TextRequired,
}

impl ReasonProblem {
    /// The key of its message in the texts.
    pub fn text_key(self) -> &'static str {
        match self {
            ReasonProblem::Unknown => "wl_invoicing.write_off.unknown_reason",
            ReasonProblem::TextRequired => "wl_invoicing.write_off.reason_text_required",
        }
    }
}

impl InvoicingConfig {
    /// The e-invoicing settings of the entity with this code, if it has a
    /// go-live date.
    pub fn einvoice_of(&self, entity_code: &str) -> Option<&EInvoiceEntity> {
        self.einvoice.iter().find(|e| e.entity_code == entity_code)
    }

    /// Whether e-invoicing applies to the entity on `date`: its go-live date
    /// has come.
    pub fn einvoice_applies(&self, entity_code: &str, date: renox::chrono::NaiveDate) -> bool {
        self.einvoice_of(entity_code)
            .is_some_and(|e| e.go_live <= date)
    }

    /// How long after `issue` the transmission is tried for the
    /// `failed`-th time, `failed` from 1: the configured minutes, then every
    /// [`InvoicingConfig::einvoice_repeat_hours`] after the last of them
    /// (5 min, 30 min, 2 h, 6 h, then 12 h, 18 h, 24 h, ...).
    pub fn einvoice_retry_after(&self, failed: i64) -> renox::chrono::Duration {
        let steps = &self.einvoice_retry_minutes;
        let failed = failed.max(1);
        let index = usize::try_from(failed - 1).unwrap_or(usize::MAX);
        if let Some(minutes) = steps.get(index) {
            return renox::chrono::Duration::minutes(*minutes);
        }
        let last = steps.last().copied().unwrap_or(0);
        let beyond = failed - i64::try_from(steps.len()).unwrap_or(i64::MAX);
        renox::chrono::Duration::minutes(last)
            + renox::chrono::Duration::hours(self.einvoice_repeat_hours.max(1) * beyond)
    }

    /// The projected balance the entity with this code wants to keep, in AED
    /// fils (default 0).
    pub fn cash_threshold_of(&self, entity_code: &str) -> i64 {
        self.cash_thresholds
            .iter()
            .find(|(code, _)| code == entity_code)
            .map_or(0, |(_, threshold)| *threshold)
    }

    /// The tax period of the entity with this code.
    pub fn tax_period_of(&self, entity_code: &str) -> TaxPeriod {
        self.tax_periods
            .iter()
            .find(|(code, _)| code == entity_code)
            .map_or(self.default_tax_period, |(_, period)| *period)
    }

    /// The configured reason with this code, if any.
    pub fn write_off_reason(&self, code: &str) -> Option<&WriteOffReason> {
        self.write_off_reasons.iter().find(|r| r.code == code)
    }

    /// Checks a write-off's reason (`write_off` guards): `code` is one of
    /// the configured reasons, and a reason that needs a text (`other`)
    /// has a non-blank one.
    pub fn check_write_off_reason(
        &self,
        code: &str,
        text: Option<&str>,
    ) -> Result<&WriteOffReason, ReasonProblem> {
        let reason = self
            .write_off_reason(code.trim())
            .ok_or(ReasonProblem::Unknown)?;
        let blank = text.is_none_or(|text| text.trim().is_empty());
        if reason.text_required && blank {
            return Err(ReasonProblem::TextRequired);
        }
        Ok(reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_spec_ones() {
        let config = InvoicingConfig::default();
        assert_eq!(config.payment_terms_days, 30);
        assert_eq!(config.reminder_steps, [-3, 1, 7, 14]);
        assert!(config.write_off_notice);
        assert_eq!(config.write_off_approvers, ["receivables", "admin"]);
        assert!(config.write_off_reason("other").unwrap().text_required);
        assert!(!config.write_off_reason("insolvent").unwrap().text_required);
        assert!(config.write_off_reason("bored").is_none());
        assert_eq!(
            config.tax_period_of("PGP"),
            TaxPeriod::Quarterly { first_month: 1 }
        );
        assert!(config.bad_debt_relief);
        assert_eq!(config.bad_debt_relief_run_at, "02:00");
        assert_eq!(config.bad_debt_relief_months.get(), 6);
        assert_eq!(config.relief_notify_role, "accountant");
        assert!(!config.portal_disputes);
        assert_eq!(config.dispute_grace_days, 7);
        assert_eq!(config.ageing_buckets, [30, 60, 90]);
        assert_eq!(config.dispute_escalation_days, 14);
        assert_eq!(config.dispute_escalation_role, "finance_manager");
        assert!(config.duplicate_check);
        assert_eq!(config.duplicate_keys, [DuplicateKey::SupplierInvoiceNumber]);
        assert_eq!(config.duplicate_notify_role, "accountant");
        assert!(config.auto_issue_sources.is_empty());
        assert_eq!(config.budget_fiscal_start_month, 1);
        assert!(!config.budget_revise_past_months);
        assert_eq!(config.budget_reminder_days, 30);
        assert_eq!(
            config.budget_reminder_roles,
            ["accountant", "finance_manager"]
        );
        assert_eq!(config.budget_variance_percent, 100);
        assert_eq!(config.budget_variance_role, "finance_manager");
        assert_eq!(config.cash_forecast_weeks, 13);
        assert_eq!(config.cash_week_start, renox::chrono::Weekday::Mon);
        assert!(config.cash_vat_counted && config.cash_unapproved_bills_counted);
        assert!(!config.cash_disputed_counted);
        assert_eq!(config.cash_stale_days, 7);
        assert!(!config.cash_days_late);
        assert_eq!(config.cash_threshold_of("PGP"), 0);
        assert_eq!(config.cash_alert_time, "06:00");
        assert_eq!(config.cash_alert_roles, ["finance_manager", "admin"]);
        assert!(!config.cash_income_fill_counted);
        assert!(config.einvoice.is_empty());
        assert_eq!(config.einvoice_retry_minutes, [5, 30, 120, 360]);
        assert_eq!(
            (config.einvoice_repeat_hours, config.einvoice_task_hours),
            (6, 24)
        );
        assert!(config.drawings_self_approve);
        assert_eq!(config.drawings_notify_role, "accountant");
    }

    #[test]
    fn the_retries_are_measured_from_issue_and_then_every_six_hours() {
        let config = InvoicingConfig::default();
        let minutes = |failed: i64| config.einvoice_retry_after(failed).num_minutes();
        // Issued at 10:00: 10:05, 10:30, 12:00, 16:00, 22:00, 04:00, ...
        assert_eq!(minutes(1), 5);
        assert_eq!(minutes(2), 30);
        assert_eq!(minutes(3), 120);
        assert_eq!(minutes(4), 360);
        assert_eq!(minutes(5), 720);
        assert_eq!(minutes(6), 1_080);
        assert_eq!(minutes(7), 1_440);
        assert_eq!(minutes(0), 5);
    }

    #[test]
    fn e_invoicing_applies_from_the_go_live_date() {
        let mut config = InvoicingConfig::default();
        let go_live = renox::chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
        config.einvoice.push(EInvoiceEntity {
            entity_code: "PGP".into(),
            go_live,
            adapter: Some("peppol".into()),
        });
        let day = |m, d| renox::chrono::NaiveDate::from_ymd_opt(2027, m, d).unwrap();
        assert!(config.einvoice_applies("PGP", day(1, 1)));
        assert!(config.einvoice_applies("PGP", day(1, 5)));
        assert!(!config.einvoice_applies(
            "PGP",
            renox::chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()
        ));
        assert!(!config.einvoice_applies("PGFZ", day(1, 5)));
    }

    #[test]
    fn relief_months_are_never_below_six() {
        let error = ReliefMonths::new(5).unwrap_err();
        assert_eq!(
            error.to_string(),
            "The months after the date of supply cannot be below 6"
        );
        assert_eq!(ReliefMonths::new(6).unwrap().get(), 6);
        assert_eq!(ReliefMonths::new(12).unwrap().get(), 12);
    }

    #[test]
    fn a_reason_must_be_configured_and_other_needs_a_text() {
        let config = InvoicingConfig::default();
        assert_eq!(
            config
                .check_write_off_reason("insolvent", None)
                .unwrap()
                .code,
            "insolvent"
        );
        assert_eq!(
            config.check_write_off_reason("bored", None).unwrap_err(),
            ReasonProblem::Unknown
        );
        assert_eq!(
            config.check_write_off_reason("", Some("x")).unwrap_err(),
            ReasonProblem::Unknown
        );
        assert_eq!(
            config.check_write_off_reason("other", None).unwrap_err(),
            ReasonProblem::TextRequired
        );
        assert_eq!(
            config
                .check_write_off_reason("other", Some("  "))
                .unwrap_err(),
            ReasonProblem::TextRequired
        );
        assert!(
            config
                .check_write_off_reason("other", Some("Moved abroad"))
                .is_ok()
        );
    }
}
