//! What an employee owes (wl-hr §4 *Run types*, §5.6): the negative
//! difference of a supplementary run for an employee who left after their
//! settlement was paid, or who has no settlement to take it ([`crate::supplementary`]).
//! A receivable is kept on the employee's party. Decision: no kernel contract
//! for receivables exists yet (wl-payments would provide it), so wl-hr keeps
//! the row; the accounting side reads it when that contract comes. A final
//! settlement makes one for the recoveries above the Art. 25 caps
//! ([`crate::settlement_offset`]).

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// Money an employee owes the company.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_receivables", index(employee_id, status))]
pub struct Receivable {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// The employee's party, the debtor.
    pub party_id: i64,
    /// The amount, AED fils, positive.
    pub amount: i64,
    /// The run that caused it.
    pub source_run_id: Option<i64>,
    /// The payroll line that caused it.
    pub source_line_id: Option<i64>,
    /// The final settlement that caused it (recoveries above the caps, or a
    /// revised net below what was paid).
    pub settlement_id: Option<i64>,
    /// `open` or `settled`.
    pub status: String,
    /// A note.
    pub note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Receivable {
    /// The receivables of `employee_id`, oldest first.
    pub async fn of_employee(db: &Db, employee_id: i64) -> Result<Vec<Receivable>> {
        Receivable::query()
            .where_(Receivable::EMPLOYEE_ID.eq(employee_id))
            .order_by(Receivable::ID)
            .get(db)
            .await
    }
}
