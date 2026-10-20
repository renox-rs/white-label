//! The engagement pages: the list, an engagement with its history and the
//! actions on it (04 §7.1 *Engagement*).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /accounting/engagements` | `accounting.engagements` | `engagements.view` |
//! | `POST /accounting/engagements` | `accounting.engagement_create` | `engagements.manage` |
//! | `GET /accounting/engagements/{id}` | `accounting.engagement` | `engagements.view` |
//! | `POST /accounting/engagements/{id}/letter` | `accounting.engagement_letter` | `engagements.manage` |
//! | `POST /accounting/engagements/{id}/activate` | `accounting.engagement_activate` | `engagements.manage` |
//! | `POST /accounting/engagements/{id}/decline` | `accounting.engagement_decline` | `engagements.manage` |
//! | `POST /accounting/engagements/{id}/suspend` | `accounting.engagement_suspend` | `engagements.manage` |
//! | `POST /accounting/engagements/{id}/resume` | `accounting.engagement_resume` | `engagements.manage` |
//! | `POST /accounting/engagements/{id}/end` | `accounting.engagement_end` | `engagements.end` |
//! | `GET /accounting/engagements/export` | `accounting.engagements_export` | `engagements.view` |
//! | `POST /accounting/engagements/{id}/client-notice` | `accounting.engagement_client_notice` | `parties.block_reason.view` |
//!
//! The reason shown everywhere (the list, the page, the history, the export)
//! is the engagement's neutral code ([`crate::reasons`]): no page reads a
//! block's reason (`ACC.ACL.14`). A holder of `parties.block_reason.view`
//! gets a link to the client's party page, where the Block record shows the
//! real reason, and chooses the neutral template that tells the client. The
//! actions answer with a toast and a refresh; a refusal answers 422 with its
//! lines.

use renox::prelude::*;
use renox::state_machine::history;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::deadlines::deadline_tasks;
use wl_kernel::documents::{dubai_text, today};

use super::{Texts, invalid, parse_aed, parse_id, party_name, require, text, user_name};
use crate::cdd::CddFile;
use crate::client_notices::{self, ClientNotice, NeutralTemplate};
use crate::deadline_notices::{self, DeadlineNotice};
use crate::engagement::{Engagement, EngagementStatus, FeeBasis, NewEngagement};
use crate::engagement_machine::{DECLINE, END, SUSPEND};
use crate::engagements;
use crate::permissions;
use crate::reasons;
use crate::tax_profile::TaxProfile;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/accounting/engagements", index)
        .name("accounting.engagements")
        .post("/accounting/engagements", create)
        .name("accounting.engagement_create")
        .get("/accounting/engagements/{id}", show)
        .name("accounting.engagement")
        .post("/accounting/engagements/{id}/letter", letter)
        .name("accounting.engagement_letter")
        .post("/accounting/engagements/{id}/activate", activate)
        .name("accounting.engagement_activate")
        .post("/accounting/engagements/{id}/decline", decline)
        .name("accounting.engagement_decline")
        .post("/accounting/engagements/{id}/suspend", suspend)
        .name("accounting.engagement_suspend")
        .post("/accounting/engagements/{id}/resume", resume)
        .name("accounting.engagement_resume")
        .post("/accounting/engagements/{id}/end", end)
        .name("accounting.engagement_end")
        .get("/accounting/engagements/export", export)
        .name("accounting.engagements_export")
        .post("/accounting/engagements/{id}/client-notice", client_notice)
        .name("accounting.engagement_client_notice")
        .require_auth()
}

/// The badge kind of an engagement status.
pub(crate) fn status_kind(status: EngagementStatus) -> &'static str {
    match status {
        EngagementStatus::Active => "success",
        EngagementStatus::Suspended => "warning",
        EngagementStatus::Onboarding | EngagementStatus::Ended | EngagementStatus::Declined => {
            "info"
        }
    }
}

/// The status of an engagement in the page's language.
pub(crate) fn status_label(lang: &Lang, status: EngagementStatus) -> String {
    text(lang, &format!("engagements.status.{}", status.as_str()))
}

/// The neutral reason of an engagement in the page's language; empty when
/// it has none.
pub(crate) fn reason_label(lang: &Lang, engagement: &Engagement) -> String {
    engagement
        .reason_code
        .as_deref()
        .map(|code| reasons::label(lang, code))
        .unwrap_or_default()
}

