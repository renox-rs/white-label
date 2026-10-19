//! The owner drawings contract (ARCHITECTURE §5, wl-invoicing §5.16, wl-payments
//! §3 *Payment*, §5.7 step 3, `FIN.ACL.21`, `FIN.ACL.23`): what `wl-payments`
//! asks of the drawings approval that `wl-invoicing` owns, and what
//! `wl-invoicing` reads of the drawings payments `wl-payments` records.
//!
//! - [`OwnerDrawings`]: `wl-invoicing` provides it with
//!   [`provide_owner_drawings`]; [`owner_drawings`] falls back to
//!   [`NoOwnerDrawings`], which knows no drawings category and refuses a
//!   request. [`OwnerDrawings::category`] says whether a budget category is
//!   owner drawings and who its owner party is; [`OwnerDrawings::request`]
//!   makes the approval of a drawing on a record of the caller (the approval
//!   request goes to a holder of `drawings.approve` other than the
//!   requester, `PLT.APR.01`; a sole owner's own drawing is approved at once
//!   and flagged, `FIN.ACL.23`).
//! - [`OwnerDrawingDecided`]: emitted when a pending request ends, approved or
//!   not (a rejection, a withdrawal or an expiry). The caller records the
//!   drawing on an approval and records nothing otherwise.
//! - [`DrawingPaymentSource`]: the drawings payments of an entity between two
//!   dates, registered by name with [`register_drawing_payments`] (a
//!   process-wide registry, like [`crate::cash_forecast`]), because budget vs
//!   actual reads them with a database handle only.
//!
//! Money is `i64` fils of AED.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;
use crate::payments::{Direction, PaymentId};
use crate::subject::Subject;

/// The purpose of a payment that is an owner drawing (outgoing) or a repayment
/// by the owner (incoming).
pub const PURPOSE: &str = "owner_drawings";

/// The subject type of a drawing a payment record waits to make.
pub const DRAWING_REQUEST_SUBJECT_TYPE: &str = "payments.drawing_request";

/// A budget category of kind `owner_drawings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawingCategory {
    /// The category's code, e.g. `owner_drawings`.
    pub code: String,
    /// Its name in English.
    pub name_en: String,
    /// The owner party whose drawings it records; `None` until the client
    /// sets it.
    pub owner_party: Option<PartyId>,
}

/// A drawing to ask the approval for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawingRequest {
    /// The legal entity the drawing is in.
    pub legal_entity_id: i64,
    /// The code of the drawings category.
    pub category: String,
    /// The caller's record the drawing is on, e.g. a
    /// [`DRAWING_REQUEST_SUBJECT_TYPE`] subject.
    pub source: Subject,
    /// The drawing in AED fils.
    pub amount: i64,
    /// The user who asks.
    pub requested_by: i64,
}

/// Where a drawing stands right after [`OwnerDrawings::request`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrawingStart {
    /// Waiting for a holder of `drawings.approve`; [`OwnerDrawingDecided`]
    /// follows.
    Pending,
    /// Approved at once: the requester is the entity's only holder of
    /// `drawings.approve` (`FIN.ACL.23`, `self_approved`), or the amount is
    /// at or below the limit.
    Approved {
        /// Whether it was approved without a second approver.
        self_approved: bool,
    },
}

/// A pending drawing request ended (`PLT.APR.01`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerDrawingDecided {
    /// The caller's record the drawing is on.
    pub source: Subject,
    /// The legal entity.
    pub legal_entity_id: i64,
    /// Whether the last approver said yes.
    pub approved: bool,
    /// The user who gave the last yes, if approved.
    pub approved_by: Option<i64>,
    /// Why it was not approved: the approver's comment or the kernel's text.
    pub reason: Option<String>,
}

impl Event for OwnerDrawingDecided {}

