//! The certificate machine (10 §7.1 *Certificate*; `EDU.CRT.01`).
//!
//! | Transition | From | To | By | Guards |
//! |---|---|---|---|---|
//! | [`ISSUE`] | draft | issued | `certificates.issue` | `criteria_met` (`EDU.ACL.06`), `document_valid` (BL-EDU-07) |
//! | [`REISSUE`] | issued | superseded | `certificates.issue` | a reason, `criteria_met` |
//! | [`REVOKE`] | issued | revoked | `certificates.revoke` | a reason |
//!
//! Decisions:
//!
//! - As for the other machines, the guards see only the record and the run's
//!   input; [`crate::certificates`] computes whether the criteria are met (the
//!   enrolment is `completed`) and the date the documents are checked on, and
//!   a run without them is refused.
//! - `issue` takes the next number of the gapless series [`SERIES`] inside its
//!   own transaction, so a refused issue leaves no gap and a number is never
//!   reused. A reissued draft links the certificate it replaces to itself in
//!   the same step: the old public page then points to the new number.
//! - `document_valid` reads the verdict of the document guard that
//!   [`crate::certificates`] asks before the run ([`crate::documents`]): the
//!   centre permit must be valid on `on_date`, and the course approval too when
//!   the certificate states it, with no override. A run without a verdict is
//!   refused. For a reissued draft `on_date` is the enrolment's completion
//!   date, so a correct certificate earned under a valid approval can be
//!   reissued after the approval expired (BL-EDU-16).
//! - `reissue` is the seam of BL-EDU-16: it supersedes the certificate and
//!   creates the linked draft with the corrected data; the number comes when
//!   the draft is issued. Printing a copy and the student's notification are
//!   BL-EDU-16's.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::{Reset, Series};

use crate::certificate::{Certificate, CertificateStatus, new_code};
use crate::permissions;
use crate::texts::english;

/// `draft → issued`: published, numbered.
pub const ISSUE: &str = "issue";
/// `issued → superseded`: replaced by a corrected draft.
pub const REISSUE: &str = "reissue";
/// `issued → revoked`: withdrawn.
pub const REVOKE: &str = "revoke";

/// The numbers of the certificates: `CRT-0150`, across the centre, never
/// reset, never skipped.
pub const SERIES: Series = Series {
    key: "edtech_certificate",
    pattern: "{prefix}-{0000}",
    prefix: "CRT",
    reset: Reset::Never,
    gapless: true,
    per_entity: false,
};

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateInput {
    /// Whether the completion criteria are met (`EDU.ACL.06`).
    #[serde(default)]
    pub criteria_met: Option<bool>,
    /// The date the documents are checked on ([`ISSUE`]).
    #[serde(default)]
    pub on_date: Option<NaiveDate>,
    /// The user who acted.
    #[serde(default)]
    pub by: Option<i64>,
    /// The corrected name ([`REISSUE`]).
    #[serde(default)]
    pub student_name: Option<String>,
    /// The corrected completion date ([`REISSUE`]).
    #[serde(default)]
    pub completion_date: Option<NaiveDate>,
    /// Whether the coordinator confirms that a recorded KHDA attestation is
    /// copied to the new draft ([`REISSUE`]).
    #[serde(default)]
    pub copy_attestation: bool,
    /// What the document guard answered for the centre permit and the course
    /// approval on [`CertificateInput::on_date`] ([`ISSUE`]).
    #[serde(default)]
    pub documents: Option<DocumentsVerdict>,
}

/// The answer of the document guard (`wl-documents`) for `certificate.issue`,
/// gathered by the service because a guard of the machine cannot reach the
/// database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentsVerdict {
    /// Every document the certificate needs is valid on the day.
    Valid,
    /// One or more are expired or missing; the lines name them.
    Refused(String),
}

fn input_of(ctx: &TransitionCtx) -> CertificateInput {
    ctx.input_as::<CertificateInput>().unwrap_or_default()
}

/// `Ok` when `ok`, else the English text `key`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// Sets the columns of certificate `id` other than the status.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    Certificate::query()
        .where_(Certificate::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

impl StateMachine for Certificate {
    type State = CertificateStatus;
    const COLUMN: renox::db::Col<Certificate, CertificateStatus> = Certificate::STATUS;

    fn machine() -> Machine<Self> {
        use CertificateStatus::{Draft, Issued, Revoked, Superseded};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Issued)
            .tone("success")
            .state(Superseded)
            .tone("neutral")
            .terminal()
            .state(Revoked)
            .tone("error")
            .terminal()
            .transition(issue())
            .transition(reissue())
            .transition(revoke())
    }
}

