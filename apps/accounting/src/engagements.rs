//! The engagement services: one function per action, each gathering what
//! the machine's guards need and running the transition (04 §7.1
//! *Engagement*). The effects of `suspend` and `resume` on tasks, retainer,
//! request lists and notices run after the commit ([`crate::events`],
//! BL-ACC-03 to BL-ACC-05); so do those of `end` ([`crate::ending`],
//! BL-ACC-06).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use wl_kernel::PartyId;
use wl_kernel::approvals::{self, submit::REQUEST_KEY};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceSummary, invoicer};
use wl_kernel::parties::{PartyStatus, directory};
use wl_kernel::portal::portal_users;

use crate::activation;
use crate::client_notices::{self, NeutralTemplate};
use crate::db::{refused, set_engagement};
use crate::engagement::{Engagement, EngagementStatus, NewEngagement};
use crate::engagement_machine::{
    AUTO_DECLINE, AUTO_SUSPEND, DECLINE, END, EngagementInput, RESUME, SUSPEND,
};
use crate::permissions;
use crate::reasons::{self, NOT_ACCEPTED};
use crate::texts::english;
use crate::{deadline_notices, ending, exits, letters, request_lists, suspension};

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
/// A 422 for a blank title, a negative fee, a client that is not a known
/// party or a client that is *on hold* after a CDD exit or rejection
/// ([`exits::refuse_new_work`]); the database's errors.
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
    // No new work for a client whose relationship ended through CDD.
    exits::refuse_new_work(&state.db, new.client).await?;
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
    letters::mark_signed_by_staff(state, by, id, on).await?;
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
/// [`reasons::DECLINE_REASONS`]), telling the client nothing; see
/// [`decline_with`].
///
/// # Errors
///
/// As the transition: `Forbidden`, a 422 for another reason or state.
pub async fn decline(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<Engagement> {
    decline_with(state, by, id, reason, None, None).await
}

/// `by` declines engagement `id` with the neutral `reason` (one of
/// [`reasons::DECLINE_REASONS`]; *other* needs `note`), and tells the client
/// with the neutral `template` when one is chosen. After the commit the
/// client company's portal users lose their grants for it
/// ([`disable_client_users`]).
///
/// *CDD rejected* ([`reasons::NOT_ACCEPTED`]) is restricted: it shows as
/// **Declined** to everyone, and its one template ([`NeutralTemplate::Unable`])
/// never says why. Decision: the template must be the one of the reason's
/// group ([`NeutralTemplate::for_decline`]); `None` tells the client
/// nothing.
///
/// # Errors
///
/// As the transition: `Forbidden`, a 422 for another reason, state or
/// template.
pub async fn decline_with(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    note: Option<&str>,
    template: Option<NeutralTemplate>,
) -> Result<Engagement> {
    let engagement = load(state, id).await?;
    if let Some(template) = template
        && template != NeutralTemplate::for_decline(reason)
    {
        return Err(refused(english(
            "accounting.client_notices.wrong_template",
            &[],
        )));
    }
    let input = EngagementInput {
        restricted: reason.trim() == NOT_ACCEPTED,
        note: note.map(str::to_owned),
        ..EngagementInput::on(today())
    };
    let _outcome = engagement
        .transition(DECLINE)
        .by(by)
        .reason(reason.trim())
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    let declined = load(state, id).await?;
    if let Some(template) = template {
        client_notices::send_template(state, &declined, template, by.id).await?;
    }
    disable_client_users(state, declined.client()).await?;
    Ok(declined)
}

/// The client company's portal users lose their grants for it, unless the
/// client is still served by another engagement (`onboarding`, `active` or
/// `suspended`). Grants for other companies of the users are not touched.
/// Answers how many grants were ended.
///
/// # Errors
///
/// The database's and the portal's errors.
pub async fn disable_client_users(state: &AppState, client: PartyId) -> Result<usize> {
    let still_served = Engagement::of_client(&state.db, client)
        .await?
        .iter()
        .any(|e| {
            matches!(
                e.status,
                EngagementStatus::Onboarding
                    | EngagementStatus::Active
                    | EngagementStatus::Suspended
            )
        });
    if still_served {
        return Ok(0);
    }
    portal_users(state)
        .close_access(state, client, "engagement declined")
        .await
}

/// The system declines the `onboarding` engagements of `client` whose CDD
/// file was rejected, with [`NOT_ACCEPTED`] and no automatic message (the
/// MLRO decides what the client is told); the client's portal users are
/// disabled ([`disable_client_users`]).
///
/// # Errors
///
/// The database's and the transition's errors.
pub async fn decline_not_accepted(state: &AppState, client: PartyId) -> Result {
    let mut declined = false;
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
        declined = true;
    }
    if declined {
        disable_client_users(state, client).await?;
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

/// What a resume came to.
#[derive(Debug, Clone)]
pub struct Resumed {
    /// The engagement after the commit: `active`, or still `suspended`
    /// while the MLRO decides.
    pub engagement: Engagement,
    /// The approval request to the MLRO or deputy, when the suspension came
    /// from a block.
    pub approval_request: Option<i64>,
}

/// `by` resumes engagement `id` with `note` (what changed, or the client's
/// instruction) while its client is not blocked. Whether the reason is gone
/// is the manager's decision; for *non-payment* the page shows the client's
/// overdue invoices as information only. After a block (the reason is *on
/// hold*) the resume becomes an approval request to the MLRO or deputy and
/// the engagement stays `suspended` until their yes. After the commit the
/// tasks are released, the retainer resumes, the skipped request lists are
/// created and the deadline notice reminders cancelled ([`crate::events`]).
///
/// # Errors
///
/// As the transition: `Forbidden` without `engagements.manage`; a 422
/// without a note or while the client is on hold.
pub async fn resume(state: &AppState, by: &AuthUser, id: i64, note: &str) -> Result<Resumed> {
    let engagement = load(state, id).await?;
    let blocked = directory(state)
        .status(&state.db, engagement.client())
        .await?
        == Some(PartyStatus::Blocked);
    let after_block = engagement.reason_code.as_deref() == Some(reasons::ON_HOLD);
    let note = note.trim();
    let input = EngagementInput {
        client_blocked: blocked,
        note: (!note.is_empty()).then(|| note.to_owned()),
        after_block: i64::from(after_block),
        ..EngagementInput::on(today())
    };
    let mut tx = state.db.begin().await?;
    let outcome = engagement
        .transition(RESUME)
        .by(by)
        .input(&input)
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    tx.commit().await?;
    approvals::announce(state, &outcome).await?;
    let approval_request = match &outcome {
        Outcome::Pending(pending) => pending.data.get(REQUEST_KEY).and_then(Value::as_i64),
        Outcome::Done(_) => None,
    };
    outcome.emit(state).await?;
    Ok(Resumed {
        engagement: load(state, id).await?,
        approval_request,
    })
}

/// The overdue invoices of `client` and their total in fils, as the resume
/// of a suspension for non-payment shows them (information, not a guard).
///
/// # Errors
///
/// The invoicing module's errors.
pub async fn overdue_invoices(
    state: &AppState,
    client: PartyId,
) -> Result<(Vec<InvoiceSummary>, i64)> {
    let invoices = invoicer(state).overdue_of(&state.db, client).await?;
    let total = invoices.iter().map(|i| i.balance).sum();
    Ok((invoices, total))
}

/// The engagement resumed (by the manager, or on the MLRO's approval) on
/// `on`: its tasks are released (the manager gets those that went overdue
/// while held), its retainer resumes from the next cycle, the request lists
/// of the periods skipped while suspended are created with one notice to
/// the client, and the deadline notice reminders still pending are
/// cancelled.
///
/// # Errors
///
/// The services' errors.
pub async fn after_resume(state: &AppState, id: i64, on: NaiveDate) -> Result {
    let engagement = load(state, id).await?;
    suspension::release(state, &engagement, on).await?;
    request_lists::create_skipped(state, &engagement, on).await?;
    deadline_notices::cancel_pending(state, id, on).await?;
    Ok(())
}

/// `by` ends engagement `id` today (Asia/Dubai) with the neutral `reason`
/// (one of [`reasons::END_REASONS`]); see [`end_on`].
///
/// # Errors
///
/// As [`end_on`].
pub async fn end(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<Engagement> {
    end_on(state, by, id, reason, today()).await
}

/// `by` ends engagement `id` on `on` with the neutral `reason` (one of
/// [`reasons::END_REASONS`]). Today it ends at once, if the End checklist is
/// clear ([`ending::checklist`]: every open return filed or cancelled, the
/// unbilled time billed or written off, a final invoice or credit issued);
/// a later date is scheduled and the engagement stays as it is until the
/// daily run ends it ([`ending::run_scheduled`]); a past date is refused.
/// After a CDD exit or rejection of the client the stored reason is always
/// *relationship ended* and nothing goes to the client automatically
/// (`ACC.ACL.14`). After the commit `ACC.RUL.09` sends or prepares the
/// deadline notice, then the retainer is cancelled and the tasks are waived
/// or listed ([`ending::after_end`], [`crate::events`]).
///
/// # Errors
///
/// As the transition: `Forbidden` without `engagements.end`; a 422 listing
/// the open items of the checklist, or for a past end date.
pub async fn end_on(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    on: NaiveDate,
) -> Result<Engagement> {
    if on > today() {
        return ending::schedule(state, by, id, on, reason).await;
    }
    let engagement = load(state, id).await?;
    let input = ending::end_input(state, &engagement, on).await?;
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
/// code is `on_hold` is treated the safe way. The `auto_suspend` holds the
/// tasks and pauses the retainer with *on hold* after its commit
/// ([`crate::suspension`]); nothing goes to the client automatically, and
/// `ACC.RUL.09` gives the partner the deadline list only when the block's
/// reason is not restricted.
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
