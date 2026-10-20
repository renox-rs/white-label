//! The engagement services: one function per action, each gathering what
//! the machine's guards need and running the transition (04 §7.1
//! *Engagement*). Later issues extend these: the effects of `suspend`,
//! `resume` and `end` on tasks, retainer and notices (BL-ACC-03 to
//! BL-ACC-06).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::today;
use wl_kernel::PartyId;
use wl_kernel::parties::{PartyStatus, directory};

use crate::activation;
use crate::cdd::CddFile;
use crate::db::{refused, set_engagement};
use crate::engagement::{Engagement, EngagementStatus, NewEngagement};
use crate::engagement_machine::{
    AUTO_DECLINE, AUTO_SUSPEND, DECLINE, END, EngagementInput, RESUME, SUSPEND,
};
use crate::permissions;
use crate::reasons::{self, NOT_ACCEPTED};
use crate::texts::english;

/// The engagement `id`, or 404.
async fn load(state: &AppState, id: i64) -> Result<Engagement> {
    Engagement::find_or_404(&state.db, id).await
}

/// `Forbidden` unless `user` holds `permission`.
fn require(user: &AuthUser, permission: &str) -> Result {
    if user.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// Creates an `onboarding` engagement of `new.client` for `new.title`.
///
/// # Errors
///
/// A 422 for a blank title, a negative fee or a client that is not a known
/// party; the database's errors.
pub async fn create(state: &AppState, by: Option<i64>, new: NewEngagement) -> Result<Engagement> {
    if new.title.trim().is_empty() {
        return Err(refused(english("accounting.problems.title_required", &[])));
    }
    if new.retainer_fee < 0 {
        return Err(refused(english("accounting.problems.fee_negative", &[])));
    }
    if directory(state)
        .find(&state.db, new.client)
        .await?
        .is_none()
    {
        return Err(refused(english("accounting.problems.client_unknown", &[])));
    }
    Engagement::create(
        &state.db,
        Engagement {
            client_party_id: new.client.0,
            title: new.title.trim().to_owned(),
            fee_basis: new.fee_basis,
            retainer_fee: new.retainer_fee,
            start_date: new.start_date,
            manager_user_id: new.manager_user_id,
            created_by: by,
            ..Engagement::default()
        },
    )
    .await
}

/// `by` records that the client signed the engagement letter on `on`.
///
/// # Errors
///
/// `Forbidden` without `engagements.manage`; a 422 unless `onboarding`.
pub async fn record_letter_signed(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    on: NaiveDate,
) -> Result<Engagement> {
    require(by, permissions::ENGAGEMENTS_MANAGE)?;
    let engagement = load(state, id).await?;
    if engagement.status != EngagementStatus::Onboarding {
        return Err(refused(english("accounting.problems.not_onboarding", &[])));
    }
    let mut tx = state.db.begin().await?;
    set_engagement(
        &mut tx,
        id,
        &[("letter_signed", &true), ("letter_signed_on", &Some(on))],
    )
    .await?;
    tx.commit().await?;
    load(state, id).await
}

/// `by` activates engagement `id` today (Asia/Dubai): every check of
/// [`crate::activation`] must pass. After the commit the compliance calendar
/// is generated and the retainer started ([`crate::events`]).
///
/// # Errors
///
/// A 422 listing every failing check; `Forbidden` without
/// `engagements.manage`.
pub async fn activate(state: &AppState, by: &AuthUser, id: i64) -> Result<Engagement> {
    let engagement = load(state, id).await?;
    activation::run(state, by, &engagement, today()).await?;
    load(state, id).await
}

/// `by` declines engagement `id` with the neutral `reason` (one of
/// [`reasons::DECLINE_REASONS`]).
///
/// # Errors
///
/// As the transition: `Forbidden`, a 422 for another reason or state.
pub async fn decline(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<Engagement> {
    let engagement = load(state, id).await?;
    let _outcome = engagement
        .transition(DECLINE)
        .by(by)
        .reason(reason.trim())
        .input(&EngagementInput::on(today()))
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    load(state, id).await
}

/// The system declines the `onboarding` engagements of `client` whose CDD
/// file was rejected, with [`NOT_ACCEPTED`] and no automatic message (the
/// MLRO decides what the client is told).
///
/// # Errors
///
/// The database's and the transition's errors.
pub async fn decline_not_accepted(state: &AppState, client: PartyId) -> Result {
    for engagement in Engagement::of_client(&state.db, client).await? {
        if engagement.status != EngagementStatus::Onboarding {
            continue;
        }
        let input = EngagementInput {
            restricted: true,
            ..EngagementInput::on(today())
        };
        let _outcome = engagement
            .transition(AUTO_DECLINE)
            .by_system()
            .reason(NOT_ACCEPTED)
            .input(&input)
            .run(state)
            .await
            .map_err(TransitionError::into_error)?;
    }
    Ok(())
}

/// `by` suspends engagement `id` with the neutral `reason` (one of
/// [`reasons::SUSPEND_REASONS`]; never a restricted one).
///
/// # Errors
///
/// As the transition.
pub async fn suspend(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<Engagement> {
    let engagement = load(state, id).await?;
    let _outcome = engagement
        .transition(SUSPEND)
        .by(by)
        .reason(reason.trim())
        .input(&EngagementInput::on(today()))
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    load(state, id).await
}

/// `by` resumes engagement `id` while its client is not blocked.
///
/// # Errors
///
/// As the transition; a 422 while the client is on hold.
pub async fn resume(state: &AppState, by: &AuthUser, id: i64) -> Result<Engagement> {
    let engagement = load(state, id).await?;
    let blocked = directory(state)
        .status(&state.db, engagement.client())
        .await?
        == Some(PartyStatus::Blocked);
    let input = EngagementInput {
        client_blocked: blocked,
        ..EngagementInput::on(today())
    };
    let _outcome = engagement
        .transition(RESUME)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    load(state, id).await
}

/// `by` ends engagement `id` today with the neutral `reason` (one of
/// [`reasons::END_REASONS`]). After a CDD exit or rejection of the client
/// the stored reason is always *relationship ended* and nothing goes to the
/// client automatically (`ACC.ACL.14`).
///
/// # Errors
///
/// As the transition.
pub async fn end(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<Engagement> {
    let engagement = load(state, id).await?;
    let restricted = CddFile::current(&state.db, engagement.client())
        .await?
        .is_some_and(|file| file.ended_relationship());
    let input = EngagementInput {
        restricted,
        ..EngagementInput::on(today())
    };
    let _outcome = engagement
        .transition(END)
        .by(by)
        .reason(reason.trim())
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    load(state, id).await
}

/// The client `party` was blocked (`PartyBlocked`, its display reason
/// `display_reason`): each of its `active` engagements is suspended by the
/// system with the neutral reason *on hold*, and on one already `suspended`
/// the shown reason becomes *on hold* without a second transition.
///
/// The engagement never learns the block's reason: the event carries
/// [`reasons::ON_HOLD`] for a restricted one (sanctions, AML concern), and
/// then automatic messages to the client are off (`ACC.ACL.14`). Decision: a
/// display reason of *on hold* counts as restricted, so a block whose own
/// code is `on_hold` is treated the safe way. The held tasks and the paused
/// retainer come with BL-ACC-03.
///
/// # Errors
///
/// The database's and the transition's errors.
pub async fn suspend_for_block(state: &AppState, party: PartyId, display_reason: &str) -> Result {
    let restricted = display_reason == reasons::ON_HOLD;
    for engagement in Engagement::of_client(&state.db, party).await? {
        match engagement.status {
            EngagementStatus::Active => {
                let input = EngagementInput {
                    restricted,
                    ..EngagementInput::on(today())
                };
                let _outcome = engagement
                    .transition(AUTO_SUSPEND)
                    .by_system()
                    .reason(reasons::ON_HOLD)
                    .input(&input)
                    .run(state)
                    .await
                    .map_err(TransitionError::into_error)?;
            }
            EngagementStatus::Suspended => {
                let suppressed = engagement.client_messages_suppressed || restricted;
                let mut tx = state.db.begin().await?;
                set_engagement(
                    &mut tx,
                    engagement.id,
                    &[
                        ("reason_code", &Some(reasons::ON_HOLD.to_owned())),
                        ("client_messages_suppressed", &suppressed),
                    ],
                )
                .await?;
                tx.commit().await?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// The reasons a user may give for `transition` (for the forms).
pub fn user_reasons(transition: &str) -> &'static [&'static str] {
    match transition {
        DECLINE => &reasons::DECLINE_REASONS,
        SUSPEND => &reasons::SUSPEND_REASONS,
        END => &reasons::END_REASONS,
        _ => &[],
    }
}
