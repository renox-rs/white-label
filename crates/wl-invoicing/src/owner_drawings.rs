//! The owner drawings contract of the kernel (wl-invoicing §5.16,
//! `FIN.ACL.21`, `FIN.ACL.23`): what `wl-payments` asks of the drawings
//! approval for a payment (or, later, a statement line) with purpose *owner
//! drawings*.
//!
//! [`Drawings`] is the module's [`OwnerDrawings`]: it says whether a budget
//! category is a drawings category and who its owner party is, and makes the
//! approval of a drawing on a record of the caller with
//! [`crate::drawings::open_request`], the same path a supplier bill's drawing
//! takes. When a request that waited ends, [`register`]'s listener emits
//! `OwnerDrawingDecided` for the caller. An approval made at once (the sole
//! owner, `FIN.ACL.23`, or below a limit) is answered by the request itself;
//! the sole owner's drawing also tells the accountant role.

use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::Transitioned;
use wl_kernel::drawings::{
    DRAWING_REQUEST_SUBJECT_TYPE, DrawingCategory, DrawingFuture, DrawingRequest, DrawingStart,
    OwnerDrawingDecided, OwnerDrawings,
};
use wl_kernel::notify::{self, NotifyRequest};
use wl_kernel::PartyId;

use crate::allocate::amount_text;
use crate::budget_category::{BudgetCategory, refused};
use crate::budget_notices::{DRAWING_PAYMENT_SELF_APPROVED, holders_of};
use crate::config_of;
use crate::drawings;
use crate::drawings_machine::{BACK_TO_DRAFT, DrawingApproval, DrawingStatus, REQUEST_APPROVED};
use crate::texts::english;

/// The module's [`OwnerDrawings`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Drawings;

impl OwnerDrawings for Drawings {
    fn category<'a>(
        &'a self,
        db: &'a Db,
        code: &'a str,
    ) -> DrawingFuture<'a, Option<DrawingCategory>> {
        Box::pin(async move {
            Ok(BudgetCategory::by_code(db, code.trim())
                .await?
                .filter(BudgetCategory::is_drawings)
                .map(|c| DrawingCategory {
                    code: c.code,
                    name_en: c.name_en,
                    owner_party: c.owner_party_id.map(PartyId),
                }))
        })
    }

    fn request<'a>(
        &'a self,
        state: &'a AppState,
        request: DrawingRequest,
    ) -> DrawingFuture<'a, DrawingStart> {
        Box::pin(async move {
            let category = self.category(&state.db, &request.category).await?;
            let Some(category) = category else {
                return Err(refused(english("wl_invoicing.drawings.problem.category")));
            };
            let (approval, created) = drawings::open_request(
                state,
                request.legal_entity_id,
                &request.source,
                request.amount,
                request.requested_by,
            )
            .await?;
            if created && approval.self_approved {
                tell_self_approved(state, &category, &approval).await?;
            }
            Ok(match approval.status {
                DrawingStatus::Approved => DrawingStart::Approved {
                    self_approved: approval.self_approved,
                },
                _ => DrawingStart::Pending,
            })
        })
    }
}

/// Tells the accountant role that the sole owner's drawing on a payment was
/// approved without a second approver (`FIN.ACL.23`).
async fn tell_self_approved(
    state: &AppState,
    category: &DrawingCategory,
    approval: &DrawingApproval,
) -> Result {
    let config = config_of(state);
    let recipients =
        holders_of(&state.db, std::slice::from_ref(&config.drawings_notify_role)).await?;
    if recipients.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(DRAWING_PAYMENT_SELF_APPROVED, approval.subject()).data(
        json!({
            "amount": amount_text("AED", approval.amount),
            "category": category.name_en,
            "link": "/payments",
        }),
    );
    for id in recipients {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// `OwnerDrawingDecided` for the approval `approval` of a drawing on another
/// module's record, or `None` for one on a supplier bill.
pub(crate) fn decided(approval: &DrawingApproval) -> Option<OwnerDrawingDecided> {
    if approval.source_type != DRAWING_REQUEST_SUBJECT_TYPE {
        return None;
    }
    Some(OwnerDrawingDecided {
        source: approval.source(),
        legal_entity_id: approval.legal_entity_id,
        approved: approval.status == DrawingStatus::Approved,
        approved_by: approval.approved_by,
        reason: approval.rejected_reason.clone(),
    })
}

/// Registers the listener that emits `OwnerDrawingDecided` when a waiting
/// request ends.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<DrawingApproval>, state: AppState| async move {
            if event.transition != REQUEST_APPROVED && event.transition != BACK_TO_DRAFT {
                return Ok(());
            }
            let approval = DrawingApproval::find_or_404(&state.db, event.key).await?;
            match decided(&approval) {
                Some(decision) => state.emit(decision).await,
                None => Ok(()),
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::Subject;

    fn approval(source_type: &str, status: DrawingStatus) -> DrawingApproval {
        DrawingApproval {
            id: 4,
            legal_entity_id: 2,
            source_type: source_type.to_owned(),
            source_id: 9,
            status,
            approved_by: Some(5),
            rejected_reason: Some("no".into()),
            ..Default::default()
        }
    }

    #[test]
    fn only_a_drawing_on_a_payment_request_is_announced() {
        assert!(decided(&approval("invoicing.supplier_bill", DrawingStatus::Approved)).is_none());
        let yes = decided(&approval(DRAWING_REQUEST_SUBJECT_TYPE, DrawingStatus::Approved))
            .expect("announced");
        assert!(yes.approved);
        assert_eq!(yes.source, Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, 9));
        assert_eq!((yes.legal_entity_id, yes.approved_by), (2, Some(5)));
        let no = decided(&approval(DRAWING_REQUEST_SUBJECT_TYPE, DrawingStatus::Rejected))
            .expect("announced");
        assert!(!no.approved);
        assert_eq!(no.reason.as_deref(), Some("no"));
    }
}
