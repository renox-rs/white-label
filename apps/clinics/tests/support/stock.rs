//! The real wl-inventory behind the consumables of a visit: the branch store
//! of the doctor's branch with the filler lots L1 (2027-01-31), L2 (2027-03-31)
//! and the held L3, and the visits that use them.

#![allow(dead_code)]

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::item::value_of;
use wl_inventory::{Item, Location, Lot, LotStatus, StockMovement};
use wl_kernel::{PartyId, Subject};
use wl_scheduling::Booking;

use clinics::{ConsumableIssue, Visit};

use super::desk::Desk;
use super::{at, branch, day};

pub fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// The branch store with the filler lots L1 (expires 2027-01-31) and L2
/// (2027-03-31), and the lot L3 on hold.
pub struct Store {
    /// The branch store.
    pub store: Location,
    /// The filler.
    pub item: Item,
    /// Expires 2027-01-31.
    pub l1: Lot,
    /// Expires 2027-03-31.
    pub l2: Lot,
    /// On hold.
    pub l3: Lot,
}

/// A lot of `item` with its expiry date in `status`.
pub async fn lot(d: &Desk, item: &Item, number: &str, expiry: NaiveDate, status: LotStatus) -> Lot {
    let mut lot = Lot {
        item_id: item.id,
        number: number.to_owned(),
        status,
        expiry_date: Some(expiry),
        ..Lot::default()
    };
    let mut tx = d.t.db().begin().await.unwrap();
    lot.insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    lot
}

/// `quantity` units of `lot` received at the store.
pub async fn receive(d: &Desk, s: &Store, lot: &Lot, quantity: i64) {
    let source = Subject::new("inventory.goods_receipt", 900);
    let mut moved = StockMovement::receive(
        s.item.id,
        Some(lot.id),
        s.store.id,
        quantity * 1_000,
        500,
        value_of(quantity * 1_000, 500),
        &source,
        None,
    );
    moved.save(d.t.db()).await.unwrap();
}

/// The store of the branch the doctor works at: `l1` units of L1, `l2` of L2
/// and 5 of the held L3.
pub async fn store(d: &Desk, l1: i64, l2: i64) -> Store {
    let branch: Subject = branch();
    let mut location = Location::new("STORE-B2", "Branch store").in_scope(&branch);
    location.save(d.t.db()).await.unwrap();
    let mut item = Item::new("filler", "Filler", "pcs").lot_tracked();
    item.save(d.t.db()).await.unwrap();
    let first = lot(d, &item, "L1", date(2027, 1, 31), LotStatus::Released).await;
    let second = lot(d, &item, "L2", date(2027, 3, 31), LotStatus::Released).await;
    let held = lot(d, &item, "L3", date(2027, 2, 28), LotStatus::OnHold).await;
    let s = Store {
        store: location,
        item,
        l1: first,
        l2: second,
        l3: held,
    };
    if l1 > 0 {
        receive(d, &s, &s.l1, l1).await;
    }
    if l2 > 0 {
        receive(d, &s, &s.l2, l2).await;
    }
    receive(d, &s, &s.l3, 5).await;
    d.t.prices.set("filler", day(-400), 20_000);
    s
}

/// A checked-in, started visit of the consultation.
pub async fn open_visit(d: &Desk) -> (Booking, Visit) {
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    (booking, visit)
}

/// Records `units` of filler on the visit with `lot` (blank: none).
pub async fn record(
    d: &Desk,
    visit: &Visit,
    units: i64,
    lot: &str,
) -> renox::testing::TestResponse {
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/charges", visit.id),
        &[
            ("kind", "consumable"),
            ("code", "filler"),
            ("units", units.to_string().as_str()),
            ("lot", lot),
        ],
    )
    .await
}

/// The rows the stock issued for the visit, as (lot number, units, pending).
pub async fn issued(d: &Desk, visit: &Visit) -> Vec<(Option<String>, i64, bool)> {
    ConsumableIssue::of_visit(d.t.db(), visit.id)
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.lot_number.clone(),
                row.quantity_milli / 1_000,
                row.pending_issue_id.is_some(),
            )
        })
        .collect()
}

