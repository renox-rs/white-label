//! The labour operations catalogue (08 `AUT.JOB.03`): the menu of the
//! workshop. A flat-rate operation has a flat price for standard hours (oil
//! change, brakes, periodic service); an hourly one an hourly rate
//! (diagnostics, body work). A labour line is drafted from an operation and
//! keeps its own price, so a change of the menu never moves an estimate.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::labour_line::LabourPricing;

/// A labour operation of the menu.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_labour_operations", unique(code), index(active))]
pub struct LabourOperation {
    /// The primary key.
    pub id: i64,
    /// The code the workshop knows it by (`OIL-CHG`); unique.
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// Hourly or flat-rate.
    pub pricing: LabourPricing,
    /// The standard (flat) hours, in thousandths; 0 for an hourly operation.
    pub hours_milli: i64,
    /// The hourly rate, or the flat price, in fils.
    pub rate: i64,
    /// Whether it is on the menu.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LabourOperation {
    /// The operation `code`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn by_code(db: &Db, code: &str) -> Result<Option<LabourOperation>> {
        LabourOperation::query()
            .where_(LabourOperation::CODE.eq(code.trim().to_uppercase()))
            .first(db)
            .await
    }

    /// The operation for people: its code and its English name.
    pub fn title(&self) -> String {
        format!("{} {}", self.code, self.name_en)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_operation_is_named_by_code_and_name() {
        let op = LabourOperation {
            code: "OIL-CHG".to_owned(),
            name_en: "Oil change".to_owned(),
            ..LabourOperation::default()
        };
        assert_eq!(op.title(), "OIL-CHG Oil change");
    }
}
