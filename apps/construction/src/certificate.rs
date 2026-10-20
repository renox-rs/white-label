//! The client's certificate of an IPA (`certify`, 02 Construction §4.4 step
//! 5, §7.1 *IPA*; BL-CON-04, extended by the deductions of BL-CON-05).
//!
//! The QS (`ipas.certify` in the project) records the certificate on a
//! `submitted` IPA with the **certified amount**: the cumulative work the
//! client certifies, before its deductions, and the certificate's document
//! reference. The effect records the **claim gap** (claimed less certified
//! work, never the net paid) and refreshes the drafts of the project: a draft's
//! "previous certified" is the cumulative certified amount of the latest IPA
//! (by sequence number) that is `certified` or `invoiced`, a `submitted` IPA
//! never counts, and the *previous pending* flag clears when no earlier IPA
//! is `submitted` any more.
//!
//! **A late certificate for a superseded IPA** is recorded on the IPA that
//! replaced it ([`record`]): as its `certify` when it is still `submitted`
//! (the certified amount is what the client certified; the gap to the
//! replacing claim is tracked as usual); when it is `certified` or `invoiced`
//! already, the certificate is attached for reference only, no amount
//! changes, and the QS gets a task.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::ipa::{self, CERTIFY, Ipa, IpaStatus};
use crate::permissions;
use crate::project::Project;
use crate::tasks::{self, Task};
use crate::texts::english;

/// What a `certify` run carries to its guard and effect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertifyInput {
    /// The cumulative certified work before deductions, in fils.
    #[serde(default)]
    pub certified_amount: Option<i64>,
    /// The certificate, a document reference.
    #[serde(default)]
    pub certificate: String,
    /// The day the client certified; today when empty.
    #[serde(default)]
    pub certified_on: Option<NaiveDate>,
}

/// The input of a `certify` run.
pub(crate) fn certify_input(ctx: &TransitionCtx) -> CertifyInput {
    ctx.input_as::<CertifyInput>().unwrap_or_default()
}

/// Why the IPA cannot be certified with `input`, if it cannot: the amount and
/// the certificate are needed.
pub fn certify_problem(input: &CertifyInput) -> Option<String> {
    if input.certified_amount.is_none_or(|amount| amount < 0) {
        return Some(english("construction.ipa.certified_amount_needed", &[]));
    }
    if input.certificate.trim().is_empty() {
        return Some(english("construction.ipa.certificate_needed", &[]));
    }
    None
}

/// The effect of `certify` inside its transaction: the certificate, the claim
/// gap, and the refresh of the drafts.
pub(crate) async fn certify_effects(
    tx: &mut Transaction,
    ipa: &Ipa,
    input: &CertifyInput,
) -> Result {
    let amount = input.certified_amount.unwrap_or_default();
    let certified = Some(amount);
    let gap = Some(ipa.claimed_amount - amount);
    let on = Some(input.certified_on.unwrap_or_else(today));
    let reference = Some(input.certificate.trim().to_owned());
    Ipa::query()
        .where_(Ipa::ID.eq(ipa.id))
        .update(
            &mut *tx,
            &[
                ("certified_amount", &certified),
                ("claim_gap", &gap),
                ("certified_on", &on),
                ("certificate_ref", &reference),
            ],
        )
        .await?;
    refresh_drafts_in(tx, ipa.project_id).await
}

/// The cumulative certified amount of the latest `certified` or `invoiced`
/// IPA of `project_id` numbered below `before`, inside `tx`; 0 when none.
///
/// # Errors
///
/// A database error.
pub async fn previous_certified_in(
    tx: &mut Transaction,
    project_id: i64,
    before: i64,
) -> Result<i64> {
    let certified = Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project_id))
        .where_(Ipa::STATUS.is_in([IpaStatus::Certified, IpaStatus::Invoiced]))
        .where_(Ipa::NUMBER.lt(before))
        .order_by(Ipa::NUMBER)
        .get(&mut *tx)
        .await?;
    Ok(certified
        .last()
        .and_then(|latest| latest.certified_amount)
        .unwrap_or_default())
}

/// Whether an IPA of `project_id` numbered below `before` is `submitted`,
/// inside `tx`.
///
/// # Errors
///
/// A database error.
pub async fn pending_before_in(tx: &mut Transaction, project_id: i64, before: i64) -> Result<bool> {
    Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project_id))
        .where_(Ipa::STATUS.eq(IpaStatus::Submitted))
        .where_(Ipa::NUMBER.lt(before))
        .exists(&mut *tx)
        .await
}