/// `issue`: the criteria are met and the documents valid; the number is
/// assigned and a certificate this one replaces now links it.
fn issue() -> Transition<Certificate> {
    Transition::new(ISSUE)
        .label("Issue")
        .tone("primary")
        .permission(permissions::CERTIFICATES_ISSUE)
        .from([CertificateStatus::Draft])
        .to(CertificateStatus::Issued)
        .guard("criteria_met", |_c: &Certificate, ctx| {
            let ok = input_of(ctx).criteria_met == Some(true);
            Box::pin(async move { check(ok, "edtech.certificates.problem.criteria") })
        })
        .guard("document_valid", |_c: &Certificate, ctx| {
            let verdict = input_of(ctx).documents;
            Box::pin(async move {
                match verdict {
                    Some(DocumentsVerdict::Valid) => Ok(()),
                    Some(DocumentsVerdict::Refused(lines)) => Err(lines),
                    None => Err(english("edtech.certificates.problem.documents", &[])),
                }
            })
        })
        .effect(|c: &Certificate, ctx, tx| {
            let c = c.clone();
            let input = input_of(ctx);
            Box::pin(async move {
                let on = today();
                let number = SERIES.next(&mut *tx, None, None, on).await?;
                set(
                    tx,
                    c.id,
                    &[
                        ("number", &Some(number)),
                        ("issued_on", &Some(on)),
                        ("issued_by", &input.by),
                    ],
                )
                .await?;
                if let Some(old) = c.supersedes_id {
                    set_old(tx, old, c.id).await?;
                }
                Ok(())
            })
        })
}

/// The replaced certificate `old` links its replacement `new`.
async fn set_old(tx: &mut renox::db::Transaction, old: i64, new: i64) -> Result {
    set(tx, old, &[("superseded_by_id", &Some(new))]).await
}

/// `reissue`: the certificate is superseded and a linked draft is created with
/// the corrected data (never an edit of the issued one).
fn reissue() -> Transition<Certificate> {
    Transition::new(REISSUE)
        .label("Reissue")
        .permission(permissions::CERTIFICATES_ISSUE)
        .from([CertificateStatus::Issued])
        .to(CertificateStatus::Superseded)
        .reason_required()
        .guard("criteria_met", |_c: &Certificate, ctx| {
            let ok = input_of(ctx).criteria_met == Some(true);
            Box::pin(async move { check(ok, "edtech.certificates.problem.reissue_criteria") })
        })
        .effect(|c: &Certificate, ctx, tx| {
            let c = c.clone();
            let input = input_of(ctx);
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                let name = input
                    .student_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map_or_else(|| c.student_name.clone(), str::to_owned);
                let mut draft = Certificate {
                    enrolment_id: c.enrolment_id,
                    student_party_id: c.student_party_id,
                    student_name: name,
                    course_title: c.course_title.clone(),
                    approval_no: c.approval_no.clone(),
                    states_approval: c.states_approval,
                    completion_date: input.completion_date.unwrap_or(c.completion_date),
                    verification_code: new_code(),
                    supersedes_id: Some(c.id),
                    attestation_ref: if input.copy_attestation {
                        c.attestation_ref.clone()
                    } else {
                        None
                    },
                    ..Certificate::default()
                };
                draft.insert(&mut *tx).await?;
                set(tx, c.id, &[("reissue_reason", &reason)]).await
            })
        })
}

/// `revoke`: the certificate is withdrawn; the public page says so.
fn revoke() -> Transition<Certificate> {
    Transition::new(REVOKE)
        .label("Revoke")
        .permission(permissions::CERTIFICATES_REVOKE)
        .from([CertificateStatus::Issued])
        .to(CertificateStatus::Revoked)
        .reason_required()
        .effect(|c: &Certificate, ctx, tx| {
            let id = c.id;
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                set(
                    tx,
                    id,
                    &[("revoke_reason", &reason), ("revoked_on", &Some(today()))],
                )
                .await
            })
        })
}

/// Registers the machine for `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<Certificate>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition_of_the_spec() {
        let machine = Certificate::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("draft", Some("neutral"), false),
                ("issued", Some("success"), false),
                ("superseded", Some("neutral"), true),
                ("revoked", Some("error"), true),
            ]
        );
        let expected = [
            (ISSUE, "draft", "issued"),
            (REISSUE, "issued", "superseded"),
            (REVOKE, "issued", "revoked"),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), [from], "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert!(!transition.is_system(), "{name}");
        }
    }

    #[test]
    fn the_guards_and_the_guard_slot_are_declared() {
        let machine = Certificate::machine();
        let guards = |name: &str| {
            machine
                .find(name)
                .unwrap()
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(guards(ISSUE), ["criteria_met", "document_valid"]);
        assert_eq!(guards(REISSUE), ["criteria_met"]);
        assert!(guards(REVOKE).is_empty());
        for name in [REISSUE, REVOKE] {
            assert!(machine.find(name).unwrap().needs_reason(), "{name}");
        }
    }
}
