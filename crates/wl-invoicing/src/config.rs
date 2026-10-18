//! The module's settings (wl-invoicing §7). They are client defaults kept in
//! code, set on [`WlInvoicing`](crate::WlInvoicing) with builder calls; a
//! settings page comes later.

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
