//! A test drive of a job card (08 §6 *Job card*, `AUT.ACL.22`, BL-AUT-06): who
//! drove, when the vehicle left and came back and the mileage at both ends. A
//! drive without mileage in is open and blocks `pass_qc`.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// A test drive.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_test_drives", index(job_card_id))]
pub struct TestDrive {
    /// The primary key.
    pub id: i64,
    /// The job card.
    #[model(references = "auto_job_cards")]
    pub job_card_id: i64,
    /// The user who drove.
    pub driver_user_id: i64,
    /// When the vehicle left the workshop.
    pub out_at: DateTime,
    /// The odometer reading out, in kilometres.
    pub odometer_out: i64,
    /// When it came back.
    pub in_at: Option<DateTime>,
    /// The odometer reading back, in kilometres.
    pub odometer_in: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl TestDrive {
    /// Whether the vehicle is still out (no mileage in).
    pub fn is_open(&self) -> bool {
        self.odometer_in.is_none()
    }

    /// The kilometres driven, once it is back.
    pub fn distance(&self) -> Option<i64> {
        self.odometer_in.map(|back| back - self.odometer_out)
    }

    /// The test drives of job card `job_card_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_job_card(db: &Db, job_card_id: i64) -> Result<Vec<TestDrive>> {
        TestDrive::query()
            .where_(TestDrive::JOB_CARD_ID.eq(job_card_id))
            .order_by(TestDrive::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drive_is_open_until_the_mileage_is_back() {
        let mut drive = TestDrive {
            odometer_out: 45_210,
            ..TestDrive::default()
        };
        assert!(drive.is_open());
        assert_eq!(drive.distance(), None);
        drive.odometer_in = Some(45_222);
        assert!(!drive.is_open());
        assert_eq!(drive.distance(), Some(12));
    }
}
