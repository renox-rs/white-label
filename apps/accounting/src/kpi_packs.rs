//! The services of the KPI pack: importing a trial balance (`ACC.RUL.06`),
//! mapping its accounts, the commentary, `publish`, `withdraw` and *copy*
//! (04 §4.2 "When it goes wrong", §7.1 *KPI pack*, §7.2 `ACC.RUL.06`;
//! BL-ACC-20, BL-ACC-21).
//!
//! **Import** ([`import`]): a trial balance imported for a client and a
//! period creates the first draft, or, when the period has a published pack
//! and no draft, a new draft version that carries the published pack's
//! commentary; while a draft exists it is recalculated in place, so a period
//! never has two drafts. The published pack is never changed: the portal
//! shows it as *being revised* until the new version is published. The
//! engagement manager is told to add the commentary; the client is told
//! nothing at the import, only at the `publish`.
//!
//! **After a transition** ([`after`], a listener): `publish` (which
//! superseded the earlier published version in its own transaction) tells
//! the client users;
//! `withdraw` tells them the pack was withdrawn (for a wrong client without
//! naming any company and without the period) and, for a wrong client, opens
//! a privacy incident task for the company admin listing who of the
//! receiving company opened the pack (the audit trail of
//! [`record_view`]).

use std::collections::BTreeMap;

use renox::audit::{self, Entry};
use renox::chrono::{Datelike, Duration, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use wl_kernel::documents::{dubai_text, today};
use wl_kernel::notify::{
    self, GROUP_PORTAL, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRecipient,
    NotifyRequest, notifier,
};
use wl_kernel::parties::directory;
use wl_kernel::portal::portal_users;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::{COMPANY_ADMIN_ROLE, PartyId};

use crate::db::refused;
use crate::engagement::{Engagement, EngagementStatus};
use crate::kpi::{CATEGORIES, KpiFigure, KpiMapping, KpiPack, KpiStatus};
use crate::kpi_machine::{KpiInput, PUBLISH, WITHDRAW, WRONG_CLIENT};
use crate::permissions::{self, MANAGER_ROLE};
use crate::texts::english;

/// The rule.
pub const RULE: &str = "ACC.RUL.06";
/// Whether the template requires commentary before a pack is published.
/// Decision: it does; a deployment that does not changes this value.
pub const COMMENTARY_REQUIRED: bool = true;
/// The audit action of a client user opening a pack in the portal.
pub const VIEWED: &str = "kpi_pack.viewed";
/// The audit action of the privacy incident task being opened for a pack.
pub const INCIDENT_OPENED: &str = "kpi_pack.incident_opened";
/// When the daily catch-up of missing incident tasks runs, Asia/Dubai.
pub const CATCH_UP_AT: &str = "07:30";
/// The scheduled task of the catch-up.
pub const CATCH_UP_TASK: &str = "accounting:kpi_incident_catch_up";
/// The engagement manager is told a trial balance was imported.
pub const IMPORTED_NOTICE: &str = "accounting.kpi_imported";
/// The client users are told a pack was published.
pub const PUBLISHED_NOTICE: &str = "accounting.kpi_published";
/// The client users are told a pack was withdrawn.
pub const WITHDRAWN_NOTICE: &str = "accounting.kpi_withdrawn";

/// The key of the manager's task to add the commentary when no manager is
/// named on the client's engagement.
pub fn commentary_key(pack_id: i64) -> String {
    format!("accounting.kpi_commentary:{pack_id}")
}

/// The key of the privacy incident task of a pack.
pub fn incident_key(pack_id: i64) -> String {
    format!("accounting.kpi_privacy_incident:{pack_id}")
}

/// What an import came to.
#[derive(Debug, Clone)]
pub struct Imported {
    /// The draft that now holds the figures.
    pub pack: KpiPack,
    /// Whether a new version was created (false: an existing draft was
    /// recalculated in place).
    pub created: bool,
}

/// One line of a trial balance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The account's code.
    pub code: String,
    /// The account's name.
    pub name: String,
    /// Debit minus credit, in fils.
    pub amount: i64,
}

