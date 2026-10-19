//! The module's settings (wl-payments §7). They are client defaults kept in
//! code, set on [`WlPayments`](crate::WlPayments) with builder calls; a
//! settings page comes later.

/// The payment methods enabled by default (§7): cash, transfer, cheque and
/// card at the counter.
pub const DEFAULT_METHODS: &[&str] = &["cash", "transfer", "cheque", "card"];

/// Hours after a gateway refund failed before `FIN.RUL.21` retries it (§7).
pub const DEFAULT_REFUND_RETRY_HOURS: i64 = 24;

/// How many times `FIN.RUL.21` and the accountant may retry one refund (§7).
pub const DEFAULT_REFUND_MAX_RETRIES: i64 = 1;

/// The settings of the payments module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentsConfig {
    /// The enabled payment method codes, e.g. `cash`.
    pub methods: Vec<String>,
    /// Hours after a gateway refund failed before it is retried
    /// (`FIN.RUL.21`).
    pub refund_retry_hours: i64,
    /// The most retries of one refund, automatic and manual together.
    pub refund_max_retries: i64,
}

impl PaymentsConfig {
    /// Whether the method `code` is enabled.
    pub fn method_enabled(&self, code: &str) -> bool {
        self.methods.iter().any(|m| m == code)
    }
}

impl Default for PaymentsConfig {
    fn default() -> Self {
        Self {
            methods: DEFAULT_METHODS.iter().map(|m| (*m).to_owned()).collect(),
            refund_retry_hours: DEFAULT_REFUND_RETRY_HOURS,
            refund_max_retries: DEFAULT_REFUND_MAX_RETRIES,
        }
    }
}
