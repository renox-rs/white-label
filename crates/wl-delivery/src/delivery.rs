//! The Delivery, its lines, attempts and proof (wl-delivery §3, §4.1). A
//! delivery's status moves only through the machine in
//! [`crate::delivery_machine`]; a saved proof is never edited (`WRK.ACL.06`).

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::{DeliveryId, PodMethod};
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::{PartyId, Subject};

/// `DLV-{yyyy}-{0000}` (§7).
pub const DELIVERY_SERIES: Series = Series {
    key: "delivery",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "DLV",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// Where a delivery is in its life (wl-delivery §4.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DeliveryStatus {
    /// Ready to be planned onto a run.
    #[default]
    Pending,
    /// On a planned run with a driver.
    Assigned,
    /// The run has left.
    OutForDelivery,
    /// The last visit failed.
    Attempted,
    /// Handed over with proof, in full or with refused lines.
    Delivered,
    /// The recipient denies the delivery; COD and billing frozen.
    UnderReview,
    /// Goods untraced.
    Lost,
    /// Brought back to the origin.
    Returned,
    /// Perishable goods destroyed.
    Discarded,
    /// Not needed any more.
    Cancelled,
}

/// A drop or a pickup.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Direction {
    /// Goods go to the recipient.
    #[default]
    Delivery,
    /// Goods are collected from the recipient.
    Pickup,
}

/// A delivery. Money is in fils; times are instants.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_delivery_deliveries",
    state_machine,
    unique(number),
    index(status),
    index(run_id),
    index(source_type, source_id)
)]
pub struct Delivery {
    /// The primary key.
    pub id: i64,
    /// The number from the `delivery` series.
    pub number: String,
    /// Drop or pickup.
    pub direction: Direction,
    /// The type of the record it was requested for.
    pub source_type: String,
    /// The id of that record.
    pub source_id: i64,
    /// The recipient party, if there is one.
    pub recipient_party_id: Option<i64>,
    /// The recipient's name.
    pub recipient_name: String,
    /// The recipient's phone (a driver sees it only while the stop is open).
    pub recipient_phone: String,
    /// The address as text.
    pub address: String,
    /// The area.
    pub area: String,
    /// The latitude in millionths of a degree.
    pub latitude_micro: Option<i64>,
    /// The longitude in millionths of a degree.
    pub longitude_micro: Option<i64>,
    /// The start of the time window.
    pub window_from: Option<DateTime>,
    /// The end of the time window.
    pub window_to: Option<DateTime>,
    /// Instructions for the driver.
    pub instructions: String,
    /// Whether the goods are perishable.
    pub perishable: bool,
    /// The cash to collect, in fils.
    pub amount_to_collect: i64,
    /// The cheque expected, in fils.
    pub cheque_expected: i64,
    /// The proof methods required, comma separated.
    pub pod_methods: String,
    /// Whether any one of the methods is enough.
    pub pod_any: bool,
    /// Whether the delivery may be partly accepted.
    pub partial_acceptance: bool,
    /// The attempts that count so far.
    pub attempts: i64,
    /// The attempts allowed.
    pub max_attempts: i64,
    /// The run it is on.
    pub run_id: Option<i64>,
    /// Its place on the run.
    pub run_sequence: Option<i64>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// The frozen flag (`open_review`): while set, the cash on delivery is
    /// held out of remittances and settlements and the source does not bill
    /// the delivery. Changed only by the transitions that set or clear it.
    pub frozen: bool,
    /// The review's ticket; `None` when ticketing is off or no review.
    pub review_ticket_id: Option<i64>,
    /// Why it was put under review.
    pub review_reason: Option<String>,
    /// The note the review was closed with.
    pub review_note: Option<String>,
    /// When the review was opened.
    pub reviewed_at: Option<DateTime>,
    /// Why it was marked lost.
    pub lost_reason: Option<String>,
    /// Who bears the loss: `driver`, `business`, `client` or `none`.
    pub lost_liability: Option<String>,
    /// When it was marked lost.
    pub lost_at: Option<DateTime>,
    /// The claim ticket opened when it was lost; `None` when ticketing is off.
    pub claim_ticket_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: DeliveryStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Delivery {
    /// The id as the kernel's [`DeliveryId`].
    pub fn delivery_id(&self) -> DeliveryId {
        DeliveryId(self.id)
    }

    /// The record it was requested for.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// The recipient party.
    pub fn recipient(&self) -> Option<PartyId> {
        self.recipient_party_id.map(PartyId)
    }

    /// The proof methods required.
    pub fn required_methods(&self) -> Vec<PodMethod> {
        self.pod_methods
            .split(',')
            .filter_map(PodMethod::parse)
            .collect()
    }

    /// The lines, in order.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn lines(&self, db: &Db) -> Result<Vec<DeliveryLine>> {
        DeliveryLine::query()
            .where_(DeliveryLine::DELIVERY_ID.eq(self.id))
            .order_by(DeliveryLine::ID)
            .get(db)
            .await
    }

    /// The attempts, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn attempts_made(&self, db: &Db) -> Result<Vec<Attempt>> {
        Attempt::query()
            .where_(Attempt::DELIVERY_ID.eq(self.id))
            .order_by(Attempt::ID)
            .get(db)
            .await
    }

    /// The latest saved proof that was not reversed, if the delivery has one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn latest_proof(&self, db: &Db) -> Result<Option<Proof>> {
        Proof::query()
            .where_(Proof::DELIVERY_ID.eq(self.id))
            .where_(Proof::REVERSED.eq(false))
            .order_by_desc(Proof::ID)
            .first(db)
            .await
    }
}

