//! A WPS file the bank pays in part, and the payment of the run (wl-hr §4
//! *Payroll run*: `record_partial`, `regenerate`, `confirm_paid`; §5.3 steps 6
//! and 7; issue #203).
//!
//! - `record_partial` (submitted -> partially_rejected, `payroll.prepare`):
//!   needs the bank's result file and a reason for each failed line. The
//!   lines of the file that are not failed become `paid` and their payslips
//!   are published ([`crate::payroll::publish_payslip_in`]); the failed lines
//!   become `failed` (a line of an absconded employee is `held` with *absence
//!   report* instead, §5.6). The result file is the bank's confirmation of
//!   this SIF version, so `confirm_paid` needs a confirmation only for the
//!   lines of later versions. A task per failed line and the notices follow
//!   (`HRM.RUL.11`, [`crate::wps_alerts`]) and `PayrollLinesFailed` is emitted.
//! - `regenerate` (partially_rejected -> submitted, `payroll.prepare`): a
//!   new SIF version with only the lines that are `included` again. A line
//!   still `failed` refuses it: it is fixed ([`crate::payroll::fix_line`],
//!   which the change of the pay account under `HRM.ACL.05` calls), held
//!   ([`hold_line`]) or paid another approved way ([`pay_line_other_way`]).
//!   When every failed line was moved out the SIF would be empty, so
//!   `regenerate` is refused and the run goes straight to `confirm_paid`.
//! - `confirm_paid` (submitted, partially_rejected -> paid, `payroll.approve`):
//!   refused while a line is `failed` and while a line that is in a SIF
//!   version has no bank confirmation (not a held line, a line with a net of
//!   zero or less, or a line paid another way). The lines the bank confirms
//!   become `paid`, every paid line gets its payslip published once, held
//!   lines wait for `HRM.RUL.12`, and `PayrollRunPaid` follows.
//!
//! `record_partial` and `regenerate` also exist as system transitions
//! ([`crate::payroll::RECORD_PARTIAL_BY_SYSTEM`],
//! [`crate::payroll::REGENERATE_BY_SYSTEM`]) with the same effects, for a
//! bank result that arrives without a user.
//!
//! Decisions on what is not built yet: the payment of a line moved out to a
//! non-WPS method is recorded on the line (method and reference); recording it
//! as a payment in wl-payments (`FIN.PAY.01`), the salary cost on cost objects
//! (`FIN.CST.01`) and the recovery adjustments of a supplementary run come
//! with BL-HRM-16 to 18. The payslip document (`DOC.GEN.01`) is made by
//! wl-documents; the line records that it is published.

use renox::db::{Transaction, now};
use renox::prelude::*;
use renox::state_machine::Transition;

use crate::payroll::{
    CONFIRM_PAID, ConfirmInput, LineStatus, PartialInput, PayrollLine, PayrollRun,
    PayrollRunStatus, RECORD_PARTIAL, RECORD_PARTIAL_BY_SYSTEM, REGENERATE, REGENERATE_BY_SYSTEM,
    publish_payslip_in, record_result_in,
};
use crate::permissions;
use crate::sif::SifFile;
use crate::texts::english;

/// The ways a failed line can be paid instead of through WPS.
pub const OTHER_METHODS: [&str; 3] = ["transfer", "cheque", "cash"];

