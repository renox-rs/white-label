//! How a source asks for a delivery (wl-delivery §6 `DeliveryRequester`,
//! `ProofQuery`): [`Deliveries`] creates a `pending` delivery with its lines
//! from a [`DeliveryRequest`], taking the proof required and whether partial
//! acceptance is on from the settings of the source type, and cancels one
//! when the source is cancelled. It is the app's
//! [`DeliveryRequester`](wl_kernel::delivery::DeliveryRequester).

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::Subject;
use wl_kernel::delivery::{DeliveryFuture, DeliveryId, DeliveryRequest, DeliveryRequester};
use wl_kernel::documents::today;
use wl_kernel::tickets::TicketId;

use crate::delivery::{DELIVERY_SERIES, Delivery, DeliveryLine, DeliveryStatus, Direction, Proof};
use crate::plan::cancel_as_system;
use crate::review;
use crate::settings::settings;

/// The attempts a delivery gets (§7: 3); a perishable one gets one.
pub const DEFAULT_MAX_ATTEMPTS: i64 = 3;

/// The delivery service.
#[derive(Debug, Clone, Copy, Default)]
pub struct Deliveries;

fn unprocessable(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

impl Deliveries {
    /// Creates a `pending` delivery for `request`.
    ///
    /// # Errors
    ///
    /// 422 for a delivery without a recipient name or without items, an
    /// item without a positive quantity, or a negative amount to collect.
    pub async fn request(state: &AppState, request: DeliveryRequest) -> Result<Delivery> {
        if request.recipient_name.trim().is_empty() {
            return Err(unprocessable("A delivery needs the recipient's name."));
        }
        if request.items.is_empty() {
            return Err(unprocessable("A delivery needs at least one item."));
        }
        if request.items.iter().any(|item| item.quantity_milli <= 0) {
            return Err(unprocessable("Every item needs a quantity above zero."));
        }
        if request.collect_amount < 0 {
            return Err(unprocessable("The amount to collect cannot be negative."));
        }
        let settings = settings(state);
        let source_type = request.source.subject_type.clone();
        let (methods, any) = if request.pod_methods.is_empty() {
            settings.required_proof(&source_type)
        } else {
            (request.pod_methods.clone(), false)
        };
        let number = DELIVERY_SERIES
            .next_allowing_gaps(&state.db, None, None, today())
            .await?;
        let mut delivery = Delivery {
            number,
            direction: if request.pickup {
                Direction::Pickup
            } else {
                Direction::Delivery
            },
            source_type,
            source_id: request.source.subject_id,
            recipient_party_id: request.recipient_party.map(|party| party.0),
            recipient_name: request.recipient_name.trim().to_owned(),
            recipient_phone: request.recipient_phone.trim().to_owned(),
            address: request.address.trim().to_owned(),
            area: request.area.trim().to_owned(),
            window_from: request.window_from,
            window_to: request.window_to,
            instructions: request.instructions.trim().to_owned(),
            perishable: request.perishable,
            amount_to_collect: request.collect_amount,
            pod_methods: methods
                .iter()
                .map(|method| method.as_str())
                .collect::<Vec<_>>()
                .join(","),
            pod_any: any,
            partial_acceptance: settings.partial_acceptance(&request.source.subject_type),
            max_attempts: if request.perishable {
                1
            } else {
                DEFAULT_MAX_ATTEMPTS
            },
            status: DeliveryStatus::Pending,
            ..Delivery::default()
        };
        let mut tx = state.db.begin().await?;
        delivery.insert(&mut tx).await?;
        for item in &request.items {
            let mut line = DeliveryLine {
                delivery_id: delivery.id,
                source_line_type: item.source_line.as_ref().map(|s| s.subject_type.clone()),
                source_line_id: item.source_line.as_ref().map(|s| s.subject_id),
                description: item.description.trim().to_owned(),
                quantity_milli: item.quantity_milli,
                amount_to_collect: item.amount_to_collect,
                ..DeliveryLine::default()
            };
            line.insert(&mut tx).await?;
        }
        tx.commit().await?;
        Ok(delivery)
    }

    /// The latest proof of the delivery requested for `source`, if one was
    /// saved (`ProofQuery`, for documents, disputes and portals).
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn latest_proof(state: &AppState, source: &Subject) -> Result<Option<Proof>> {
        let delivery = Delivery::query()
            .where_(Delivery::SOURCE_TYPE.eq(source.subject_type.as_str()))
            .where_(Delivery::SOURCE_ID.eq(source.subject_id))
            .where_(Delivery::STATUS.eq(DeliveryStatus::Delivered))
            .order_by_desc(Delivery::ID)
            .first(&state.db)
            .await?;
        match delivery {
            Some(delivery) => delivery.latest_proof(&state.db).await,
            None => Ok(None),
        }
    }
}

impl DeliveryRequester for Deliveries {
    fn request<'a>(
        &'a self,
        state: &'a AppState,
        request: DeliveryRequest,
    ) -> DeliveryFuture<'a, DeliveryId> {
        Box::pin(async move { Ok(Deliveries::request(state, request).await?.delivery_id()) })
    }

    fn cancel<'a>(
        &'a self,
        state: &'a AppState,
        delivery: DeliveryId,
        reason: &'a str,
    ) -> DeliveryFuture<'a, ()> {
        Box::pin(async move {
            let delivery = Delivery::find_or_404(&state.db, delivery.0).await?;
            cancel_as_system(state, &delivery, reason)
                .await
                .map_err(TransitionError::into_error)
        })
    }

    fn update_address<'a>(
        &'a self,
        state: &'a AppState,
        delivery: DeliveryId,
        address: &'a str,
        area: &'a str,
    ) -> DeliveryFuture<'a, ()> {
        Box::pin(async move {
            let delivery = Delivery::find_or_404(&state.db, delivery.0).await?;
            if !matches!(
                delivery.status,
                DeliveryStatus::Pending | DeliveryStatus::Assigned | DeliveryStatus::Attempted
            ) {
                return Ok(());
            }
            let (address, area) = (address.to_owned(), area.to_owned());
            Delivery::query()
                .where_(Delivery::ID.eq(delivery.id))
                .update(&state.db, &[("address", &address), ("area", &area)])
                .await?;
            Ok(())
        })
    }

    fn open_review<'a>(
        &'a self,
        state: &'a AppState,
        delivery: DeliveryId,
        reason: &'a str,
        ticket: Option<TicketId>,
    ) -> DeliveryFuture<'a, ()> {
        Box::pin(async move {
            let delivery = Delivery::find_or_404(&state.db, delivery.0).await?;
            review::open_review_as_system(state, &delivery, reason, ticket)
                .await
                .map_err(TransitionError::into_error)
        })
    }

    fn is_frozen<'a>(
        &'a self,
        state: &'a AppState,
        delivery: DeliveryId,
    ) -> DeliveryFuture<'a, bool> {
        Box::pin(async move {
            Ok(Delivery::find(&state.db, delivery.0)
                .await?
                .is_some_and(|d| review::is_frozen(&d)))
        })
    }

    fn remittable_cod<'a>(
        &'a self,
        state: &'a AppState,
        delivery: DeliveryId,
    ) -> DeliveryFuture<'a, i64> {
        Box::pin(async move {
            match Delivery::find(&state.db, delivery.0).await? {
                Some(delivery) => review::remittable_cod(&state.db, &delivery).await,
                None => Ok(0),
            }
        })
    }
}
