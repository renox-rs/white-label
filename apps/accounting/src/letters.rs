//! The engagement letter sent for acceptance and the rule that chases it
//! (04 §4.1 "When it goes wrong", §7.2 `ACC.RUL.11`; BL-ACC-02).
//!
//! The letter is generated and signed in the portal (`DOC.GEN.01`,
//! `DOC.SIG.01`). wl-documents has no signature request yet, so the app
//! keeps the minimal record the returns use ([`crate::tax_return::ReturnSignature`]
//! is the same approach): an [`EngagementLetter`] per version sent, with the
//! sent date, the document hash, and, once a client approver signs in the
//! portal ([`sign`]), the signer, the time and the IP address. Decision: the
//! letter's text is a short summary of the engagement until wl-documents
//! generates the PDF, and the hash is of that summary
//! ([`crate::tax_return::document_hash`]).
//!
//! **`ACC.RUL.11`** ([`run`], daily at 09:00 Asia/Dubai): for each letter
//! sent and not signed of an `onboarding` engagement, counted in calendar
//! days from its sent date,
//!
//! - on day 7 and on day 14 the client approvers are reminded
//!   ([`REMIND_DAYS`]);
//! - on day 30 the manager gets one task to chase the client or `decline` the
//!   engagement with the reason *letter not signed* ([`CHASE_DAYS`]). The
//!   engagement is never declined automatically.
//!
//! A new version sent ([`send`]) supersedes the open one: its reminders and
//! task stop, and the count restarts from the new sent date. A signature, in
//! the portal or recorded by staff, stops the reminders and closes the open
//! task. A run that was missed catches up: the latest step due and not yet
//! done is sent, never two reminders on one day.

use std::sync::{Arc, Mutex, PoisonError};

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::compliance::{occurrence_key, schedule_subject};
use wl_kernel::documents::today;
use wl_kernel::notify::{
    self, GROUP_SIGNATURES, NotificationTypeDef, NotifyChannel, NotifyRequest, notifier,
};
use wl_kernel::portal::portal_users;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, TicketId, task_creator};

use crate::db::{refused, set_engagement};
use crate::deadline_notices::{APPROVER_ROLES, client_recipients};
use crate::engagement::{Engagement, EngagementStatus};
use crate::permissions::{self, MANAGER_ROLE};
use crate::tax_return::document_hash;
use crate::texts::english;

/// The rule.
pub const RULE: &str = "ACC.RUL.11";
/// The days after the sent date the client approver is reminded (N = 7, 14).
pub const REMIND_DAYS: [i64; 2] = [7, 14];
/// The days after the sent date the manager gets the task (M = 30).
pub const CHASE_DAYS: i64 = 30;
/// When the daily run happens, Asia/Dubai.
pub const RUN_AT: &str = "09:00";
/// The scheduled task and command of the daily run.
pub const TASK: &str = "accounting:letters";
/// The reminder to the client approvers.
pub const LETTER_REMINDER: &str = "accounting.letter_reminder";

/// An engagement letter sent for acceptance, in one version.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "accounting_engagement_letters", index(engagement_id))]
pub struct EngagementLetter {
    /// The primary key.
    pub id: i64,
    /// The engagement.
    #[model(references = "accounting_engagements")]
    pub engagement_id: i64,
    /// 1 for the first letter, then one more for each re-send.
    pub version: i64,
    /// The day it was sent for acceptance (Asia/Dubai): the reminders count
    /// from it.
    pub sent_on: NaiveDate,
    /// The user who sent it.
    pub sent_by: Option<i64>,
    /// The hash of the letter's text (`DOC.SIG.01`).
    pub document_hash: String,
    /// When it was signed.
    pub signed_at: Option<DateTime>,
    /// The day it was signed (Asia/Dubai).
    pub signed_on: Option<NaiveDate>,
    /// The portal user who signed, or the staff who recorded the signature.
    pub signer_user_id: Option<i64>,
    /// The signer's name as it was when signing.
    pub signer_name: Option<String>,
    /// The IP address of the signer's browser (portal signature only).
    pub ip_address: Option<String>,
    /// The day a newer version replaced it.
    pub superseded_on: Option<NaiveDate>,
    /// The day the day-7 reminder was sent.
    pub reminded_7_on: Option<NaiveDate>,
    /// The day the day-14 reminder was sent.
    pub reminded_14_on: Option<NaiveDate>,
    /// The manager's chase task, once opened.
    pub chase_task_id: Option<i64>,
    /// The day it was opened.
    pub chase_task_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl EngagementLetter {
    /// The letters of engagement `id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_engagement(db: &Db, id: i64) -> Result<Vec<EngagementLetter>> {
        EngagementLetter::query()
            .where_(EngagementLetter::ENGAGEMENT_ID.eq(id))
            .order_by(EngagementLetter::ID)
            .get(db)
            .await
    }

