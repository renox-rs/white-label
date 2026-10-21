//! The visa case as far as an exit needs it (wl-hr §4 *Visa case*, §8
//! `VisaStepTemplate`, issue #197): the *cancellation* case that opens when
//! an employee is reported as absconded, from the *absconding* variant of the
//! cancellation step template. It starts from the MoHRE absence report and
//! needs no worker signature. The steps, fees and the other transitions
//! (`start`, `hold`, `complete`, `reject`) come with the visa issues; here
//! are the case, its variant and `cancel`.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};

use crate::permissions;

/// A cancellation case.
pub const TYPE_CANCELLATION: &str = "cancellation";

/// The standard variant of the cancellation step template.
pub const VARIANT_STANDARD: &str = "standard";
/// The *absconding* variant: it starts from the MoHRE absence report.
pub const VARIANT_ABSCONDING: &str = "absconding";

/// Abandoned (the exit was reversed).
pub const CANCEL: &str = "cancel";

/// Where a visa case stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VisaCaseStatus {
    /// Created; no step done.
    #[default]
    Open,
    /// At least one step done.
    InProgress,
    /// Waiting.
    OnHold,
    /// All mandatory steps done.
    Completed,
    /// Refused by the authority.
    Rejected,
    /// Abandoned.
    Cancelled,
}

/// One visa process.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_visa_cases",
    state_machine,
    index(employee_id, case_type, status)
)]
pub struct VisaCase {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// `new`, `renewal` or `cancellation`.
    pub case_type: String,
    /// The step template variant of a cancellation.
    pub variant: String,
    /// Where it stands; changed only by the machine.
    pub status: VisaCaseStatus,
    /// The MoHRE absence report an absconding case starts from.
    pub report_number: Option<String>,
    /// The date of that report.
    pub report_date: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StateMachine for VisaCase {
    type State = VisaCaseStatus;
    const COLUMN: renox::db::Col<VisaCase, VisaCaseStatus> = VisaCase::STATUS;

    fn machine() -> Machine<Self> {
        use VisaCaseStatus::{Cancelled, Completed, InProgress, OnHold, Open, Rejected};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("neutral")
            .state(InProgress)
            .tone("info")
            .state(OnHold)
            .tone("warning")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("error")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .permission(permissions::VISAS_MANAGE)
                    .from([Open, InProgress, OnHold])
                    .to(Cancelled)
                    .reason_required(),
            )
    }
}

impl VisaCase {
    /// The cancellation cases of `employee`, oldest first.
    pub async fn cancellations_of(db: &Db, employee: i64) -> Result<Vec<VisaCase>> {
        VisaCase::query()
            .where_(VisaCase::EMPLOYEE_ID.eq(employee))
            .where_(VisaCase::CASE_TYPE.eq(TYPE_CANCELLATION))
            .order_by(VisaCase::ID)
            .get(db)
            .await
    }
}

/// Opens the *cancellation* case of `employee_id` from the *absconding*
/// variant of the step template, started by the absence report `report`
/// (number and date), inside `tx`. A cancellation case that is already open
/// (the one `give_notice` opened) is not duplicated: it becomes the
/// absconding variant and takes the report.
pub async fn open_absconding_in(
    tx: &mut Transaction,
    employee_id: i64,
    report_number: &str,
    report_date: NaiveDate,
) -> Result<VisaCase> {
    let open = VisaCase::query()
        .where_(VisaCase::EMPLOYEE_ID.eq(employee_id))
        .where_(VisaCase::CASE_TYPE.eq(TYPE_CANCELLATION))
        .where_(VisaCase::STATUS.is_in([
            VisaCaseStatus::Open,
            VisaCaseStatus::InProgress,
            VisaCaseStatus::OnHold,
        ]))
        .order_by_desc(VisaCase::ID)
        .first(&mut *tx)
        .await?;
    if let Some(mut case) = open {
        VisaCase::query()
            .where_(VisaCase::ID.eq(case.id))
            .update(
                &mut *tx,
                &[
                    ("variant", &VARIANT_ABSCONDING.to_owned()),
                    ("report_number", &Some(report_number.to_owned())),
                    ("report_date", &Some(report_date)),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        case.variant = VARIANT_ABSCONDING.to_owned();
        case.report_number = Some(report_number.to_owned());
        case.report_date = Some(report_date);
        return Ok(case);
    }
    let mut case = VisaCase {
        employee_id,
        case_type: TYPE_CANCELLATION.to_owned(),
        variant: VARIANT_ABSCONDING.to_owned(),
        status: VisaCaseStatus::Open,
        report_number: Some(report_number.to_owned()),
        report_date: Some(report_date),
        ..VisaCase::default()
    };
    case.insert(&mut *tx).await?;
    Ok(case)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_case_starts_open_and_cancel_leaves_the_live_states() {
        let machine = VisaCase::machine();
        assert_eq!(machine.initial_states(), ["open"]);
        let cancel = machine.find(CANCEL).unwrap();
        assert_eq!(cancel.from_states(), ["open", "in_progress", "on_hold"]);
        assert_eq!(cancel.to_state(), Some("cancelled"));
    }
}
