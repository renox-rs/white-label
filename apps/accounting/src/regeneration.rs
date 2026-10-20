//! `ACC.RUL.10`: the compliance calendar is regenerated after an approved
//! tax profile change (BL-ACC-15). The rule run is started here once the
//! approval of a [`TaxProfileChange`](crate::tax_profile_change::TaxProfileChange)
//! commits (BL-ACC-14).

use renox::prelude::*;
use wl_kernel::automation::{RuleRunSpec, RunOutcome, run_rule};

use crate::tax_profile_change::TaxProfileChange;

/// The rule's id.
pub const RULE: &str = "ACC.RUL.10";

/// Runs `ACC.RUL.10` for the approved change `change`, once per change: a
/// second call for the same change does nothing.
///
/// # Errors
///
/// The database's errors (a failing action is retried by the rule run).
pub async fn run_for_change(state: &AppState, change: i64) -> Result<RunOutcome> {
    let row = TaxProfileChange::find_or_404(&state.db, change).await?;
    let subject = row.subject();
    let key = format!("{RULE}:{subject}");
    let spec = RuleRunSpec::new(RULE, subject, key);
    run_rule(state, spec, move |_state: AppState| async move { Ok(()) }).await
}
