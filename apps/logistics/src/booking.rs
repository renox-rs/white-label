//! Booking a shipment (03 Logistics §4.1 steps 1 and 2, `LOG.SHP.01`;
//! BL-LOG-01): one at a time from the portal or the back office, or many from
//! a CSV file.
//!
//! - **Who books.** A user with `shipments.book`: staff with
//!   `shipments.manage` for any client, a client user only for the company
//!   the portal says they act for.
//! - **Client reference.** Optional; when given it is unique per client across
//!   all states, `voided` and `cancelled` included (a real re-booking needs a
//!   new reference). A booking without one is not checked. The unique index
//!   on (client, client reference) is the guard; the read before it only
//!   gives the message "client reference already used (SHP-0007)".
//! - **CSV.** The header names the columns (`client_reference`,
//!   `recipient_name`, `recipient_phone`, `address`, `area`, `parcels`,
//!   `weight_grams`, `cod`, with `cod` in dirhams). A dry run lists every row
//!   with what would happen, and books nothing; two rows with one reference in
//!   a file: the first books, the second is refused. The other rows book.
//! - **Delivery.** When the delivery module is on, booking asks wl-delivery for
//!   the shipment's delivery (the shipment is its source); the dispatcher
//!   plans it onto a run once the shipment is at the hub.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::Serialize;
use wl_kernel::delivery::{DeliveryItem, DeliveryRequest, DeliveryRequester};
use wl_kernel::documents::today;
use wl_kernel::portal::portal_users;
use wl_kernel::{PartyId, Subject};

use crate::permissions;
use crate::shipment::{Parcel, SHIPMENT_SERIES, Shipment, ShipmentStatus, parcel_state, track};
use crate::texts::english;

/// What a booking carries.
#[derive(Debug, Clone, Default)]
pub struct NewShipment {
    /// The client (a party).
    pub client: PartyId,
    /// The shipper's own reference; blank is none.
    pub client_reference: Option<String>,
    /// The recipient's name.
    pub recipient_name: String,
    /// The recipient's phone.
    pub recipient_phone: String,
    /// The address as text.
    pub address: String,
    /// The area.
    pub area: String,
    /// The weight of each parcel, in grams; one entry per parcel.
    pub parcel_weights_grams: Vec<i64>,
    /// The cash to collect, in fils.
    pub cod_amount: i64,
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// Whether `user` may book for `client`: a booker with `shipments.manage`
/// for any client, any other booker for the company the portal says they act
/// for.
///
/// # Errors
///
/// The portal's errors.
pub async fn may_book_for(state: &AppState, user: &AuthUser, client: PartyId) -> Result<bool> {
    if !user.has_permission(permissions::SHIPMENTS_BOOK) {
        return Ok(false);
    }
    if user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Ok(true);
    }
    Ok(portal_users(state)
        .acting_for(&state.db, user.id, client, today())
        .await?
        .is_some())
}

/// Whether `user` acts for the company `client` in the portal (a client
/// user's `cancel`).
///
/// # Errors
///
/// The portal's errors.
pub async fn acts_for(state: &AppState, user: &AuthUser, client: PartyId) -> Result<bool> {
    Ok(portal_users(state)
        .acting_for(&state.db, user.id, client, today())
        .await?
        .is_some())
}

/// The reference as stored: trimmed, none when blank.
fn clean_reference(reference: &Option<String>) -> Option<String> {
    reference
        .as_deref()
        .map(str::trim)
        .filter(|reference| !reference.is_empty())
        .map(str::to_owned)
}

/// What is wrong with `new`, if anything.
pub fn problem_with(new: &NewShipment) -> Option<String> {
    if new.recipient_name.trim().is_empty() {
        return Some(english("logistics.booking.recipient_needed", &[]));
    }
    if new.parcel_weights_grams.is_empty() {
        return Some(english("logistics.booking.parcel_needed", &[]));
    }
    if new.parcel_weights_grams.iter().any(|weight| *weight < 0) || new.cod_amount < 0 {
        return Some(english("logistics.booking.negative", &[]));
    }
    None
}

