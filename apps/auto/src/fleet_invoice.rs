//! One period of a fleet contract billed in a consolidated invoice (08 §4.5
//! step 4, `FIN.USG.01`, `AUT.RUL.05`).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// The monthly invoice of a contract.
pub const KIND_MONTHLY: &str = "monthly";
/// The last invoice of a contract that ended.
pub const KIND_FINAL: &str = "final";

/// A period of a contract that was billed.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_fleet_invoices", unique(contract_id, period_start_date))]
pub struct FleetInvoice {
    /// The primary key.
    pub id: i64,
    /// The contract.
    #[model(references = "auto_fleet_contracts")]
    pub contract_id: i64,
    /// The first day of the period.
    pub period_start_date: NaiveDate,
    /// The last day of the period.
    pub period_end_date: NaiveDate,
    /// `monthly` or `final`.
    pub kind: String,
    /// The invoice in wl-invoicing; none for a period with nothing to bill.
    pub invoice_id: Option<i64>,
    /// How many jobs the invoice lists.
    pub jobs: i64,
    /// The invoice's total before VAT, in fils.
    pub net_total: i64,
    /// The user who billed it; none for the daily job.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl FleetInvoice {
    /// The periods of `contract_id`, newest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_contract(db: &Db, contract_id: i64) -> Result<Vec<FleetInvoice>> {
        FleetInvoice::query()
            .where_(FleetInvoice::CONTRACT_ID.eq(contract_id))
            .order_by_desc(FleetInvoice::PERIOD_START_DATE)
            .get(db)
            .await
    }
}