/// One engagement of the list.
#[derive(Serialize, Debug, Clone)]
pub struct EngagementRow {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The client's name.
    pub client: String,
    /// The services.
    pub title: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// The neutral reason, if any.
    pub reason: String,
}

/// The rows of `engagements` as `lang` reads them.
pub(crate) async fn rows(
    state: &AppState,
    lang: &Lang,
    engagements: &[Engagement],
) -> Vec<EngagementRow> {
    let mut rows = Vec::with_capacity(engagements.len());
    for engagement in engagements {
        rows.push(EngagementRow {
            id: engagement.id,
            number: engagement.number(),
            client: party_name(state, lang, engagement.client()).await,
            title: engagement.title.clone(),
            status: status_label(lang, engagement.status),
            status_kind: status_kind(engagement.status),
            reason: reason_label(lang, engagement),
        });
    }
    rows
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/engagements.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    engagements: Vec<EngagementRow>,
    can_create: bool,
    fee_bases: Vec<[String; 2]>,
}

/// `GET /accounting/engagements`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::ENGAGEMENTS_VIEW)?;
    let all = Engagement::query()
        .order_by_desc(Engagement::ID)
        .get(&state.db)
        .await?;
    let fee_bases = FeeBasis::ALL
        .iter()
        .map(|b| {
            [
                b.as_str().to_owned(),
                text(&lang, &format!("engagements.fee_basis.{}", b.as_str())),
            ]
        })
        .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        engagements: rows(&state, &lang, &all).await,
        can_create: user.has_permission(permissions::ENGAGEMENTS_MANAGE),
        fee_bases,
    }))
}

/// The form that creates an engagement.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CreateForm {
    /// The client, a party's id.
    #[serde(default)]
    pub client: String,
    /// The services.
    #[serde(default)]
    pub title: String,
    /// `retainer`, `fixed` or `time`.
    #[serde(default)]
    pub fee_basis: String,
    /// The monthly retainer in dirhams.
    #[serde(default)]
    pub retainer_fee: String,
}

/// `POST /accounting/engagements`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<CreateForm>,
) -> Result<Response> {
    require(&user, permissions::ENGAGEMENTS_MANAGE)?;
    let client = parse_id(&form.client).ok_or_else(|| invalid("client: 1, 2, 3 ..."))?;
    let fee = parse_aed(&form.retainer_fee).ok_or_else(|| invalid("retainer_fee: 5000.00"))?;
    let basis = FeeBasis::ALL
        .iter()
        .copied()
        .find(|b| b.as_str() == form.fee_basis.trim())
        .unwrap_or_default();
    let new = NewEngagement::retainer(PartyId(client), &form.title, fee)
        .fee_basis(basis)
        .manager(user.id);
    engagements::create(&state, Some(user.id), new).await?;
    Ok((
        Toast::success(text(&lang, "engagements.created")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The engagement as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct EngagementView {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The client's name.
    pub client: String,
    /// The services.
    pub title: String,
    /// How it is billed.
    pub fee_basis: String,
    /// The monthly retainer.
    pub retainer: String,
    /// The first day.
    pub start: String,
    /// The letter: "Signed on …" or "Not signed".
    pub letter: String,
    /// The tax profile: complete or what is missing.
    pub tax_profile: String,
    /// The neutral reason, if any.
    pub reason: String,
    /// The day it was activated.
    pub activated: String,
    /// The day it ended.
    pub ended: String,
}

/// One row of the history: a transition with its neutral reason.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    /// When (Asia/Dubai).
    pub at: String,
    /// What happened.
    pub action: String,
    /// The neutral reason, if any.
    pub reason: String,
    /// Who ran it, or the system.
    pub by: String,
}

/// The history of `engagement` as `lang` reads it. The reasons are the
/// neutral codes the transitions kept; a restricted reason never reaches
/// them (`ACC.ACL.14`).
///
/// # Errors
///
/// The database's errors.
pub async fn history_rows(
    state: &AppState,
    lang: &Lang,
    engagement: &Engagement,
) -> Result<Vec<HistoryRow>> {
    let mut rows = Vec::new();
    for row in history(&state.db, Engagement::TABLE, engagement.id).await? {
        let by = match row.actor_id {
            Some(id) => user_name(&state.db, id).await,
            None => text(lang, "history.system"),
        };
        rows.push(HistoryRow {
            at: dubai_text(row.at),
            action: text(lang, &format!("engagements.transition.{}", row.transition)),
            reason: row
                .reason
                .as_deref()
                .map(|code| reasons::label(lang, code))
                .unwrap_or_default(),
            by,
        });
    }
    Ok(rows)
}

/// One neutral template sent to the client.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct NoticeRow {
    /// When (Asia/Dubai).
    pub at: String,
    /// The template's text.
    pub template: String,
    /// Who sent it.
    pub by: String,
}

/// One statutory deadline notice of the engagement (`ACC.RUL.09`).
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct DeadlineNoticeRow {
    /// The transition day.
    pub on: String,
    /// After a suspension or an end.
    pub cause: String,
    /// To send, sent, delivered, acknowledged or cancelled.
    pub state: String,
    /// The deadlines listed.
    pub deadlines: i64,
}

/// One overdue invoice of the client, shown before a resume after
/// *non-payment*.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct OverdueRow {
    /// The invoice number.
    pub number: String,
    /// Its due date.
    pub due: String,
    /// What is still open.
    pub balance: String,
}

/// One deadline task of the engagement's compliance calendar.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    /// The title.
    pub title: String,
    /// The due date.
    pub due: String,
    /// The state in the page's language.
    pub status: String,
    /// "Held" while the engagement is suspended; empty otherwise.
    pub held: String,
}

