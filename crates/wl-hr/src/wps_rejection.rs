//! The bank or exchange house rejects the whole SIF file (wl-hr §4 *Payroll
//! run*: `record_rejection`, `resubmit`; §5.3 step 6; issue #202).
//!
//! - `record_rejection` (submitted -> rejected, `payroll.prepare`): needs the
//!   bank's rejection notice and a reason ([`REASONS`]). Every line of the
//!   rejected file is marked `failed` with the reason; a held line, which was
//!   not in the file, stays held. The preparer and the approver are told, the
//!   tasks of `HRM.RUL.11` follow, and `PayrollLinesFailed` is emitted
//!   ([`crate::payroll_events`]). `HRM.RUL.03`, the WPS deadline alerts, keeps
//!   running: it looks at runs that are not `paid`, and a rejected run is not.
//! - `resubmit` (rejected -> submitted, `payroll.prepare`): the preparer
//!   enters a note on the fix (every reason). For the reason *establishment
//!   ID* the legal entity's MoHRE establishment ID must differ now from the
//!   one the rejected version was named with. The lines are exactly those of
//!   the rejected version and every net is what it was: nothing is added,
//!   removed or changed (an amount that must change is corrected after
//!   payment by a supplementary run, §5.6). The lines go back from `failed` to
//!   `included` and a new SIF version is made; the earlier version stays.
//!
//! Decisions: the notice is a reference to the attached file (a document id or
//! the bank's reference), as for the exits; only the establishment ID reason
//! is checked by the system, the others (file format, funding, other) are the
//! preparer's word.

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::Transition;
use serde::{Deserialize, Serialize};

use crate::payroll::{
    LineStatus, PayrollLine, PayrollRun, PayrollRunStatus, RECORD_REJECTION, RESUBMIT,
};
use crate::sif::{self, SifFile};
use crate::texts::english;
use crate::{establishment, permissions};

/// The file's format was wrong.
pub const REASON_FILE_FORMAT: &str = "file_format";
/// The establishment ID in the file's name was wrong.
pub const REASON_ESTABLISHMENT_ID: &str = "establishment_id";
/// The account that funds the payroll could not cover it.
pub const REASON_FUNDING: &str = "funding";
/// Another reason.
pub const REASON_OTHER: &str = "other";

/// The reasons a rejection can have.
pub const REASONS: [&str; 4] = [
    REASON_FILE_FORMAT,
    REASON_ESTABLISHMENT_ID,
    REASON_FUNDING,
    REASON_OTHER,
];

/// The input of `record_rejection`.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct RejectionInput {
    /// One of [`REASONS`].
    pub reason: String,
    /// The bank's rejection notice: a reference to the attached file.
    pub notice: String,
}

/// The input of `resubmit`.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct ResubmitInput {
    /// What was done to fix the cause.
    pub note: String,
}

