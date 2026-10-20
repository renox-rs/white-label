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

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::PartyBlocked;
use wl_kernel::documents::today;

use crate::cdd::CddFile;
use crate::cdd_machine::REJECT;
use crate::engagement::Engagement;
use crate::engagement_machine::ACTIVATE;
use crate::{calendar, engagements, retainer};

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
