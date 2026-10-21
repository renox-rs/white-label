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
    ("logistics.remittance_built", "Draft remittance built."),
    ("logistics.remittance_approved", "Remittance approved."),
    ("logistics.remittance_reopened", "Remittance back to a draft."),
    ("logistics.remittance_paid", "Remittance paid."),
    // A COD difference (`WRK.ACL.09`, BL-LOG-08).
    (
        "logistics.cod.customer_not_offered",
        "The customer cannot absorb a COD difference: choose the client, the business or the driver.",
    ),
    (
        "logistics.cod.instruction_needed",
        "The client absorbs a difference only on its instruction: give who asked, when and how.",
    ),
    (
        "logistics.cod_instruction_recorded",
        "The client's instruction is recorded.",
    ),
    // Part of a multi-parcel shipment refused (`WRK.POD.01`, BL-LOG-07).
    (
        "logistics.partial.single_parcel",
        "A shipment of one parcel has no partial acceptance: refusing it is a failed attempt.",
    ),
    (
        "logistics.partial.reason_needed",
        "Every refused parcel needs a reason.",
    ),
    (
        "logistics.partial.not_a_parcel",
        "A refused parcel is not a parcel of this shipment.",
    ),
    (
        "logistics.partial.plan_missing",
        "The return shipment for the refused parcels could not be prepared.",
    ),
    ("logistics.partial.client_name", "Client {{ client }}"),
    (
        "logistics.partial.no_pickup_address",
        "Pickup address of client {{ client }} (not on file)",
    ),
    (
        "logistics.partial.created_note",
        "Return of the refused parcels of {{ number }}: {{ parcels }}",
    ),
    (
        "logistics.partial.refused_note",
        "Refused parcels {{ parcels }} go back on {{ number }}",
    ),
    // A delivery under review (`WRK.ACL.06`, BL-LOG-09).
    (
        "logistics.review.ticket_needed",
        "Link the dispute ticket of the delivery first.",
    ),
    (
        "logistics.review.title",
        "Shipment {{ number }} is under review",
    ),
    (
        "logistics.review.body",
        "The delivery of shipment {{ number }} is disputed (ticket {{ ticket }}). Its cash on delivery and its billing wait until the review is closed.",
    ),
    (
        "logistics.review.body_no_ticket",
        "The delivery of shipment {{ number }} is disputed. Its cash on delivery and its billing wait until the review is closed.",
    ),
    (
        "logistics.review.reason_needed",
        "Give the reason of the dispute.",
    ),
    (
        "logistics.review.not_delivered",
        "Only a delivered shipment can be disputed.",
    ),
    (
        "logistics.review.no_delivery",
        "The shipment has no delivery to put under review.",
    ),
    (
        "logistics.disputed",
        "The delivery is under review; its cash on delivery and billing are held.",
    ),
    (
        "logistics.remittance.reopened_subject",
        "COD remittance {{ number }} was sent back to a draft",
    ),
    (
        "logistics.remittance.reopened_body",
        "Shipment {{ shipment }} on it is under review; its line was taken off and the draft rebuilt.",
    ),
    // COD remittance (`LOG.RUL.06`, `LOG.ACL.07`, `LOG.ACL.12`; BL-LOG-14).
    (
        "logistics.remittance.notice_subject",
        "COD remittance {{ number }} {{ what }}",
    ),
    ("logistics.remittance.notice_body", "{{ summary }}"),
    ("logistics.remittance.notice_created", "was drafted"),
    ("logistics.remittance.notice_rebuilt", "was rebuilt"),
    (
        "logistics.remittance.notice_summary",
        "COD {{ cod }}, fees {{ fees }}, adjustments {{ adjustments }}, net {{ net }}.",
    ),
    (
        "logistics.remittance.notice_carried",
        "{{ carried }} is carried to the next draft as an adjustment line.",
    ),
    (
        "logistics.remittance.carried_reason",
        "Carried forward from {{ number }}: the adjustments were larger than the COD.",
    ),
    (
        "logistics.remittance.carried_note",
        "Derived by the weekly run from approved lines.",
    ),
    (
        "logistics.remittance.nothing_to_draft",
        "There is no delivered COD and no open adjustment line to put on a draft.",
    ),
    (
        "logistics.remittance.own_draft",
        "You prepared this remittance: another user approves it.",
    ),
    (
        "logistics.remittance.unapproved_lines",
        "{{ count }} adjustment line(s) on this remittance are not approved yet.",
    ),
    (
        "logistics.remittance.reference_needed",
        "Give the bank reference of the payment.",
    ),
    (
        "logistics.remittance.account_needed",
        "The client has no bank account on file for its remittances.",
    ),
    (
        "logistics.remittance.client_blocked",
        "This client is on hold: the remittance waits.",
    ),
    (
        "logistics.remittance.nothing_to_pay",
        "The net is AED 0.00: there is nothing to pay.",
    ),
    (
        "logistics.remittance.payout_reversed",
        "Payout of {{ number }} reversed: the remittance was not paid.",
    ),
    (
        "logistics.adjustment.own_line",
        "You prepared this adjustment line: another user approves it.",
    ),
    (
        "logistics.adjustment.not_your_line",
        "Only the user who prepared the line resubmits it.",
    ),
    (
        "logistics.adjustment.amount_needed",
        "Give the amount of the line: a positive amount is owed to the client, a negative one is taken back.",
    ),
    ("logistics.adjustment.reason_needed", "Give the reason: the client reads it on its statement."),
    ("logistics.adjustment.source_unknown", "Choose where the line comes from."),
    (
        "logistics.adjustment.shipment_unknown",
        "That shipment is not one of the client's.",
    ),
    (
        "logistics.adjustment.remittance_unknown",
        "That remittance is not one of the client's.",
    ),
    (
        "logistics.adjustment.reopen_first",
        "The remittance is approved: reopen it to a draft before adding a line.",
    ),
    (
        "logistics.adjustment.paid_not_edited",
        "{{ number }} is paid and is never edited: the line goes on the client's next draft.",
    ),
    (
        "logistics.adjustment.approved_kept",
        "An approved line is never deleted or edited: correct it with another line.",
    ),
    ("logistics.adjustment_added", "Adjustment line added."),
    ("logistics.adjustment_approved", "Adjustment line approved."),
    ("logistics.adjustment_rejected", "Adjustment line rejected."),
    ("logistics.adjustment_resubmitted", "Adjustment line sent again."),
    ("logistics.adjustment_deleted", "Adjustment line deleted."),
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