/// A started visit of `patient` on `days` days from today at `hour`, with the
/// booker's notes (the reason for the visit).
pub async fn visit_of(
    d: &Desk,
    patient: PartyId,
    days: i64,
    hour: u32,
    notes: &str,
) -> (Booking, Visit) {
    visit_with(d, &d.doctor, &d.consultation, patient, days, hour, notes).await
}

/// Like [`visit_of`] with `doctor` and the service `service`.
pub async fn visit_with(
    d: &Desk,
    doctor: &wl_scheduling::Resource,
    service_type: &wl_scheduling::ServiceType,
    patient: PartyId,
    days: i64,
    hour: u32,
    notes: &str,
) -> (Booking, Visit) {
    use wl_scheduling::service::{self, Actor, NewBooking};
    let mut new = NewBooking::new(
        patient,
        service_type.id,
        &[doctor.id],
        at(day(days), hour),
    )
    .for_subject(patient.subject());
    new.notes = Some(notes.to_owned());
    let booking = service::book(d.t.state(), new, Actor::staff(1), super::scheduling::now())
        .await
        .expect("the booking is made");
    if booking.deposit_amount > 0 {
        d.t.deposits
            .paid(booking.id, patient, booking.deposit_amount);
    }
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    (booking, visit)
}

/// The patient's visit on `days` days from today at `hour` that used `units`
/// of filler from `lot` (blank: none) and was checked out; the visit as it
/// is now.
pub async fn used_filler(
    d: &Desk,
    patient: PartyId,
    days: i64,
    hour: u32,
    units: i64,
    lot: &str,
) -> Visit {
    used_filler_with(d, &d.doctor, &d.consultation, patient, days, hour, units, lot).await
}

/// Like [`used_filler`] with `doctor` and the service `service_type`.
#[allow(clippy::too_many_arguments)]
pub async fn used_filler_with(
    d: &Desk,
    doctor: &wl_scheduling::Resource,
    service_type: &wl_scheduling::ServiceType,
    patient: PartyId,
    days: i64,
    hour: u32,
    units: i64,
    lot: &str,
) -> Visit {
    use wl_scheduling::service::{self, Actor};
    let (booking, visit) = visit_with(
        d,
        doctor,
        service_type,
        patient,
        days,
        hour,
        "Chest pain, suspected angina",
    )
    .await;
    record(d, &visit, units, lot).await.assert_ok();
    service::complete(
        d.t.state(),
        booking.id,
        Actor::staff(d.rita.id),
        at(day(days), 18),
    )
    .await
    .expect("the visit is checked out");
    d.visit(visit.id).await
}

/// The second branch: a practitioner and a service there, and a store of its
/// own holding `l1` units of the lot L1 of the filler.
pub struct Elsewhere {
    /// The practitioner of the second branch.
    pub doctor: wl_scheduling::Resource,
    /// The service the practitioner offers.
    pub service: wl_scheduling::ServiceType,
    /// The store of the second branch.
    pub store: Location,
}

/// Sets up the second branch next to the store `s` of the first.
pub async fn elsewhere(d: &Desk, s: &Store, l1: i64) -> Elsewhere {
    let doctor = d
        .t
        .practitioner_at("Dr B", 62, &super::other_branch(), None)
        .await;
    let service = d.t.service_type("consultation_b", &[&doctor]).await;
    let mut store = Location::new("STORE-B3", "Second branch store").in_scope(&super::other_branch());
    store.save(d.t.db()).await.unwrap();
    let source = Subject::new("inventory.goods_receipt", 901);
    let mut moved = StockMovement::receive(
        s.item.id,
        Some(s.l1.id),
        store.id,
        l1 * 1_000,
        500,
        value_of(l1 * 1_000, 500),
        &source,
        None,
    );
    moved.save(d.t.db()).await.unwrap();
    Elsewhere {
        doctor,
        service,
        store,
    }
}
