//! The AML case services and `ACC.RUL.12` (04 §4.6 step 4, §7.1 *AML case*,
//! §7.2 `ACC.RUL.12`; BL-ACC-10).
//!
//! - [`raise`]: a member of staff (`aml.raise`) raises a concern about a
//!   client; the MLRO and deputy are notified; the person sees nothing of the
//!   case after.
//! - [`on_match_confirmed`] (`ACC.RUL.12`, compliance-critical, a rule run
//!   per screening, owned by the partner role): on `ScreeningMatchConfirmed`
//!   for a client company or its UBO, director or signatory, the client
//!   company is blocked with the restricted reason `sanctions` when
//!   wl-parties did not block it already (and that is written on the case);
//!   an AML case is opened, linked to the screening and to the freeze and
//!   report tasks wl-parties opened, or the match is added to the client
//!   company's open sanctions case (one per company); the MLRO and deputy
//!   are notified. The block suspends the engagement with *on hold* and
//!   pauses the retainer by [`crate::engagements::suspend_for_block`]; no
//!   message goes to the client.
//! - [`review`], [`close`] and [`report`]: the MLRO's transitions. `close`
//!   is refused while a linked freeze or report task is open and never
//!   unblocks the client: an `aml_concern` block is the MLRO's decision, a
//!   `sanctions` block needs wl-parties' evidence.
//!
//! Every function that shows or changes a case answers 404 (`NotFound`),
//! never 403, to a user without `aml.manage`, so a case's existence is not
//! revealed (`ACC.ACL.05`).

use renox::auth::permissions::users_with_permission;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::PartyId;
use wl_kernel::automation::{RuleRunSpec, RunOutcome, run_rule};
use wl_kernel::block_guard::{SystemBlock, guard as block_guard};
use wl_kernel::calendar::calendar;
use wl_kernel::compliance::CriticalRule;
use wl_kernel::documents::today;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest, notifier};
use wl_kernel::parties::{ScreeningMatchConfirmed, directory};
use wl_kernel::screening_guard::{ConfirmedMatch, MatchTaskKind, screening_guard};

use crate::aml::{AmlCase, AmlKind, AmlMatch, AmlStatus};
use crate::aml_machine::{AmlInput, CLOSE, OPEN, REPORT, REVIEW};
use crate::cdd::{CddFile, CddParty, ScreenedRole};
use crate::db::refused;
use crate::engagement::Engagement;
use crate::permissions::{self, PARTNER_ROLE};
use crate::texts::english;

/// The rule.
pub const RULE: &str = "ACC.RUL.12";
/// The reason code of the block the rule places (wl-parties).
pub const SANCTIONS: &str = "sanctions";
/// The notification to the MLRO and deputy: a case needs them.
pub const CASE_NOTICE: &str = "accounting.aml_case";

/// `NotFound` unless `user` holds `aml.manage` (ACC.ACL.05).
fn require_mlro(user: &AuthUser) -> Result {
    if user.has_permission(permissions::AML_MANAGE) {
        Ok(())
    } else {
        Err(Error::NotFound)
    }
}

/// Case `id` for `user`: 404 unless they hold `aml.manage`.
///
/// # Errors
///
/// `NotFound` for anybody else, and for an unknown case.
pub async fn visible(state: &AppState, user: &AuthUser, id: i64) -> Result<AmlCase> {
    require_mlro(user)?;
    AmlCase::find_or_404(&state.db, id).await
}

/// Inserts `case` in `open` with the history row [`OPEN`], and its
/// `matches`.
async fn insert_case(
    state: &AppState,
    mut case: AmlCase,
    matches: Vec<AmlMatch>,
) -> Result<AmlCase> {
    let mut tx = state.db.begin().await?;
    case.insert_in_state(&mut tx, AmlStatus::Open, OPEN).await?;
    for mut row in matches {
        row.case_id = case.id;
        row.insert(&mut tx).await?;
    }
    tx.commit().await?;
    AmlCase::find_or_404(&state.db, case.id).await
}

/// `by` (`aml.raise`, any member of staff) raises a concern about the client
/// company `client` with the `facts`. The MLRO and deputy are notified. The
/// caller learns nothing more: the case is not theirs to see.
///
/// # Errors
///
/// `Forbidden` without `aml.raise`; a 422 without facts or for an unknown
/// client; the database's and the notifier's errors.
pub async fn raise(
    state: &AppState,
    by: &AuthUser,
    client: PartyId,
    facts: &str,
) -> Result<AmlCase> {
    if !by.has_permission(permissions::AML_RAISE) {
        return Err(Error::Forbidden);
    }
    if facts.trim().is_empty() {
        return Err(refused(english("accounting.aml.problem.facts", &[])));
    }
    if directory(state).find(&state.db, client).await?.is_none() {
        return Err(refused(english("accounting.problems.client_unknown", &[])));
    }
    let case = insert_case(
        state,
        AmlCase {
            client_party_id: client.0,
            kind: AmlKind::Concern,
            facts: facts.trim().to_owned(),
            raised_by: Some(by.id),
            ..AmlCase::default()
        },
        Vec::new(),
    )
    .await?;
    notify_mlro(state, &case).await?;
    Ok(case)
}

