//! Management agreements: the contract with a landlord that lets the company
//! manage the landlord's units and report to him (`PRP.PRT.03`, 01 Property
//! §6 *Management agreement*, §7.1 *Management agreement*).
//!
//! This part has what owner statements need (BL-PRP-19 to BL-PRP-21): the
//! agreement with its owner party, its fee (a percentage of the rent
//! collected, in basis points, and a fixed fee per statement), the payout
//! frequency, the flag `may_invoice_negative_balance` (default off) and its
//! status; the units it covers with the days they come in and go out
//! ([`AgreementUnit`]); and the machine with its four states declared. The
//! transitions `activate`, `give_notice`, `end` and `transfer_unit` come with
//! BL-PRP-22 and BL-PRP-23, which is why the machine has none yet: a draft
//! agreement is created by [`create_draft`] and the statements are generated
//! for the agreements that are `active` (`ending` agreements keep their
//! statements until `end`).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine};
use serde::{Deserialize, Serialize};
use wl_kernel::parties::directory;
use wl_kernel::{PartyId, Subject};

use crate::building::refused;
use crate::texts::english;
use crate::unit::Unit;

/// The subject type of a management agreement.
pub const AGREEMENT_SUBJECT_TYPE: &str = "property.management_agreement";

/// Where an agreement stands (§7.1 *Management agreement*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AgreementStatus {
    /// Being prepared.
    #[default]
    Draft,
    /// The company manages the owner's units.
    Active,
    /// Notice given by either side; the handover is being prepared.
    Ending,
    /// Handed over; no more charges or statements, except the final
    /// statement.
    Ended,
}

impl AgreementStatus {
    /// The states in which owner statements are prepared.
    pub const REPORTING: [AgreementStatus; 2] = [AgreementStatus::Active, AgreementStatus::Ending];
}

/// A management agreement with a landlord.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_management_agreements", state_machine, index(owner_party_id))]
pub struct ManagementAgreement {
    /// The primary key.
    pub id: i64,
    /// The owner, a party.
    pub owner_party_id: i64,
    /// The fee in basis points of the rent collected (500 is 5%).
    pub fee_bp: i32,
    /// A fixed fee per statement, in fils.
    pub fee_fixed: i64,
    /// How often the owner is paid (`monthly`).
    pub payout_frequency: String,
    /// The days of notice either side gives.
    pub notice_days: i32,
    /// Whether the property manager may invoice a negative balance to the
    /// owner (`PRP.ACL.21`).
    pub may_invoice_negative_balance: bool,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day, when the contract has one.
    pub end_date: Option<NaiveDate>,
    /// Why it ended.
    pub end_reason: Option<String>,
    /// The day the units are handed over.
    pub handover_date: Option<NaiveDate>,
    /// The management company's legal entity, when it is not the default.
    pub legal_entity_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: AgreementStatus,
    /// The user who drafted it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StateMachine for ManagementAgreement {
    type State = AgreementStatus;
    const COLUMN: renox::db::Col<ManagementAgreement, AgreementStatus> = ManagementAgreement::STATUS;

    fn machine() -> Machine<Self> {
        use AgreementStatus::{Active, Draft, Ended, Ending};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Active)
            .tone("success")
            .state(Ending)
            .tone("warning")
            .state(Ended)
            .tone("neutral")
            .terminal()
    }
}

/// A unit under an agreement, from the day it comes in to the day it goes out.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_agreement_units", index(agreement_id), index(unit_id))]
pub struct AgreementUnit {
    /// The primary key.
    pub id: i64,
    /// The agreement.
    #[model(references = "property_management_agreements")]
    pub agreement_id: i64,
    /// The unit.
    #[model(references = "property_units")]
    pub unit_id: i64,
    /// The day it comes in.
    pub date_in: NaiveDate,
    /// The day it goes out.
    pub date_out: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AgreementUnit {
    /// Whether the unit is under the agreement on some day of `from..=to`.
    pub fn covers(&self, from: NaiveDate, to: NaiveDate) -> bool {
        self.date_in <= to && self.date_out.is_none_or(|out| out >= from)
    }
}

impl ManagementAgreement {
    /// The agreement as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(AGREEMENT_SUBJECT_TYPE, self.id)
    }

    /// The owner.
    pub fn owner(&self) -> PartyId {
        PartyId(self.owner_party_id)
    }

    /// The agreement shown to people: `MA-0007`.
    pub fn number(&self) -> String {
        format!("MA-{:04}", self.id)
    }

    /// The units under the agreement, in the order they were added.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn units(&self, db: &Db) -> Result<Vec<AgreementUnit>> {
        AgreementUnit::query()
            .where_(AgreementUnit::AGREEMENT_ID.eq(self.id))
            .order_by(AgreementUnit::ID)
            .get(db)
            .await
    }

    /// The agreements that report to their owners: `active` and `ending`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn reporting(db: &Db) -> Result<Vec<ManagementAgreement>> {
        ManagementAgreement::query()
            .where_(ManagementAgreement::STATUS.is_in(AgreementStatus::REPORTING))
            .order_by(ManagementAgreement::ID)
            .get(db)
            .await
    }

    /// The reporting agreement that covers `unit` on `day`, if any.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn for_unit(
        db: &Db,
        unit_id: i64,
        day: NaiveDate,
    ) -> Result<Option<ManagementAgreement>> {
        let rows = AgreementUnit::query()
            .where_(AgreementUnit::UNIT_ID.eq(unit_id))
            .order_by_desc(AgreementUnit::ID)
            .get(db)
            .await?;
        for row in rows {
            if !row.covers(day, day) {
                continue;
            }
            if let Some(agreement) = ManagementAgreement::find(db, row.agreement_id).await?
                && AgreementStatus::REPORTING.contains(&agreement.status)
            {
                return Ok(Some(agreement));
            }
        }
        Ok(None)
    }
}

