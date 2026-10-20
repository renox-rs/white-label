//! The delivery contract (wl-delivery §6): how a source asks for a delivery
//! ([`DeliveryRequester`]) and the events `wl-delivery` emits as the delivery
//! and its run move on: assigned, out for delivery, delivered, attempt
//! failed, disputed, reviewed, reversed or lost, and a run dispatched.
//!
//! `wl-invoicing` and the source modules listen to these events through the
//! kernel, so no crate depends on `wl-delivery`. The dispute, review,
//! reversal and loss events carry the delivery's `lines`, the source line
//! references ([`Subject`]) the delivery was requested for, so a listener
//! can find the records that bill them (an invoice line's `origin`).
//! [`DeliveryDelivered`] carries the quantities per line instead
//! ([`DeliveredLine`]): sources use the delivered quantities only.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;
use crate::subject::Subject;
use crate::tickets::TicketId;

/// The subject type of a delivery, for [`Subject`] references.
pub const DELIVERY_SUBJECT_TYPE: &str = "delivery.delivery";

/// The id of a delivery.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct DeliveryId(pub i64);

impl DeliveryId {
    /// The delivery as a [`Subject`]: `delivery.delivery:42`.
    pub fn subject(self) -> Subject {
        Subject::new(DELIVERY_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for DeliveryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for DeliveryId {
    fn from(id: i64) -> Self {
        DeliveryId(id)
    }
}

/// The subject type of a run, for [`Subject`] references.
pub const RUN_SUBJECT_TYPE: &str = "delivery.run";

/// The id of a run.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct RunId(pub i64);

impl RunId {
    /// The run as a [`Subject`]: `delivery.run:7`.
    pub fn subject(self) -> Subject {
        Subject::new(RUN_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for RunId {
    fn from(id: i64) -> Self {
        RunId(id)
    }
}

/// A way to prove a delivery (wl-delivery §3, `WRK.POD.01`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodMethod {
    /// A photo at the door.
    Photo,
    /// The recipient's signature.
    Signature,
    /// The one-time code the recipient reads out.
    Otp,
}

impl PodMethod {
    /// Every method.
    pub const ALL: [PodMethod; 3] = [PodMethod::Photo, PodMethod::Signature, PodMethod::Otp];

    /// The stored key: `photo`, `signature` or `otp`.
    pub fn as_str(self) -> &'static str {
        match self {
            PodMethod::Photo => "photo",
            PodMethod::Signature => "signature",
            PodMethod::Otp => "otp",
        }
    }

    /// The method stored as `key`.
    pub fn parse(key: &str) -> Option<PodMethod> {
        PodMethod::ALL.into_iter().find(|m| m.as_str() == key.trim())
    }
}

impl fmt::Display for PodMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One item or parcel a source asks to deliver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryItem {
    /// The line of the source this item comes from, if it has one.
    pub source_line: Option<Subject>,
    /// What it is.
    pub description: String,
    /// The quantity in thousandths of the line's unit (`2500` is 2.5).
    pub quantity_milli: i64,
    /// The amount to collect for the whole line, in fils, when the source
    /// gives line amounts.
    pub amount_to_collect: Option<i64>,
}

impl DeliveryItem {
    /// An item of `quantity_milli` thousandths.
    pub fn new(description: &str, quantity_milli: i64) -> Self {
        DeliveryItem {
            source_line: None,
            description: description.to_owned(),
            quantity_milli,
            amount_to_collect: None,
        }
    }

    /// Names the source line.
    pub fn source_line(mut self, line: Subject) -> Self {
        self.source_line = Some(line);
        self
    }

    /// Gives the amount to collect for the whole line, in fils.
    pub fn amount(mut self, amount: i64) -> Self {
        self.amount_to_collect = Some(amount);
        self
    }
}

/// What a source asks of [`DeliveryRequester::request`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryRequest {
    /// The record the delivery is for (a sales order, a dispatch note, a
    /// shipment, an F&B order).
    pub source: Subject,
    /// A pickup from the recipient instead of a drop.
    pub pickup: bool,
    /// The recipient party, if there is one.
    pub recipient_party: Option<PartyId>,
    /// The recipient's name.
    pub recipient_name: String,
    /// The recipient's phone.
    pub recipient_phone: String,
    /// The address as text.
    pub address: String,
    /// The area, for grouping onto runs.
    pub area: String,
    /// The start of the time window.
    pub window_from: Option<DateTime>,
    /// The end of the time window.
    pub window_to: Option<DateTime>,
    /// Instructions for the driver.
    pub instructions: String,
    /// The items.
    pub items: Vec<DeliveryItem>,
    /// The amount to collect in cash, in fils; 0 when nothing is collected.
    pub collect_amount: i64,
    /// The proof methods required; empty takes the module's setting for the
    /// source type.
    pub pod_methods: Vec<PodMethod>,
    /// Whether the goods are perishable.
    pub perishable: bool,
}

impl DeliveryRequest {
    /// A delivery for `source` to `recipient_name`, with nothing else set.
    pub fn new(source: Subject, recipient_name: &str) -> Self {
        DeliveryRequest {
            source,
            pickup: false,
            recipient_party: None,
            recipient_name: recipient_name.to_owned(),
            recipient_phone: String::new(),
            address: String::new(),
            area: String::new(),
            window_from: None,
            window_to: None,
            instructions: String::new(),
            items: Vec::new(),
            collect_amount: 0,
            pod_methods: Vec::new(),
            perishable: false,
        }
    }

    /// Adds an item.
    pub fn item(mut self, item: DeliveryItem) -> Self {
        self.items.push(item);
        self
    }

    /// Sets the address and the area.
    pub fn address(mut self, address: &str, area: &str) -> Self {
        self.address = address.to_owned();
        self.area = area.to_owned();
        self
    }

    /// Sets the phone of the recipient.
    pub fn phone(mut self, phone: &str) -> Self {
        self.recipient_phone = phone.to_owned();
        self
    }

    /// Sets the amount to collect in cash, in fils.
    pub fn collect(mut self, amount: i64) -> Self {
        self.collect_amount = amount;
        self
    }

    /// Sets the proof methods required.
    pub fn pod(mut self, methods: &[PodMethod]) -> Self {
        self.pod_methods = methods.to_vec();
        self
    }

    /// Sets the time window.
    pub fn window(mut self, from: DateTime, to: DateTime) -> Self {
        self.window_from = Some(from);
        self.window_to = Some(to);
        self
    }

    /// Flags the goods as perishable.
    pub fn perishable(mut self) -> Self {
        self.perishable = true;
        self
    }
}

/// The future a [`DeliveryRequester`] method returns.
pub type DeliveryFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// How a source module asks for a delivery and takes the request back
/// (wl-delivery §6 *Provides*). `wl-delivery` provides it with
/// [`provide_requester`]; without that module, [`requester`] gives
/// [`NoDeliveries`], which refuses every request.
pub trait DeliveryRequester: Send + Sync + 'static {
    /// Creates a `pending` delivery for `request`.
    fn request<'a>(
        &'a self,
        state: &'a AppState,
        request: DeliveryRequest,
    ) -> DeliveryFuture<'a, DeliveryId>;

    /// Cancels a delivery that is not on a dispatched run (`cancel`).
    fn cancel<'a>(
        &'a self,
        state: &'a AppState,
        delivery: DeliveryId,
        reason: &'a str,
    ) -> DeliveryFuture<'a, ()>;