/// Dirhams typed in as fils: `1,234.50`, `-3`, `(12.5)`; empty is 0.
pub fn parse_fils(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, body) = match text.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        Some(inner) => (true, inner),
        None => match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        },
    };
    let clean: String = body
        .chars()
        .filter(|c| *c != ',' && !c.is_whitespace())
        .collect();
    if clean.is_empty() {
        return Some(0);
    }
    let (whole, part) = clean.split_once('.').unwrap_or((clean.as_str(), ""));
    if part.len() > 2
        || whole.is_empty() && part.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let part: i64 = if part.is_empty() {
        0
    } else {
        format!("{part:0<2}").parse().ok()?
    };
    let fils = whole.checked_mul(100)?.checked_add(part)?;
    Some(if negative { -fils } else { fils })
}

/// The last day of the month `text` names (`2026-10` or a date in it).
pub fn parse_period(text: &str) -> Option<NaiveDate> {
    let text = text.trim();
    let first = if text.len() == 7 {
        NaiveDate::parse_from_str(&format!("{text}-01"), "%Y-%m-%d").ok()?
    } else {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?
    };
    let (year, month) = if first.month() == 12 {
        (first.year() + 1, 1)
    } else {
        (first.year(), first.month() + 1)
    };
    Some(NaiveDate::from_ymd_opt(year, month, 1)? - Duration::days(1))
}

/// The cells of one CSV line; `delimiter` outside quotes splits.
fn split_line(line: &str, delimiter: char) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if quoted && chars.peek() == Some(&'"') {
                cell.push('"');
                chars.next();
            } else {
                quoted = !quoted;
            }
        } else if c == delimiter && !quoted {
            cells.push(cell.trim().to_owned());
            cell.clear();
        } else {
            cell.push(c);
        }
    }
    cells.push(cell.trim().to_owned());
    cells
}

/// Reads the pasted trial balance: `code, name, debit, credit` or `code,
/// name, balance` per line (comma, semicolon or tab); a first line whose
/// amount is not a number is the header.
///
/// # Errors
///
/// A 422 naming every rejected row, or saying the text has no rows.
pub fn parse_trial_balance(text: &str) -> Result<Vec<Line>> {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let delimiter = [',', ';', '\t']
        .into_iter()
        .rev()
        .max_by_key(|d| first.matches(*d).count())
        .unwrap_or(',');
    let mut lines: Vec<Line> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    let mut seen_first = false;
    for (index, line) in text.lines().enumerate() {
        let row = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let cells = split_line(line, delimiter);
        let cell = |n: usize| cells.get(n).map(String::as_str).unwrap_or("");
        let header = !seen_first && cells.len() >= 3 && parse_fils(cell(2)).is_none();
        seen_first = true;
        if header {
            continue;
        }
        if cells.len() < 3 || cell(0).is_empty() {
            problems.push(english("accounting.kpi.import.bad_row", &[("row", &row)]));
            continue;
        }
        let amount = if cells.len() >= 4 {
            match (parse_fils(cell(2)), parse_fils(cell(3))) {
                (Some(debit), Some(credit)) => Some(debit - credit),
                _ => None,
            }
        } else {
            parse_fils(cell(2))
        };
        let Some(amount) = amount else {
            problems.push(english(
                "accounting.kpi.import.bad_amount",
                &[("row", &row), ("account", &cell(0))],
            ));
            continue;
        };
        if lines.iter().any(|l| l.code == cell(0)) {
            problems.push(english(
                "accounting.kpi.import.repeated",
                &[("row", &row), ("account", &cell(0))],
            ));
            continue;
        }
        lines.push(Line {
            code: cell(0).to_owned(),
            name: cell(1).to_owned(),
            amount,
        });
    }
    if !problems.is_empty() {
        return Err(refused(problems.join("\n")));
    }
    if lines.is_empty() {
        return Err(refused(english("accounting.kpi.import.empty", &[])));
    }
    Ok(lines)
}

