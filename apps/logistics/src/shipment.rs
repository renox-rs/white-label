//! The Shipment, its parcels, tracking events and charge lines (03 Logistics
//! §6, §7.1 *Shipment*). A shipment's status moves only through the machine
//! in [`crate::shipment_machine`]; this is the base that BL-LOG-01 (void),
//! BL-LOG-02 (cancel after pickup), BL-LOG-03 (partial pickup), BL-LOG-04
//! (re-weigh) and BL-LOG-05 (hold) build on.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::{PartyId, Subject};

/// The subject type of a shipment, for [`Subject`] references and as the
/// source of its delivery.
pub const SUBJECT_TYPE: &str = "logistics.shipment";

/// `SHP-{0000}`: the number a shipment keeps, also when it is voided.
pub const SHIPMENT_SERIES: Series = Series {
    key: "shipment",
    pattern: "{prefix}-{0000}",
    prefix: "SHP",
    reset: Reset::Never,
    gapless: false,
    per_entity: false,
};

/// Where a shipment is in its life (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShipmentStatus {
    /// Booked, label printed, not yet collected.
    #[default]
    Created,
    /// Collected from the client.
    PickedUp,
    /// Scanned in at a depot.
    AtHub,
    /// On a dispatched run.
    OutForDelivery,
    /// Delivery failed; waiting for reattempt or return.
    Attempted,
    /// Exception: damaged, address issue, client instruction, re-weigh
    /// dispute.
    OnHold,
    /// Being returned to the client.
    Returning,
    /// Delivered with e-POD.
    Delivered,
    /// The delivery is denied or was marked by mistake.
    UnderReview,
    /// Untraced.
    Lost,
    /// Back with the client (terminal).
    Returned,
    /// Destroyed on the client's written instruction (terminal).
    Disposed,
    /// Cancelled before pickup (terminal).
    Cancelled,
    /// Entered in error or duplicate; number kept, never charged (terminal).
    Voided,
}

/// One consignment to one recipient. Money is fils; weights are grams.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "logistics_shipments",
    state_machine,
    unique(number),
    index(status),
    index(delivery_id)
)]
pub struct Shipment {
    /// The primary key.
    pub id: i64,
    /// The number from the `shipment` series.
    pub number: String,
    /// The client (a party).
    pub client_party_id: i64,
    /// The shipper's own reference; unique per client across all states.
    pub client_reference: Option<String>,
    /// The user who booked it.
    pub booked_by_user_id: Option<i64>,
    /// The recipient's name.
    pub recipient_name: String,
    /// The recipient's phone.
    pub recipient_phone: String,
    /// The address as text.
    pub address: String,
    /// The area.
    pub area: String,
    /// The cash to collect, in fils.
    pub cod_amount: i64,
    /// The weight booked, in grams.
    pub booked_weight_grams: i64,
    /// The weight it is charged on, in grams: the booked weight, then the
    /// weight of the parcels received once it is picked up (BL-LOG-03).
    pub weight_grams: i64,
    /// The price on that weight from the client's bands, in fils; 0 when the
    /// client has no band for the zone.
    pub price: i64,
    /// How many parcels were booked.
    pub parcel_count: i64,
    /// How many parcels were scanned.
    pub parcels_scanned: i64,
    /// The wl-delivery delivery of the shipment.
    pub delivery_id: Option<i64>,
    /// The client's instruction to send it back (`LOG.ACL.09`): who asked,
    /// when and how.
    pub return_instruction: Option<String>,
    /// The user who recorded the instruction.
    pub return_instruction_by: Option<i64>,
    /// When the instruction was recorded.
    pub return_instruction_at: Option<DateTime>,
    /// The reference of the client's e-POD of the returned goods.
    pub return_pod_ref: Option<String>,
    /// The lifecycle state; changed only by a transition.
    pub status: ShipmentStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Shipment {
    /// The shipment as a [`Subject`]: `logistics.shipment:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The client.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// Whether the client's instruction to send it back is recorded.
    pub fn has_return_instruction(&self) -> bool {
        self.return_instruction
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
    }

    /// The parcels, in order.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn parcels(&self, db: &Db) -> Result<Vec<Parcel>> {
        Parcel::query()
            .where_(Parcel::SHIPMENT_ID.eq(self.id))
            .order_by(Parcel::ID)
            .get(db)
            .await
    }