/// The state of `notice` in the page's language.
fn notice_state(lang: &Lang, notice: &DeadlineNotice) -> String {
    let key = if notice.cancelled_on.is_some() {
        "cancelled"
    } else if notice.acknowledged_at.is_some() {
        "acknowledged"
    } else if notice.delivered_on.is_some() {
        "delivered"
    } else if notice.sent_at.is_some() {
        "sent"
    } else {
        "prepared"
    };
    text(lang, &format!("deadline_notices.state.{key}"))
}

/// What an engagement page needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/engagement.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    engagement: EngagementView,
    status: String,
    status_kind: &'static str,
    history: Vec<HistoryRow>,
    cdd_id: Option<i64>,
    cdd_status: String,
    today: String,
    can_letter: bool,
    can_activate: bool,
    can_decline: bool,
    can_suspend: bool,
    can_resume: bool,
    can_end: bool,
    decline_reasons: Vec<[String; 2]>,
    suspend_reasons: Vec<[String; 2]>,
    end_reasons: Vec<[String; 2]>,
    notices: Vec<NoticeRow>,
    can_notify: bool,
    notice_template: String,
    notice_template_text: String,
    block_record: String,
    deadline_notices: Vec<DeadlineNoticeRow>,
    send_notice_id: Option<i64>,
    tasks: Vec<TaskRow>,
    show_overdue: bool,
    overdue_invoices: Vec<OverdueRow>,
    overdue_total: String,
    resume_needs_approval: bool,
}

/// The options of a reason select for `transition`.
fn reason_options(lang: &Lang, transition: &str) -> Vec<[String; 2]> {
    engagements::user_reasons(transition)
        .iter()
        .map(|code| [(*code).to_owned(), reasons::label(lang, code)])
        .collect()
}

