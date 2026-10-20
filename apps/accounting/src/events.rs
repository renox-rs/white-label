//! What happens after a transition commits (04 §7.1): the work that reaches
//! other modules runs here, in listeners that re-read the row, never inside
//! an effect.
//!
//! - Engagement `activate`: the compliance calendar is generated
//!   ([`crate::calendar`]) and the retainer started ([`crate::retainer`]).
//! - CDD `reject`: the client's `onboarding` engagements are declined
//!   ([`crate::engagements::decline_not_accepted`]).
//! - CDD `exit`, and `reject` of the file of a client with an `active` or
//!   `suspended` engagement: the partner gets the task to end it
//!   ([`crate::exits::after`], BL-ACC-09); the task closes when the last
//!   engagement has ended ([`crate::exits::on_ended`]).
//! - `PartyBlocked`: the client's engagements are suspended with the neutral
//!   reason *on hold* ([`crate::engagements::suspend_for_block`],
//!   `ACC.ACL.14`).
//! - Engagement `suspend` and `auto_suspend`: its tasks are held and its
//!   retainer paused ([`crate::suspension::hold`]).
//! - Engagement `resume` and `resume_approved`: its tasks are released, its
//!   retainer resumed, the skipped request lists created and the deadline
//!   notice reminders cancelled ([`crate::engagements::after_resume`]).
//! - `ScreeningMatchConfirmed`: `ACC.RUL.12` checks the client company is
//!   blocked, opens the AML case or adds to the open one and notifies the
//!   MLRO ([`crate::aml_cases::on_match_confirmed`], BL-ACC-10).
//! - AML case `report`: the CDD file is flagged for review, the compliance
//!   officer, the partners and the MLRO get their tasks and notice
//!   ([`crate::aml_cases::after_report`], BL-ACC-11).
//! - Return transitions: the notices, the preparer's task and the linked
//!   deadline task ([`crate::return_effects`]); `TaskOverdue` tells the
//!   preparer's task to start the overdue linked task.
//! - Engagement `suspend`, `auto_suspend`, `end` and `auto_end`: `ACC.RUL.09`
//!   sends the statutory deadline notice, or prepares it for the partner or
//!   the MLRO ([`crate::deadline_notices::run_for`]).
//! - Engagement `end` and `auto_end`, after the notice: the retainer is
//!   cancelled with its pro rata document, the upcoming tasks are waived and
//!   the started ones listed ([`crate::ending::after_end`], BL-ACC-06).
//! - Tax profile change `request_approved`: `ACC.RUL.10` regenerates the
//!   calendar from the effective date ([`crate::regeneration`], BL-ACC-14,
//!   BL-ACC-15).
//! - Write-off `write_off` and `write_off_approved`: the time module writes
//!   the unbilled time off ([`crate::wip::after`], `ACC.ACL.09`).

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::deadlines::TaskOverdue;
use wl_kernel::documents::today;
use wl_kernel::{PartyBlocked, ScreeningMatchConfirmed};

use crate::aml::AmlCase;
use crate::aml_machine::REPORT;
use crate::cdd::CddFile;
use crate::cdd_machine::{EXIT, REJECT};
use crate::engagement::Engagement;
use crate::engagement_machine::{
    ACTIVATE, AUTO_END, AUTO_SUSPEND, END, RESUME, RESUME_APPROVED, SUSPEND,
};
use crate::tax_profile_change::{REQUEST_APPROVED, TaxProfileChange};
use crate::tax_return::Return;
use crate::wip::{WRITE_OFF, WRITE_OFF_APPROVED, WipWriteOff};
use crate::{
    aml_cases, calendar, deadline_notices, ending, engagements, exits, regeneration, retainer,
    return_effects, suspension, wip,
};

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
            if ![SUSPEND, AUTO_SUSPEND, END, AUTO_END].contains(&event.transition.as_str()) {
                return Ok(());
            }
            let ended = [END, AUTO_END].contains(&event.transition.as_str());
            if !ended {
                let engagement = Engagement::find_or_404(&state.db, event.key).await?;
                suspension::hold(&state, &engagement, today()).await?;
            }
            // The notice lists the deadlines before the end waives the
            // upcoming tasks.
            deadline_notices::run_for(&state, event.key, &event.transition).await?;
            if ended {
                ending::after_end(&state, event.key).await?;
                exits::on_ended(&state, event.key).await?;
            }
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
        if event.transition != REJECT && event.transition != EXIT {
            return Ok(());
        }
        let file = CddFile::find_or_404(&state.db, event.key).await?;
        if event.transition == REJECT {
            engagements::decline_not_accepted(&state, file.client()).await?;
        }
        // A client served by an engagement (an exit, or a file rejected at a
        // review) gives the partner the task to end it.
        exits::after(&state, &file).await?;
        Ok(())
    });
    app.listen(|event: Transitioned<AmlCase>, state: AppState| async move {
        if event.transition != REPORT {
            return Ok(());
        }
        aml_cases::after_report(&state, event.key).await
    });
    app.listen(|event: Transitioned<Return>, state: AppState| async move {
        return_effects::after(&state, &event).await
    });
    app.listen(
        |event: Transitioned<TaxProfileChange>, state: AppState| async move {
            if event.transition != REQUEST_APPROVED {
                return Ok(());
            }
            regeneration::run_for_change(&state, event.key)
                .await
                .map(|_| ())
        },
    );
    app.listen(
        |event: Transitioned<WipWriteOff>, state: AppState| async move {
            if event.transition != WRITE_OFF && event.transition != WRITE_OFF_APPROVED {
                return Ok(());
            }
            wip::after(&state, event.key).await
        },
    );
    app.listen(|event: TaskOverdue, state: AppState| async move {
        return_effects::on_task_overdue(&state, &event).await
    });
    app.listen(
        |event: ScreeningMatchConfirmed, state: AppState| async move {
            aml_cases::on_match_confirmed(&state, &event)
                .await
                .map(|_| ())
        },
    );
    app.listen(|event: PartyBlocked, state: AppState| async move {
        engagements::suspend_for_block(&state, event.party, &event.display_reason).await
    });
}