    /// Puts the `delivered` delivery under review as the system (`open_review`
    /// by a dispute ticket from the recipient or the client). `ticket` is
    /// the dispute ticket that raised it and becomes the review's ticket; with
    /// none, the module opens one. The default refuses: the `delivery` module
    /// is off.
    fn open_review<'a>(
        &'a self,
        _state: &'a AppState,
        _delivery: DeliveryId,
        _reason: &'a str,
        _ticket: Option<TicketId>,
    ) -> DeliveryFuture<'a, ()> {
        Box::pin(async { Err(Error::NotFound) })
    }

    /// Whether the delivery is frozen: under review (or lost after a
    /// review), so its cash on delivery is held out of remittances and
    /// settlements and its source must not bill it. The default is `false`.
    fn is_frozen<'a>(
        &'a self,
        _state: &'a AppState,
        _delivery: DeliveryId,
    ) -> DeliveryFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// The cash collected on a `delivered` delivery that a remittance or a
    /// settlement may take now, in fils: 0 while the delivery is frozen, not
    /// delivered, or unknown. The default is 0.
    fn remittable_cod<'a>(
        &'a self,
        _state: &'a AppState,
        _delivery: DeliveryId,
    ) -> DeliveryFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }
}

/// The requester when the `delivery` module is off: every request is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDeliveries;