/// The shipment of `client` that already uses `reference`, in any state.
///
/// # Errors
///
/// Database errors.
pub async fn using_reference(
    db: &Db,
    client: PartyId,
    reference: &str,
) -> Result<Option<Shipment>> {
    Shipment::query()
        .where_(Shipment::CLIENT_PARTY_ID.eq(client.0))
        .where_(Shipment::CLIENT_REFERENCE.eq(reference))
        .first(db)
        .await
}

fn reference_used(number: &str) -> TransitionError {
    guard(english(
        "logistics.booking.reference_used",
        &[("number", &number)],
    ))
}

/// `user` books `new`: the shipment is `created` with its parcels and
/// labels, and its delivery is requested.
///
/// # Errors
///
/// `Forbidden` when `user` may not book for the client; `Guard` (422) when
/// the booking is incomplete or the client reference is already used (the
/// message names the shipment that uses it).
pub async fn book(
    state: &AppState,
    user: &AuthUser,
    new: &NewShipment,
) -> std::result::Result<Shipment, TransitionError> {
    if !may_book_for(state, user, new.client).await? {
        return Err(TransitionError::Forbidden);
    }
    // A blocked client books nothing new, with the neutral text (`PTY.ACL.03`).
    crate::block::refuse_start(state, new.client).await?;
    create(state, Some(user.id), new).await
}

/// Books `new` for the user `booked_by` (none for the system: the demo seed),
/// without asking who may: [`book`] and the demo seed are the callers.
///
/// # Errors
///
/// As [`book`], without `Forbidden`.
pub async fn create(
    state: &AppState,
    booked_by: Option<i64>,
    new: &NewShipment,
) -> std::result::Result<Shipment, TransitionError> {
    if let Some(problem) = problem_with(new) {
        return Err(guard(problem));
    }
    let reference = clean_reference(&new.client_reference);
    if let Some(reference) = &reference
        && let Some(used) = using_reference(&state.db, new.client, reference).await?
    {
        return Err(reference_used(&used.number));
    }
    let number = SHIPMENT_SERIES
        .next_allowing_gaps(&state.db, None, None, today())
        .await?;
    let booked_weight: i64 = new.parcel_weights_grams.iter().sum();
    let price = crate::rates::band_price(&state.db, new.client, new.area.trim(), booked_weight)
        .await?
        .unwrap_or(0);
    let mut shipment = Shipment {
        number: number.clone(),
        client_party_id: new.client.0,
        client_reference: reference.clone(),
        booked_by_user_id: booked_by,
        recipient_name: new.recipient_name.trim().to_owned(),
        recipient_phone: new.recipient_phone.trim().to_owned(),
        address: new.address.trim().to_owned(),
        area: new.area.trim().to_owned(),
        cod_amount: new.cod_amount,
        booked_weight_grams: booked_weight,
        weight_grams: booked_weight,
        price,
        parcel_count: i64::try_from(new.parcel_weights_grams.len()).unwrap_or(i64::MAX),
        status: ShipmentStatus::Created,
        ..Shipment::default()
    };
    let mut tx = state.db.begin().await?;
    if let Err(error) = shipment.insert(&mut tx).await {
        // Give the connection back before reading again.
        drop(tx);
        if error.is_unique_violation()
            && let Some(reference) = &reference
            && let Some(used) = using_reference(&state.db, new.client, reference).await?
        {
            return Err(reference_used(&used.number));
        }
        return Err(error.into());
    }
    let mut parcels = Vec::new();
    for (at, weight) in new.parcel_weights_grams.iter().enumerate() {
        let mut parcel = Parcel {
            shipment_id: shipment.id,
            barcode: format!("{number}-{}", at + 1),
            weight_grams: *weight,
            state: parcel_state::BOOKED.to_owned(),
            ..Parcel::default()
        };
        parcel.insert(&mut tx).await?;
        parcels.push(parcel);
    }
    track(&mut tx, shipment.id, "booked", "", booked_by).await?;
    tx.commit().await?;
    request_delivery(state, &mut shipment, &parcels).await?;
    Ok(shipment)
}

