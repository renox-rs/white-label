//! What happens after a transition commits (04 §7.1): the work that reaches
//! other modules runs here, in listeners that re-read the row, never inside
//! an effect.
//!
//! - Engagement `activate`: the compliance calendar is generated
//!   ([`crate::calendar`]) and the retainer started ([`crate::retainer`]).
//! - CDD `reject`: the client's `onboarding` engagements are declined
//!   ([`crate::engagements::decline_not_accepted`]).
//! - `PartyBlocked`: the client's engagements are suspended with the neutral
//!   reason *on hold* ([`crate::engagements::suspend_for_block`],
//!   `ACC.ACL.14`).
//! - Engagement `suspend` and `auto_suspend`: its tasks are held and its
//!   retainer paused ([`crate::suspension::hold`]).
//! - Engagement `resume` and `resume_approved`: its tasks are released, its
//!   retainer resumed, the skipped request lists created and the deadline
//!   notice reminders cancelled ([`crate::engagements::after_resume`]).
//! - Engagement `suspend`, `auto_suspend` and `end`: `ACC.RUL.09` sends the
//!   statutory deadline notice, or prepares it for the partner or the MLRO
//!   ([`crate::deadline_notices::run_for`]).

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::PartyBlocked;
use wl_kernel::documents::today;

use crate::cdd::CddFile;
use crate::cdd_machine::REJECT;
use crate::engagement::Engagement;
use crate::engagement_machine::{ACTIVATE, AUTO_SUSPEND, END, RESUME, RESUME_APPROVED, SUSPEND};
use crate::{calendar, deadline_notices, engagements, retainer, suspension};

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<Engagement>, state: AppState| async move {
            if event.transition != ACTIVATE {
                return Ok(());
            }
            let engagement = Engagement::find_or_404(&state.db, event.key).await?;
            let on = engagement.activated_on.unwrap_or_else(today);
            calendar::generate(&state, &engagement, on).await?;
            retainer::start(&state, &engagement, on).await?;
            Ok(())
        },
    );
    app.listen(
        |event: Transitioned<Engagement>, state: AppState| async move {
            if ![SUSPEND, AUTO_SUSPEND, END].contains(&event.transition.as_str()) {
                return Ok(());
            }
            if event.transition != END {
                let engagement = Engagement::find_or_404(&state.db, event.key).await?;
                suspension::hold(&state, &engagement, today()).await?;
            }
            deadline_notices::run_for(&state, event.key, &event.transition).await?;
            Ok(())
        },
    );
    app.listen(
        |event: Transitioned<Engagement>, state: AppState| async move {
            if event.transition != RESUME && event.transition != RESUME_APPROVED {
                return Ok(());
            }
            engagements::after_resume(&state, event.key, today()).await
        },
    );
    app.listen(|event: Transitioned<CddFile>, state: AppState| async move {
        if event.transition != REJECT {
            return Ok(());
        }
        let file = CddFile::find_or_404(&state.db, event.key).await?;
        engagements::decline_not_accepted(&state, file.client()).await
    });
    app.listen(|event: PartyBlocked, state: AppState| async move {
        engagements::suspend_for_block(&state, event.party, &event.display_reason).await
    });
}
