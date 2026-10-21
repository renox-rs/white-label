//! The advertising permit of a campaign (05 §9 *Advertising*, `CLN.ACL.13`;
//! BL-CLN-15).
//!
//! A campaign that promotes services may be activated and may send only
//! with the regulator's advertising permit number and validity recorded and
//! approved by the medical director. The permit is stored here, by the id of
//! the wl-sales campaign: the app never reads wl-sales' tables, and the host
//! wires [`problem`] and [`last_day`] into wl-sales' `CampaignGuard`, which
//! refuses the activation, stops a send, and ends the campaign the day after
//! the permit's last valid day.
//!
//! Decisions:
//!
//! - A campaign with **no** permit row is taken to promote services, so the
//!   guard never lets one through by default. The medical director (the
//!   holder of `advertising_permits.manage`) may record that a campaign
//!   promotes no service; the marketing coordinator cannot.
//! - The validity is the last valid day, inclusive, like a document's expiry.
//! - A permit already lapsed on the day it is recorded is refused.
//! - Recording again changes the same row (the renewal), and the approver and
//!   the time are those of the last record; the audit log keeps the history.
//! - Recording that a campaign promotes **no** service after a permit was
//!   recorded keeps the permit's number and last valid day: the guard stops
//!   asking for it, but the campaign that started under it still ends the day
//!   after (a flip must not make a campaign immortal).
//! - A permit is recorded for a campaign that exists, in one transaction; two
//!   records at once end as a renewal, never as a unique-key error.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::permissions;
use crate::texts::english;

/// The audit action of a recorded permit.
pub const RECORDED: &str = "clinics.permit_recorded";

/// The advertising permit of one campaign.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_advertising_permits", unique(campaign_id))]
pub struct AdvertisingPermit {
    /// The primary key.
    pub id: i64,
    /// The wl-sales campaign.
    pub campaign_id: i64,
    /// Whether the campaign promotes services; when not, it needs no permit.
    pub promotes_services: bool,
    /// The regulator's permit number.
    pub permit_number: String,
    /// The last day the permit is valid.
    pub valid_to: Option<NaiveDate>,
    /// The user who approved it (the medical director).
    pub approved_by: Option<i64>,
    /// When it was approved.
    pub approved_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Why a campaign has no valid permit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// Nothing is recorded for the campaign.
    Missing,
    /// The permit number is blank.
    NoNumber,
    /// The validity is not recorded.
    NoValidity,
    /// The permit's last valid day is before the day asked about.
    Lapsed,
}

impl Problem {
    /// The code the guard answers for this problem (`advertising_permit_`
    /// then the problem): kept as the reason of a broadcast that is not
    /// sent.
    pub fn code(self) -> &'static str {
        match self {
            Problem::Missing => "advertising_permit_missing",
            Problem::NoNumber => "advertising_permit_no_number",
            Problem::NoValidity => "advertising_permit_no_validity",
            Problem::Lapsed => "advertising_permit_lapsed",
        }
    }

    /// The problem a [`code`](Problem::code) stands for.
    pub fn from_code(code: &str) -> Option<Problem> {
        [
            Problem::Missing,
            Problem::NoNumber,
            Problem::NoValidity,
            Problem::Lapsed,
        ]
        .into_iter()
        .find(|p| p.code() == code)
    }

    /// The words for staff.
    pub fn message(self) -> String {
        let key = match self {
            Problem::Missing => "clinics.permit.problem.missing",
            Problem::NoNumber => "clinics.permit.problem.no_number",
            Problem::NoValidity => "clinics.permit.problem.no_validity",
            Problem::Lapsed => "clinics.permit.problem.lapsed",
        };
        english(key, &[])
    }
}

impl AdvertisingPermit {
    /// The permit of campaign `campaign`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_campaign(db: &Db, campaign: i64) -> Result<Option<AdvertisingPermit>> {
        AdvertisingPermit::query()
            .where_(AdvertisingPermit::CAMPAIGN_ID.eq(campaign))
            .first(db)
            .await
    }

    /// Every permit, by campaign.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn listed(db: &Db) -> Result<Vec<AdvertisingPermit>> {
        AdvertisingPermit::query()
            .order_by(AdvertisingPermit::CAMPAIGN_ID)
            .get(db)
            .await
    }

    /// What is wrong with the permit on `day`, or `None` when the campaign
    /// may run: it promotes no service, or the number and the validity are
    /// recorded and `day` is not after the last valid day.
    pub fn problem_on(&self, day: NaiveDate) -> Option<Problem> {
        if !self.promotes_services {
            return None;
        }
        if self.permit_number.trim().is_empty() {
            return Some(Problem::NoNumber);
        }
        match self.valid_to {
            None => Some(Problem::NoValidity),
            Some(last) if last < day => Some(Problem::Lapsed),
            Some(_) => None,
        }
    }
}

/// Why campaign `campaign` may not be activated or send on `day`, or `None`
/// when it may.
///
/// # Errors
///
/// The database's errors.
pub async fn problem(db: &Db, campaign: i64, day: NaiveDate) -> Result<Option<Problem>> {
    Ok(match AdvertisingPermit::of_campaign(db, campaign).await? {
        None => Some(Problem::Missing),
        Some(permit) => permit.problem_on(day),
    })
}

/// The last day campaign `campaign` may run: the last valid day of the permit
/// recorded for it. Recording later that the campaign promotes no service
/// keeps that day (see [`record`]), so a campaign that started under a permit
/// still ends with it.
///
/// # Errors
///
/// The database's errors.
pub async fn last_day(db: &Db, campaign: i64) -> Result<Option<NaiveDate>> {
    Ok(AdvertisingPermit::of_campaign(db, campaign)
        .await?
        .and_then(|permit| permit.valid_to))
}