/// One item or parcel on a delivery, copied from the source. Quantities are
/// in thousandths of the unit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_delivery_lines", index(delivery_id))]
pub struct DeliveryLine {
    /// The primary key.
    pub id: i64,
    /// The delivery.
    #[model(references = "wl_delivery_deliveries")]
    pub delivery_id: i64,
    /// The type of the source line.
    pub source_line_type: Option<String>,
    /// The id of the source line.
    pub source_line_id: Option<i64>,
    /// What it is.
    pub description: String,
    /// The quantity ordered.
    pub quantity_milli: i64,
    /// The amount to collect for the whole line, in fils, when the source
    /// gives line amounts.
    pub amount_to_collect: Option<i64>,
    /// The quantity the recipient accepted (set at `deliver`, never edited).
    pub delivered_milli: i64,
    /// The quantity the recipient refused.
    pub refused_milli: i64,
    /// The refusal reason's code.
    pub refusal_reason: Option<String>,
    /// The photo of a refused (damaged) quantity.
    pub refusal_photo_ref: Option<String>,
    /// The part of the refused quantity received back at the depot at run
    /// close.
    pub refused_received_milli: i64,
    /// The part of the refused quantity reported missing at run close.
    pub refused_missing_milli: i64,
    /// When the depot accounted for the refused quantity.
    pub accounted_at: Option<DateTime>,
    /// The user who did.
    pub accounted_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DeliveryLine {
    /// The source line, if the line came with one.
    pub fn source_line(&self) -> Option<Subject> {
        match (&self.source_line_type, self.source_line_id) {
            (Some(kind), Some(id)) => Some(Subject {
                subject_type: kind.clone(),
                subject_id: id,
            }),
            _ => None,
        }
    }
}

/// A moment the goods were traced (a dispatch, a review, a scan, a note, a
/// depot receipt, a find). The untraced clock of `WRK.RUL.17` reads the
/// latest one (#162).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_delivery_traces", index(delivery_id, occurred_at))]
pub struct Trace {
    /// The primary key.
    pub id: i64,
    /// The delivery.
    #[model(references = "wl_delivery_deliveries")]
    pub delivery_id: i64,
    /// `dispatch`, `review`, `scan`, `note`, `depot_receipt` or `found`.
    pub kind: String,
    /// The note, for a note.
    pub note: String,
    /// The depot, for a scan or a receipt.
    pub depot: String,
    /// When it happened.
    pub occurred_at: DateTime,
    /// The user who recorded it; none for the system.
    pub actor_user_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The outcome of one visit.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AttemptOutcome {
    /// Handed over.
    #[default]
    Delivered,
    /// The visit failed.
    Failed,
}

/// One visit (wl-delivery §3 *Attempt*).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_delivery_attempts", index(delivery_id))]
pub struct Attempt {
    /// The primary key.
    pub id: i64,
    /// The delivery.
    #[model(references = "wl_delivery_deliveries")]
    pub delivery_id: i64,
    /// The run it was made on.
    pub run_id: Option<i64>,
    /// Its number on the delivery (1 is the first visit).
    pub number: i64,
    /// Delivered or failed.
    pub outcome: AttemptOutcome,
    /// The failure reason's code.
    pub failure_reason: Option<String>,
    /// Whether it counts against the maximum.
    pub counts: bool,
    /// When it happened.
    pub occurred_at: DateTime,
    /// The latitude in millionths of a degree, captured at that moment.
    pub latitude_micro: Option<i64>,
    /// The longitude in millionths of a degree.
    pub longitude_micro: Option<i64>,
    /// The user who recorded it.
    pub actor_user_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The evidence of a delivered attempt. Never edited once saved.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_delivery_proofs", unique(attempt_id), index(delivery_id))]
pub struct Proof {
    /// The primary key.
    pub id: i64,
    /// The delivery.
    #[model(references = "wl_delivery_deliveries")]
    pub delivery_id: i64,
    /// The attempt it belongs to.
    #[model(references = "wl_delivery_attempts")]
    pub attempt_id: i64,
    /// The methods used, comma separated.
    pub methods: String,
    /// Who received it.
    pub recipient_name: String,
    /// The photo's reference.
    pub photo_ref: Option<String>,
    /// The signature image's reference.
    pub signature_ref: Option<String>,
    /// Whether the recipient's one-time code was verified.
    pub otp_verified: bool,
    /// The cash expected, in fils.
    pub cash_expected: i64,
    /// The cash collected, in fils.
    pub cash_collected: i64,
    /// The cheque reference, if a cheque was collected.
    pub cheque_reference: Option<String>,
    /// The driver's notes.
    pub notes: String,
    /// Whether a reversal marked it reversed (only `reverse_delivery`).
    pub reversed: bool,
    /// Why the cash collected is below the cash expected, when it is
    /// (`WRK.ACL.09`).
    pub cod_difference_reason: Option<String>,
    /// Who absorbs the difference: `customer`, `client`, `business` or
    /// `driver`.
    pub absorbed_by: Option<String>,
    /// The approval request that allowed a difference above the tolerance.
    pub approval_request_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let keys: Vec<_> = DeliveryStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            keys,
            [
                "pending",
                "assigned",
                "out_for_delivery",
                "attempted",
                "delivered",
                "under_review",
                "lost",
                "returned",
                "discarded",
                "cancelled"
            ]
        );
    }

    #[test]
    fn the_required_methods_are_read_from_the_stored_list() {
        let delivery = Delivery {
            pod_methods: "photo,signature".into(),
            ..Delivery::default()
        };
        assert_eq!(
            delivery.required_methods(),
            vec![PodMethod::Photo, PodMethod::Signature]
        );
    }
}