/// `GET /accounting/engagements/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::ENGAGEMENTS_VIEW)?;
    let found = Engagement::find_or_404(&state.db, id).await?;
    let manage = user.has_permission(permissions::ENGAGEMENTS_MANAGE);
    let on = today();
    let tax_profile = match TaxProfile::current(&state.db, found.client(), on).await? {
        None => text(&lang, "engagements.tax_profile_none"),
        Some(profile) if profile.is_complete() => text(&lang, "engagements.tax_profile_complete"),
        Some(profile) => profile.missing().join(", "),
    };
    let letter = match (found.letter_signed, found.letter_signed_on) {
        (true, Some(signed)) => texts::t_with(
            &lang,
            "accounting.engagements.letter_signed_on",
            &[("date", &signed)],
        ),
        (true, None) => text(&lang, "engagements.letter_signed"),
        (false, _) => text(&lang, "engagements.letter_not_signed"),
    };
    let cdd = CddFile::current(&state.db, found.client()).await?;
    let view = EngagementView {
        id: found.id,
        number: found.number(),
        client: party_name(&state, &lang, found.client()).await,
        title: found.title.clone(),
        fee_basis: text(
            &lang,
            &format!("engagements.fee_basis.{}", found.fee_basis.as_str()),
        ),
        retainer: crate::money_text(found.retainer_fee),
        start: found.start_date.map(|d| d.to_string()).unwrap_or_default(),
        letter,
        tax_profile,
        reason: reason_label(&lang, &found),
        activated: found
            .activated_on
            .map(|d| d.to_string())
            .unwrap_or_default(),
        ended: found.ended_on.map(|d| d.to_string()).unwrap_or_default(),
    };
    let onboarding = found.status == EngagementStatus::Onboarding;
    // The MLRO and deputy: the link to the Block record and the neutral
    // template that fits the status.
    let sees_reasons = user.has_permission(permissions::BLOCK_REASON_VIEW);
    let template = [NeutralTemplate::OnHold, NeutralTemplate::Ended]
        .into_iter()
        .find(|t| t.fits(found.status));
    let mut notices = Vec::new();
    for notice in ClientNotice::of_engagement(&state.db, found.id).await? {
        notices.push(NoticeRow {
            at: dubai_text(notice.sent_at),
            template: template_text(&lang, notice.template),
            by: user_name(&state.db, notice.sent_by).await,
        });
    }
    let mut deadline_rows = Vec::new();
    let mut send_notice_id = None;
    for notice in DeadlineNotice::of_engagement(&state.db, found.id).await? {
        if deadline_notices::may_send(&user, &notice) {
            send_notice_id = Some(notice.id);
        }
        deadline_rows.push(DeadlineNoticeRow {
            on: notice.on_date.to_string(),
            cause: text(
                &lang,
                &format!("deadline_notices.cause.{}", notice.cause.as_str()),
            ),
            state: notice_state(&lang, &notice),
            deadlines: notice.deadline_count,
        });
    }
    let tasks: Vec<TaskRow> = deadline_tasks(&state)
        .of_subject(&state.db, &found.subject())
        .await?
        .into_iter()
        .map(|task| TaskRow {
            title: task.title,
            due: task.due_date.to_string(),
            status: text(&lang, &format!("tasks.status.{}", task.status.as_str())),
            held: if task.held {
                text(&lang, "tasks.held")
            } else {
                String::new()
            },
        })
        .collect();
    // A suspension for non-payment: the client's overdue invoices and their
    // total, as information before the resume (not a guard).
    let show_overdue = found.status == EngagementStatus::Suspended
        && found.reason_code.as_deref() == Some(reasons::NON_PAYMENT);
    let (overdue_invoices, overdue_total) = if show_overdue {
        let (invoices, total) = engagements::overdue_invoices(&state, found.client()).await?;
        let rows: Vec<OverdueRow> = invoices
            .iter()
            .map(|invoice| OverdueRow {
                number: invoice
                    .number
                    .clone()
                    .unwrap_or_else(|| format!("#{}", invoice.id.0)),
                due: invoice.due_date.map(|d| d.to_string()).unwrap_or_default(),
                balance: crate::money_text(invoice.balance),
            })
            .collect();
        (rows, crate::money_text(total))
    } else {
        (Vec::new(), String::new())
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        tasks,
        show_overdue,
        overdue_invoices,
        overdue_total,
        resume_needs_approval: found.status == EngagementStatus::Suspended
            && found.reason_code.as_deref() == Some(reasons::ON_HOLD),
        deadline_notices: deadline_rows,
        send_notice_id,
        status: status_label(&lang, found.status),
        status_kind: status_kind(found.status),
        history: history_rows(&state, &lang, &found).await?,
        cdd_id: cdd.as_ref().map(|f| f.id),
        cdd_status: cdd
            .as_ref()
            .map(|f| text(&lang, &format!("cdd.status.{}", f.status.as_str())))
            .unwrap_or_default(),
        today: on.to_string(),
        can_letter: manage && onboarding && !found.letter_signed,
        can_activate: manage && onboarding,
        can_decline: manage && onboarding,
        can_suspend: manage && found.status == EngagementStatus::Active,
        can_resume: manage && found.status == EngagementStatus::Suspended,
        can_end: user.has_permission(permissions::ENGAGEMENTS_END)
            && matches!(
                found.status,
                EngagementStatus::Active | EngagementStatus::Suspended
            ),
        decline_reasons: reason_options(&lang, DECLINE),
        suspend_reasons: reason_options(&lang, SUSPEND),
        end_reasons: reason_options(&lang, END),
        notices,
        can_notify: sees_reasons && template.is_some(),
        notice_template: template.map(|t| t.as_str().to_owned()).unwrap_or_default(),
        notice_template_text: template
            .map(|t| template_text(&lang, t))
            .unwrap_or_default(),
        block_record: if sees_reasons {
            format!("/parties/{}", found.client_party_id)
        } else {
            String::new()
        },
        engagement: view,
    }))
}

