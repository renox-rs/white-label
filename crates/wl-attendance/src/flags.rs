//! Attendance flags (wl-attendance §3 *Attendance flag*): a reason a record
//! needs review. The three that hold a record for the reviewer
//! (`HRM.ACL.09`) are `overtime_cap`, `permit_invalid` and `overlap`.
//!
//! A flag is added with [`add_in`]: one row per record and flag type, and the
//! `flag` transition by the system when the record is `recorded` or
//! `accepted`, so the history and the event are kept. A record that is
//! already `flagged` only gets the new row; a `locked` or `rejected` record
//! cannot change and gets nothing. The events and the notifications of
//! what was added wait in a [`Pending`] until the transaction has committed.

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::Outcome;
use serde::{Deserialize, Serialize};
use wl_kernel::attendance::AttendanceFlagged;

use crate::notices;
use crate::record::{AttendanceRecord, AttendanceStatus, FLAG};

/// The type of a flag (§3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlagType {
    /// Checked in outside the geofence.
    #[default]
    OutsideGeofence,
    /// Worked across the midday break (`HRM.RUL.01`, `HRM.RUL.13`).
    MiddayBreak,
    /// No check-out at the end of the day.
    MissingCheckout,
    /// Late.
    Late,
    /// Entered after the day, within the late-entry window.
    LateEntry,
    /// Corrected by hand.
    ManualCorrection,
    /// A duplicate.
    Duplicate,
    /// Hours above the overtime cap (`HRM.RUL.13`).
    OvertimeCap,
    /// No valid permit, visa or qualification (`HRM.RUL.14`).
    PermitInvalid,
    /// Overlaps another record of the employee.
    Overlap,
}

impl FlagType {
    /// Whether the flag holds the record for the reviewer (`HRM.ACL.09`).
    pub fn holds_for_review(self) -> bool {
        matches!(
            self,
            FlagType::OvertimeCap | FlagType::PermitInvalid | FlagType::Overlap
        )
    }
}

/// A flag on a record.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_attendance_flags", unique(record_id, flag_type))]
pub struct AttendanceFlag {
    /// The primary key.
    pub id: i64,
    /// The record.
    #[model(references = "wl_attendance_records")]
    pub record_id: i64,
    /// The type.
    pub flag_type: FlagType,
    /// What the reviewer should know, e.g. the locked record it overlaps.
    pub note: String,
    /// When a reviewer settled the flag (accepted, corrected or rejected the
    /// record); open while empty. A settled flag stays as the record's
    /// history, and `permit_invalid` still marks its hours non-billable.
    pub resolved_at: Option<DateTime>,
    /// The user who settled it; none when the system noted it.
    pub resolved_by: Option<i64>,
    /// The reviewer's reason.
    pub resolution: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AttendanceFlag {
    /// The flags of a record, oldest first.
    pub async fn of_record(db: &Db, record_id: i64) -> Result<Vec<AttendanceFlag>> {
        AttendanceFlag::query()
            .where_(AttendanceFlag::RECORD_ID.eq(record_id))
            .order_by(AttendanceFlag::ID)
            .get(db)
            .await
    }

    /// The flags of a record that no reviewer has settled yet, oldest first.
    pub async fn open_of_record(db: &Db, record_id: i64) -> Result<Vec<AttendanceFlag>> {
        Ok(Self::of_record(db, record_id)
            .await?
            .into_iter()
            .filter(|f| f.resolved_at.is_none())
            .collect())
    }

    /// Whether the record carries a flag of `kind`.
    pub async fn has(db: &Db, record_id: i64, kind: FlagType) -> Result<bool> {
        Ok(Self::of_record(db, record_id)
            .await?
            .iter()
            .any(|f| f.flag_type == kind))
    }
}

/// A flag that was added.
#[derive(Debug, Clone)]
pub struct Added {
    /// The record as it was when the flag was added.
    pub record: AttendanceRecord,
    /// The flag.
    pub flag: FlagType,
    /// The note.
    pub note: String,
}

/// What to do after the transaction that added flags has committed: emit the
/// transitions, the `AttendanceFlagged` events and tell the people.
#[derive(Default)]
#[must_use]
pub struct Pending {
    outcomes: Vec<Outcome<AttendanceRecord>>,
    /// The flags added, in order.
    pub added: Vec<Added>,
}

impl Pending {
    /// Whether nothing was added.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
    }

    /// Whether a flag of `kind` was added to record `id`.
    pub fn added_to(&self, id: i64, kind: FlagType) -> bool {
        self.added
            .iter()
            .any(|a| a.record.id == id && a.flag == kind)
    }

    /// Emits the transitions and the events and sends the notifications.
    /// Call it after the commit.
    pub async fn finish(self, state: &AppState) -> Result {
        for outcome in self.outcomes {
            outcome.emit(state).await?;
        }
        for added in &self.added {
            state
                .emit(AttendanceFlagged {
                    record: added.record.record_id(),
                    employee: added.record.employee(),
                    date: added.record.date,
                    flag: added.flag.as_str().to_owned(),
                })
                .await?;
            notices::flag_added(state, added).await?;
        }
        Ok(())
    }
}