impl DeliveryRequester for NoDeliveries {
    fn request<'a>(
        &'a self,
        _state: &'a AppState,
        _request: DeliveryRequest,
    ) -> DeliveryFuture<'a, DeliveryId> {
        Box::pin(async {
            Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Deliveries are not switched on for this client.".to_owned(),
            ))
        })
    }

    fn cancel<'a>(
        &'a self,
        _state: &'a AppState,
        _delivery: DeliveryId,
        _reason: &'a str,
    ) -> DeliveryFuture<'a, ()> {
        Box::pin(async { Err(Error::NotFound) })
    }
}

/// Makes `requester` the app's [`DeliveryRequester`]; `wl-delivery` calls it
/// in `register`.
pub fn provide_requester(app: &mut Registry, requester: impl DeliveryRequester) {
    let requester: Arc<dyn DeliveryRequester> = Arc::new(requester);
    app.provide(requester);
}

/// The app's [`DeliveryRequester`], or [`NoDeliveries`] when none is provided.
pub fn requester(state: &AppState) -> Arc<dyn DeliveryRequester> {
    match state.provided::<Arc<dyn DeliveryRequester>>() {
        Some(requester) => Arc::clone(&requester),
        None => Arc::new(NoDeliveries),
    }
}

/// A delivery was put on a run (`assign`, `DeliveryAssigned`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryAssigned {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// The run.
    pub run: RunId,
    /// The driver (a user).
    pub driver: i64,
}

impl Event for DeliveryAssigned {}

/// The run of a delivery was dispatched (`DeliveryOutForDelivery`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryOutForDelivery {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// The start of the promised window.
    pub window_from: Option<DateTime>,
    /// The end of the promised window.
    pub window_to: Option<DateTime>,
}

impl Event for DeliveryOutForDelivery {}

/// The quantities of one line of a delivered delivery. Sources use the
/// delivered quantity only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveredLine {
    /// The source line, if the line came with one.
    pub line: Option<Subject>,
    /// What the line is.
    pub description: String,
    /// The quantity ordered, in thousandths.
    pub quantity_milli: i64,
    /// The quantity the recipient accepted, in thousandths.
    pub delivered_milli: i64,
    /// The quantity the recipient refused, in thousandths.
    pub refused_milli: i64,
    /// The refusal reason's code, when something was refused.
    pub refusal_reason: Option<String>,
}

/// A delivery was handed over with proof, in full or with refused lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryDelivered {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// The attempt that delivered it (its number on the delivery).
    pub attempt: i64,
    /// The saved proof's id.
    pub proof: i64,
    /// The lines with the delivered and refused quantities.
    pub lines: Vec<DeliveredLine>,
    /// The cash expected, in fils (the amount to collect less the refused
    /// lines' amounts, when the lines carry amounts).
    pub cash_expected: i64,
    /// The cash collected, in fils.
    pub cash_collected: i64,
    /// Expected less collected, in fils; 0 when they match.
    pub cod_difference: i64,
    /// The reason of a difference.
    pub cod_difference_reason: Option<String>,
    /// Who absorbs a difference (`customer`, `client`, `business`, `driver`).
    pub absorbed_by: Option<String>,
    /// The reference of a cheque collected, if one was.
    pub cheque: Option<String>,
}

