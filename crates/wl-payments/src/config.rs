//! The module's settings (wl-payments §7). They are client defaults kept in
//! code, set on [`WlPayments`](crate::WlPayments) with builder calls; a
//! settings page comes later.

/// The payment methods enabled by default (§7): cash, transfer, cheque and
/// card at the counter.
pub const DEFAULT_METHODS: &[&str] = &["cash", "transfer", "cheque", "card"];

/// The settings of the payments module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentsConfig {
    /// The enabled payment method codes, e.g. `cash`.
    pub methods: Vec<String>,
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
        }
    }
}