/// Adds a flag of `kind` to record `record_id` inside `tx`. Returns whether
/// it was added: not when the record has the flag open already (a flag type
/// is held at most once), when a reviewer settled the same condition, or when
/// the record is `locked` or `rejected`. A settled flag whose condition
/// changed (another note) is opened again.
pub async fn add_in(
    tx: &mut Transaction,
    record_id: i64,
    kind: FlagType,
    note: &str,
    pending: &mut Pending,
) -> Result<bool> {
    let Some(record) = AttendanceRecord::find(&mut *tx, record_id).await? else {
        return Ok(false);
    };
    if !matches!(
        record.status,
        AttendanceStatus::Recorded | AttendanceStatus::Flagged | AttendanceStatus::Accepted
    ) {
        return Ok(false);
    }
    let held = AttendanceFlag::query()
        .where_(AttendanceFlag::RECORD_ID.eq(record_id))
        .where_(AttendanceFlag::FLAG_TYPE.eq(kind))
        .first(&mut *tx)
        .await?;
    match held {
        // Open already: a flag type is held at most once.
        Some(held) if held.resolved_at.is_none() => return Ok(false),
        // Settled by a reviewer for the same condition: it stays settled.
        // A different note means the hours changed, so it is raised again.
        Some(held) if held.note == note => return Ok(false),
        Some(held) => {
            let open: Option<DateTime> = None;
            let nobody: Option<i64> = None;
            let none: Option<String> = None;
            AttendanceFlag::query()
                .where_(AttendanceFlag::ID.eq(held.id))
                .update(
                    &mut *tx,
                    &[
                        ("note", &note.to_owned()),
                        ("resolved_at", &open),
                        ("resolved_by", &nobody),
                        ("resolution", &none),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
        }
        None => {
            AttendanceFlag::create(
                &mut *tx,
                AttendanceFlag {
                    record_id,
                    flag_type: kind,
                    note: note.to_owned(),
                    ..AttendanceFlag::default()
                },
            )
            .await?;
        }
    }
    if record.status.can_be_flagged() {
        let outcome = record
            .transition(FLAG)
            .by_system()
            .reason(kind.as_str())
            .meta(json!({ "flag": kind.as_str(), "note": note }))
            .run_in(&mut *tx)
            .await
            .map_err(|e| e.into_error())?;
        pending.outcomes.push(outcome);
    }
    pending.added.push(Added {
        record,
        flag: kind,
        note: note.to_owned(),
    });
    Ok(true)
}

/// Settles the open flags of record `record_id` inside `tx`: a reviewer
/// accepted, corrected or rejected the record with `reason`. Returns the
/// flags settled.
pub async fn settle_in(
    tx: &mut Transaction,
    record_id: i64,
    by: Option<i64>,
    reason: &str,
) -> Result<Vec<AttendanceFlag>> {
    let open: Vec<AttendanceFlag> = AttendanceFlag::query()
        .where_(AttendanceFlag::RECORD_ID.eq(record_id))
        .where_(AttendanceFlag::RESOLVED_AT.is_null())
        .order_by(AttendanceFlag::ID)
        .get(&mut *tx)
        .await?;
    if open.is_empty() {
        return Ok(open);
    }
    AttendanceFlag::query()
        .where_(AttendanceFlag::RECORD_ID.eq(record_id))
        .where_(AttendanceFlag::RESOLVED_AT.is_null())
        .update(
            &mut *tx,
            &[
                ("resolved_at", &Some(renox::db::now())),
                ("resolved_by", &by),
                ("resolution", &Some(reason.to_owned())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(open)
}

/// Notes a flag that holds nothing (`manual_correction` of a correction) on
/// record `record_id`: kept as history and settled at once. A record has it
/// once; a second correction updates its note.
pub async fn note_in(
    tx: &mut Transaction,
    record_id: i64,
    kind: FlagType,
    note: &str,
    by: Option<i64>,
) -> Result {
    let held = AttendanceFlag::query()
        .where_(AttendanceFlag::RECORD_ID.eq(record_id))
        .where_(AttendanceFlag::FLAG_TYPE.eq(kind))
        .first(&mut *tx)
        .await?;
    match held {
        Some(held) => {
            AttendanceFlag::query()
                .where_(AttendanceFlag::ID.eq(held.id))
                .update(
                    &mut *tx,
                    &[("note", &note.to_owned()), ("updated_at", &renox::db::now())],
                )
                .await?;
        }
        None => {
            AttendanceFlag::create(
                &mut *tx,
                AttendanceFlag {
                    record_id,
                    flag_type: kind,
                    note: note.to_owned(),
                    resolved_at: Some(renox::db::now()),
                    resolved_by: by,
                    resolution: Some("noted".to_owned()),
                    ..AttendanceFlag::default()
                },
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_flags_hold_a_record_for_the_reviewer() {
        let held: Vec<FlagType> = [
            FlagType::OutsideGeofence,
            FlagType::MiddayBreak,
            FlagType::MissingCheckout,
            FlagType::Late,
            FlagType::LateEntry,
            FlagType::ManualCorrection,
            FlagType::Duplicate,
            FlagType::OvertimeCap,
            FlagType::PermitInvalid,
            FlagType::Overlap,
        ]
        .into_iter()
        .filter(|f| f.holds_for_review())
        .collect();
        assert_eq!(
            held,
            [FlagType::OvertimeCap, FlagType::PermitInvalid, FlagType::Overlap]
        );
    }

    #[test]
    fn flag_types_have_the_names_of_the_spec() {
        assert_eq!(FlagType::OvertimeCap.as_str(), "overtime_cap");
        assert_eq!(FlagType::PermitInvalid.as_str(), "permit_invalid");
        assert_eq!(FlagType::MiddayBreak.as_str(), "midday_break");
    }
}