/// Asks wl-delivery for the delivery of `shipment`, when the module is on,
/// and links it. A refused request is written to the tracking events and the
/// shipment stays booked.
async fn request_delivery(state: &AppState, shipment: &mut Shipment, parcels: &[Parcel]) -> Result {
    if state
        .provided::<std::sync::Arc<dyn DeliveryRequester>>()
        .is_none()
    {
        return Ok(());
    }
    let mut request = DeliveryRequest::new(shipment.subject(), &shipment.recipient_name)
        .address(&shipment.address, &shipment.area)
        .phone(&shipment.recipient_phone)
        .collect(shipment.cod_amount)
        // A recipient may refuse some parcels of several, never the only
        // one: refusing a single parcel is a failed attempt (BL-LOG-07,
        // `WRK.POD.01`).
        .partial_acceptance(parcels.len() > 1);
    for parcel in parcels {
        request = request.item(
            DeliveryItem::new(&format!("Parcel {}", parcel.barcode), 1_000)
                .source_line(Subject::new(crate::shipment::PARCEL_SUBJECT_TYPE, parcel.id)),
        );
    }
    match wl_kernel::delivery::requester(state)
        .request(state, request)
        .await
    {
        Ok(delivery) => {
            let linked = Some(delivery.0);
            Shipment::query()
                .where_(Shipment::ID.eq(shipment.id))
                .update(&state.db, &[("delivery_id", &linked)])
                .await?;
            shipment.delivery_id = linked;
        }
        Err(error) => {
            let note = format!("{error:?}");
            track(
                &state.db,
                shipment.id,
                "delivery_request_failed",
                &note,
                None,
            )
            .await?;
        }
    }
    Ok(())
}

/// What a CSV row came to.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RowResult {
    /// The row's line in the file (the header is line 1).
    pub row: usize,
    /// The client reference of the row.
    pub client_reference: Option<String>,
    /// The number of the shipment booked, or that would be booked (none in a
    /// dry run).
    pub shipment_number: Option<String>,
    /// Why the row is refused; none when it books.
    pub refused: Option<String>,
}

/// What an import (or its dry run) did, row by row.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct ImportReport {
    /// Whether nothing was booked.
    pub dry_run: bool,
    /// The rows, in file order.
    pub rows: Vec<RowResult>,
    /// How many rows book (or booked).
    pub accepted: usize,
    /// How many rows are refused.
    pub refused: usize,
}

/// One line of a CSV file as its fields (`"a,b"` and `""` are understood).
pub fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    fields.push(field);
    fields.into_iter().map(|f| f.trim().to_owned()).collect()
}

/// Dirhams as fils: `150`, `150.5` and `150.50` are 15000, 15050 and 15050.
pub fn parse_dirhams(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return Some(0);
    }
    let (whole, cents) = match text.split_once('.') {
        Some((whole, cents)) => (whole, cents),
        None => (text, ""),
    };
    if cents.len() > 2 || !cents.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let cents: i64 = format!("{cents:0<2}").parse().ok()?;
    whole.checked_mul(100)?.checked_add(cents)
}

/// The shipment a CSV row asks for, or why the row is refused.
fn row_to_new(
    client: PartyId,
    columns: &BTreeMap<String, usize>,
    fields: &[String],
) -> std::result::Result<NewShipment, String> {
    let get = |name: &str| -> String {
        columns
            .get(name)
            .and_then(|at| fields.get(*at))
            .cloned()
            .unwrap_or_default()
    };
    let parcels = match get("parcels").as_str() {
        "" => 1,
        text => text
            .parse::<usize>()
            .map_err(|_| english("logistics.booking.row_parcels", &[]))?,
    };
    let weight = match get("weight_grams").as_str() {
        "" => 0,
        text => text
            .parse::<i64>()
            .map_err(|_| english("logistics.booking.row_weight", &[]))?,
    };
    let cod_amount =
        parse_dirhams(&get("cod")).ok_or_else(|| english("logistics.booking.row_cod", &[]))?;
    Ok(NewShipment {
        client,
        client_reference: Some(get("client_reference")),
        recipient_name: get("recipient_name"),
        recipient_phone: get("recipient_phone"),
        address: get("address"),
        area: get("area"),
        parcel_weights_grams: vec![weight; parcels],
        cod_amount,
    })
}

