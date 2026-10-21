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

/// The subject type of a parcel: the source line of its delivery line.
pub const PARCEL_SUBJECT_TYPE: &str = "logistics.parcel";

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
    index(delivery_id),
    index(return_of_id)
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
    /// The chargeable weight of the latest re-weigh at the hub, in grams
    /// (BL-LOG-04); none before the first.
    pub measured_weight_grams: Option<i64>,
    /// The price on the measured values, in fils, when the latest re-weigh
    /// is above the client's tolerance; none otherwise (the booked price
    /// stands).
    pub reweigh_price: Option<i64>,
    /// When the shipment went on an issued monthly invoice (BL-LOG-20).
    pub invoiced_on: Option<DateTime>,
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
    /// Why it is (or was last) held: one of [`hold_reason`].
    pub hold_reason: Option<String>,
    /// The state it was in when it was last held.
    pub held_from: Option<String>,
    /// The references of the photos of a damaged parcel, one per line.
    pub hold_photos: Option<String>,
    /// The client's instruction recorded for the hold.
    pub hold_instruction: Option<String>,
    /// The user who recorded it.
    pub hold_instruction_by: Option<i64>,
    /// When it was recorded.
    pub hold_instruction_at: Option<DateTime>,
    /// When the address of an *address issue* hold was confirmed.
    pub address_confirmed_at: Option<DateTime>,
    /// The user who confirmed it (the client's, or staff for it).
    pub address_confirmed_by: Option<i64>,
    /// Whether the company is liable for the damage.
    pub company_liable: bool,
    /// The claim ticket opened for the liability.
    pub claim_ticket_id: Option<i64>,
    /// The number of that ticket.
    pub claim_ticket_number: Option<String>,
    /// When the delivery delivered it (BL-LOG-14: the COD remittance takes
    /// what was delivered by the period end).
    pub delivered_at: Option<DateTime>,
    /// The COD difference recorded at the stop (expected less collected), in
    /// fils; 0 when the cash matched.
    pub cod_difference: i64,
    /// The reason of that difference.
    pub cod_difference_reason: Option<String>,
    /// Who absorbs it: `customer`, `client`, `business` or `driver`.
    pub cod_absorbed_by: Option<String>,
    /// The cash collected at the stop, in fils (BL-LOG-08).
    pub cod_collected: i64,
    /// The client's instruction to receive less: who asked, when and how.
    /// The *client* absorbs a difference only with it.
    pub cod_instruction: Option<String>,
    /// The user who recorded it.
    pub cod_instruction_by: Option<i64>,
    /// The approver who chose who absorbs the difference.
    pub cod_approved_by: Option<i64>,
    /// Whether the COD is marked already remitted (BL-LOG-10): it is left
    /// out of every draft.
    pub cod_already_remitted: bool,
    /// The ticket of the dispute that put the shipment under review
    /// (BL-LOG-09); kept after the review closes.
    pub review_ticket_id: Option<i64>,
    /// For a return shipment, the shipment whose refused parcels it carries
    /// (BL-LOG-07).
    pub return_of_id: Option<i64>,
    /// Who carries the cost of the latest reversal or loss: `driver`,
    /// `business`, `client` or `none` (BL-LOG-10, BL-LOG-11).
    pub liability: Option<String>,
    /// What became of the cash of the latest reversal: `removed`,
    /// `refunded` or `kept` (BL-LOG-10).
    pub cash_treatment: Option<String>,
    /// The COD no longer expected (refunded to the payer, or lost with the
    /// shipment), in fils; it is expected again when the goods are found.
    pub cod_released: i64,
    /// The charge to credit on the invoice it was billed on, in fils
    /// (BL-LOG-10, BL-LOG-11).
    pub credit_due: i64,
    /// Whether the delivery was reversed and not made again: out of billing.
    pub reversed: bool,
    /// Why it was marked lost (BL-LOG-11).
    pub lost_reason: Option<String>,
    /// When it was marked lost.
    pub lost_at: Option<DateTime>,
    /// The state it was lost from.
    pub lost_from: Option<String>,
    /// The depot where the goods were scanned when found.
    pub found_depot: Option<String>,
    /// The COD it expected when it was lost, in fils; expected again when it
    /// is found.
    pub lost_cod: i64,
    /// The credit the settled claim gave the client, in fils; 0 until a claim
    /// is settled.
    pub claim_settled_amount: i64,
    /// When the claim was settled.
    pub claim_settled_at: Option<DateTime>,
    /// What the client chose when the goods were found after a settled claim:
    /// `delivery` or `keep`.
    pub claim_choice: Option<String>,
    /// The draft invoice that takes the claim credit back.
    pub claim_invoice_id: Option<i64>,
    /// Why the goods were destroyed (BL-LOG-06).
    pub disposal_reason: Option<String>,
    /// The references of the photos of the goods, one per line.
    pub disposal_photos: Option<String>,
    /// The reference of the client's written instruction to destroy them.
    pub disposal_instruction: Option<String>,
    /// Whether the hub classed the goods as dangerous: the instruction is
    /// then optional.
    pub disposal_dangerous: bool,
    /// When the shipment was disposed of.
    pub disposed_at: Option<DateTime>,
    /// When it reached `returned` (BL-LOG-20: its return charge is billed in
    /// the month of the e-POD).
    pub returned_at: Option<DateTime>,
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

    /// The price the client is charged: the price on the measured values when
    /// a re-weigh priced it higher, else the price on the weight received.
    pub fn charged_price(&self) -> i64 {
        self.reweigh_price.unwrap_or(self.price)
    }

    /// The client.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// Whether the client's instruction for the hold is recorded.
    pub fn has_hold_instruction(&self) -> bool {
        self.hold_instruction
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
    }

    /// Whether the client's instruction to receive less is recorded.
    pub fn has_cod_instruction(&self) -> bool {
        self.cod_instruction
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
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

    /// The return shipments made from this one for refused parcels
    /// (BL-LOG-07), oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn returns(&self, db: &Db) -> Result<Vec<Shipment>> {
        Shipment::query()
            .where_(Shipment::RETURN_OF_ID.eq(self.id))
            .order_by(Shipment::ID)
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
    /// one, and none whose delivery is under review until the review is
    /// closed (`LOG.RUL.07`, BL-LOG-09), none that is lost (BL-LOG-11) and
    /// none whose delivery was reversed until it is made again (BL-LOG-10),
    /// and a disposed one only when the client caused the disposal
    /// (BL-LOG-06), none that is on hold and none cancelled before pickup
    /// (BL-LOG-20). [`crate::billing`] picks from them what a month bills.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn billable(db: &Db, client: PartyId) -> Result<Vec<Shipment>> {
        let shipments = wl_kernel::voids::exclude_voided(
            Shipment::query()
                .where_(Shipment::CLIENT_PARTY_ID.eq(client.0))
                .where_(Shipment::STATUS.ne(ShipmentStatus::UnderReview))
                // A lost shipment is never billed (BL-LOG-11), and one whose
                // delivery was reversed waits until it is delivered again
                // (BL-LOG-10).
                .where_(Shipment::STATUS.ne(ShipmentStatus::Lost))
                // A held shipment is not billed until released (BL-LOG-20), and
                // a cancelled one has no delivery to bill: its cancellation
                // fee is a charge line of its own.
                .where_(Shipment::STATUS.ne(ShipmentStatus::OnHold))
                .where_(Shipment::STATUS.ne(ShipmentStatus::Cancelled))
                .where_(Shipment::REVERSED.eq(false)),
        )
        .order_by(Shipment::ID)
        .get(db)
        .await?;
        // A disposed shipment is billed only when the client caused it
        // (`LOG.RUL.07`, BL-LOG-06).
        Ok(shipments
            .into_iter()
            .filter(|shipment| {
                shipment.status != ShipmentStatus::Disposed
                    || shipment.liability.as_deref() == Some("client")
            })
            .collect())
    }
}