/// Tells every holder of `aml.manage` that `case` needs them.
async fn notify_mlro(state: &AppState, case: &AmlCase) -> Result {
    for user in users_with_permission(&state.db, permissions::AML_MANAGE).await? {
        let request = NotifyRequest::new(CASE_NOTICE, case.subject())
            .to_user(user.id)
            .subject_label(&case.number())
            .data(json!({
                "number": case.number(),
                "link": format!("/accounting/aml-cases/{}", case.id),
            }));
        notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// Runs `transition` on case `id` as `by` with `input`.
async fn run(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    transition: &str,
    input: &AmlInput,
) -> Result<AmlCase> {
    let case = visible(state, by, id).await?;
    let _outcome = case
        .transition(transition)
        .by(by)
        .input(input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    AmlCase::find_or_404(&state.db, id).await
}

/// `by` (the MLRO) takes case `id`: `open` → `under_review`.
///
/// # Errors
///
/// `NotFound` without `aml.manage`; the transition's errors.
pub async fn review(state: &AppState, by: &AuthUser, id: i64) -> Result<AmlCase> {
    run(state, by, id, REVIEW, &AmlInput::default()).await
}

/// The freeze and report tasks of `case` that are still open, one line each
/// (the check of `close` for a sanctions case).
///
/// # Errors
///
/// The database's errors.
pub async fn open_tasks(state: &AppState, case: &AmlCase) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    if case.kind != AmlKind::Sanctions {
        return Ok(lines);
    }
    let guard = screening_guard(state);
    for row in case.matches(&state.db).await? {
        let Some(confirmed) = guard.confirmed_match(&state.db, row.screening_id).await? else {
            continue;
        };
        for task in confirmed.tasks.iter().filter(|task| !task.done) {
            let key = match task.kind {
                MatchTaskKind::Freeze => "accounting.aml.problem.freeze_open",
                MatchTaskKind::Report => "accounting.aml.problem.report_open",
            };
            lines.push(english(key, &[]));
        }
    }
    Ok(lines)
}

/// `by` (the MLRO) closes case `id` with `reason`: reviewed, nothing to
/// report. For a sanctions case every linked freeze and report task must be
/// done. The client is not unblocked.
///
/// # Errors
///
/// `NotFound` without `aml.manage`; a 422 without a reason or while a linked
/// task is open; the transition's errors.
pub async fn close(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<AmlCase> {
    let case = visible(state, by, id).await?;
    let input = AmlInput {
        reason: Some(reason.to_owned()),
        problems: open_tasks(state, &case).await?,
        on: Some(today()),
        ..AmlInput::default()
    };
    run(state, by, id, CLOSE, &input).await
}

/// `by` (the MLRO) records that the report of case `id` was filed on goAML
/// with `reference` on `date`: `under_review` → `reported`.
///
/// # Errors
///
/// `NotFound` without `aml.manage`; a 422 without a reference and date; the
/// transition's errors.
pub async fn report(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reference: &str,
    date: Option<renox::chrono::NaiveDate>,
) -> Result<AmlCase> {
    let input = AmlInput {
        goaml_reference: Some(reference.to_owned()),
        goaml_date: date,
        on: Some(today()),
        ..AmlInput::default()
    };
    run(state, by, id, REPORT, &input).await
}

/// The client companies a confirmed match of `party` is about, with what
/// `party` is to each: the party itself when it is a client (it has an
/// engagement or a CDD file), and each client whose current CDD file lists
/// it as a UBO, director or signatory.
///
/// # Errors
///
/// The database's errors.
pub async fn clients_of(db: &Db, party: PartyId) -> Result<Vec<(PartyId, ScreenedRole)>> {
    let mut found = Vec::new();
    let is_client = Engagement::query()
        .where_(Engagement::CLIENT_PARTY_ID.eq(party.0))
        .exists(db)
        .await?
        || CddFile::query()
            .where_(CddFile::CLIENT_PARTY_ID.eq(party.0))
            .exists(db)
            .await?;
    if is_client {
        found.push((party, ScreenedRole::Client));
    }
    for row in CddParty::query()
        .where_(CddParty::PARTY_ID.eq(party.0))
        .order_by(CddParty::ID)
        .get(db)
        .await?
    {
        if row.role == ScreenedRole::Client {
            continue;
        }
        let Some(file) = CddFile::find(db, row.cdd_file_id).await? else {
            continue;
        };
        let current = CddFile::current(db, file.client()).await?;
        let entry = (file.client(), row.role);
        if current.is_some_and(|c| c.id == file.id) && !found.contains(&entry) {
            found.push(entry);
        }
    }
    Ok(found)
}

/// `ACC.RUL.12` for the screening of `event`, as a compliance-critical rule
/// run (once per screening; a failure is retried and alerts the partner role
/// and the company admin).
///
/// # Errors
///
/// The database's errors (a failing action is retried by the rule run).
pub async fn on_match_confirmed(
    state: &AppState,
    event: &ScreeningMatchConfirmed,
) -> Result<RunOutcome> {
    let party = event.party;
    let screening = event.screening.0;
    let spec = RuleRunSpec::new(
        RULE,
        party.subject(),
        format!("{RULE}:screening:{screening}"),
    )
    .compliance_critical();
    run_rule(state, spec, move |state: AppState| async move {
        rule_now(&state, party, screening).await
    })
    .await
}

/// The rule's work.
async fn rule_now(state: &AppState, party: PartyId, screening: i64) -> Result {
    let clients = clients_of(&state.db, party).await?;
    if clients.is_empty() {
        return Ok(());
    }
    let confirmed = screening_guard(state)
        .confirmed_match(&state.db, screening)
        .await?;
    for (client, role) in clients {
        add_match(state, client, role, party, screening, confirmed.as_ref()).await?;
    }
    Ok(())
}

/// The name of `party` in English, else `#id`.
async fn name_of(state: &AppState, party: PartyId) -> String {
    directory(state)
        .display_name(&state.db, party, "en")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", party.0))
}

/// Checks the client company is blocked, and opens the case for the match
/// or adds it to the open one.
async fn add_match(
    state: &AppState,
    client: PartyId,
    role: ScreenedRole,
    party: PartyId,
    screening: i64,
    confirmed: Option<&ConfirmedMatch>,
) -> Result {
    let already = AmlMatch::query()
        .where_(AmlMatch::SCREENING_ID.eq(screening))
        .where_(AmlMatch::CLIENT_PARTY_ID.eq(client.0))
        .exists(&state.db)
        .await?;
    if already {
        return Ok(());
    }
    // The client company is blocked too: by wl-parties' related-person
    // block, or here when that did not (the setting is off, or the person is
    // linked to the client only through the CDD file).
    let note = english("accounting.aml.block_reason_note", &[]);
    let blocked_now = block_guard(state)
        .block_by_system(
            state,
            client,
            SystemBlock::new(SANCTIONS, &note).screening(screening),
        )
        .await?;
    let block_note = blocked_now.then(|| english("accounting.aml.block_note", &[]));

    let party_name = name_of(state, party).await;
    let client_name = name_of(state, client).await;
    let role_text = english(&format!("accounting.aml.role.{}", role.as_str()), &[]);
    let list = confirmed
        .and_then(|c| c.list_name.clone())
        .unwrap_or_default();
    let entry = confirmed
        .and_then(|c| c.list_entry.clone())
        .unwrap_or_default();
    let flagged = confirmed
        .map(|c| {
            calendar(state)
                .local(c.flagged_at)
                .format("%d %b %Y %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let line = english(
        "accounting.aml.facts_sanctions",
        &[
            ("party", &party_name),
            ("role", &role_text),
            ("client", &client_name),
            ("list", &list),
            ("entry", &entry),
            ("flagged", &flagged),
        ],
    );
    let task = |kind| confirmed.and_then(|c| c.tasks.iter().find(|t| t.kind == kind).map(|t| t.id));
    let row = AmlMatch {
        client_party_id: client.0,
        screening_id: screening,
        matched_party_id: party.0,
        role,
        list_name: confirmed.and_then(|c| c.list_name.clone()),
        list_entry: confirmed.and_then(|c| c.list_entry.clone()),
        flagged_at: confirmed.map(|c| c.flagged_at),
        freeze_task_id: task(MatchTaskKind::Freeze),
        report_task_id: task(MatchTaskKind::Report),
        ..AmlMatch::default()
    };
    let case = match AmlCase::open_sanctions_of(&state.db, client).await? {
        Some(mut case) => {
            let mut row = row;
            row.case_id = case.id;
            row.insert(&state.db).await?;
            case.facts = format!("{}\n{line}", case.facts);
            if case.block_note.is_none() {
                case.block_note = block_note;
            }
            case.save(&state.db).await?;
            case
        }
        None => {
            insert_case(
                state,
                AmlCase {
                    client_party_id: client.0,
                    kind: AmlKind::Sanctions,
                    facts: line,
                    block_note,
                    ..AmlCase::default()
                },
                vec![row],
            )
            .await?
        }
    };
    notify_mlro(state, &case).await
}

/// The notification type of the MLRO's notice: it names the case only.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let subject = english("accounting.aml.notice_subject", &[]);
    let body = english("accounting.aml.notice_body", &[]);
    vec![
        NotificationTypeDef::new(CASE_NOTICE, "accounting")
            .group("compliance")
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    ]
}

/// Registers the notification type and marks `ACC.RUL.12` compliance-critical
/// (owner: the partner role, with the company admin).
pub(crate) fn register() {
    for def in notification_types() {
        notify::register_type(def);
    }
    CriticalRule::new(RULE).owner_role(PARTNER_ROLE).register();
}
