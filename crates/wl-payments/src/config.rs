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

/// Hours after the end of a holder's period when `FIN.RUL.22` gives the
/// manager the task to reach the holder (§7, N).
pub const DEFAULT_CASH_ESCALATE_HOURS: i64 = 12;

/// Hours after the end of a holder's period when `FIN.RUL.22` tells the
/// company admin and asks the manager to count what was received (§7, M).
pub const DEFAULT_CASH_ADMIN_HOURS: i64 = 48;

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
    /// The role that is the manager of a bounced cheque: it gets the decision
    /// task of `FIN.RUL.18` beside the accountant, is told when a cheque is
    /// referred to legal and, when [`Self::cheque_notify_manager`] is on, is
    /// told of the bounce and at the reminder (default `finance_manager`; 01
    /// Property: the property manager).
    pub cheque_manager_role: String,
    /// Whether the manager is also told when a received cheque bounces
    /// (`FIN.RUL.04`) and at the reminder of N days (`FIN.RUL.18`), not only
    /// given the decision task (default off; 01 Property turns it on).
    pub cheque_notify_manager: bool,
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
    /// Whether a cash shortage may be recovered by a salary deduction
    /// (`FIN.ACL.19`, §7: off, recovery in cash only; `⚠ open: lawyer L-20`).
    /// The code never decides it: only this setting does.
    pub cash_salary_deduction: bool,
    /// The cash shortage a user may write off without an approval, in fils
    /// (`FIN.ACL.06`, §7: 0, always an approval; 03 sets AED 50).
    pub cash_write_off_limit: i64,
    /// Hours after the end of a holder's period that the manager gets the task
    /// to reach the holder (`FIN.RUL.22`, N).
    pub cash_escalate_hours: i64,
    /// Hours after the end of the period that the company admin is told and
    /// the manager counts what was received (`FIN.RUL.22`, M).
    pub cash_admin_hours: i64,
    /// The role of the manager of a holder who has none recorded (default
    /// `finance_manager`; 03: the operations manager).
    pub cash_manager_role: String,
}

impl PaymentsConfig {
    /// Whether `reason` (compared without case and outer spaces) is on the
    /// criminal-grounds list.
    pub fn is_criminal_ground(&self, reason: &str) -> bool {
        let reason = reason.trim().to_lowercase();
        self.criminal_grounds.contains(&reason)
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
            cheque_manager_role: crate::followup::MANAGER_ROLE.to_owned(),
            cheque_notify_manager: false,
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
            cash_salary_deduction: false,
            cash_write_off_limit: 0,
            cash_escalate_hours: DEFAULT_CASH_ESCALATE_HOURS,
            cash_admin_hours: DEFAULT_CASH_ADMIN_HOURS,
            cash_manager_role: crate::followup::MANAGER_ROLE.to_owned(),
        }
    }
}
