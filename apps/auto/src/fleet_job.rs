//! A job card under a fleet contract (08 §4.5 steps 3 and 4): billed in the
//! contract's consolidated monthly invoice with the fleet's PO reference, not
//! on delivery.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// A job card of a vehicle under a fleet contract.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_fleet_jobs", unique(job_card_id), index(contract_id))]
pub struct FleetJob {
    /// The primary key.
    pub id: i64,
    /// The contract.
    #[model(references = "auto_fleet_contracts")]
    pub contract_id: i64,
    /// The job card.
    #[model(references = "auto_job_cards")]
    pub job_card_id: i64,
    /// The vehicle.
    pub vehicle_id: i64,
    /// The fleet's PO reference, printed on the invoice line.
    pub po_reference: Option<String>,
    /// The consolidated invoice that billed the job.
    pub invoice_id: Option<i64>,
    /// When it was billed.
    pub invoiced_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl FleetJob {
    /// The fleet job of job card `job_card_id`, if it is under a contract.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_job_card(db: &Db, job_card_id: i64) -> Result<Option<FleetJob>> {
        FleetJob::query()
            .where_(FleetJob::JOB_CARD_ID.eq(job_card_id))
            .first(db)
            .await
    }

    /// The jobs of `contract_id`, newest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_contract(db: &Db, contract_id: i64) -> Result<Vec<FleetJob>> {
        FleetJob::query()
            .where_(FleetJob::CONTRACT_ID.eq(contract_id))
            .order_by_desc(FleetJob::ID)
            .get(db)
            .await
    }
}