impl Event for DeliveryDelivered {}

/// Who absorbs a cash-on-delivery difference (`WRK.ACL.09`, wl-invoicing
/// §5.2 step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodAbsorber {
    /// The customer still owes it: it stays open on the invoice.
    Customer,
    /// The client (03) agreed to bear it: a negative line on the COD
    /// remittance.
    Client,
    /// The business bears it: a COD loss, not billed.
    Business,
    /// The driver bears it: not billed, it stays expected in the driver's
    /// cash handover.
    Driver,
}

impl CodAbsorber {
    /// The stored key: `customer`, `client`, `business` or `driver`.
    pub fn as_str(self) -> &'static str {
        match self {
            CodAbsorber::Customer => "customer",
            CodAbsorber::Client => "client",
            CodAbsorber::Business => "business",
            CodAbsorber::Driver => "driver",
        }
    }

    /// The absorber stored as `key`.
    pub fn parse(key: &str) -> Option<CodAbsorber> {
        [
            CodAbsorber::Customer,
            CodAbsorber::Client,
            CodAbsorber::Business,
            CodAbsorber::Driver,
        ]
        .into_iter()
        .find(|a| a.as_str() == key.trim())
    }

    /// Whether the difference reduces what is billed (the business or the
    /// driver bears it; the customer still owes it and the client bears it
    /// on its remittance).
    pub fn reduces_the_invoice(self) -> bool {
        matches!(self, CodAbsorber::Business | CodAbsorber::Driver)
    }
}

/// A COD difference with its absorber, as `DeliveryDelivered` carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodAbsorption {
    /// Who bears it.
    pub absorber: CodAbsorber,
    /// The difference in fils, above 0.
    pub amount: i64,
    /// The reason of the difference.
    pub reason: String,
}

impl CodAbsorption {
    /// The amount of the adjustment line this puts on the client's COD
    /// remittance: the negative difference when the client bears it, else 0
    /// (apps/logistics `LOG.COD.01`).
    pub fn remittance_adjustment(&self) -> i64 {
        if self.absorber == CodAbsorber::Client {
            -self.amount
        } else {
            0
        }
    }
}

impl DeliveryDelivered {
    /// The quantity to bill for the source line `line`, in thousandths: what
    /// the recipient accepted, never the refused quantity. `None` for a
    /// line the delivery does not carry. A source that drafts its invoice
    /// after the delivery uses it for its quantities.
    pub fn billable_milli(&self, line: &Subject) -> Option<i64> {
        self.lines
            .iter()
            .find(|l| l.line.as_ref() == Some(line))
            .map(|l| l.delivered_milli)
    }

    /// The COD difference and who bears it: `None` when cash matched or the
    /// difference has no known absorber (a difference of the driver's own
    /// accepting before `WRK.ACL.09` is built carries none).
    pub fn cod_absorption(&self) -> Option<CodAbsorption> {
        if self.cod_difference <= 0 {
            return None;
        }
        let absorber = CodAbsorber::parse(self.absorbed_by.as_deref()?)?;
        Some(CodAbsorption {
            absorber,
            amount: self.cod_difference,
            reason: self.cod_difference_reason.clone().unwrap_or_default(),
        })
    }
}

/// A visit failed (`fail`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryAttemptFailed {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// The failure reason's code.
    pub reason: String,
    /// The attempt's number on the delivery.
    pub attempt: i64,
}

impl Event for DeliveryAttemptFailed {}

/// A run left the depot (`dispatch`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDispatched {
    /// The run.
    pub run: RunId,
    /// The driver (a user).
    pub driver: i64,
    /// The vehicle (an asset); `None` for an on-foot rider.
    pub vehicle: Option<i64>,
    /// The deliveries that went out.
    pub deliveries: Vec<DeliveryId>,
}