fn refuse(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// `record_partial`, as the user's transition or the system's.
pub(crate) fn record_partial(by_system: bool) -> Transition<PayrollRun> {
    let transition = if by_system {
        Transition::new(RECORD_PARTIAL_BY_SYSTEM).system()
    } else {
        Transition::new(RECORD_PARTIAL)
            .tone("warning")
            .permission(permissions::PAYROLL_PREPARE)
    };
    transition
        .label("Record partial rejection")
        .from([PayrollRunStatus::Submitted])
        .to(PayrollRunStatus::PartiallyRejected)
        .guard("result_file_attached", move |_run: &PayrollRun, ctx| {
            let input = ctx.input_as::<PartialInput>().unwrap_or_default();
            let missing = !by_system && input.result_file.trim().is_empty();
            Box::pin(async move {
                if missing {
                    Err(english("wl_hr.payroll.result_file_required", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .guard("some_line_failed", |_run: &PayrollRun, ctx| {
            let input = ctx.input_as::<PartialInput>().unwrap_or_default();
            let none = input.failed.is_empty();
            Box::pin(async move {
                if none {
                    Err(english("wl_hr.payroll.no_failed_line", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .guard("reasons_given", |_run: &PayrollRun, ctx| {
            let input = ctx.input_as::<PartialInput>().unwrap_or_default();
            let blank = input.failed.iter().any(|f| f.reason.trim().is_empty());
            Box::pin(async move {
                if blank {
                    Err(english("wl_hr.payroll.reason_required", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|run: &PayrollRun, ctx, tx| {
            let id = run.id;
            let version = run.sif_version;
            let input = ctx.input_as::<PartialInput>().unwrap_or_default();
            Box::pin(async move {
                // A failed line must be one the file carried.
                let in_file: Vec<i64> = PayrollLine::query()
                    .where_(PayrollLine::RUN_ID.eq(id))
                    .where_(PayrollLine::STATUS.eq(LineStatus::Included))
                    .get(&mut *tx)
                    .await?
                    .into_iter()
                    .filter(|l| l.sif_version == Some(version))
                    .map(|l| l.id)
                    .collect();
                if input.failed.iter().any(|f| !in_file.contains(&f.line_id)) {
                    return Err(refuse("wl_hr.payroll.line_not_in_file"));
                }
                record_result_in(tx, id, &input).await?;
                let confirmation = input.result_file.trim().to_owned();
                if !confirmation.is_empty() {
                    confirm_version_in(tx, id, version, &confirmation).await?;
                }
                Ok(())
            })
        })
}

/// `regenerate`, as the user's transition or the system's.
pub(crate) fn regenerate(by_system: bool) -> Transition<PayrollRun> {
    let transition = if by_system {
        Transition::new(REGENERATE_BY_SYSTEM).system()
    } else {
        Transition::new(REGENERATE)
            .tone("primary")
            .permission(permissions::PAYROLL_PREPARE)
    };
    transition
        .label("Regenerate")
        .from([PayrollRunStatus::PartiallyRejected])
        .to(PayrollRunStatus::Submitted)
        .effect(|run: &PayrollRun, _ctx, tx| {
            let id = run.id;
            Box::pin(async move {
                let lines = PayrollLine::query()
                    .where_(PayrollLine::RUN_ID.eq(id))
                    .get(&mut *tx)
                    .await?;
                if lines.iter().any(|l| l.status == LineStatus::Failed) {
                    return Err(refuse("wl_hr.payroll.failed_lines_left"));
                }
                if !lines
                    .iter()
                    .any(|l| l.status == LineStatus::Included && l.net_amount > 0)
                {
                    return Err(refuse("wl_hr.payroll.nothing_to_regenerate"));
                }
                crate::sif::generate_in(tx, id, None).await
            })
        })
}

/// `confirm_paid`.
pub(crate) fn confirm_paid() -> Transition<PayrollRun> {
    Transition::new(CONFIRM_PAID)
        .label("Confirm paid")
        .tone("success")
        .permission(permissions::PAYROLL_APPROVE)
        .from([
            PayrollRunStatus::Submitted,
            PayrollRunStatus::PartiallyRejected,
        ])
        .to(PayrollRunStatus::Paid)
        .effect(|run: &PayrollRun, ctx, tx| {
            let id = run.id;
            let version = run.sif_version;
            let input = ctx.input_as::<ConfirmInput>().unwrap_or_default();
            Box::pin(async move {
                let lines = PayrollLine::query()
                    .where_(PayrollLine::RUN_ID.eq(id))
                    .order_by(PayrollLine::ID)
                    .get(&mut *tx)
                    .await?;
                if lines.iter().any(|l| l.status == LineStatus::Failed) {
                    return Err(refuse("wl_hr.payroll.failed_lines_confirm"));
                }
                let confirmation = input.confirmation.trim().to_owned();
                if !confirmation.is_empty() && version > 0 {
                    confirm_version_in(tx, id, version, &confirmation).await?;
                }
                // Every line the bank was asked to pay needs the confirmation
                // of its SIF version.
                let confirmed: Vec<i64> = SifFile::query()
                    .where_(SifFile::RUN_ID.eq(id))
                    .get(&mut *tx)
                    .await?
                    .into_iter()
                    .filter(|f| f.confirmation.is_some())
                    .map(|f| f.version)
                    .collect();
                let mut unconfirmed = 0_usize;
                for line in &lines {
                    let by_bank = line.paid_method.is_none()
                        && line.net_amount > 0
                        && matches!(line.status, LineStatus::Included | LineStatus::Paid);
                    let Some(file) = line.sif_version.filter(|_| by_bank) else {
                        continue;
                    };
                    if !confirmed.contains(&file) {
                        unconfirmed += 1;
                    }
                }
                if unconfirmed > 0 {
                    return Err(Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        english(
                            "wl_hr.payroll.confirmation_missing",
                            &[("count", &unconfirmed)],
                        ),
                    ));
                }
                for line in lines {
                    if line.status == LineStatus::Included && line.sif_version.is_some() {
                        PayrollLine::query()
                            .where_(PayrollLine::ID.eq(line.id))
                            .update(
                                &mut *tx,
                                &[
                                    ("status", &LineStatus::Paid.as_str().to_owned()),
                                    ("updated_at", &now()),
                                ],
                            )
                            .await?;
                    }
                    if line.status == LineStatus::Paid
                        || (line.status == LineStatus::Included && line.sif_version.is_some())
                    {
                        publish_payslip_in(tx, line.id).await?;
                    }
                }
                Ok(())
            })
        })
}

/// Sets the confirmation of version `version` of run `run_id` when it has none.
async fn confirm_version_in(
    tx: &mut Transaction,
    run_id: i64,
    version: i64,
    confirmation: &str,
) -> Result {
    SifFile::query()
        .where_(SifFile::RUN_ID.eq(run_id))
        .where_(SifFile::VERSION.eq(version))
        .where_(SifFile::CONFIRMATION.is_null())
        .update(
            &mut *tx,
            &[
                ("confirmation", &Some(confirmation.to_owned())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// The failed line `line_id` is moved out of the run: it is `held` with
/// `reason` and paid later off-cycle (`HRM.RUL.12`).
pub async fn hold_line(db: &Db, line_id: i64, reason: &str) -> Result {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refuse("wl_hr.payroll.hold_reason_required"));
    }
    failed_line(db, line_id).await?;
    PayrollLine::query()
        .where_(PayrollLine::ID.eq(line_id))
        .update(
            db,
            &[
                ("status", &LineStatus::Held.as_str().to_owned()),
                ("held_reason", &Some(reason.to_owned())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// The failed line `line_id` is paid another approved way, with its
/// `reference` (`TR-88`): it is `paid`, needs no bank confirmation, and its
/// payslip is published at `confirm_paid`.
pub async fn pay_line_other_way(db: &Db, line_id: i64, method: &str, reference: &str) -> Result {
    let method = method.trim();
    let reference = reference.trim();
    if !OTHER_METHODS.contains(&method) || reference.is_empty() {
        return Err(refuse("wl_hr.payroll.other_payment_invalid"));
    }
    failed_line(db, line_id).await?;
    PayrollLine::query()
        .where_(PayrollLine::ID.eq(line_id))
        .update(
            db,
            &[
                ("status", &LineStatus::Paid.as_str().to_owned()),
                ("paid_method", &Some(method.to_owned())),
                ("paid_reference", &Some(reference.to_owned())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Line `line_id` is `failed`, else 422.
async fn failed_line(db: &Db, line_id: i64) -> Result {
    let line = PayrollLine::find(db, line_id)
        .await?
        .ok_or(Error::NotFound)?;
    if line.status != LineStatus::Failed {
        return Err(refuse("wl_hr.payroll.line_not_failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::StateMachine;

    #[test]
    fn the_partial_result_and_the_regeneration_are_user_transitions_with_system_twins() {
        let machine = PayrollRun::machine();
        let table = [
            (
                RECORD_PARTIAL,
                RECORD_PARTIAL_BY_SYSTEM,
                "submitted",
                "partially_rejected",
            ),
            (
                REGENERATE,
                REGENERATE_BY_SYSTEM,
                "partially_rejected",
                "submitted",
            ),
        ];
        for (user, system, from, to) in table {
            let user = machine.find(user).unwrap();
            assert!(!user.is_system());
            assert_eq!(user.from_states(), [from]);
            assert_eq!(user.to_state(), Some(to));
            let system = machine.find(system).unwrap();
            assert!(system.is_system());
            assert_eq!(system.from_states(), [from]);
            assert_eq!(system.to_state(), Some(to));
        }
    }

    #[test]
    fn confirm_paid_leaves_submitted_and_partially_rejected() {
        let machine = PayrollRun::machine();
        let confirm = machine.find(CONFIRM_PAID).unwrap();
        assert!(!confirm.is_system());
        assert_eq!(confirm.from_states(), ["submitted", "partially_rejected"]);
        assert_eq!(confirm.to_state(), Some("paid"));
    }
}