/// Why a shipment is held (§7.1 state `on_hold`).
pub mod hold_reason {
    /// A damaged parcel; photos are required.
    pub const DAMAGED: &str = "damaged";
    /// A wrong or incomplete address; the client confirms it.
    pub const ADDRESS_ISSUE: &str = "address_issue";
    /// The client's instruction is awaited.
    pub const CLIENT_INSTRUCTION: &str = "client_instruction";
    /// The client disputes the re-weigh and asked to stop delivery.
    pub const REWEIGH_DISPUTE: &str = "reweigh_dispute";
    /// Every reason.
    pub const ALL: [&str; 4] = [DAMAGED, ADDRESS_ISSUE, CLIENT_INSTRUCTION, REWEIGH_DISPUTE];
}

/// An address correction before dispatch, with the old and the new values.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_address_changes", index(shipment_id))]
pub struct AddressChange {
    /// The primary key.
    pub id: i64,
    /// The shipment.
    #[model(references = "logistics_shipments")]
    pub shipment_id: i64,
    /// The address before.
    pub old_address: String,
    /// The area (zone) before.
    pub old_area: String,
    /// The address after.
    pub new_address: String,
    /// The area (zone) after.
    pub new_area: String,
    /// The price on the weight received before, in fils.
    pub old_price: i64,
    /// The price after (it changes with the zone), in fils.
    pub new_price: i64,
    /// The user who corrected it.
    pub changed_by: Option<i64>,
    /// When it was corrected.
    pub changed_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
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
    /// The code of the reason the recipient refused it (BL-LOG-07).
    pub refusal_reason: Option<String>,
    /// When it was scanned.
    pub scanned_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A shipment was delivered (03 §7.1 row `deliver`, event `ShipmentDelivered`),
/// whole or with refused parcels that went back on a linked return shipment.
/// The expected COD stays the full amount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipmentDelivered {
    /// The shipment.
    pub shipment: i64,
    /// Its number.
    pub number: String,
    /// The client.
    pub client: PartyId,
    /// The labels of the parcels the recipient accepted.
    pub delivered_parcels: Vec<String>,
    /// The labels of the parcels the recipient refused.
    pub refused_parcels: Vec<String>,
    /// The return shipment made for the refused parcels, if there are any.
    pub return_shipment: Option<i64>,
    /// The COD expected, in fils: the full amount, whatever was refused.
    pub cod_expected: i64,
}