impl Event for RunDispatched {}

/// How a delivery review ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewOutcome {
    /// The delivery stands: it was made as agreed.
    #[default]
    Upheld,
}

/// A delivery was disputed and went under review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryDisputed {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// Why it is disputed.
    pub reason: String,
    /// The review ticket; `None` when ticketing is off.
    pub ticket: Option<TicketId>,
    /// The source line references of the delivery's lines.
    pub lines: Vec<Subject>,
}

impl Event for DeliveryDisputed {}

/// A delivery review was closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryDisputeClosed {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// How the review ended.
    pub outcome: ReviewOutcome,
    /// The review ticket; `None` when ticketing is off.
    pub ticket: Option<TicketId>,
    /// The source line references of the delivery's lines.
    pub lines: Vec<Subject>,
}

impl Event for DeliveryDisputeClosed {}

/// A delivery was reversed (taken back).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReversed {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// Why it was reversed.
    pub reason: String,
    /// The source line references of the delivery's lines.
    pub lines: Vec<Subject>,
}

impl Event for DeliveryReversed {}

/// A delivery was lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryLost {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// Whether the delivery was under review when it was lost.
    pub was_under_review: bool,
    /// The review ticket, when it was under review and ticketing is on.
    pub ticket: Option<TicketId>,
    /// Who bears the loss: `driver`, `business`, `client` or `none` (the
    /// system sets `none`).
    pub liability: String,
    /// The run the delivery was last on, if it was on one.
    pub run: Option<RunId>,
    /// The source line references of the delivery's lines.
    pub lines: Vec<Subject>,
}

impl Event for DeliveryLost {}

/// A lost delivery was found at a depot (`found`): it is `attempted` again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryFound {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// The depot where the goods were scanned or confirmed.
    pub depot: String,
    /// The claim ticket opened when it was lost, if there is one.
    pub claim_ticket: Option<TicketId>,
    /// The source line references of the delivery's lines.
    pub lines: Vec<Subject>,
}

impl Event for DeliveryFound {}

/// One refused line of a delivered delivery, as the depot accounted for it at
/// run close: the refused quantity came back (received), was not on the
/// vehicle (missing), or part of each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusedLineReceived {
    /// The source line, if the line came with one.
    pub line: Option<Subject>,
    /// What the line is.
    pub description: String,
    /// The quantity the recipient refused, in thousandths.
    pub refused_milli: i64,
    /// The part of it received back at the depot.
    pub received_milli: i64,
    /// The part reported missing; `received_milli + missing_milli` is
    /// `refused_milli`.
    pub missing_milli: i64,
    /// The refusal reason's code.
    pub refusal_reason: Option<String>,
}

/// The refused lines of a delivery were received back (or reported missing)
/// when its run closed. Sources move the received goods (for a dispatch note,
/// into quarantine) and write off the missing ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryRefusedReceived {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// The run that closed.
    pub run: RunId,
    /// The refused lines with what came back and what is missing.
    pub lines: Vec<RefusedLineReceived>,
}

impl Event for DeliveryRefusedReceived {}

/// A run closed: its cash was counted and every undelivered and refused item
/// was received back or reported missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunClosed {
    /// The run.
    pub run: RunId,
    /// The driver (a user).
    pub driver: i64,
    /// The vehicle (an asset); `None` for an on-foot rider.
    pub vehicle: Option<i64>,
    /// The deliveries handed over.
    pub delivered: i64,
    /// The deliveries that came back undelivered (received or missing).
    pub undelivered: i64,
    /// How many of the undelivered deliveries and refused lines were reported
    /// missing.
    pub missing: i64,
    /// The cash collected on the run, in fils.
    pub cash_collected: i64,
}

impl Event for RunClosed {}