/// `user` imports the CSV `text` for `client`: every valid row books, every
/// refused row is listed with its reason. With `dry_run` nothing is booked.
///
/// # Errors
///
/// `Forbidden` when `user` may not book for the client; `Guard` (422) when
/// the file has no `recipient_name` column.
pub async fn import_csv(
    state: &AppState,
    user: &AuthUser,
    client: PartyId,
    text: &str,
    dry_run: bool,
) -> std::result::Result<ImportReport, TransitionError> {
    if !may_book_for(state, user, client).await? {
        return Err(TransitionError::Forbidden);
    }
    // For a blocked client every row is refused with the neutral text, also
    // in a dry run (`PTY.ACL.03`).
    let blocked = match crate::block::refuse_start(state, client).await {
        Ok(()) => None,
        Err(TransitionError::Guard(message)) => Some(message),
        Err(other) => return Err(other),
    };
    if let (Some(message), false) = (&blocked, dry_run) {
        return Err(guard(message.clone()));
    }
    let mut lines = text.lines().enumerate();
    let Some((_, header)) = lines.next() else {
        return Err(guard(english("logistics.booking.csv_empty", &[])));
    };
    let columns: BTreeMap<String, usize> = split_csv_line(header)
        .into_iter()
        .enumerate()
        .map(|(at, name)| (name.to_lowercase(), at))
        .collect();
    if !columns.contains_key("recipient_name") {
        return Err(guard(english("logistics.booking.csv_columns", &[])));
    }
    let mut report = ImportReport {
        dry_run,
        ..ImportReport::default()
    };
    // The reference of each row a dry run accepted, for a second row with it.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (at, line) in lines {
        if line.trim().is_empty() {
            continue;
        }
        let row = at + 1;
        let fields = split_csv_line(line);
        let outcome: std::result::Result<Option<String>, String> =
            match row_to_new(client, &columns, &fields) {
                Err(problem) => Err(problem),
                Ok(new) => {
                    let reference = clean_reference(&new.client_reference);
                    if let Some(problem) = problem_with(&new) {
                        Err(problem)
                    } else if let Some(message) = &blocked {
                        Err(message.clone())
                    } else if dry_run {
                        let used = match &reference {
                            Some(reference) => {
                                match using_reference(&state.db, client, reference).await? {
                                    Some(shipment) => Some(shipment.number),
                                    None => seen.get(reference).map(|first| format!("row {first}")),
                                }
                            }
                            None => None,
                        };
                        match used {
                            Some(number) => Err(english(
                                "logistics.booking.reference_used",
                                &[("number", &number)],
                            )),
                            None => {
                                if let Some(reference) = reference {
                                    seen.insert(reference, row);
                                }
                                Ok(None)
                            }
                        }
                    } else {
                        match book(state, user, &new).await {
                            Ok(shipment) => Ok(Some(shipment.number)),
                            Err(TransitionError::Guard(message)) => Err(message),
                            Err(other) => return Err(other),
                        }
                    }
                }
            };
        let reference = fields
            .get(
                columns
                    .get("client_reference")
                    .copied()
                    .unwrap_or(usize::MAX),
            )
            .filter(|reference| !reference.is_empty())
            .cloned();
        match outcome {
            Ok(number) => {
                report.accepted += 1;
                report.rows.push(RowResult {
                    row,
                    client_reference: reference,
                    shipment_number: number,
                    refused: None,
                });
            }
            Err(reason) => {
                report.refused += 1;
                report.rows.push(RowResult {
                    row,
                    client_reference: reference,
                    shipment_number: None,
                    refused: Some(reason),
                });
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_csv_line_keeps_quoted_commas_and_quotes() {
        assert_eq!(
            split_csv_line(r#"ORD-1, "Villa 12, Al Barsha" ,"say ""hi""""#),
            ["ORD-1", "Villa 12, Al Barsha", r#"say "hi""#]
        );
    }

    #[test]
    fn dirhams_become_fils() {
        assert_eq!(parse_dirhams("150"), Some(15_000));
        assert_eq!(parse_dirhams("150.5"), Some(15_050));
        assert_eq!(parse_dirhams("150.50"), Some(15_050));
        assert_eq!(parse_dirhams(""), Some(0));
        assert_eq!(parse_dirhams("150.505"), None);
        assert_eq!(parse_dirhams("abc"), None);
    }
}