/// `record_rejection`.
pub(crate) fn record_rejection() -> Transition<PayrollRun> {
    Transition::new(RECORD_REJECTION)
        .label("Record rejection")
        .tone("danger")
        .permission(permissions::PAYROLL_PREPARE)
        .from([PayrollRunStatus::Submitted])
        .to(PayrollRunStatus::Rejected)
        .guard("notice_attached", |_run: &PayrollRun, ctx| {
            let input = ctx.input_as::<RejectionInput>().unwrap_or_default();
            let blank = input.notice.trim().is_empty();
            Box::pin(async move {
                if blank {
                    Err(english("wl_hr.payroll.notice_required", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .guard("reason_given", |_run: &PayrollRun, ctx| {
            let input = ctx.input_as::<RejectionInput>().unwrap_or_default();
            let known = REASONS.contains(&input.reason.trim());
            Box::pin(async move {
                if known {
                    Ok(())
                } else {
                    Err(english("wl_hr.payroll.rejection_reason_invalid", &[]))
                }
            })
        })
        .effect(|run: &PayrollRun, ctx, tx| {
            let id = run.id;
            let version = run.sif_version;
            let input = ctx.input_as::<RejectionInput>().unwrap_or_default();
            Box::pin(async move {
                let reason = input.reason.trim().to_owned();
                let notice = input.notice.trim().to_owned();
                let lines = PayrollLine::query()
                    .where_(PayrollLine::RUN_ID.eq(id))
                    .where_(PayrollLine::STATUS.eq(LineStatus::Included))
                    .order_by(PayrollLine::ID)
                    .get(&mut *tx)
                    .await?;
                for line in lines.into_iter().filter(|l| l.sif_version == Some(version)) {
                    PayrollLine::query()
                        .where_(PayrollLine::ID.eq(line.id))
                        .update(
                            &mut *tx,
                            &[
                                ("status", &LineStatus::Failed.as_str().to_owned()),
                                ("failure_reason", &Some(reason.clone())),
                                ("updated_at", &now()),
                            ],
                        )
                        .await?;
                }
                PayrollRun::query()
                    .where_(PayrollRun::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("rejection_reason", &Some(reason)),
                            ("rejection_notice", &Some(notice)),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `resubmit`.
pub(crate) fn resubmit() -> Transition<PayrollRun> {
    Transition::new(RESUBMIT)
        .label("Resubmit")
        .tone("primary")
        .permission(permissions::PAYROLL_PREPARE)
        .from([PayrollRunStatus::Rejected])
        .to(PayrollRunStatus::Submitted)
        .guard("fix_noted", |_run: &PayrollRun, ctx| {
            let input = ctx.input_as::<ResubmitInput>().unwrap_or_default();
            let blank = input.note.trim().is_empty();
            Box::pin(async move {
                if blank {
                    Err(english("wl_hr.payroll.fix_note_required", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|run: &PayrollRun, ctx, tx| {
            let id = run.id;
            let version = run.sif_version;
            let reason = run.rejection_reason.clone();
            let entity = run.legal_entity_id;
            let input = ctx.input_as::<ResubmitInput>().unwrap_or_default();
            Box::pin(async move {
                let refuse =
                    |key: &str| Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]));
                let Some(rejected) = SifFile::version_in(&mut *tx, id, version).await? else {
                    return Err(refuse("wl_hr.payroll.no_file"));
                };
                // The cause named by the establishment ID is fixed when the
                // entity's ID is no longer the one the file was named with.
                if reason.as_deref() == Some(REASON_ESTABLISHMENT_ID) {
                    let now_id = establishment::id_of_in(&mut *tx, entity).await?;
                    if now_id == rejected.establishment_id {
                        return Err(refuse("wl_hr.payroll.establishment_unchanged"));
                    }
                }
                // The same lines with the same nets, every one of them failed.
                let carried = rejected.lines_in(&mut *tx).await?;
                let mut ids = Vec::new();
                for sent in &carried {
                    let line = PayrollLine::find(&mut *tx, sent.line_id).await?;
                    let Some(line) = line else {
                        return Err(refuse("wl_hr.payroll.lines_changed"));
                    };
                    if line.net_amount != sent.net_amount {
                        return Err(refuse("wl_hr.payroll.amounts_changed"));
                    }
                    if line.status != LineStatus::Failed {
                        return Err(refuse("wl_hr.payroll.lines_changed"));
                    }
                    ids.push(line.id);
                }
                PayrollLine::query()
                    .where_(PayrollLine::ID.is_in(ids.clone()))
                    .update(
                        &mut *tx,
                        &[
                            ("status", &LineStatus::Included.as_str().to_owned()),
                            ("failure_reason", &None::<String>),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                sif::generate_in(&mut *tx, id, Some(&ids)).await?;
                PayrollRun::query()
                    .where_(PayrollRun::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("resubmit_note", &Some(input.note.trim().to_owned())),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::StateMachine;

    #[test]
    fn rejection_and_resubmit_are_user_transitions_of_payroll_prepare() {
        let machine = PayrollRun::machine();
        let rejection = machine.find(RECORD_REJECTION).unwrap();
        assert!(!rejection.is_system());
        assert_eq!(rejection.from_states(), ["submitted"]);
        assert_eq!(rejection.to_state(), Some("rejected"));
        let resubmit = machine.find(RESUBMIT).unwrap();
        assert!(!resubmit.is_system());
        assert_eq!(resubmit.from_states(), ["rejected"]);
        assert_eq!(resubmit.to_state(), Some("submitted"));
    }

    #[test]
    fn there_are_four_reasons() {
        assert_eq!(REASONS.len(), 4);
        assert!(REASONS.contains(&"establishment_id"));
    }
}