/// Recomputes "previous certified" and the *previous pending* flag of every
/// `draft` IPA of `project_id`, inside `tx`.
///
/// # Errors
///
/// A database error.
pub async fn refresh_drafts_in(tx: &mut Transaction, project_id: i64) -> Result {
    for draft in ipa::in_status_in(tx, project_id, IpaStatus::Draft).await? {
        let previous = previous_certified_in(tx, project_id, draft.number).await?;
        let pending = pending_before_in(tx, project_id, draft.number).await?;
        Ipa::query()
            .where_(Ipa::ID.eq(draft.id))
            .update(
                &mut *tx,
                &[
                    ("previous_certified", &previous),
                    ("previous_pending", &pending),
                ],
            )
            .await?;
    }
    Ok(())
}

/// What the QS enters.
#[derive(Debug, Clone)]
pub struct CertificateRequest {
    /// The cumulative certified work before deductions, in fils.
    pub certified_amount: i64,
    /// The certificate, a document reference.
    pub certificate: String,
    /// The day the client certified; today when empty.
    pub certified_on: Option<NaiveDate>,
}

/// What [`record`] did.
#[derive(Debug)]
pub enum Recorded {
    /// The IPA (the one named, or the one that replaced it) is `certified`.
    Certified(Ipa),
    /// The replacing IPA was certified already: the certificate was attached
    /// for reference and nothing else changed.
    Attached(Ipa),
}

/// `user` records the client's certificate for IPA `id`. For a superseded IPA
/// the certificate goes to the IPA that replaced it (see the module).
///
/// # Errors
///
/// `Forbidden` without `ipas.certify` in the project, `Guard` (422) when the
/// amount or the certificate is missing or the IPA cannot take a certificate
/// (a `draft`, or a superseded one nobody replaced). Nothing changes then.
pub async fn record(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    request: &CertificateRequest,
) -> std::result::Result<Recorded, TransitionError> {
    let mut target = Ipa::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let mut hops = 0;
    while target.status == IpaStatus::Superseded && hops < 100 {
        let Some(next) = ipa::replacement_of(&state.db, target.id).await? else {
            break;
        };
        target = next;
        hops += 1;
    }
    let project = Project::find_or_404(&state.db, target.project_id).await?;
    if !user.has_permission_in(permissions::IPAS_CERTIFY, &project.scope()) {
        return Err(TransitionError::Forbidden);
    }
    let input = CertifyInput {
        certified_amount: Some(request.certified_amount),
        certificate: request.certificate.clone(),
        certified_on: request.certified_on,
    };
    match target.status {
        IpaStatus::Submitted => {
            let _outcome = target
                .transition(CERTIFY)
                .by(user)
                .input(&input)
                .run(state)
                .await?;
            Ok(Recorded::Certified(
                Ipa::find_or_404(&state.db, target.id).await?,
            ))
        }
        IpaStatus::Certified | IpaStatus::Invoiced if target.id != id => {
            if let Some(problem) = certify_problem(&input) {
                return Err(TransitionError::Guard(problem));
            }
            attach(state, &project, &target, &input).await
        }
        _ => Err(TransitionError::Guard(english(
            "construction.ipa.cannot_certify",
            &[("number", &target.number)],
        ))),
    }
}

/// Attaches a late certificate to the already certified IPA `target` and
/// tells the QS; no amount changes.
async fn attach(
    state: &AppState,
    project: &Project,
    target: &Ipa,
    input: &CertifyInput,
) -> std::result::Result<Recorded, TransitionError> {
    let reference = Some(input.certificate.trim().to_owned());
    Ipa::query()
        .where_(Ipa::ID.eq(target.id))
        .update(&state.db, &[("late_certificate_ref", &reference)])
        .await?;
    tasks::open(
        state,
        project,
        Task {
            subject: target.subject(),
            role: permissions::QS_ROLE,
            assignee: None,
            source_key: format!("construction.ipa_late_certificate:{}", target.id),
            title: english(
                "construction.ipa.late_certificate_title",
                &[("number", &target.number), ("project", &project.name)],
            ),
            description: english("construction.ipa.late_certificate_body", &[]),
            due: None,
        },
    )
    .await?;
    Ok(Recorded::Attached(
        Ipa::find_or_404(&state.db, target.id).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_amount_and_the_certificate_are_needed() {
        let mut input = CertifyInput {
            certified_amount: Some(52_000_000),
            certificate: "CERT-5.pdf".into(),
            certified_on: None,
        };
        assert_eq!(certify_problem(&input), None);
        input.certificate = " ".into();
        assert!(certify_problem(&input).is_some_and(|p| p.contains("certificate")));
        input.certified_amount = None;
        assert!(certify_problem(&input).is_some_and(|p| p.contains("certified amount")));
        input.certified_amount = Some(-1);
        assert!(certify_problem(&input).is_some());
    }
}
