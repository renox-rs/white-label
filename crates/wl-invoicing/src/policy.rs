//! The module's approval set-up (wl-invoicing §6, `FIN.ACL.15`,
//! ACCESS-CONTROL §5).
//!
//! The Invoice machine is registered with the kernel as `invoicing.invoice`
//! with no waiting state and no scope (approvers are found globally) and no
//! reserve or release handler: a deferred write-off leaves the invoice in
//! its state, and the pending badge comes from
//! `wl_kernel::approvals::pending_for`.
//!
//! `FIN.ACL.15` is a default policy: a write-off whose amount is above
//! AED 0 (every one) needs one approval by a holder of one of the
//! configured approver roles (`receivables`, `admin`), never the requester
//! (SoD, which this module never turns off), within 168 hours. The limit is
//! changed on the stored policy (`wl_approval_policies`), never in code.

use wl_kernel::approvals::PolicyDefault;
use wl_kernel::invoicing::{CREDIT_NOTE_SUBJECT_TYPE, INVOICE_SUBJECT_TYPE};
use wl_kernel::machines::SubjectMachine;

use crate::config::InvoicingConfig;
use crate::credit_note::CreditNote;
use crate::dispute_credit;
use crate::invoice::Invoice;
use crate::permissions;
use crate::write_off::ACTION;

/// The default write-off limit (`FIN.ACL.15`), in fils: AED 0, so every
/// write-off is approved.
pub const WRITE_OFF_LIMIT: i64 = 0;

/// Hours a write-off request may wait before it can expire.
pub const WRITE_OFF_EXPIRY_HOURS: i64 = 168;

/// The `FIN.ACL.15` default policy for `config`'s approver roles.
pub fn write_off_policy(config: &InvoicingConfig) -> PolicyDefault {
    PolicyDefault::new(ACTION)
        .amount_above(WRITE_OFF_LIMIT)
        .step(config.write_off_approvers.iter().cloned())
        .expiry_hours(Some(WRITE_OFF_EXPIRY_HOURS))
}

/// The default dispute credit limit (`FIN.ACL.07`), in fils: AED 0, so every
/// credit is approved (01 §3 "above 0%"); the limit is changed on the stored
/// policy.
pub const DISPUTE_CREDIT_LIMIT: i64 = 0;

/// Hours a dispute credit request may wait before it can expire.
pub const DISPUTE_CREDIT_EXPIRY_HOURS: i64 = 168;

/// The role besides the holders of `invoices.credit` that approves a dispute
/// credit.
pub const DISPUTE_CREDIT_ROLE: &str = "admin";

/// The `FIN.ACL.07` default policy: a dispute credit above the limit needs
/// one approval by an `admin` or a holder of `invoices.credit`, never the
/// requester (SoD), within 168 hours.
pub fn dispute_credit_policy() -> PolicyDefault {
    PolicyDefault::new(dispute_credit::ACTION)
        .amount_above(DISPUTE_CREDIT_LIMIT)
        .step_or_permission([DISPUTE_CREDIT_ROLE], permissions::CREDIT)
        .expiry_hours(Some(DISPUTE_CREDIT_EXPIRY_HOURS))
}

/// Registers the machines with the kernel and the default policies.
pub(crate) fn register(config: &InvoicingConfig) {
    SubjectMachine::of::<Invoice>(INVOICE_SUBJECT_TYPE).register();
    // The kernel runs `issue_approved` on the credit note by name.
    SubjectMachine::of::<CreditNote>(CREDIT_NOTE_SUBJECT_TYPE).register();
    write_off_policy(config).register();
    dispute_credit_policy().register();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_write_off_runs_under_the_fin_acl_15_action() {
        let machine = SubjectMachine::of::<Invoice>(INVOICE_SUBJECT_TYPE).build();
        assert_eq!(
            machine.action_key(crate::write_off::WRITE_OFF),
            "invoicing.write_off"
        );
        assert_eq!(machine.waiting_state(), None);
        assert!(!machine.has_scope());
        assert!(!machine.has_reserve() && !machine.has_release());
    }

    #[test]
    fn the_credit_note_machine_is_registered_with_its_approved_issue() {
        let machine = SubjectMachine::of::<CreditNote>(CREDIT_NOTE_SUBJECT_TYPE).build();
        assert_eq!(machine.subject_type(), "invoicing.credit_note");
        assert_eq!(machine.waiting_state(), None);
        let approved = machine
            .transition("issue_approved")
            .expect("issue_approved");
        assert!(approved.system);
        // The back office's own `issue` is not gated by FIN.ACL.07.
        assert_eq!(machine.action_key("issue"), "invoicing.issue");
    }

    #[test]
    fn every_dispute_credit_needs_an_approver_other_than_the_requester() {
        let expected = PolicyDefault::new("invoicing.dispute_credit")
            .amount_above(0)
            .step_or_permission(["admin"], "invoices.credit")
            .expiry_hours(Some(168));
        assert_eq!(dispute_credit_policy(), expected);
        assert_eq!(dispute_credit_policy().action(), dispute_credit::ACTION);
    }

    #[test]
    fn every_write_off_needs_an_approver_other_than_the_requester() {
        let policy = write_off_policy(&InvoicingConfig::default());
        let expected = PolicyDefault::new("invoicing.write_off")
            .amount_above(0)
            .step(["receivables", "admin"])
            .expiry_hours(Some(168));
        assert_eq!(policy, expected);
        assert_eq!(policy.action(), ACTION);
    }
}
