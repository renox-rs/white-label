//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::fmt::Display;

use renox::prelude::*;

use crate::screen_texts::SCREEN_TEXTS;

/// Every key the app uses besides the screens', with its English text.
const TEXTS: &[(&str, &str)] = &[
    // Booking (`LOG.SHP.01`, BL-LOG-01).
    (
        "logistics.booking.recipient_needed",
        "Give the recipient's name.",
    ),
    (
        "logistics.booking.parcel_needed",
        "A shipment has at least one parcel.",
    ),
    (
        "logistics.booking.negative",
        "Weights and the amount to collect cannot be negative.",
    ),
    (
        "logistics.booking.reference_used",
        "client reference already used ({{ number }})",
    ),
    (
        "logistics.booking.row_parcels",
        "The number of parcels is a whole number.",
    ),
    (
        "logistics.booking.row_weight",
        "The weight is a whole number of grams.",
    ),
    (
        "logistics.booking.row_cod",
        "The amount to collect is in dirhams, like 150 or 150.50.",
    ),
    (
        "logistics.booking.csv_empty",
        "The file is empty: the first line names the columns.",
    ),
    (
        "logistics.booking.csv_columns",
        "The first line must name the column recipient_name.",
    ),
    // Scanning (`LOG.HUB.01`).
    ("logistics.scan.parcel_needed", "Scan at least one parcel."),
    ("logistics.scan.label_invalid", "label invalid"),
    (
        "logistics.scan.unknown_label",
        "The label {{ barcode }} is not a parcel of this shipment.",
    ),
    (
        "logistics.scan.not_received",
        "The parcel {{ barcode }} was not handed over at pickup and cannot be scanned in for this shipment. The client books a new shipment for it.",
    ),
    (
        "logistics.scan.both_lists",
        "The parcel {{ barcode }} is both scanned and marked not received.",
    ),
    (
        "logistics.scan.not_received_late",
        "Parcels can be marked not received only at pickup or at a drop-off.",
    ),
    // Fewer parcels than booked (BL-LOG-03).
    ("logistics.pickup.failed_default", "no reason given"),
    (
        "logistics.pickup.missing_title",
        "Shipment {{ number }}: {{ count }} parcel(s) not received",
    ),
    (
        "logistics.pickup.missing_body",
        "Not handed over at pickup: {{ parcels }}. Shipment {{ number }} goes ahead with the parcels received: {{ weight }}, priced at {{ price }}. The cash to collect is not changed ({{ cod }}). Book a new shipment for the missing parcels.",
    ),
    // The re-weigh at the hub (`LOG.HUB.01`, BL-LOG-04).
    (
        "logistics.reweigh.not_now",
        "A shipment is measured once it was picked up or dropped off, and while it is not closed.",
    ),
    (
        "logistics.reweigh.bad_values",
        "Give the measured weight above zero; the dimensions cannot be negative.",
    ),
    (
        "logistics.reweigh.photo_needed",
        "Attach the photo of the scale: the measured weight is above the client's tolerance.",
    ),
    (
        "logistics.reweigh.event",
        "measured {{ weight }}, chargeable {{ chargeable }}, surcharge {{ surcharge }}",
    ),
    ("logistics.reweigh.charge_note", "re-weigh surcharge"),
    (
        "logistics.reweigh.notice_title",
        "Shipment {{ number }} weighs more than booked",
    ),
    (
        "logistics.reweigh.notice_body",
        "Shipment {{ number }} was booked at {{ booked }} and measured at {{ measured }} at the hub. It is priced at {{ price }}; the re-weigh surcharge of {{ surcharge }} goes on your next invoice. You can dispute it from the shipment before the invoice is issued.",
    ),
    (
        "logistics.reweigh.no_surcharge",
        "There is no re-weigh surcharge to dispute. Once it is invoiced, dispute the invoice.",
    ),
    (
        "logistics.reweigh.already_disputed",
        "The surcharge is already disputed.",
    ),
    (
        "logistics.reweigh.no_dispute",
        "There is no open re-weigh dispute on this shipment.",
    ),
    (
        "logistics.reweigh.ticket_title",
        "Re-weigh surcharge of shipment {{ number }} disputed",
    ),
    (
        "logistics.reweigh.resolved_confirmed",
        "The surcharge is confirmed and goes on the next invoice.",
    ),
    (
        "logistics.reweigh.resolved_removed",
        "The surcharge is removed; the shipment is priced on the booked weight.",
    ),
    // Hold and release (BL-LOG-05).
    (
        "logistics.hold.reason_needed",
        "Give the exception: damaged, address issue, client instruction or re-weigh dispute.",
    ),
    (
        "logistics.hold.photos_needed",
        "Attach photos of the damaged parcel to hold the shipment.",
    ),
    (
        "logistics.hold.address_needed",
        "The client has not confirmed the address yet: record the confirmed address first.",
    ),
    (
        "logistics.hold.instruction_needed",
        "Record the client's instruction first: who asked, when and how.",
    ),
    ("logistics.hold.not_on_hold", "The shipment is not on hold."),
    (
        "logistics.hold.not_damaged",
        "Company liability is for a shipment held as damaged.",
    ),
    (
        "logistics.hold.not_address_hold",
        "The shipment is not held for an address issue.",
    ),
    ("logistics.hold.address_blank", "Give the address."),
    (
        "logistics.hold.address_locked",
        "The address can be corrected until the shipment is out for delivery.",
    ),
    (
        "logistics.hold.address_event",
        "address: {{ address }}, area: {{ area }}",
    ),
    (
        "logistics.hold.address_unchanged",
        "address unchanged, confirmed by the client",
    ),
    ("logistics.hold.title", "Shipment {{ number }} is on hold"),
    (
        "logistics.hold.body.damaged",
        "Shipment {{ number }} was damaged at the hub. Tell us what to do: deliver it as it is, return it to you, or dispose of it.",
    ),
    (
        "logistics.hold.body.address_issue",
        "We could not deliver shipment {{ number }}: the address needs your confirmation. Confirm or correct the address in the portal and we will deliver it.",
    ),
    (
        "logistics.hold.body.client_instruction",
        "Shipment {{ number }} is on hold until we have your instruction.",
    ),
    (
        "logistics.hold.body.reweigh_dispute",
        "Shipment {{ number }} is on hold while the re-weigh surcharge is disputed, as you asked.",
    ),
    (
        "logistics.hold.claim_title",
        "Claim: shipment {{ number }} damaged at the hub",
    ),
    (
        "logistics.hold.claim_description",
        "Shipment {{ number }} was damaged at the hub and the company is liable. Photos: {{ photos }}.",
    ),
    // Void (`PLT.ACL.09`, BL-LOG-01).
    (
        "logistics.void.scanned",
        "A parcel was scanned: the shipment can no longer be voided. Send it back with cancel and return.",
    ),
    (
        "logistics.void.not_your_shipment",
        "Only the user who booked the shipment, or staff who manage shipments, can void it.",
    ),
    (
        "logistics.void.reason_required",
        "Give the reason: duplicate, or entered in error.",
    ),
    ("logistics.void.delivery_reason", "shipment voided"),
    // Cancel and return (`LOG.ACL.09`, BL-LOG-02).
    (
        "logistics.cancel.picked_up",
        "The shipment was picked up: ask the dispatcher to send it back.",
    ),
    (
        "logistics.cancel.not_your_shipment",
        "Only a user of the client's company, or staff who manage shipments, can cancel it.",
    ),
    (
        "logistics.cancel.instruction_needed",
        "Record the client's instruction first: who asked, when and how.",
    ),
    (
        "logistics.cancel.no_return_price",
        "no return price for client {{ client }}",
    ),
    ("logistics.cancel.delivery_reason", "shipment cancelled"),
    (
        "logistics.cancel.returning_title",
        "Shipment {{ number }} is on its way back to you",
    ),
    (
        "logistics.cancel.returning_body",
        "As you asked, shipment {{ number }} goes back to you. A return charge applies as on your rate card.",
    ),
    (
        "logistics.return.pod_needed",
        "Capture the client's e-POD for the returned goods.",
    ),
    // The notice to a client.
    ("logistics.notice.subject", "{{ title }}"),
    ("logistics.notice.body", "{{ body }}"),
    // The pages.
    ("logistics.bad_input", "Check the fields and try again."),
    ("logistics.forbidden", "You may not do this."),
    ("logistics.booked", "Shipment booked."),
    ("logistics.imported", "File imported."),
    ("logistics.picked_up", "Shipment picked up."),
    ("logistics.hub_scanned", "Shipment scanned in at the hub."),
    ("logistics.cancelled", "Shipment cancelled."),
    ("logistics.voided", "Shipment voided."),
    ("logistics.instruction_recorded", "Instruction recorded."),
    (
        "logistics.cancel_returned",
        "Shipment is going back to the client.",
    ),
    ("logistics.returned", "Shipment returned to the client."),
    ("logistics.held", "Shipment put on hold."),
    ("logistics.released", "Shipment released to the hub."),
    ("logistics.hold_instruction", "Instruction recorded."),
    ("logistics.address_corrected", "Address corrected."),
    ("logistics.address_confirmed", "Address confirmed."),
    (
        "logistics.liable",
        "Company liability recorded; the claim ticket is linked.",
    ),
    (
        "logistics.sent_back",
        "Shipment is going back to the client.",
    ),
    ("logistics.reweighed", "Measurement recorded."),
    ("logistics.reweigh_disputed", "Surcharge disputed."),
    ("logistics.reweigh_resolved", "Dispute resolved."),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key, &[])
}