/// The text of a neutral template, as the client reads it.
fn template_text(lang: &Lang, template: NeutralTemplate) -> String {
    text(lang, &format!("client_notices.{}_body", template.as_str()))
}

/// A CSV field: quoted when it holds a comma, a quote or a line break.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// `GET /accounting/engagements/export`: every engagement as CSV, with its
/// neutral reason only (`ACC.ACL.14`).
async fn export(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
) -> Result<renox::Download> {
    require(&user, permissions::ENGAGEMENTS_VIEW)?;
    let all = Engagement::query()
        .order_by(Engagement::ID)
        .get(&state.db)
        .await?;
    let header = [
        text(&lang, "engagements.number"),
        text(&lang, "engagements.client"),
        text(&lang, "engagements.services"),
        text(&lang, "fields.status"),
        text(&lang, "engagements.reason"),
    ];
    let mut lines = vec![
        header
            .iter()
            .map(|h| csv_field(h))
            .collect::<Vec<_>>()
            .join(","),
    ];
    for row in rows(&state, &lang, &all).await {
        lines.push(
            [row.number, row.client, row.title, row.status, row.reason]
                .iter()
                .map(|f| csv_field(f))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    let mut body = lines.join("\n");
    body.push('\n');
    Ok(renox::Download::bytes(
        "engagements.csv",
        "text/csv; charset=utf-8",
        body.into_bytes(),
    ))
}

/// The form that sends a neutral template.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct NoticeForm {
    /// `on_hold` or `ended`.
    #[serde(default)]
    pub template: String,
}

/// `POST /accounting/engagements/{id}/client-notice`.
async fn client_notice(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<NoticeForm>,
) -> Result<Response> {
    let template = NeutralTemplate::parse(&form.template)
        .ok_or_else(|| invalid("template: on_hold, ended"))?;
    client_notices::send(&state, &user, id, template).await?;
    Ok((
        Toast::success(text(&lang, "client_notices.sent")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that records the signed letter.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct LetterForm {
    /// The day it was signed.
    #[serde(default)]
    pub signed_on: String,
}

/// `POST /accounting/engagements/{id}/letter`.
async fn letter(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<LetterForm>,
) -> Result<Response> {
    let on = form
        .signed_on
        .trim()
        .parse()
        .map_err(|_| invalid("signed_on: YYYY-MM-DD"))?;
    engagements::record_letter_signed(&state, &user, id, on).await?;
    Ok((
        Toast::success(text(&lang, "engagements.letter_recorded")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /accounting/engagements/{id}/activate`.
async fn activate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    engagements::activate(&state, &user, id).await?;
    Ok((
        Toast::success(text(&lang, "engagements.activated")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form of an action with a reason.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ReasonForm {
    /// The neutral reason code.
    #[serde(default)]
    pub reason: String,
}

/// `POST /accounting/engagements/{id}/decline`.
async fn decline(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    engagements::decline(&state, &user, id, &form.reason).await?;
    Ok((
        Toast::success(text(&lang, "engagements.declined")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /accounting/engagements/{id}/suspend`.
async fn suspend(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    engagements::suspend(&state, &user, id, &form.reason).await?;
    Ok((
        Toast::success(text(&lang, "engagements.suspended")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form of a resume.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ResumeForm {
    /// What changed, or the client's instruction.
    #[serde(default)]
    pub note: String,
}

/// `POST /accounting/engagements/{id}/resume`: resumed, or sent to the
/// MLRO for approval after a block.
async fn resume(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ResumeForm>,
) -> Result<Response> {
    let resumed = engagements::resume(&state, &user, id, &form.note).await?;
    let message = if resumed.approval_request.is_some() {
        text(&lang, "engagements.resume_pending")
    } else {
        text(&lang, "engagements.resumed")
    };
    Ok((Toast::success(message), renox::HxRefresh).into_response())
}

/// `POST /accounting/engagements/{id}/end`.
async fn end(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    engagements::end(&state, &user, id, &form.reason).await?;
    Ok((
        Toast::success(text(&lang, "engagements.ended")),
        renox::HxRefresh,
    )
        .into_response())
}
