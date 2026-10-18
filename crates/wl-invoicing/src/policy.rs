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
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::machines::SubjectMachine;

use crate::config::InvoicingConfig;
use crate::invoice::Invoice;
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

/// Registers the machine with the kernel and the default policy.
pub(crate) fn register(config: &InvoicingConfig) {
    SubjectMachine::of::<Invoice>(INVOICE_SUBJECT_TYPE).register();
    write_off_policy(config).register();
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
