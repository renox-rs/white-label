//! Portal actions while the party is blocked (wl-portal §6, `PTY.ACL.02`,
//! `PTY.ACL.03`, issue #44).
//!
//! A portal user whose party is blocked must not learn it. Their grants stay
//! as they are, history, documents and statements stay visible, and paying
//! an open invoice still works (money in continues). The actions that start
//! new business or move money out are refused on submit with one neutral
//! text, the same whatever the reason ([`crate::texts::neutral`]); they stay
//! visible, because hiding them would reveal a change. The list is closed
//! ([`PortalAction`]): every other portal action works as usual.
//!
//! A page or a signed link that does one of these calls [`PortalGuard::require`]
//! before it writes. The block itself comes from the kernel's
//! [`BlockGuard`](wl_kernel::block_guard::BlockGuard), asked as the system
//! (no viewer), so the answer is the neutral one for everybody and a reason is
//! never at hand to leak. Each refusal is logged once for staff with
//! `parties.block_reason.view` ([`crate::refusals`]).

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::block_guard::{self, ActionKind, GuardOutcome};
use wl_kernel::documents::today;

use crate::access::PortalAccess;
use crate::refusals::RefusalLog;

/// The user id a refusal is logged with when it came from a signed link,
/// which has no logged-in user.
pub const LINK_USER: i64 = 0;

/// What a portal user may try to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortalAction {
    /// A new booking.
    NewBooking,
    /// A new order.
    NewOrder,
    /// Signing a new agreement (a signature request).
    SignAgreement,
    /// A request for a refund.
    RefundRequest,
    /// A request to release a deposit.
    DepositReleaseRequest,
    /// Cancelling a booking or order where the cancellation produces a refund.
    CancelWithRefund,
    /// Paying an open invoice: money in.
    PayInvoice,
    /// A maintenance request.
    MaintenanceRequest,
    /// A complaint.
    Complaint,
    /// A message to the company.
    Message,
    /// A document upload.
    DocumentUpload,
    /// A profile change.
    ProfileChange,
    /// Cancelling where the cancellation produces no refund.
    CancelWithoutRefund,
}

impl PortalAction {
    /// Every action.
    pub const ALL: [PortalAction; 13] = [
        PortalAction::NewBooking,
        PortalAction::NewOrder,
        PortalAction::SignAgreement,
        PortalAction::RefundRequest,
        PortalAction::DepositReleaseRequest,
        PortalAction::CancelWithRefund,
        PortalAction::PayInvoice,
        PortalAction::MaintenanceRequest,
        PortalAction::Complaint,
        PortalAction::Message,
        PortalAction::DocumentUpload,
        PortalAction::ProfileChange,
        PortalAction::CancelWithoutRefund,
    ];

    /// The stored key, e.g. `deposit_release_request`.
    pub fn key(self) -> &'static str {
        match self {
            PortalAction::NewBooking => "new_booking",
            PortalAction::NewOrder => "new_order",
            PortalAction::SignAgreement => "sign_agreement",
            PortalAction::RefundRequest => "refund_request",
            PortalAction::DepositReleaseRequest => "deposit_release_request",
            PortalAction::CancelWithRefund => "cancel_with_refund",
            PortalAction::PayInvoice => "pay_invoice",
            PortalAction::MaintenanceRequest => "maintenance_request",
            PortalAction::Complaint => "complaint",
            PortalAction::Message => "message",
            PortalAction::DocumentUpload => "document_upload",
            PortalAction::ProfileChange => "profile_change",
            PortalAction::CancelWithoutRefund => "cancel_without_refund",
        }
    }

    /// The action with this key.
    pub fn from_key(key: &str) -> Option<PortalAction> {
        Self::ALL.into_iter().find(|a| a.key() == key)
    }

    /// What the block guard is asked about: `Start` for new business,
    /// `MoneyOut` for a refund or a deposit release, `MoneyIn` for a payment.
    /// `None` for every other action: it is not new business and the block
    /// never refuses it.
    pub fn guarded_kind(self) -> Option<ActionKind> {
        match self {
            PortalAction::NewBooking | PortalAction::NewOrder | PortalAction::SignAgreement => {
                Some(ActionKind::Start)
            }
            PortalAction::RefundRequest
            | PortalAction::DepositReleaseRequest
            | PortalAction::CancelWithRefund => Some(ActionKind::MoneyOut),
            PortalAction::PayInvoice => Some(ActionKind::MoneyIn),
            PortalAction::MaintenanceRequest
            | PortalAction::Complaint
            | PortalAction::Message
            | PortalAction::DocumentUpload
            | PortalAction::ProfileChange
            | PortalAction::CancelWithoutRefund => None,
        }
    }

    /// Whether a blocked party's user is refused this action.
    pub fn refused_while_blocked(self) -> bool {
        self.guarded_kind().is_some_and(ActionKind::is_guarded)
    }
}