/// The English text for `key` with `params` filled in as `{{ name }}`; the
/// key itself when it has no text.
pub(crate) fn english(key: &str, params: &[(&str, &(dyn Display + Sync))]) -> String {
    let text = TEXTS
        .iter()
        .chain(SCREEN_TEXTS)
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| *text);
    let mut data = renox::serde_json::Map::new();
    for (name, value) in params {
        data.insert(
            (*name).to_owned(),
            renox::serde_json::Value::String(value.to_string()),
        );
    }
    wl_kernel::notify::fill(text, &renox::serde_json::Value::Object(data))
}

/// The English text of `key`, unfilled: a notification template.
pub(crate) fn template(key: &str) -> &'static str {
    TEXTS
        .iter()
        .chain(SCREEN_TEXTS)
        .find(|(k, _)| *k == key)
        .map_or("", |(_, text)| *text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS
            .iter()
            .chain(SCREEN_TEXTS)
            .map(|(key, _)| *key)
            .collect();
        assert!(keys.iter().all(|key| key.starts_with("logistics.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_text_is_filled_and_an_unknown_key_is_itself() {
        assert_eq!(
            english(
                "logistics.booking.reference_used",
                &[("number", &"SHP-0007")]
            ),
            "client reference already used (SHP-0007)"
        );
        assert_eq!(english("logistics.nothing", &[]), "logistics.nothing");
    }
}