impl Event for ShipmentDelivered {}

/// A shipment was marked lost (03 §7.1 row `mark_lost`, event `ShipmentLost`),
/// by the system with its delivery or with an approval off a delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipmentLost {
    /// The shipment.
    pub shipment: i64,
    /// Its number.
    pub number: String,
    /// The client.
    pub client: PartyId,
    /// The state it was lost from.
    pub from: String,
    /// Why.
    pub reason: String,
    /// Who carries the loss: `driver`, `business`, `client` or `none`.
    pub liability: String,
    /// The claim ticket linked to the loss.
    pub claim_ticket: Option<i64>,
}

impl Event for ShipmentLost {}

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
    /// The re-weigh surcharge (BL-LOG-04): the price on the measured values
    /// less the price on the weight received, once per shipment.
    pub const REWEIGH: &str = "reweigh";
    /// The disposal charge of goods destroyed because of the client's
    /// packing or contents (BL-LOG-06); 0 with a note when the rate card has
    /// no disposal line.
    pub const DISPOSAL: &str = "disposal";
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
    /// `cancellation`, `return`, `reweigh` or `disposal` ([`charge_kind`]).
    pub kind: String,
    /// The amount, in fils.
    pub amount: i64,
    /// A note for the accountant ("no return price for client 7").
    pub note: String,
    /// Held out of the draft invoice while a re-weigh ticket is open.
    pub held: bool,
    /// The ticket that holds it.
    pub ticket_id: Option<i64>,
    /// When it went on an invoice; none while it waits for the next draft.
    pub billed_on: Option<DateTime>,
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