/// Why a portal action was not allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The user has no `active` grant for the party.
    NoAccess,
    /// The party is blocked and the action starts new business or moves money
    /// out: the neutral text, whatever the reason.
    Blocked,
}

/// The portal's block guard.
pub struct PortalGuard;

impl PortalGuard {
    /// Checks that `user` may do `action` for the party `acting_for`: they
    /// hold an `active` grant for it (the acting-for party of the current
    /// grant, not the user's own party), and the party is not blocked for an
    /// action that starts new business or moves money out. A refusal because
    /// of the block is logged.
    pub async fn check(
        state: &AppState,
        user: i64,
        acting_for: PartyId,
        action: PortalAction,
    ) -> Result<std::result::Result<(), Refused>> {
        if PortalAccess::active_grant(&state.db, user, acting_for, today())
            .await?
            .is_none()
        {
            return Ok(Err(Refused::NoAccess));
        }
        Self::block_check(state, user, acting_for, action).await
    }

    /// Asks the block about `action` for `acting_for`; a refusal is logged
    /// for `user` ([`LINK_USER`] for a signed link).
    async fn block_check(
        state: &AppState,
        user: i64,
        acting_for: PartyId,
        action: PortalAction,
    ) -> Result<std::result::Result<(), Refused>> {
        let Some(kind) = action.guarded_kind() else {
            return Ok(Ok(()));
        };
        let mut tx = state.db.begin().await?;
        // As the system, never as the user: the answer is the neutral one.
        let outcome = block_guard::guard(state)
            .check_in(&mut tx, None, acting_for, kind)
            .await?;
        match outcome {
            GuardOutcome::Allowed => Ok(Ok(())),
            GuardOutcome::Blocked(_) => {
                RefusalLog::record(&mut tx, user, acting_for, action).await?;
                tx.commit().await?;
                Ok(Err(Refused::Blocked))
            }
        }
    }

    /// [`require`](PortalGuard::require) for a signed link, which has no
    /// logged-in user: the signature is the credential, so only the block is
    /// asked. Paying an invoice is allowed; an action that starts new
    /// business or moves money out is a 422 with the neutral text, logged
    /// with the user [`LINK_USER`].
    pub async fn require_link(
        state: &AppState,
        lang: &Lang,
        acting_for: PartyId,
        action: PortalAction,
    ) -> Result {
        match Self::block_check(state, LINK_USER, acting_for, action).await? {
            Ok(()) => Ok(()),
            Err(_) => Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                crate::texts::neutral(lang),
            )),
        }
    }

    /// [`check`](PortalGuard::check) as an HTTP result: `Forbidden` without a
    /// grant, and for a blocked party a 422 with the neutral text in `lang`
    /// (the same for every reason, and for English and Arabic alike).
    pub async fn require(
        state: &AppState,
        lang: &Lang,
        user: &AuthUser,
        acting_for: PartyId,
        action: PortalAction,
    ) -> Result {
        match Self::check(state, user.id, acting_for, action).await? {
            Ok(()) => Ok(()),
            Err(Refused::NoAccess) => Err(Error::Forbidden),
            Err(Refused::Blocked) => Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                crate::texts::neutral(lang),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_of_refused_actions_is_closed() {
        let refused: Vec<&str> = PortalAction::ALL
            .into_iter()
            .filter(|a| a.refused_while_blocked())
            .map(PortalAction::key)
            .collect();
        assert_eq!(
            refused,
            [
                "new_booking",
                "new_order",
                "sign_agreement",
                "refund_request",
                "deposit_release_request",
                "cancel_with_refund",
            ]
        );
    }

    #[test]
    fn paying_an_invoice_is_money_in_and_never_refused() {
        assert_eq!(
            PortalAction::PayInvoice.guarded_kind(),
            Some(ActionKind::MoneyIn)
        );
        assert!(!PortalAction::PayInvoice.refused_while_blocked());
    }

    #[test]
    fn any_other_action_is_not_new_business() {
        for action in [
            PortalAction::MaintenanceRequest,
            PortalAction::Complaint,
            PortalAction::Message,
            PortalAction::DocumentUpload,
            PortalAction::ProfileChange,
            PortalAction::CancelWithoutRefund,
        ] {
            assert_eq!(action.guarded_kind(), None, "{}", action.key());
            assert!(!action.refused_while_blocked());
        }
    }

    #[test]
    fn a_cancellation_that_refunds_is_money_out() {
        assert_eq!(
            PortalAction::CancelWithRefund.guarded_kind(),
            Some(ActionKind::MoneyOut)
        );
    }

    #[test]
    fn keys_round_trip() {
        for action in PortalAction::ALL {
            assert_eq!(PortalAction::from_key(action.key()), Some(action));
        }
        assert_eq!(PortalAction::from_key("unknown"), None);
    }
}