/// The future the [`OwnerDrawings`] methods return.
pub type DrawingFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What other modules ask of the owner drawings approval. `wl-invoicing`
/// provides it.
pub trait OwnerDrawings: Send + Sync + 'static {
    /// The drawings category with this code, or `None` when there is none or
    /// it is not of kind `owner_drawings`. The default knows none.
    fn category<'a>(
        &'a self,
        _db: &'a Db,
        _code: &'a str,
    ) -> DrawingFuture<'a, Option<DrawingCategory>> {
        Box::pin(async { Ok(None) })
    }

    /// Asks for the approval of `request`. The default refuses (422): without
    /// the `invoicing` module no drawing can be approved.
    fn request<'a>(
        &'a self,
        _state: &'a AppState,
        _request: DrawingRequest,
    ) -> DrawingFuture<'a, DrawingStart> {
        Box::pin(async {
            Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Owner drawings need the invoicing module.".to_owned(),
            ))
        })
    }
}

/// The owner drawings when the `invoicing` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOwnerDrawings;

impl OwnerDrawings for NoOwnerDrawings {}

/// Makes `drawings` the app's [`OwnerDrawings`]; `wl-invoicing` calls it in
/// `register`.
pub fn provide_owner_drawings(app: &mut Registry, drawings: impl OwnerDrawings) {
    let drawings: Arc<dyn OwnerDrawings> = Arc::new(drawings);
    app.provide(drawings);
}

/// The app's [`OwnerDrawings`], or [`NoOwnerDrawings`] when none is provided.
pub fn owner_drawings(state: &AppState) -> Arc<dyn OwnerDrawings> {
    match state.provided::<Arc<dyn OwnerDrawings>>() {
        Some(drawings) => Arc::clone(&drawings),
        None => Arc::new(NoOwnerDrawings),
    }
}

/// A payment with purpose *owner drawings*: a drawing (outgoing) or a
/// repayment by the owner (incoming).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawingPayment {
    /// The payment.
    pub payment: PaymentId,
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The code of the drawings category.
    pub category: String,
    /// Out: a drawing; in: a repayment.
    pub direction: Direction,
    /// The amount in AED fils, above 0.
    pub amount: i64,
    /// The payment date.
    pub date: NaiveDate,
    /// The owner party.
    pub party: PartyId,
    /// Whether it was recorded without a second approver (`FIN.ACL.23`).
    pub self_approved: bool,
}

/// Reads the drawings payments for budget vs actual.
pub trait DrawingPaymentSource: Send + Sync + 'static {
    /// The recorded (not reversed) payments with purpose *owner drawings* of
    /// the legal entities `entities` dated from `from` to `to`, both
    /// included.
    fn payments<'a>(
        &'a self,
        db: &'a Db,
        entities: &'a [i64],
        from: NaiveDate,
        to: NaiveDate,
    ) -> DrawingFuture<'a, Vec<DrawingPayment>>;
}

static SOURCES: RwLock<BTreeMap<String, Arc<dyn DrawingPaymentSource>>> =
    RwLock::new(BTreeMap::new());

/// Adds `source` under `name` to the process-wide registry; registering the
/// same name again replaces it.
pub fn register_drawing_payments(name: &str, source: impl DrawingPaymentSource) {
    SOURCES
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(name.to_owned(), Arc::new(source));
}

/// The registered sources, sorted by name.
pub fn drawing_payment_sources() -> Vec<Arc<dyn DrawingPaymentSource>> {
    SOURCES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .map(Arc::clone)
        .collect()
}

/// Removes the source `name`, for tests.
pub fn remove_drawing_payments(name: &str) {
    SOURCES
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(name);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_purpose_and_the_subject_type_are_fixed_keys() {
        assert_eq!(PURPOSE, "owner_drawings");
        assert_eq!(DRAWING_REQUEST_SUBJECT_TYPE, "payments.drawing_request");
    }

    #[test]
    fn the_decided_event_round_trips() {
        let event = OwnerDrawingDecided {
            source: Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, 3),
            legal_entity_id: 1,
            approved: false,
            approved_by: None,
            reason: Some("not now".into()),
        };
        let json = renox::serde_json::to_value(&event).unwrap();
        assert_eq!(json["approved"], false);
        let back: OwnerDrawingDecided = renox::serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }
}