    /// The tracking events, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn events(&self, db: &Db) -> Result<Vec<TrackingEvent>> {
        TrackingEvent::query()
            .where_(TrackingEvent::SHIPMENT_ID.eq(self.id))
            .order_by(TrackingEvent::ID)
            .get(db)
            .await
    }

    /// The charge lines recorded for the next monthly invoice.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn charges(&self, db: &Db) -> Result<Vec<Charge>> {
        Charge::query()
            .where_(Charge::SHIPMENT_ID.eq(self.id))
            .order_by(Charge::ID)
            .get(db)
            .await
    }

    /// The shipments of `client` a monthly invoice may bill: never a voided
    /// one (`LOG.RUL.07`, BL-LOG-20 adds the held, lost and cancelled ones
    /// and wires this to the usage billing run).
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn billable(db: &Db, client: PartyId) -> Result<Vec<Shipment>> {
        wl_kernel::voids::exclude_voided(
            Shipment::query().where_(Shipment::CLIENT_PARTY_ID.eq(client.0)),
        )
        .order_by(Shipment::ID)
        .get(db)
        .await
    }
}

/// What a parcel is in: booked, scanned or marked *not received*.
pub mod parcel_state {
    /// Booked, not yet scanned.
    pub const BOOKED: &str = "booked";
    /// Scanned at pickup or at a hub.
    pub const SCANNED: &str = "scanned";
    /// The client did not hand it over (BL-LOG-03): left out of the weight,
    /// the price and the scans, and never picked up later on the shipment.
    pub const NOT_RECEIVED: &str = "not_received";
}

/// One labelled piece of a shipment.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_parcels", unique(barcode), index(shipment_id))]
pub struct Parcel {
    /// The primary key.
    pub id: i64,
    /// The shipment.
    #[model(references = "logistics_shipments")]
    pub shipment_id: i64,
    /// The barcode on the label: `SHP-0007-1`.
    pub barcode: String,
    /// The weight booked, in grams.
    pub weight_grams: i64,
    /// `booked`, `scanned` or `not_received` ([`parcel_state`]).
    pub state: String,
    /// When it was scanned.
    pub scanned_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A status change or a scan (§6 *Tracking event*).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_tracking_events", index(shipment_id))]
pub struct TrackingEvent {
    /// The primary key.
    pub id: i64,
    /// The shipment.
    #[model(references = "logistics_shipments")]
    pub shipment_id: i64,
    /// `booked`, `picked_up`, `hub_scan`, `dispatched`, `delivered`,
    /// `attempt_failed`, `cancelled`, `voided`, `return_started`,
    /// `returned` or `note`.
    pub kind: String,
    /// A note.
    pub note: String,
    /// When it happened.
    pub occurred_at: DateTime,
    /// The user who did it; none for the system.
    pub actor_user_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The kinds of charge line a shipment can carry.
pub mod charge_kind {
    /// The cancellation fee of `cancel` (only when the rate card has one).
    pub const CANCELLATION: &str = "cancellation";
    /// The return charge, once per shipment, whichever path sent it back.
    pub const RETURN: &str = "return";
}

/// A charge recorded for the client's next monthly invoice (`LOG.RUL.07`,
/// billed by BL-LOG-20).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_charges", index(shipment_id))]
pub struct Charge {
    /// The primary key.
    pub id: i64,
    /// The shipment.
    #[model(references = "logistics_shipments")]
    pub shipment_id: i64,
    /// The client charged.
    pub client_party_id: i64,
    /// `cancellation` or `return` ([`charge_kind`]).
    pub kind: String,
    /// The amount, in fils.
    pub amount: i64,
    /// A note for the accountant ("no return price for client 7").
    pub note: String,
    /// When it was recorded.
    pub recorded_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Writes a tracking event for `shipment_id`.
///
/// # Errors
///
/// Database errors.
pub async fn track<'c, E: renox::db::Executor<'c>>(
    executor: E,
    shipment_id: i64,
    kind: &str,
    note: &str,
    actor: Option<i64>,
) -> Result {
    let mut event = TrackingEvent {
        shipment_id,
        kind: kind.to_owned(),
        note: note.to_owned(),
        occurred_at: renox::db::now(),
        actor_user_id: actor,
        ..TrackingEvent::default()
    };
    event.insert(executor).await?;
    Ok(())
}
