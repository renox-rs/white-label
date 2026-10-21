//! The driver's day list (wl-delivery §6 `DriverDayList`, `WRK.ACL.05`): the
//! stops of the runs a driver is on for one day, in order. The recipient's
//! phone and the full address are shown only while the stop is open; the
//! driver sees nothing of other drivers' runs.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;

use crate::delivery::{Delivery, DeliveryStatus};
use crate::run::{Run, RunStatus};

/// One stop on the driver's list.
#[derive(Debug, Clone)]
pub struct Stop {
    /// The run it is on.
    pub run: Run,
    /// The delivery.
    pub delivery: Delivery,
}

impl Stop {
    /// Whether the stop still waits for its outcome: the contact details are
    /// shown only then.
    pub fn is_open(&self) -> bool {
        matches!(
            self.delivery.status,
            DeliveryStatus::Assigned | DeliveryStatus::OutForDelivery
        )
    }

    /// The stop as the driver's page shows it.
    pub fn view(&self) -> StopView {
        let open = self.is_open();
        StopView {
            id: self.delivery.id,
            number: self.delivery.number.clone(),
            run: self.run.number.clone(),
            sequence: self.delivery.run_sequence.unwrap_or_default(),
            recipient: self.delivery.recipient_name.clone(),
            area: self.delivery.area.clone(),
            address: open.then(|| self.delivery.address.clone()),
            phone: open.then(|| self.delivery.recipient_phone.clone()),
            status: self.delivery.status.as_str().to_owned(),
            open,
            can_complete: self.delivery.status == DeliveryStatus::OutForDelivery,
        }
    }
}

/// A stop for the template.
#[derive(Debug, Clone, Serialize)]
pub struct StopView {
    /// The delivery's id.
    pub id: i64,
    /// The delivery's number.
    pub number: String,
    /// The run's number.
    pub run: String,
    /// The place on the run.
    pub sequence: i64,
    /// The recipient's name.
    pub recipient: String,
    /// The area.
    pub area: String,
    /// The full address, while the stop is open.
    pub address: Option<String>,
    /// The phone, while the stop is open.
    pub phone: Option<String>,
    /// The delivery's status key.
    pub status: String,
    /// Whether the stop waits for its outcome.
    pub open: bool,
    /// Whether the driver can record the outcome now (the run left).
    pub can_complete: bool,
}

/// The stops of `driver_user_id` on `day`, run by run in stop order. Runs
/// that were cancelled or closed show nothing.
///
/// # Errors
///
/// Database errors.
pub async fn stops_of(db: &Db, driver_user_id: i64, day: NaiveDate) -> Result<Vec<Stop>> {
    let runs = Run::query()
        .where_(Run::DRIVER_USER_ID.eq(driver_user_id))
        .where_(Run::RUN_DATE.eq(day))
        .order_by(Run::ID)
        .get(db)
        .await?;
    let mut stops = Vec::new();
    for run in runs.into_iter().filter(|run| {
        matches!(
            run.status,
            RunStatus::Planned | RunStatus::Dispatched | RunStatus::ReturnedToDepot
        )
    }) {
        let deliveries = Delivery::query()
            .where_(Delivery::RUN_ID.eq(run.id))
            .order_by(Delivery::RUN_SEQUENCE)
            .get(db)
            .await?;
        for delivery in deliveries {
            stops.push(Stop {
                run: run.clone(),
                delivery,
            });
        }
    }
    Ok(stops)
}

/// The stop `delivery_id` when it is on a run of `driver_user_id` for `day`;
/// `None` for anyone else's stop (the page answers 404).
///
/// # Errors
///
/// Database errors.
pub async fn stop_of(
    db: &Db,
    driver_user_id: i64,
    day: NaiveDate,
    delivery_id: i64,
) -> Result<Option<Stop>> {
    Ok(stops_of(db, driver_user_id, day)
        .await?
        .into_iter()
        .find(|stop| stop.delivery.id == delivery_id))
}