#[cfg(test)]
mod tests {
    use renox::serde_json::{json, to_value};

    use super::*;

    #[test]
    fn a_delivery_id_is_a_subject_and_prints_bare() {
        let id = DeliveryId::from(42);
        assert_eq!(id.subject(), Subject::new("delivery.delivery", 42));
        assert_eq!(id.to_string(), "42");
        assert_eq!(to_value(id).unwrap(), json!(42));
    }

    #[test]
    fn a_pod_method_round_trips_through_its_key() {
        for method in PodMethod::ALL {
            assert_eq!(PodMethod::parse(method.as_str()), Some(method));
        }
        assert_eq!(PodMethod::parse("fingerprint"), None);
        assert_eq!(RunId(3).subject(), Subject::new("delivery.run", 3));
    }

    #[test]
    fn the_review_outcome_defaults_to_upheld_and_serializes_snake_case() {
        assert_eq!(ReviewOutcome::default(), ReviewOutcome::Upheld);
        assert_eq!(to_value(ReviewOutcome::Upheld).unwrap(), json!("upheld"));
    }

    #[test]
    fn events_carry_the_delivery_ticket_and_lines() {
        let line = Subject::new("leasing.lease", 5);
        let lost = DeliveryLost {
            delivery: DeliveryId(7),
            source: line.clone(),
            was_under_review: true,
            ticket: Some(TicketId(11)),
            liability: "none".to_owned(),
            run: Some(RunId(3)),
            lines: vec![line.clone()],
        };
        let value = to_value(&lost).unwrap();
        assert_eq!(value["delivery"], json!(7));
        assert_eq!(value["ticket"], json!(11));
        assert_eq!(value["was_under_review"], json!(true));
        assert_eq!(value["lines"].as_array().unwrap().len(), 1);
    }

    fn delivered(difference: i64, absorbed_by: Option<&str>) -> DeliveryDelivered {
        let line = Subject::new("sales.order_line", 1);
        DeliveryDelivered {
            delivery: DeliveryId(7),
            source: Subject::new("sales.order", 9),
            attempt: 1,
            proof: 1,
            lines: vec![DeliveredLine {
                line: Some(line),
                description: "Cartons".to_owned(),
                quantity_milli: 10_000,
                delivered_milli: 8_000,
                refused_milli: 2_000,
                refusal_reason: Some("damaged".to_owned()),
            }],
            cash_expected: 40_000,
            cash_collected: 40_000 - difference,
            cod_difference: difference,
            cod_difference_reason: Some("recipient short of cash".to_owned()),
            absorbed_by: absorbed_by.map(str::to_owned),
            cheque: None,
        }
    }

    #[test]
    fn the_billable_quantity_is_the_delivered_one() {
        let event = delivered(0, None);
        assert_eq!(
            event.billable_milli(&Subject::new("sales.order_line", 1)),
            Some(8_000)
        );
        assert_eq!(event.billable_milli(&Subject::new("sales.order_line", 2)), None);
    }

    #[test]
    fn a_cod_difference_names_its_absorber_and_only_the_client_adjusts_a_remittance() {
        assert_eq!(delivered(0, Some("client")).cod_absorption(), None);
        assert_eq!(delivered(3_000, None).cod_absorption(), None);
        assert_eq!(delivered(3_000, Some("carrier")).cod_absorption(), None);
        let client = delivered(3_000, Some("client")).cod_absorption().unwrap();
        assert_eq!(client.absorber, CodAbsorber::Client);
        assert_eq!(client.remittance_adjustment(), -3_000);
        let business = delivered(3_000, Some("business")).cod_absorption().unwrap();
        assert_eq!(business.remittance_adjustment(), 0);
        assert!(business.absorber.reduces_the_invoice());
        assert!(CodAbsorber::Driver.reduces_the_invoice());
        assert!(!CodAbsorber::Customer.reduces_the_invoice());
        assert!(!CodAbsorber::Client.reduces_the_invoice());
    }
}