/// `Forbidden` unless `user` holds `permission`.
fn require(user: &AuthUser, permission: &str) -> Result {
    if user.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// `by` imports the trial balance `csv` for `client` and `period_end`
/// (`ACC.RUL.06`): the first draft, a new draft version when only a published
/// pack exists, or the existing draft recalculated in place. Each account
/// takes the client's saved KPI category when it has one; the engagement
/// manager is told to add the commentary.
///
/// # Errors
///
/// `Forbidden` without `kpi.manage`; a 422 for an unknown client or a trial
/// balance that cannot be read (the rows are named).
pub async fn import(
    state: &AppState,
    by: &AuthUser,
    client: PartyId,
    period_end: NaiveDate,
    csv: &str,
) -> Result<Imported> {
    require(by, permissions::KPI_MANAGE)?;
    if directory(state).find(&state.db, client).await?.is_none() {
        return Err(refused(english("accounting.kpi.problem.client", &[])));
    }
    let lines = parse_trial_balance(csv)?;
    let mappings: BTreeMap<String, String> = KpiMapping::of_client(&state.db, client)
        .await?
        .into_iter()
        .map(|m| (m.account_code, m.category))
        .collect();
    let packs = KpiPack::of_period(&state.db, client, period_end).await?;
    let draft = packs.iter().find(|p| p.status == KpiStatus::Draft);
    let at: DateTime = renox::db::now();
    let mut tx = state.db.begin().await?;
    let (pack_id, created) = if let Some(draft) = draft {
        KpiFigure::query()
            .where_(KpiFigure::PACK_ID.eq(draft.id))
            .delete(&mut tx)
            .await?;
        crate::db::set_kpi_pack(
            &mut tx,
            draft.id,
            &[("imported_by", &Some(by.id)), ("imported_at", &Some(at))],
        )
        .await?;
        (draft.id, false)
    } else {
        let version = packs.iter().map(|p| p.version).max().unwrap_or(0) + 1;
        let commentary = packs
            .iter()
            .rev()
            .find(|p| p.status == KpiStatus::Published)
            .and_then(|p| p.commentary.clone());
        let pack = KpiPack::create(
            &mut tx,
            KpiPack {
                client_party_id: client.0,
                period_end,
                version,
                commentary,
                commentary_required: COMMENTARY_REQUIRED,
                imported_by: Some(by.id),
                imported_at: Some(at),
                ..KpiPack::default()
            },
        )
        .await?;
        (pack.id, true)
    };
    for line in &lines {
        let mut figure = KpiFigure {
            pack_id,
            account_code: line.code.clone(),
            account_name: line.name.clone(),
            amount: line.amount,
            category: mappings.get(&line.code).cloned(),
            ..KpiFigure::default()
        };
        figure.insert(&mut tx).await?;
    }
    tx.commit().await?;
    let pack = KpiPack::find_or_404(&state.db, pack_id).await?;
    tell_manager(state, &pack).await?;
    Ok(Imported { pack, created })
}

/// Tells the engagement manager of the client to add the commentary; when no
/// manager is named, gives the manager role a task.
async fn tell_manager(state: &AppState, pack: &KpiPack) -> Result {
    let client = pack.client();
    let manager = Engagement::of_client(&state.db, client)
        .await?
        .into_iter()
        .filter(|e| e.status != EngagementStatus::Ended)
        .find_map(|e| e.manager_user_id);
    let name = client_name(state, client).await;
    let data = json!({
        "number": pack.number(),
        "client": name,
        "period": pack.period_end.to_string(),
        "version": pack.version,
        "link": format!("/accounting/kpi-packs/{}", pack.id),
    });
    match manager {
        Some(manager) => {
            let request = NotifyRequest::new(IMPORTED_NOTICE, pack.subject())
                .subject_label(&pack.number())
                .to_user(manager)
                .data(data);
            notifier(state).notify(state, request).await?;
        }
        None => {
            let title = english("accounting.kpi.task_title", &[("number", &pack.number())]);
            let description = english(
                "accounting.kpi.task_description",
                &[
                    ("client", &name),
                    ("period", &pack.period_end),
                    ("link", &format!("/accounting/kpi-packs/{}", pack.id)),
                ],
            );
            let task = RoleTask::new(
                pack.subject(),
                MANAGER_ROLE,
                &title,
                &commentary_key(pack.id),
            )
            .description(&description);
            role_tasks::open(state, task).await?;
        }
    }
    Ok(())
}

/// The name of `client` in English, else `#id`.
async fn client_name(state: &AppState, client: PartyId) -> String {
    directory(state)
        .display_name(&state.db, client, "en")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", client.0))
}

/// `by` saves the KPI category of ledger account `code` of `client`
/// (reused every period) and applies it to the accounts of the client's
/// drafts; a published pack's figures are not touched. Answers the number of
/// draft accounts it mapped.
///
/// # Errors
///
/// `Forbidden` without `kpi.manage`; a 422 for an unknown category or a
/// blank code.
pub async fn map_account(
    state: &AppState,
    by: &AuthUser,
    client: PartyId,
    code: &str,
    category: &str,
) -> Result<usize> {
    require(by, permissions::KPI_MANAGE)?;
    let code = code.trim();
    if code.is_empty() || !CATEGORIES.contains(&category) {
        return Err(refused(english("accounting.kpi.problem.category", &[])));
    }
    let mut tx = state.db.begin().await?;
    let found = KpiMapping::query()
        .where_(KpiMapping::CLIENT_PARTY_ID.eq(client.0))
        .where_(KpiMapping::ACCOUNT_CODE.eq(code))
        .exists(&mut tx)
        .await?;
    if found {
        KpiMapping::query()
            .where_(KpiMapping::CLIENT_PARTY_ID.eq(client.0))
            .where_(KpiMapping::ACCOUNT_CODE.eq(code))
            .update(&mut tx, &[("category", &category.to_owned())])
            .await?;
    } else {
        let mut row = KpiMapping {
            client_party_id: client.0,
            account_code: code.to_owned(),
            category: category.to_owned(),
            ..KpiMapping::default()
        };
        row.insert(&mut tx).await?;
    }
    let drafts = KpiPack::query()
        .where_(KpiPack::CLIENT_PARTY_ID.eq(client.0))
        .where_(KpiPack::STATUS.eq(KpiStatus::Draft))
        .get(&mut tx)
        .await?;
    let mut mapped = 0;
    for draft in drafts {
        let count = KpiFigure::query()
            .where_(KpiFigure::PACK_ID.eq(draft.id))
            .where_(KpiFigure::ACCOUNT_CODE.eq(code))
            .count(&mut tx)
            .await?;
        if count > 0 {
            KpiFigure::query()
                .where_(KpiFigure::PACK_ID.eq(draft.id))
                .where_(KpiFigure::ACCOUNT_CODE.eq(code))
                .update(&mut tx, &[("category", &Some(category.to_owned()))])
                .await?;
            mapped += 1;
        }
    }
    tx.commit().await?;
    Ok(mapped)
}

/// `by` writes the commentary of draft pack `id`.
///
/// # Errors
///
/// `Forbidden` without `kpi.manage`; a 422 unless the pack is a draft.
pub async fn set_commentary(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    text: &str,
) -> Result<KpiPack> {
    require(by, permissions::KPI_MANAGE)?;
    let pack = KpiPack::find_or_404(&state.db, id).await?;
    if pack.status != KpiStatus::Draft {
        return Err(refused(english("accounting.kpi.problem.not_draft", &[])));
    }
    let text = text.trim();
    let value = if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    };
    KpiPack::query()
        .where_(KpiPack::ID.eq(id))
        .update(&state.db, &[("commentary", &value)])
        .await?;
    KpiPack::find_or_404(&state.db, id).await
}

