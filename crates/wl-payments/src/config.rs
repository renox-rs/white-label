//! The module's settings (wl-payments §7). They are client defaults kept in
//! code, set on [`WlPayments`](crate::WlPayments) with builder calls; a
//! settings page comes later.

use wl_kernel::invoicing::VatCategory;

/// The payment methods enabled by default (§7): cash, transfer, cheque and
/// card at the counter.
pub const DEFAULT_METHODS: &[&str] = &["cash", "transfer", "cheque", "card"];

/// Hours after a gateway refund failed before `FIN.RUL.21` retries it (§7).
pub const DEFAULT_REFUND_RETRY_HOURS: i64 = 24;

/// How many times `FIN.RUL.21` and the accountant may retry one refund (§7).
pub const DEFAULT_REFUND_MAX_RETRIES: i64 = 1;

/// Days before a cheque's date when `FIN.RUL.03` puts it on the deposit list
/// (§7).
pub const DEFAULT_CHEQUE_LEAD_DAYS: i64 = 1;

/// Days after a cheque bounced when `FIN.RUL.18` reminds the party and the
/// responsible staff (§7).
pub const DEFAULT_CHEQUE_REMINDER_DAYS: i64 = 3;

/// Days after a cheque bounced when `FIN.RUL.18` leaves the accountant and
/// the manager the task to decide (§7).
pub const DEFAULT_CHEQUE_DECISION_DAYS: i64 = 14;

/// Days the party may dispute the published deposit deductions (§7).
pub const DEFAULT_DISPUTE_WINDOW_DAYS: i64 = 14;

/// Days staff have to answer a deductions dispute (§7).
pub const DEFAULT_DISPUTE_RESPONSE_DAYS: i64 = 7;

/// How many times one cheque may be presented, the first deposit included
/// (§7): 2 allows one re-presentation.
pub const DEFAULT_CHEQUE_MAX_PRESENTATIONS: i64 = 2;

/// The bank return reasons that still allow a criminal complaint (§7,
/// `⚠ open: lawyer L-21`).
pub const DEFAULT_CRIMINAL_GROUNDS: &[&str] = &[
    "account closed",
    "payment stopped by the drawer",
    "signature or alteration irregular",
];

/// The bounce fee of a received cheque (§7): the charge type of the client's
/// charge catalogue and the amount before VAT, in fils.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BounceFee {
    /// The charge type, e.g. `bounce_fee`.
    pub charge_type: String,
    /// The amount before VAT, in fils; above 0.
    pub amount: i64,
}

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
    /// Days before its date that a received cheque is put on the deposit list
    /// (`FIN.RUL.03`).
    pub cheque_lead_days: i64,
    /// The bounce fee added when a received cheque bounces; `None` (the §7
    /// default) adds none.
    pub bounce_fee: Option<BounceFee>,
    /// The VAT treatment of the bounce fee: standard rated by default
    /// (`⚠ open: accountant A-05`, §10).
    pub bounce_fee_vat: VatCategory,
    /// Days after a bounce that the party and the responsible staff are
    /// reminded (`FIN.RUL.18`, N).
    pub cheque_reminder_days: i64,
    /// Days after a bounce that the accountant and the manager get the task to
    /// decide (`FIN.RUL.18`, M).
    pub cheque_decision_days: i64,
    /// Whether the party may dispute the published deposit deductions in the
    /// portal (§7: off; 01 turns it on). While on, `refund` and `forfeit` wait
    /// for the dispute window of the latest published version.
    pub deductions_dispute: bool,
    /// Days after publication that the party may dispute.
    pub dispute_window_days: i64,
    /// Days staff have to answer a dispute.
    pub dispute_response_days: i64,
    /// The most presentations of one cheque, the first deposit included.
    pub cheque_max_presentations: i64,
    /// Whether a cheque collected in the field is deposited only after a
    /// cash handover confirms it (`FIN.ACL.20`, on by default).
    pub field_cheque_handover: bool,
    /// The return reasons that allow a criminal complaint, lower case
    /// (`refer_to_legal`).
    pub criminal_grounds: Vec<String>,
    /// The open amount a legal case needs to be above, in fils (§7: 0, any).
    pub legal_minimum_amount: i64,
}

impl PaymentsConfig {
    /// Whether `reason` (compared without case and outer spaces) is on the
    /// criminal-grounds list.
    pub fn is_criminal_ground(&self, reason: &str) -> bool {
        let reason = reason.trim().to_lowercase();
        self.criminal_grounds.iter().any(|r| *r == reason)
    }

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
            cheque_lead_days: DEFAULT_CHEQUE_LEAD_DAYS,
            bounce_fee: None,
            bounce_fee_vat: VatCategory::Standard,
            cheque_reminder_days: DEFAULT_CHEQUE_REMINDER_DAYS,
            cheque_decision_days: DEFAULT_CHEQUE_DECISION_DAYS,
            deductions_dispute: false,
            dispute_window_days: DEFAULT_DISPUTE_WINDOW_DAYS,
            dispute_response_days: DEFAULT_DISPUTE_RESPONSE_DAYS,
            cheque_max_presentations: DEFAULT_CHEQUE_MAX_PRESENTATIONS,
            field_cheque_handover: true,
            criminal_grounds: DEFAULT_CRIMINAL_GROUNDS
                .iter()
                .map(|r| (*r).to_owned())
                .collect(),
            legal_minimum_amount: 0,
        }
    }
}