    /// The version in force for engagement `id`: the latest not replaced.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn current(db: &Db, id: i64) -> Result<Option<EngagementLetter>> {
        Ok(EngagementLetter::of_engagement(db, id)
            .await?
            .into_iter()
            .rev()
            .find(|letter| letter.superseded_on.is_none()))
    }

    /// Whether it was signed.
    pub fn is_signed(&self) -> bool {
        self.signed_at.is_some()
    }

    /// Whether it still waits for a signature: not signed, not replaced.
    pub fn is_open(&self) -> bool {
        self.signed_at.is_none() && self.superseded_on.is_none()
    }
}

/// The key of the manager's task for `letter`.
pub fn chase_key(letter: &EngagementLetter) -> String {
    format!(
        "accounting.letter_chase:{}:{}",
        letter.engagement_id, letter.id
    )
}

/// The text the client signs, and the signature's hash is of.
///
/// Decision: the letter PDF comes with wl-documents' templates; until then
/// the text is this summary of the engagement and the version.
pub fn letter_text(engagement: &Engagement, version: i64) -> String {
    format!(
        "{}|{}|{}|{}|v{}",
        engagement.number(),
        engagement.title,
        engagement.fee_basis.as_str(),
        engagement.retainer_fee,
        version
    )
}

/// `Forbidden` unless `user` holds `permission`.
fn require(user: &AuthUser, permission: &str) -> Result {
    if user.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// `by` sends the engagement letter of engagement `id` for acceptance in the
/// portal on `on`. A letter sent before and not signed is replaced: its
/// reminders and its task stop and the count restarts from `on`.
///
/// # Errors
///
/// `Forbidden` without `engagements.manage`; a 422 unless the engagement is
/// `onboarding` and its letter is not signed yet.
pub async fn send(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    on: NaiveDate,
) -> Result<EngagementLetter> {
    require(by, permissions::ENGAGEMENTS_MANAGE)?;
    let engagement = Engagement::find_or_404(&state.db, id).await?;
    if engagement.status != EngagementStatus::Onboarding {
        return Err(refused(english("accounting.problems.not_onboarding", &[])));
    }
    if engagement.letter_signed {
        return Err(refused(english("accounting.letters.problem.signed", &[])));
    }
    let mut version = 1;
    for mut old in EngagementLetter::of_engagement(&state.db, id).await? {
        version = version.max(old.version + 1);
        if old.is_open() {
            old.superseded_on = Some(on);
            old.save(&state.db).await?;
            close_chase(
                state,
                &old,
                &english("accounting.letters.chase_closed_resent", &[]),
            )
            .await?;
        }
    }
    let letter = EngagementLetter {
        engagement_id: id,
        version,
        sent_on: on,
        sent_by: Some(by.id),
        document_hash: document_hash(&letter_text(&engagement, version)),
        ..EngagementLetter::default()
    };
    EngagementLetter::create(&state.db, letter).await
}

/// The portal user `by` signs the letter of engagement `id`: a client
/// approver (a portal grant with the role `owner` or `approver`) of the
/// client company. The signature is kept with the signer, the time, the IP
/// address and the document hash (`DOC.SIG.01`); the engagement's letter is
/// signed and the open chase task is closed.
///
/// # Errors
///
/// `Forbidden` unless `by` is a client approver of the client; a 422 unless
/// the engagement is `onboarding` with a letter waiting for a signature.
pub async fn sign(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    ip: Option<&str>,
) -> Result<EngagementLetter> {
    let engagement = Engagement::find_or_404(&state.db, id).await?;
    let acting = portal_users(state)
        .acting_for(&state.db, by.id, engagement.client(), today())
        .await?;
    if !acting.is_some_and(|user| user.has_any_role(&APPROVER_ROLES)) {
        return Err(Error::Forbidden);
    }
    if engagement.status != EngagementStatus::Onboarding {
        return Err(refused(english("accounting.problems.not_onboarding", &[])));
    }
    let Some(mut letter) = EngagementLetter::current(&state.db, id)
        .await?
        .filter(EngagementLetter::is_open)
    else {
        return Err(refused(english(
            "accounting.letters.problem.no_letter",
            &[],
        )));
    };
    letter.signed_at = Some(now());
    letter.signed_on = Some(today());
    letter.signer_user_id = Some(by.id);
    letter.signer_name = Some(by.name.clone());
    letter.ip_address = ip.map(str::to_owned);
    letter.save(&state.db).await?;
    let mut tx = state.db.begin().await?;
    set_engagement(
        &mut tx,
        id,
        &[
            ("letter_signed", &true),
            ("letter_signed_on", &Some(today())),
        ],
    )
    .await?;
    tx.commit().await?;
    close_chase(
        state,
        &letter,
        &english("accounting.letters.chase_closed_signed", &[]),
    )
    .await?;
    Ok(letter)
}

/// Staff recorded that the client signed the letter of engagement `id` on
/// `on` (the paper copy): the open version, if one was sent, counts as
/// signed, so no reminder follows and the open task is closed.
///
/// # Errors
///
/// The database's and the task service's errors.
pub async fn mark_signed_by_staff(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    on: NaiveDate,
) -> Result {
    let Some(mut letter) = EngagementLetter::current(&state.db, id)
        .await?
        .filter(EngagementLetter::is_open)
    else {
        return Ok(());
    };
    letter.signed_at = Some(now());
    letter.signed_on = Some(on);
    letter.signer_user_id = Some(by.id);
    letter.signer_name = Some(by.name.clone());
    letter.save(&state.db).await?;
    close_chase(
        state,
        &letter,
        &english("accounting.letters.chase_closed_signed", &[]),
    )
    .await
}

/// Completes the chase task of `letter` if one is open.
async fn close_chase(state: &AppState, letter: &EngagementLetter, note: &str) -> Result {
    if let Some(task) = letter.chase_task_id {
        task_creator(state)
            .complete(state, TicketId(task), note)
            .await?;
    }
    Ok(())
}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LetterRun {
    /// The letters whose client approvers were reminded, with the day count
    /// (7 or 14).
    pub reminded: Vec<(i64, i64)>,
    /// The letters whose manager got the task.
    pub chased: Vec<i64>,
}

/// `ACC.RUL.11` on `on`, as a rule run: once per day.
///
/// # Errors
///
/// The database's errors (a failing run is retried by the rule run).
pub async fn run(state: &AppState, on: NaiveDate) -> Result<LetterRun> {
    let spec = RuleRunSpec::new(RULE, schedule_subject(), occurrence_key(RULE, on));
    let seen = Arc::new(Mutex::new(LetterRun::default()));
    let out = Arc::clone(&seen);
    run_rule(state, spec, move |state: AppState| async move {
        let report = run_now(&state, on).await?;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = report;
        Ok(())
    })
    .await?;
    let report = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(report)
}

/// The reminders and tasks due on `on`.
async fn run_now(state: &AppState, on: NaiveDate) -> Result<LetterRun> {
    let mut report = LetterRun::default();
    let letters = EngagementLetter::query()
        .where_(EngagementLetter::SIGNED_AT.is_null())
        .where_(EngagementLetter::SUPERSEDED_ON.is_null())
        .order_by(EngagementLetter::ID)
        .get(&state.db)
        .await?;
    for mut letter in letters {
        let Some(engagement) = Engagement::find(&state.db, letter.engagement_id).await? else {
            continue;
        };
        if engagement.status != EngagementStatus::Onboarding {
            continue;
        }
        let days = (on - letter.sent_on).num_days();
        let step = if days >= REMIND_DAYS[1] && letter.reminded_14_on.is_none() {
            Some(REMIND_DAYS[1])
        } else if days >= REMIND_DAYS[0] && days < REMIND_DAYS[1] && letter.reminded_7_on.is_none()
        {
            Some(REMIND_DAYS[0])
        } else {
            None
        };
        if let Some(step) = step
            && engagement.may_message_client()
        {
            remind(state, &engagement, &letter, on).await?;
            if step == REMIND_DAYS[1] {
                letter.reminded_14_on = Some(on);
            } else {
                letter.reminded_7_on = Some(on);
            }
            letter.save(&state.db).await?;
            report.reminded.push((letter.id, step));
        }
        if days >= CHASE_DAYS && letter.chase_task_id.is_none() {
            let opened = open_chase(state, &engagement, &letter, days).await?;
            letter.chase_task_id = opened;
            letter.chase_task_on = Some(on);
            letter.save(&state.db).await?;
            report.chased.push(letter.id);
        }
    }
    Ok(report)
}

/// Reminds the client approvers of `engagement` to sign `letter`.
async fn remind(
    state: &AppState,
    engagement: &Engagement,
    letter: &EngagementLetter,
    on: NaiveDate,
) -> Result {
    let mut request = NotifyRequest::new(LETTER_REMINDER, engagement.subject())
        .subject_label(&engagement.number())
        .data(json!({
            "number": engagement.number(),
            "sent_on": letter.sent_on.to_string(),
            "version": letter.version,
        }));
    for recipient in client_recipients(state, engagement.client(), on).await? {
        request = request.to(recipient);
    }
    notifier(state).notify(state, request).await?;
    Ok(())
}

/// Opens the manager's task to chase the client or decline; answers its id
/// (`None` when ticketing is off).
async fn open_chase(
    state: &AppState,
    engagement: &Engagement,
    letter: &EngagementLetter,
    days: i64,
) -> Result<Option<i64>> {
    let number = engagement.number();
    let title = english("accounting.letters.chase_title", &[("number", &number)]);
    let description = english(
        "accounting.letters.chase_description",
        &[
            ("number", &number),
            ("version", &letter.version),
            ("sent", &letter.sent_on),
            ("days", &days),
            (
                "link",
                &format!("/accounting/engagements/{}", engagement.id),
            ),
        ],
    );
    let key = chase_key(letter);
    let creator = task_creator(state);
    if let Some(found) = creator.find_open(state, &key).await? {
        return Ok(Some(found.id.0));
    }
    let opened = match engagement.manager_user_id {
        Some(manager) => {
            let ticket = OpenTicket::new(engagement.subject(), TASK_CATEGORY, &title)
                .description(&description)
                .assign_to(Assignee::user(manager))
                .source_key(&key);
            creator.open(state, ticket).await?
        }
        None => {
            let task = RoleTask::new(engagement.subject(), MANAGER_ROLE, &title, &key)
                .description(&description);
            role_tasks::open(state, task).await?
        }
    };
    Ok(opened.map(|t| t.id.0))
}

/// The notification type of the reminder.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let subject = english("accounting.letters.reminder_subject", &[]);
    let body = english("accounting.letters.reminder_body", &[]);
    vec![
        // 04 §9: engagement letters are in the *portal and signatures* group;
        // a reminder that gives up is a task for the engagement manager.
        NotificationTypeDef::new(LETTER_REMINDER, "accounting")
            .group(GROUP_SIGNATURES)
            .staff_role(MANAGER_ROLE)
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    ]
}

/// Registers the notification type and the daily run.
pub(crate) fn register(app: &mut Registry) {
    for def in notification_types() {
        notify::register_type(def);
    }
    app.schedule()
        .daily_at(RUN_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Remind the client approvers of unsigned engagement letters and open the manager's task (ACC.RUL.11)",
        |_args, state: AppState| async move {
            let report = run(&state, today()).await?;
            println!(
                "{} reminders sent, {} chase tasks opened.",
                report.reminded.len(),
                report.chased.len()
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_are_the_spec_days() {
        assert_eq!(REMIND_DAYS, [7, 14]);
        assert_eq!(CHASE_DAYS, 30);
    }

    #[test]
    fn the_text_names_the_engagement_and_the_version() {
        let engagement = Engagement {
            id: 7,
            title: "Bookkeeping".into(),
            retainer_fee: 500_000,
            ..Engagement::default()
        };
        assert_eq!(
            letter_text(&engagement, 2),
            "ENG-0007|Bookkeeping|retainer|500000|v2"
        );
        assert_ne!(
            document_hash(&letter_text(&engagement, 1)),
            document_hash(&letter_text(&engagement, 2))
        );
    }
}