/// `by` publishes draft pack `id`: every account mapped, the commentary there
/// when the template requires it. After the commit the earlier published
/// version of the period is superseded and the client users are told
/// ([`after`]).
///
/// # Errors
///
/// `Forbidden` without `kpi.publish`; a 422 naming the unmapped accounts, or
/// the missing commentary.
pub async fn publish(state: &AppState, by: &AuthUser, id: i64) -> Result<KpiPack> {
    let pack = KpiPack::find_or_404(&state.db, id).await?;
    let unmapped: Vec<String> = pack
        .figures(&state.db)
        .await?
        .into_iter()
        .filter(|f| f.category.is_none())
        .map(|f| f.account_code)
        .collect();
    let commentary_ok = !pack.commentary_required
        || pack
            .commentary
            .as_deref()
            .is_some_and(|c| !c.trim().is_empty());
    let input = KpiInput {
        unmapped,
        commentary_ok,
        reason: None,
        on: Some(today()),
    };
    let _outcome = pack
        .transition(PUBLISH)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    KpiPack::find_or_404(&state.db, id).await
}

/// `by` withdraws published pack `id` with `reason` (*wrong client* or
/// *wrong figures*). The pack leaves the portal at once; the client users
/// are told and, for a wrong client, the company admin gets the privacy
/// incident task ([`after`]).
///
/// # Errors
///
/// `Forbidden` without `kpi.publish`; a 422 without one of the two reasons or
/// for a pack that is not published.
pub async fn withdraw(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<KpiPack> {
    let pack = KpiPack::find_or_404(&state.db, id).await?;
    let input = KpiInput {
        reason: Some(reason.trim().to_owned()),
        on: Some(today()),
        ..KpiInput::default()
    };
    let _outcome = pack
        .transition(WITHDRAW)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    KpiPack::find_or_404(&state.db, id).await
}

/// `by` copies withdrawn pack `id` into a new draft of the same period for
/// `to` (the client the figures belong to): the imported figures, their KPI
/// categories (kept as the client's saved mapping where it has none) and the
/// commentary. The withdrawn pack is not changed.
///
/// # Errors
///
/// `Forbidden` without `kpi.manage`; a 422 unless the pack is withdrawn, for
/// an unknown client, or when the client's period already has a draft.
pub async fn copy(state: &AppState, by: &AuthUser, id: i64, to: PartyId) -> Result<KpiPack> {
    require(by, permissions::KPI_MANAGE)?;
    let source = KpiPack::find_or_404(&state.db, id).await?;
    if source.status != KpiStatus::Withdrawn {
        return Err(refused(english(
            "accounting.kpi.problem.not_withdrawn",
            &[],
        )));
    }
    if directory(state).find(&state.db, to).await?.is_none() {
        return Err(refused(english("accounting.kpi.problem.client", &[])));
    }
    let packs = KpiPack::of_period(&state.db, to, source.period_end).await?;
    if packs.iter().any(|p| p.status == KpiStatus::Draft) {
        return Err(refused(english("accounting.kpi.problem.draft_exists", &[])));
    }
    let figures = source.figures(&state.db).await?;
    let version = packs.iter().map(|p| p.version).max().unwrap_or(0) + 1;
    let known: Vec<String> = KpiMapping::of_client(&state.db, to)
        .await?
        .into_iter()
        .map(|m| m.account_code)
        .collect();
    let mut tx = state.db.begin().await?;
    let pack = KpiPack::create(
        &mut tx,
        KpiPack {
            client_party_id: to.0,
            period_end: source.period_end,
            version,
            commentary: source.commentary.clone(),
            commentary_required: source.commentary_required,
            imported_by: Some(by.id),
            imported_at: Some(renox::db::now()),
            copied_from_id: Some(source.id),
            ..KpiPack::default()
        },
    )
    .await?;
    for figure in figures {
        if let Some(category) = &figure.category
            && !known.contains(&figure.account_code)
        {
            let mut mapping = KpiMapping {
                client_party_id: to.0,
                account_code: figure.account_code.clone(),
                category: category.clone(),
                ..KpiMapping::default()
            };
            mapping.insert(&mut tx).await?;
        }
        let mut copy = KpiFigure {
            pack_id: pack.id,
            account_code: figure.account_code,
            account_name: figure.account_name,
            amount: figure.amount,
            category: figure.category,
            ..KpiFigure::default()
        };
        copy.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(pack)
}

/// Records that `user` opened pack `pack` in the portal, unless they are
/// staff who see packs anyway: the trail of the privacy incident task.
///
/// # Errors
///
/// The database's errors.
pub async fn record_view(state: &AppState, user: &AuthUser, pack: &KpiPack) -> Result {
    if user.has_permission(permissions::KPI_VIEW) {
        return Ok(());
    }
    audit::record(
        &state.db,
        Entry::new(VIEWED)
            .user(user.id)
            .subject(KpiPack::TABLE, pack.id),
    )
    .await
}

/// Who opened pack `pack` and when, one line each: the name, the first and
/// the last time.
///
/// # Errors
///
/// The database's errors.
pub async fn view_log(state: &AppState, pack: &KpiPack) -> Result<Vec<String>> {
    let entries = audit::for_subject(&state.db, KpiPack::TABLE, pack.id, 1000).await?;
    let mut opened: BTreeMap<i64, (DateTime, DateTime)> = BTreeMap::new();
    for entry in entries.iter().rev().filter(|e| e.action == VIEWED) {
        if let Some(user) = entry.user_id {
            let slot = opened
                .entry(user)
                .or_insert((entry.created_at, entry.created_at));
            slot.1 = entry.created_at;
        }
    }
    let mut lines = Vec::new();
    for (user, (first, last)) in opened {
        let name = User::find(&state.db, user)
            .await?
            .map_or_else(|| format!("#{user}"), |u| u.name);
        lines.push(english(
            "accounting.kpi.incident.viewer",
            &[
                ("name", &name),
                ("first", &dubai_text(first)),
                ("last", &dubai_text(last)),
            ],
        ));
    }
    Ok(lines)
}

/// Who a message to the client users goes to: the person of each portal user
/// acting for it, once; else the client company.
async fn client_users(
    state: &AppState,
    client: PartyId,
    on: NaiveDate,
) -> Result<Vec<NotifyRecipient>> {
    let mut recipients = Vec::new();
    for user in portal_users(state).users_for(&state.db, client, on).await? {
        let recipient = NotifyRecipient::Party { id: user.person };
        if !recipients.contains(&recipient) {
            recipients.push(recipient);
        }
    }
    if recipients.is_empty() {
        recipients.push(NotifyRecipient::Party { id: client });
    }
    Ok(recipients)
}

/// Tells the client users of `pack` with the notice `key` and `data`.
async fn tell_client(
    state: &AppState,
    key: &str,
    pack: &KpiPack,
    data: renox::serde_json::Value,
) -> Result {
    // `ACC.ACL.14`: nothing automatic goes to the client while the
    // engagement's reason is restricted.
    let suppressed = Engagement::of_client(&state.db, pack.client())
        .await?
        .iter()
        .any(|engagement| !engagement.may_message_client());
    if suppressed {
        return Ok(());
    }
    let mut request = NotifyRequest::new(key, pack.subject())
        .subject_label(&pack.number())
        .data(data);
    for recipient in client_users(state, pack.client(), today()).await? {
        request = request.to(recipient);
    }
    notifier(state).notify(state, request).await?;
    Ok(())
}

/// What happens after a KPI pack transition commits.
///
/// # Errors
///
/// The database's, the notifier's and the task service's errors.
pub async fn after(state: &AppState, transition: &str, id: i64) -> Result {
    let pack = KpiPack::find_or_404(&state.db, id).await?;
    match transition {
        PUBLISH => {
            let data = json!({
                "number": pack.number(),
                "period": pack.period_end.to_string(),
                "link": format!("/accounting/portal/kpi-packs/{}", pack.id),
            });
            tell_client(state, PUBLISHED_NOTICE, &pack, data).await
        }
        WITHDRAW => {
            let wrong_client = pack.withdraw_reason.as_deref() == Some(WRONG_CLIENT);
            // Decision: for a wrong client the message says only that a
            // report was withdrawn and must be disregarded: no period, no
            // number, no company.
            let message = if wrong_client {
                english("accounting.kpi.withdrawn_wrong_client", &[])
            } else {
                english(
                    "accounting.kpi.withdrawn_wrong_figures",
                    &[("period", &pack.period_end)],
                )
            };
            // The incident first: it is the part that must not be lost.
            if wrong_client {
                open_incident(state, &pack).await?;
            }
            tell_client(
                state,
                WITHDRAWN_NOTICE,
                &pack,
                json!({ "message": message }),
            )
            .await
        }
        _ => Ok(()),
    }
}

/// The privacy incident task for the company admin: the pack, the receiving
/// company and who of it opened the pack.
async fn open_incident(state: &AppState, pack: &KpiPack) -> Result {
    let company = client_name(state, pack.client()).await;
    let log = view_log(state, pack).await?;
    let viewers = if log.is_empty() {
        english("accounting.kpi.incident.nobody", &[])
    } else {
        log.join("\n")
    };
    let title = english(
        "accounting.kpi.incident.title",
        &[("number", &pack.number())],
    );
    let description = english(
        "accounting.kpi.incident.description",
        &[
            ("number", &pack.number()),
            ("company", &company),
            ("viewers", &viewers),
            ("link", &format!("/accounting/kpi-packs/{}", pack.id)),
        ],
    );
    let task = RoleTask::new(
        pack.subject(),
        COMPANY_ADMIN_ROLE,
        &title,
        &incident_key(pack.id),
    )
    .description(&description);
    role_tasks::open(state, task).await?;
    audit::record(
        &state.db,
        Entry::new(INCIDENT_OPENED).subject(KpiPack::TABLE, pack.id),
    )
    .await
}

/// Opens the privacy incident task of every pack withdrawn as sent to the
/// wrong client that has none: the process ending between the commit of
/// `withdraw` and [`after`] must not lose it. Idempotent (the task has a
/// source key, and the pack is marked once served). Answers the ids served.
///
/// # Errors
///
/// The database's and the task service's errors.
pub async fn catch_up_incidents(state: &AppState) -> Result<Vec<i64>> {
    let withdrawn = KpiPack::query()
        .where_(KpiPack::STATUS.eq(KpiStatus::Withdrawn))
        .order_by(KpiPack::ID)
        .get(&state.db)
        .await?;
    let mut served = Vec::new();
    for pack in withdrawn {
        if pack.withdraw_reason.as_deref() != Some(WRONG_CLIENT) {
            continue;
        }
        let trail = audit::for_subject(&state.db, KpiPack::TABLE, pack.id, 1000).await?;
        if trail.iter().any(|entry| entry.action == INCIDENT_OPENED) {
            continue;
        }
        open_incident(state, &pack).await?;
        served.push(pack.id);
    }
    Ok(served)
}

/// The notification types of the KPI pack.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    // 04 §9: KPI pack notices are in the *portal and signatures* group; one
    // that gives up marks the contact invalid and gives the engagement
    // manager a task. The import notice goes to staff only: no reaction.
    let def = |key: &str, texts: &str| {
        let client = key != IMPORTED_NOTICE;
        NotificationTypeDef::new(key, "accounting")
            .group(GROUP_PORTAL)
            .give_up(&if client {
                vec![
                    GiveUpReaction::MarkContactInvalid,
                    GiveUpReaction::StaffTask,
                ]
            } else {
                vec![GiveUpReaction::None]
            })
            .staff_role(MANAGER_ROLE)
            .template(
                NotifyChannel::Email,
                "en",
                Some(&english(&format!("accounting.kpi.{texts}_subject"), &[])),
                &english(&format!("accounting.kpi.{texts}_body"), &[]),
            )
    };
    vec![
        def(IMPORTED_NOTICE, "imported"),
        def(PUBLISHED_NOTICE, "published"),
        def(WITHDRAWN_NOTICE, "withdrawn"),
    ]
}

/// Registers the notification types and the listener.
pub(crate) fn register(app: &mut Registry) {
    for def in notification_types() {
        notify::register_type(def);
    }
    app.listen(|event: Transitioned<KpiPack>, state: AppState| async move {
        after(&state, &event.transition, event.key).await
    });
    app.schedule()
        .daily_at(CATCH_UP_AT, CATCH_UP_TASK, |state: AppState| async move {
            catch_up_incidents(&state).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirhams_read_as_fils() {
        assert_eq!(parse_fils("1,234.50"), Some(123_450));
        assert_eq!(parse_fils("-3"), Some(-300));
        assert_eq!(parse_fils("(12.5)"), Some(-1_250));
        assert_eq!(parse_fils(""), Some(0));
        assert_eq!(parse_fils(".5"), Some(50));
        assert_eq!(parse_fils("1.234"), None);
        assert_eq!(parse_fils("abc"), None);
    }

    #[test]
    fn a_period_is_the_last_day_of_its_month() {
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(parse_period("2026-10"), Some(day(2026, 10, 31)));
        assert_eq!(parse_period("2026-02-10"), Some(day(2026, 2, 28)));
        assert_eq!(parse_period("2026-12"), Some(day(2026, 12, 31)));
        assert_eq!(parse_period("soon"), None);
    }

    #[test]
    fn a_trial_balance_reads_both_layouts_with_or_without_a_header() {
        let four = "code,name,debit,credit\n4000,Sales,,1000.00\n5000,Cost of sales,400,0\n";
        let lines = parse_trial_balance(four).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].amount, -100_000);
        assert_eq!(lines[1].amount, 40_000);
        let three = "1000;Bank;250.00\n";
        assert_eq!(parse_trial_balance(three).unwrap()[0].amount, 25_000);
    }

    #[test]
    fn a_bad_trial_balance_names_its_rows() {
        let text = "code,name,debit,credit\n4000,Sales,abc,0\n4000,Again,1,0\n";
        let Err(Error::Status(_, message)) = parse_trial_balance(text) else {
            panic!("refused");
        };
        assert!(message.contains("Row 2"), "{message}");
        assert!(parse_trial_balance("").is_err());
    }
}
