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