/// What a user enters for a new agreement.
#[derive(Debug, Clone)]
pub struct NewAgreement {
    /// The owner, a party.
    pub owner: PartyId,
    /// The fee in basis points of the rent collected.
    pub fee_bp: i32,
    /// The fixed fee per statement, in fils.
    pub fee_fixed: i64,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day.
    pub end_date: Option<NaiveDate>,
    /// Whether a negative balance may be invoiced to the owner.
    pub may_invoice_negative_balance: bool,
}

impl NewAgreement {
    /// An agreement with `owner` from `start_date` with a fee of `fee_bp`
    /// basis points.
    pub fn new(owner: PartyId, start_date: NaiveDate, fee_bp: i32) -> Self {
        NewAgreement {
            owner,
            fee_bp,
            fee_fixed: 0,
            start_date,
            end_date: None,
            may_invoice_negative_balance: false,
        }
    }

    /// With the flag that allows invoicing a negative balance.
    pub fn may_invoice_negative_balance(mut self) -> Self {
        self.may_invoice_negative_balance = true;
        self
    }
}

/// Drafts a management agreement for the user `by`.
///
/// # Errors
///
/// 422 for an owner that is not a party, a fee below 0 or above 100%, or an
/// end before the start.
pub async fn create_draft(
    state: &AppState,
    by: Option<i64>,
    new: NewAgreement,
) -> Result<ManagementAgreement> {
    if directory(state)
        .find(&state.db, new.owner)
        .await?
        .is_none()
    {
        return Err(refused(english("property.agreements.no_owner", &[])));
    }
    if !(0..=10_000).contains(&new.fee_bp) || new.fee_fixed < 0 {
        return Err(refused(english("property.agreements.fee_invalid", &[])));
    }
    if new.end_date.is_some_and(|end| end < new.start_date) {
        return Err(refused(english("property.agreements.dates_invalid", &[])));
    }
    ManagementAgreement::create(
        &state.db,
        ManagementAgreement {
            owner_party_id: new.owner.0,
            fee_bp: new.fee_bp,
            fee_fixed: new.fee_fixed,
            payout_frequency: "monthly".to_owned(),
            notice_days: 90,
            may_invoice_negative_balance: new.may_invoice_negative_balance,
            start_date: new.start_date,
            end_date: new.end_date,
            status: AgreementStatus::Draft,
            created_by: by,
            ..Default::default()
        },
    )
    .await
}

/// Puts `unit` under `agreement` from `date_in`.
///
/// # Errors
///
/// 422 when the unit does not exist, or is under another agreement that is
/// not ended on that day.
pub async fn add_unit(
    db: &Db,
    agreement: &ManagementAgreement,
    unit_id: i64,
    date_in: NaiveDate,
) -> Result<AgreementUnit> {
    if Unit::find(db, unit_id).await?.is_none() {
        return Err(refused(english("property.agreements.no_unit", &[])));
    }
    let rows = AgreementUnit::query()
        .where_(AgreementUnit::UNIT_ID.eq(unit_id))
        .get(db)
        .await?;
    if rows
        .iter()
        .any(|row| row.covers(date_in, date_in) && row.agreement_id != agreement.id)
    {
        return Err(refused(english("property.agreements.unit_taken", &[])));
    }
    AgreementUnit::create(
        db,
        AgreementUnit {
            agreement_id: agreement.id,
            unit_id,
            date_in,
            ..Default::default()
        },
    )
    .await
}

/// Registers the machine with `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<ManagementAgreement>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    #[test]
    fn an_agreement_starts_a_draft_and_declares_its_four_states() {
        let machine = ManagementAgreement::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let names: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["draft", "active", "ending", "ended"]);
        assert!(machine.states().iter().all(|s| s.tone.is_some()));
        let ended = machine.states().iter().find(|s| s.name == "ended");
        assert!(ended.is_some_and(|s| s.terminal));
    }

    #[test]
    fn a_unit_is_covered_from_the_day_it_comes_in_to_the_day_it_goes_out() {
        let row = AgreementUnit {
            date_in: day(3, 1),
            date_out: Some(day(6, 30)),
            ..Default::default()
        };
        assert!(row.covers(day(3, 1), day(3, 31)));
        assert!(row.covers(day(6, 30), day(7, 31)));
        assert!(!row.covers(day(7, 1), day(7, 31)));
        assert!(!row.covers(day(2, 1), day(2, 28)));
        let open = AgreementUnit {
            date_in: day(3, 1),
            ..Default::default()
        };
        assert!(open.covers(day(12, 1), day(12, 31)));
    }

    #[test]
    fn the_number_is_padded() {
        let agreement = ManagementAgreement {
            id: 7,
            ..Default::default()
        };
        assert_eq!(agreement.number(), "MA-0007");
        assert_eq!(agreement.subject().to_string(), "property.management_agreement:7");
    }
}
