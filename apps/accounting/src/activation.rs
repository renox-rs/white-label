//! What engagement `activate` checks (04 §7.1 *Engagement*, row `activate`).
//!
//! Every failing check is gathered first and listed in one refusal, each
//! line saying what to do: the CDD file approved (`ACC.ACL.07`), the
//! engagement letter signed and the tax profile complete.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};

use crate::cdd::{CddFile, CddStatus};
use crate::db::refused;
use crate::engagement::{Engagement, EngagementStatus};
use crate::engagement_machine::{ACTIVATE, EngagementInput};
use crate::permissions;
use crate::tax_profile::TaxProfile;
use crate::texts::english;

/// The checks on the engagement itself that fail on `on`, one English line
/// each: the CDD file approved, the letter signed, the tax profile complete.
///
/// # Errors
///
/// The database's errors.
pub async fn readiness(db: &Db, engagement: &Engagement, on: NaiveDate) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    let approved = CddFile::current(db, engagement.client())
        .await?
        .is_some_and(|file| file.status == CddStatus::Approved);
    if !approved {
        problems.push(english("accounting.activation.cdd_not_approved", &[]));
    }
    if !engagement.letter_signed {
        problems.push(english("accounting.activation.letter_not_signed", &[]));
    }
    match TaxProfile::current(db, engagement.client(), on).await? {
        None => problems.push(english("accounting.activation.no_tax_profile", &[])),
        Some(profile) => {
            let missing = profile.missing();
            if !missing.is_empty() {
                problems.push(english(
                    "accounting.activation.tax_profile_incomplete",
                    &[("missing", &missing.join(", "))],
                ));
            }
        }
    }
    Ok(problems)
}

/// The refusal that lists `problems`: "This engagement cannot be activated:"
/// and one line each.
pub fn refusal(problems: &[String]) -> Error {
    let mut lines = vec![english("accounting.activation.refused", &[])];
    lines.extend(problems.iter().map(|p| format!("- {p}")));
    refused(lines.join("\n"))
}

/// `by` activates `engagement` on `on` once every check passes.
///
/// # Errors
///
/// A 422 listing every failing check; `Forbidden` without
/// `engagements.manage`; a 422 unless `onboarding`.
pub(crate) async fn run(
    state: &AppState,
    by: &AuthUser,
    engagement: &Engagement,
    on: NaiveDate,
) -> Result {
    if !by.has_permission(permissions::ENGAGEMENTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    if engagement.status == EngagementStatus::Onboarding {
        let problems = readiness(&state.db, engagement, on).await?;
        if !problems.is_empty() {
            return Err(refusal(&problems));
        }
    }
    let _outcome = engagement
        .transition(ACTIVATE)
        .by(by)
        .input(&EngagementInput::on(on))
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}
