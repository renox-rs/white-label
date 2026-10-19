//! The owner drawing actions on a supplier bill's page (wl-invoicing §5.16,
//! `FIN.ACL.21`):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /drawing-approvals/{id}` | `wl_invoicing.drawing_approval` | `supplier_bills.view` |
//! | `POST /supplier-bills/{id}/drawings/request` | `wl_invoicing.drawing_request` | `supplier_bills.create` |
//! | `POST /supplier-bills/{id}/recode` | `wl_invoicing.bill_recode` | `supplier_bills.create` |
//!
//! The first sends an approver to the bill, or the payment drawing request, the
//! approval is about (the
//! "Needs your action" list links to it). The second asks again for the
//! approval of the bill's drawing (a rejected drawing whose line stays a
//! drawing, or one whose amount changed, needs a new request). The third
//! codes a line of a bill that is not approved to another cost code; it is
//! logged and shown to whoever approves the bill. The decisions themselves
//! are the kernel's approval card buttons.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::validation::{Errors, ValidationError};

use crate::drawings::{self, request_for_bill};
use crate::drawings_machine::DrawingApproval;
use crate::permissions;
use crate::supplier_bill::SupplierBill;
use crate::texts;

/// The module's drawing routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/drawing-approvals/{id}", open)
                .name("wl_invoicing.drawing_approval")
                .require_permission(permissions::SUPPLIER_BILLS_VIEW),
        )
        .merge(
            Routes::new()
                .post("/supplier-bills/{id}/drawings/request", request)
                .name("wl_invoicing.drawing_request")
                .post("/supplier-bills/{id}/recode", recode)
                .name("wl_invoicing.bill_recode")
                .require_permission(permissions::SUPPLIER_BILLS_CREATE),
        )
}

/// `GET /drawing-approvals/{id}`: the bill, or the payment request, the
/// approval is about.
async fn open(Found(approval): Found<DrawingApproval>) -> Redirect {
    if approval.source_type == wl_kernel::drawings::DRAWING_REQUEST_SUBJECT_TYPE {
        return Redirect::to(&format!("/payments/drawings/{}", approval.source_id));
    }
    Redirect::to(&format!("/supplier-bills/{}", approval.source_id))
}

/// `POST /supplier-bills/{id}/drawings/request`: asks for the approval of the
/// bill's drawing and reloads the page; a refusal comes back as a message.
async fn request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<SupplierBill>,
) -> Result<Response> {
    let toast = match request_for_bill(&state, bill.id, user.id).await {
        Ok(Some(_)) => Toast::success(texts::t(&lang, "wl_invoicing.drawings.requested")),
        Ok(None) => Toast::info(texts::t(&lang, "wl_invoicing.drawings.none")),
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => Toast::error(message),
        Err(other) => return Err(other),
    };
    Ok((toast, Redirect::route("wl_invoicing.supplier_bill", &[&bill.id])?).into_response())
}

/// A 422 with `message` under `field`, keeping the input.
fn invalid(field: &str, message: String, form: &BTreeMap<String, String>) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// `POST /supplier-bills/{id}/recode`: codes a line to another cost code.
async fn recode(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<SupplierBill>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let line_id: i64 = get("line_id").parse().map_err(|_| {
        invalid(
            "line_id",
            texts::t(&lang, "wl_invoicing.drawings.errors.line"),
            &form,
        )
    })?;
    drawings::recode_line(&state, bill.id, line_id, get("cost_code"), Some(user.id))
        .await
        .map_err(|e| match e {
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => {
                invalid("cost_code", message, &form)
            }
            other => other,
        })?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.drawings.recoded")),
        renox::HxRefresh,
    )
        .into_response())
}