/// What the medical director records for a campaign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPermit {
    /// The wl-sales campaign.
    pub campaign_id: i64,
    /// Whether the campaign promotes services.
    pub promotes_services: bool,
    /// The regulator's permit number.
    pub permit_number: String,
    /// The last valid day.
    pub valid_to: Option<NaiveDate>,
}

impl NewPermit {
    /// A permit for a campaign that promotes services.
    pub fn new(campaign_id: i64, permit_number: &str, valid_to: NaiveDate) -> Self {
        NewPermit {
            campaign_id,
            promotes_services: true,
            permit_number: permit_number.trim().to_owned(),
            valid_to: Some(valid_to),
        }
    }

    /// Records that the campaign promotes no service, so it needs no permit.
    pub fn no_services(campaign_id: i64) -> Self {
        NewPermit {
            campaign_id,
            promotes_services: false,
            permit_number: String::new(),
            valid_to: None,
        }
    }
}

fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// Records the permit of a campaign, as the medical director who approves it
/// (`advertising_permits.manage`). A second record of the same campaign is a
/// renewal and changes the same row.
///
/// # Errors
///
/// `Forbidden` without the permission; 422 for a campaign that does not
/// exist as a number, a permit with no number or no validity, or one that has
/// lapsed; the database's errors.
pub async fn record(state: &AppState, by: &AuthUser, new: NewPermit) -> Result<AdvertisingPermit> {
    if !by.has_permission(permissions::PERMITS_MANAGE) {
        return Err(Error::Forbidden);
    }
    if new.campaign_id < 1
        || !wl_kernel::sales::campaign_attribution(state)
            .campaign_exists(state, wl_kernel::sales::CampaignId(new.campaign_id))
            .await?
    {
        return Err(refused("clinics.permit.problem.campaign"));
    }
    if new.promotes_services {
        if new.permit_number.trim().is_empty() {
            return Err(refused("clinics.permit.problem.no_number"));
        }
        match new.valid_to {
            None => return Err(refused("clinics.permit.problem.no_validity")),
            Some(last) if last < today() => return Err(refused("clinics.permit.problem.lapsed")),
            Some(_) => {}
        }
    }
    // The second of two parallel records finds the first one's row.
    let mut attempt = 0;
    let permit = loop {
        attempt += 1;
        match store(&state.db, by.id, &new).await {
            Err(error) if attempt < 3 && error.is_unique_violation() => {}
            other => break other?,
        }
    };
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new(RECORDED)
            .user(by.id)
            .subject(wl_kernel::sales::CAMPAIGN_SUBJECT_TYPE, permit.campaign_id)
            .data(renox::serde_json::json!({
                "promotes_services": permit.promotes_services,
                "permit_number": permit.permit_number,
                "valid_to": permit.valid_to.map(|d| d.to_string()),
            })),
    )
    .await?;
    Ok(permit)
}

/// Writes the permit in one transaction that holds the write lock: the row
/// of the campaign is changed when there is one, made when there is not.
async fn store(db: &Db, by: i64, new: &NewPermit) -> Result<AdvertisingPermit> {
    let mut tx = db.begin_immediate().await?;
    let existing = AdvertisingPermit::query()
        .where_(AdvertisingPermit::CAMPAIGN_ID.eq(new.campaign_id))
        .first(&mut tx)
        .await?;
    let mut permit = existing.unwrap_or_else(|| AdvertisingPermit {
        campaign_id: new.campaign_id,
        ..AdvertisingPermit::default()
    });
    permit.promotes_services = new.promotes_services;
    if new.promotes_services || permit.permit_number.is_empty() {
        permit.permit_number = new.permit_number.trim().to_owned();
        permit.valid_to = new.valid_to;
    }
    permit.approved_by = Some(by);
    permit.approved_at = Some(now());
    permit.save(&mut tx).await?;
    tx.commit().await?;
    Ok(permit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permit(number: &str, valid_to: Option<&str>) -> AdvertisingPermit {
        AdvertisingPermit {
            campaign_id: 1,
            promotes_services: true,
            permit_number: number.to_owned(),
            valid_to: valid_to.map(|d| d.parse().expect("a date")),
            ..AdvertisingPermit::default()
        }
    }

    #[test]
    fn a_permit_is_valid_through_its_last_day() {
        let permit = permit("AD-123", Some("2026-12-31"));
        let day = |text: &str| text.parse::<NaiveDate>().expect("a date");
        assert_eq!(permit.problem_on(day("2026-12-31")), None);
        assert_eq!(permit.problem_on(day("2027-01-01")), Some(Problem::Lapsed));
    }

    #[test]
    fn a_permit_needs_its_number_and_its_validity() {
        let day = "2026-11-01".parse::<NaiveDate>().expect("a date");
        assert_eq!(
            permit(" ", Some("2026-12-31")).problem_on(day),
            Some(Problem::NoNumber)
        );
        assert_eq!(
            permit("AD-1", None).problem_on(day),
            Some(Problem::NoValidity)
        );
    }

    #[test]
    fn every_problem_has_a_code_that_names_it_again() {
        for problem in [
            Problem::Missing,
            Problem::NoNumber,
            Problem::NoValidity,
            Problem::Lapsed,
        ] {
            assert_eq!(Problem::from_code(problem.code()), Some(problem));
            assert!(!problem.message().is_empty());
        }
        assert_eq!(Problem::from_code("something_else"), None);
    }

    #[test]
    fn a_campaign_that_promotes_no_service_needs_no_permit() {
        let mut none = permit("", None);
        none.promotes_services = false;
        let day = "2026-11-01".parse::<NaiveDate>().expect("a date");
        assert_eq!(none.problem_on(day), None);
    }
}
