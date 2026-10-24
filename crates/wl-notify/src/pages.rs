//! The back-office pages (wl-notify §6, DESIGN-SYSTEM):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /notification-types` | `wl_notify.types` | `notifications.templates.manage` |
//! | `GET /notification-types/{key}` | `wl_notify.type_edit` | `notifications.templates.manage` (an unknown key is 404) |
//! | `POST /notification-types/{key}` | `wl_notify.type_update` | the same; a refused give-up combination is a 422 and nothing is saved |
//! | `GET /notification-log` | `wl_notify.log` | `notifications.log.view` |
//! | `GET /notify/send-windows` | `wl_notify.send_windows` | `notifications.channels.manage` |
//! | `POST /notify/send-windows` | `wl_notify.send_windows_update` | the same (one channel per save; a refused window is a 422 and nothing is saved) |
//! | `GET /statutory-notices/{id}/proof` | `wl_notify.notice_proof` | `notifications.notices.record_proof` (an unknown notice is 404) |
//! | `POST /statutory-notices/{id}/proof` | `wl_notify.notice_proof_store` | the same (multipart; a refused proof is a 422, a second one a 409) |
//! | `GET /statutory-notices/{id}/proof/file` | `wl_notify.notice_proof_file` | the same (the attachment) |
//! | `GET /notify/broadcasts` | `wl_notify.broadcasts` | signed in; a holder of `notifications.marketing.send` sees all, anyone else the ones they asked for |
//! | `GET /notify/broadcasts/{id}` | `wl_notify.broadcast` | the requester or a holder of `notifications.marketing.send` (others 403, unknown 404); no party names |
//! | `POST /notify/broadcasts/{id}/cancel` | `wl_notify.broadcast_cancel` | the same (a broadcast that cannot be cancelled is a 422) |
//! | `POST /notify/broadcasts/{id}/stop` | `wl_notify.broadcast_stop` | a holder of `notifications.marketing.send` (others 403); nothing left to stop is a 422 |
//! | `GET /unsubscribe/{token}` | `wl_notify.unsubscribe` | nobody: public, the signed token is the key (a bad token shows a neutral page) |
//! | `POST /unsubscribe/{token}` | `wl_notify.unsubscribe_confirm` | the same (withdraws the e-mail marketing consent, idempotent) |
//!
//! (`/notifications` is the Renox bell's page.) The log never shows a
//! message that gave up or fell back as sent or delivered: an `abandoned`
//! message reads "Not delivered (gave up)" or "Not delivered (fell back to
//! SMS)", and the "Sent" counter counts only `sent`, `delivered` and `read`.
//! Every template is compiled in and can be replaced by the app
//! (`resources/views/wl-notify/…`).

use std::collections::{BTreeMap, HashMap};

use renox::Download;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::notify::{self, Category, GiveUpReaction, NotifyChannel, ProofMethod, Urgency};

use crate::broadcast::{Broadcast, BroadcastStatus};
use crate::broadcast_results;
use crate::broadcast_run;
use crate::broadcasts;
use crate::message::{Message, MessageStatus, RecipientKind};
use crate::notices::{self, ProofError, ProofFile, ProofInput, ProofRefusal, StatutoryNotice};
use crate::notifier::type_def;
use crate::opt_out;
use crate::permissions;
use crate::send_window::SendWindow;
use crate::stale;
use crate::texts;
use crate::types::NotificationType;
use crate::window::Window;

/// The texts a template reads as `tx['fields.key']`.
type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_notify.` prefix) in a language.
type Text<'a> = &'a dyn Fn(&str) -> String;

/// The module's routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .get("/notification-types", types_index)
        .name("wl_notify.types")
        .require_permission(permissions::TEMPLATES_MANAGE)
        .merge(
            Routes::new()
                .get("/notification-types/{key}", type_edit)
                .name("wl_notify.type_edit")
                .post("/notification-types/{key}", type_update)
                .name("wl_notify.type_update")
                .require_permission(permissions::TEMPLATES_MANAGE),
        )
        .merge(
            Routes::new()
                .get("/notification-log", log_index)
                .name("wl_notify.log")
                .require_permission(permissions::LOG_VIEW),
        )
        .merge(
            Routes::new()
                .get("/notify/send-windows", send_windows_index)
                .name("wl_notify.send_windows")
                .post("/notify/send-windows", send_windows_update)
                .name("wl_notify.send_windows_update")
                .require_permission(permissions::CHANNELS_MANAGE),
        )
        .merge(
            Routes::new()
                .get("/statutory-notices/{id}/proof", proof_page)
                .name("wl_notify.notice_proof")
                .post("/statutory-notices/{id}/proof", proof_store)
                .name("wl_notify.notice_proof_store")
                .get("/statutory-notices/{id}/proof/file", proof_file)
                .name("wl_notify.notice_proof_file")
                .require_permission(permissions::RECORD_PROOF),
        )
        // The broadcasts: the handlers check "requester or holder".
        .merge(
            Routes::new()
                .get("/notify/broadcasts", broadcasts_index)
                .name("wl_notify.broadcasts")
                .get("/notify/broadcasts/{id}", broadcast_show)
                .name("wl_notify.broadcast")
                .post("/notify/broadcasts/{id}/cancel", broadcast_cancel)
                .name("wl_notify.broadcast_cancel")
                .post("/notify/broadcasts/{id}/stop", broadcast_stop)
                .name("wl_notify.broadcast_stop"),
        )
        // Public: a recipient opens it from an e-mail, signed out. No
        // guard; the token is the proof.
        .merge(
            Routes::new()
                .get("/unsubscribe/{token}", unsubscribe_page)
                .name(opt_out::ROUTE)
                .post("/unsubscribe/{token}", unsubscribe_confirm)
                .name("wl_notify.unsubscribe_confirm"),
        )
}

/// The public unsubscribe page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-notify/unsubscribe.html")]
pub(crate) struct UnsubscribePage {
    tx: Texts,
    /// `ask` (the button), `done` or `invalid`.
    step: &'static str,
    token: String,
    brand: String,
}

/// The page for `token` in `step`; a token that opens nothing is `invalid`.
fn unsubscribe_view(state: &AppState, lang: &Lang, token: String, step: &'static str) -> View {
    let valid = opt_out::message_of(state, &token).is_some();
    View::typed(UnsubscribePage {
        tx: texts::all(lang),
        step: if valid { step } else { "invalid" },
        token,
        brand: opt_out::brand(state),
    })
}

/// `GET /unsubscribe/{token}`: asks for the confirmation. It changes nothing
/// (mail scanners open links).
async fn unsubscribe_page(
    State(state): State<AppState>,
    lang: Lang,
    Path(token): Path<String>,
) -> Result<View> {
    Ok(unsubscribe_view(&state, &lang, token, "ask"))
}

/// `POST /unsubscribe/{token}`: withdraws the e-mail marketing consent and
/// suppresses the address as `opt_out`. A bad token changes nothing and shows
/// the neutral page.
async fn unsubscribe_confirm(
    State(state): State<AppState>,
    lang: Lang,
    Path(token): Path<String>,
) -> Result<View> {
    if let Some(id) = opt_out::message_of(&state, &token) {
        opt_out::confirm(&state, id).await?;
    }
    Ok(unsubscribe_view(&state, &lang, token, "done"))
}

/// The package texts in `lang`.
fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_notify.{key}"))
}

/// A channel's label: "Email", "SMS".
fn channel_label(t: Text<'_>, channel: NotifyChannel) -> String {
    t(&format!("channel.{}", channel.as_str()))
}

/// A staff role's label: the text `wl_notify.role.<key>` when the app has
/// one, else the key in words (`collector` → "Collector").
fn role_label(t: Text<'_>, role: &str) -> String {
    let key = format!("role.{role}");
    let found = t(&key);
    if found != format!("wl_notify.{key}") {
        return found;
    }
    let words = role.replace(['_', '-', '.'], " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// One notification type as the types page shows it.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct TypeRow {
    /// The type's key.
    pub key: String,
    /// The type in words, from its key: `accounting.return_approval_withdrawn`
    /// is "Return approval withdrawn".
    pub name: String,
    /// The sending module.
    pub module: String,
    /// "Reminder".
    pub category: String,
    /// "Normal" or "Urgent".
    pub urgency: String,
    /// "Email → SMS".
    pub channels: String,
    /// Whether the type is statutory.
    pub statutory: bool,
    /// "Yes" or "No".
    pub statutory_label: String,
    /// The reactions in run order: "Mark contact invalid, Staff task", or
    /// "None".
    pub give_up: String,
    /// "Collector", or "—".
    pub staff_role: String,
}

/// The row of `kind` with the texts of `t`.
/// A type's key in words: the part after the module, its underscores as
/// spaces, the first letter capitalised (`accounting.return_approval_withdrawn`
/// is "Return approval withdrawn"). A long key is one unbreakable word that
/// widens a page past a phone's screen (#494); the name wraps.
pub(crate) fn type_name(key: &str) -> String {
    let event = key.split_once('.').map_or(key, |(_, rest)| rest);
    let words = event.replace(['_', '.'], " ");
    let mut chars = words.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => key.to_owned(),
    }
}

pub(crate) fn type_row(t: Text<'_>, kind: &NotificationType) -> TypeRow {
    let reactions = kind.reactions();
    let give_up = if reactions.is_empty() {
        t(&format!("reaction.{}", GiveUpReaction::None.as_str()))
    } else {
        reactions
            .iter()
            .map(|r| t(&format!("reaction.{}", r.as_str())))
            .collect::<Vec<_>>()
            .join(", ")
    };
    TypeRow {
        key: kind.key.clone(),
        name: type_name(&kind.key),
        module: kind.module.clone(),
        category: t(&format!("category.{}", kind.category.as_str())),
        urgency: t(&format!("urgency.{}", kind.urgency.as_str())),
        channels: kind
            .channels
            .0
            .iter()
            .map(|c| channel_label(t, *c))
            .collect::<Vec<_>>()
            .join(" → "),
        statutory: kind.statutory,
        statutory_label: t(if kind.statutory { "yes" } else { "no" }),
        give_up,
        staff_role: kind
            .staff_role
            .as_deref()
            .filter(|role| !role.trim().is_empty())
            .map_or_else(|| t("none"), |role| role_label(t, role)),
    }
}

/// What the types page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-notify/types.html")]
pub(crate) struct TypesPage {
    tx: Texts,
    types: GridPage<NotificationType>,
}

/// The types grid: search on the key, module and give-up texts, filters by
/// module, category, urgency and statutory. The channels, the reactions and
/// the staff role are drawn by the template (`custom`).
fn types_grid(lang: &Lang) -> Grid {
    let t = texts_in(lang);
    let categories: Vec<(String, String)> = Category::ALL
        .iter()
        .map(|c| {
            (
                c.as_str().to_owned(),
                t(&format!("category.{}", c.as_str())),
            )
        })
        .collect();
    let urgencies: Vec<(String, String)> = Urgency::ALL
        .iter()
        .map(|u| (u.as_str().to_owned(), t(&format!("urgency.{}", u.as_str()))))
        .collect();
    Grid::new("notify-types")
        .title(&t("types.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("key", &t("fields.key"))
                .frozen()
                .mobile()
                .searchable()
                .copyable(),
        )
        .column(
            Column::text("module", &t("fields.module"))
                .mobile()
                .searchable(),
        )
        .column(Column::select("category", &t("fields.category"), categories).mobile())
        .column(Column::select("urgency", &t("fields.urgency"), urgencies).hidden())
        .column(Column::custom("channels", &t("fields.channels")))
        .column(Column::bool("statutory", &t("fields.statutory")).mobile())
        .column(Column::custom("give_up", &t("fields.give_up")))
        .column(Column::custom("staff_role", &t("fields.staff_role")).hidden())
        .column(Column::custom("edit", &t("types.edit")).frozen_right())
        .sort_by("module,key")
        .cards_on_mobile()
        .exports()
        .empty_state(&t("types.empty"), Some(&t("types.empty_hint")))
}

/// `GET /notification-types`: every type with its statutory flag, give-up
/// reaction and staff role told, searched, filtered, sorted and paged by the
/// grid.
async fn types_index(lang: Lang, request: GridRequest) -> Result<Response> {
    let t = texts_in(&lang);
    // The route needs `notifications.templates.manage`, so the export is open.
    let grid = types_grid(&lang);
    let query = NotificationType::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let types = grid.page(query, &request).await?.extend(|kind| {
        let row = type_row(&t, kind);
        json!({
            "channels": row.channels,
            "give_up": row.give_up,
            "staff_role": row.staff_role,
            "edit_url": format!("/notification-types/{}", kind.key),
        })
    });
    Ok(View::typed(TypesPage {
        tx: texts::all(&lang),
        types,
    })
    .into_response())
}

/// The reactions the edit form offers, in run order. `statutory_notice` is
/// left to the statutory-notice work (#59): a stored one is kept as it is.
const EDITABLE: [GiveUpReaction; 3] = [
    GiveUpReaction::MarkContactInvalid,
    GiveUpReaction::AlternativeRecipient,
    GiveUpReaction::StaffTask,
];

/// One reaction checkbox of the edit form.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct ReactionBox {
    /// The form field and reaction key, e.g. `staff_task`.
    pub name: &'static str,
    /// "Staff task".
    pub label: String,
    /// Whether the type has it.
    pub checked: bool,
}

/// What the edit page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-notify/type_edit.html")]
pub(crate) struct TypeEditPage {
    tx: Texts,
    kind: TypeRow,
    reactions: Vec<ReactionBox>,
    staff_role: String,
}

/// The type `key`; 404 when there is none.
async fn stored_type(db: &Db, key: &str) -> Result<NotificationType> {
    NotificationType::by_key(db, key)
        .await?
        .ok_or(Error::NotFound)
}

/// The checkboxes of `kind`, in run order.
pub(crate) fn reaction_boxes(t: Text<'_>, kind: &NotificationType) -> Vec<ReactionBox> {
    let reactions = kind.reactions();
    EDITABLE
        .iter()
        .map(|reaction| ReactionBox {
            name: reaction.as_str(),
            label: t(&format!("reaction.{}", reaction.as_str())),
            checked: reactions.contains(reaction),
        })
        .collect()
}

/// `GET /notification-types/{key}`: the form for the type's give-up
/// reactions and the staff role told.
async fn type_edit(State(db): State<Db>, lang: Lang, Path(key): Path<String>) -> Result<View> {
    let t = texts_in(&lang);
    let kind = stored_type(&db, &key).await?;
    Ok(View::typed(TypeEditPage {
        tx: texts::all(&lang),
        kind: type_row(&t, &kind),
        reactions: reaction_boxes(&t, &kind),
        staff_role: kind.staff_role.clone().unwrap_or_default(),
    }))
}

/// The edit form: one checkbox per reaction, and the staff role.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct TypeForm {
    /// Mark the contact point invalid after a hard failure.
    #[serde(default)]
    pub mark_contact_invalid: bool,
    /// Try another contact of the party.
    #[serde(default)]
    pub alternative_recipient: bool,
    /// Open a task for staff.
    #[serde(default)]
    pub staff_task: bool,
    /// The role told, e.g. `collector`; blank for none.
    #[validate(max = 64)]
    pub staff_role: Option<String>,
}

impl TypeForm {
    /// The reactions of the form, in run order, with the reactions the form
    /// does not offer (`statutory_notice`) kept from `current`. `none` when
    /// nothing is left.
    pub(crate) fn reactions(&self, current: &[GiveUpReaction]) -> Vec<GiveUpReaction> {
        let ticked = |reaction: GiveUpReaction| match reaction {
            GiveUpReaction::MarkContactInvalid => self.mark_contact_invalid,
            GiveUpReaction::AlternativeRecipient => self.alternative_recipient,
            GiveUpReaction::StaffTask => self.staff_task,
            _ => false,
        };
        let mut reactions: Vec<GiveUpReaction> =
            EDITABLE.into_iter().filter(|r| ticked(*r)).collect();
        reactions.extend(
            current
                .iter()
                .copied()
                .filter(|r| *r != GiveUpReaction::None && !EDITABLE.contains(r)),
        );
        if reactions.is_empty() {
            reactions.push(GiveUpReaction::None);
        }
        reactions
    }
}

/// `POST /notification-types/{key}`: saves the reactions and the staff role.
/// A combination `check_give_up` refuses (`mark_contact_invalid` alone on a
/// non-marketing type) is a 422 under the checkbox, and nothing is saved.
async fn type_update(
    State(db): State<Db>,
    lang: Lang,
    Path(key): Path<String>,
    Valid(form): Valid<TypeForm>,
) -> Result<Response> {
    let mut kind = stored_type(&db, &key).await?;
    let reactions = form.reactions(&kind.give_up.0);
    if let Err(error) = notify::check_give_up(kind.category, &reactions) {
        let mut errors = Errors::new();
        errors.add(
            GiveUpReaction::MarkContactInvalid.as_str(),
            error.to_string(),
        );
        return Err(Error::Validation(
            ValidationError::new(errors).with_input(&form),
        ));
    }
    kind.give_up = renox::db::Json(reactions);
    kind.staff_role = form
        .staff_role
        .as_deref()
        .map(str::trim)
        .filter(|role| !role.is_empty())
        .map(str::to_owned);
    kind.save(&db).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_notify.types.saved")),
        Redirect::route("wl_notify.types", &[])?,
    )
        .into_response())
}

/// Whether a message in `status` counts as sent: `sent`, `delivered` or
/// `read`, never `abandoned`, `failed` or `suppressed`.
pub fn counts_as_sent(status: MessageStatus) -> bool {
    matches!(
        status,
        MessageStatus::Sent | MessageStatus::Delivered | MessageStatus::Read
    )
}

/// A message's status label and badge tone. `fell_back_to` is the channel
/// of the message that took over from this one, if any: an `abandoned`
/// message reads "Not delivered (fell back to SMS)" then, else "Not
/// delivered (gave up)".
pub(crate) fn status_label(
    t: Text<'_>,
    status: MessageStatus,
    fell_back_to: Option<NotifyChannel>,
) -> (String, &'static str) {
    match status {
        MessageStatus::Abandoned => match fell_back_to {
            Some(channel) => (
                t("status.fell_back").replace(":channel", &channel_label(t, channel)),
                "warning",
            ),
            None => (t("status.gave_up"), "error"),
        },
        MessageStatus::Sent => (t("status.sent"), "info"),
        MessageStatus::Delivered | MessageStatus::Read => {
            (t(&format!("status.{}", status.as_str())), "success")
        }
        MessageStatus::Failed => (t("status.failed"), "error"),
        MessageStatus::Queued | MessageStatus::Deferred | MessageStatus::Suppressed => {
            (t(&format!("status.{}", status.as_str())), "")
        }
    }
}

/// One message as the log shows it.
#[derive(Serialize, Debug, Clone)]
pub struct LogRow {
    /// The message's id.
    pub id: i64,
    /// When it was queued.
    pub created_at: Option<DateTime>,
    /// The notification type's key.
    pub type_key: String,
    /// The record it is about, e.g. `invoice #12`.
    pub subject: String,
    /// The address, or who it is for.
    pub recipient: String,
    /// "Email", "SMS".
    pub channel: String,
    /// The status as people read it.
    pub status: String,
    /// The status badge's kind (`info`, `success`, `warning`, `error`), or
    /// empty for a plain one.
    pub tone: &'static str,
    /// How many sends were tried.
    pub attempts: i64,
    /// The last error, e.g. `550 mailbox unknown`, or empty.
    pub last_error: String,
}

/// The log row of `message` with the texts of `t`.
pub(crate) fn log_row(
    t: Text<'_>,
    message: &Message,
    fell_back_to: Option<NotifyChannel>,
) -> LogRow {
    let (status, tone) = status_label(t, message.status, fell_back_to);
    let recipient = match (&message.address, message.recipient_kind) {
        (Some(address), _) => address.clone(),
        (None, RecipientKind::Party) => format!("party #{}", message.party_id.unwrap_or_default()),
        (None, RecipientKind::User) => format!("user #{}", message.user_id.unwrap_or_default()),
        (None, RecipientKind::Contact) => {
            format!("party #{}", message.party_id.unwrap_or_default())
        }
        (None, RecipientKind::Address) => t("none"),
    };
    let last_error = [
        message.last_error_code.as_deref(),
        message.last_error.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    LogRow {
        id: message.id,
        created_at: message.created_at,
        type_key: message.type_key.clone(),
        subject: format!("{} #{}", message.subject_type, message.subject_id),
        recipient,
        channel: channel_label(t, message.channel),
        status,
        tone,
        attempts: message.attempts,
        last_error,
    }
}

/// The log's counters, over every message.
#[derive(Serialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LogCounts {
    /// `sent`, `delivered` and `read` only.
    pub sent: u64,
    /// `queued`, `deferred` and `failed` (waiting for a retry).
    pub pending: u64,
    /// `abandoned`: gave up or fell back.
    pub not_delivered: u64,
    /// `suppressed`.
    pub suppressed: u64,
}

impl LogCounts {
    /// The counter `status` adds to.
    fn bucket(&mut self, status: MessageStatus) -> &mut u64 {
        match status {
            s if counts_as_sent(s) => &mut self.sent,
            MessageStatus::Abandoned => &mut self.not_delivered,
            MessageStatus::Suppressed => &mut self.suppressed,
            _ => &mut self.pending,
        }
    }

    /// Counts the messages of the database.
    async fn of(db: &Db) -> Result<LogCounts> {
        let mut counts = LogCounts::default();
        for status in MessageStatus::ALL.iter().copied() {
            let n = Message::query()
                .where_(Message::STATUS.eq(status))
                .count(db)
                .await?;
            *counts.bucket(status) += n;
        }
        Ok(counts)
    }
}

/// What the log page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-notify/log.html")]
pub(crate) struct LogPage {
    tx: Texts,
    counts: LogCounts,
    messages: GridPage<Message>,
}

/// The log grid: search on the type, recipient, reference and error, filters
/// by channel, status and date. The query stays cheap: newest first by the
/// primary key, no join. What a given-up message did next is drawn by the
/// template (`custom`).
fn log_grid(lang: &Lang) -> Grid {
    let t = texts_in(lang);
    let channels: Vec<(String, String)> = NotifyChannel::ALL
        .iter()
        .map(|c| (c.as_str().to_owned(), channel_label(&t, *c)))
        .collect();
    let statuses: Vec<(String, String)> = MessageStatus::ALL
        .iter()
        .map(|s| {
            let label = match s {
                MessageStatus::Abandoned => t("counters.not_delivered"),
                other => t(&format!("status.{}", other.as_str())),
            };
            (s.as_str().to_owned(), label)
        })
        .collect();
    Grid::new("notify-log")
        .title(&t("log.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(Column::number("id", &t("fields.id")).hidden())
        .column(Column::datetime("created_at", &t("fields.created_at")).mobile())
        .column(
            Column::text("type_key", &t("fields.key"))
                .frozen()
                .mobile()
                .searchable()
                .copyable(),
        )
        .column(Column::custom("subject", &t("fields.subject")))
        .column(Column::text("subject_label", &t("fields.reference")).searchable().hidden())
        .column(
            Column::text("address", &t("fields.recipient"))
                .computed(
                    "COALESCE({T}.\"address\", \
                     CASE WHEN {T}.\"user_id\" IS NOT NULL THEN 'user #' || CAST({T}.\"user_id\" AS TEXT) \
                     WHEN {T}.\"party_id\" IS NOT NULL THEN 'party #' || CAST({T}.\"party_id\" AS TEXT) END, \
                     '\u{2014}')",
                )
                .mobile()
                .searchable(),
        )
        .column(Column::select("channel", &t("fields.channel"), channels).mobile())
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("queued", "neutral"),
                    ("deferred", "neutral"),
                    ("suppressed", "neutral"),
                    ("sent", "info"),
                    ("delivered", "success"),
                    ("read", "success"),
                    ("failed", "danger"),
                    ("abandoned", "danger"),
                ])
                .mobile(),
        )
        .column(Column::custom("outcome", &t("fields.outcome")))
        .column(Column::number("attempts", &t("fields.attempts")).hidden())
        .column(
            Column::text("last_error", &t("fields.last_error"))
                .computed(
                    "NULLIF(TRIM(COALESCE({T}.\"last_error_code\", '') || ' ' || COALESCE({T}.\"last_error\", '')), '')",
                )
                .searchable(),
        )
        .sort_by("-id")
        .cards_on_mobile()
        .exports()
        .empty_state(&t("log.empty"), Some(&t("log.empty_hint")))
}

/// `GET /notification-log`: the messages, newest first, paged by the grid
/// (no cap: older messages are on the next pages), with their status labels,
/// and the counters.
async fn log_index(State(db): State<Db>, lang: Lang, request: GridRequest) -> Result<Response> {
    let t = texts_in(&lang);
    // The route needs `notifications.log.view`, so the export is open.
    let grid = log_grid(&lang);
    let query = Message::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    // What took over from each given-up message of this page.
    let abandoned: Vec<i64> = page
        .items()
        .iter()
        .filter(|m| m.status == MessageStatus::Abandoned)
        .map(|m| m.id)
        .collect();
    let mut fell_back: HashMap<i64, NotifyChannel> = HashMap::new();
    if !abandoned.is_empty() {
        for next in Message::query()
            .where_(Message::FALLBACK_OF.is_in(abandoned))
            .get(&db)
            .await?
        {
            if let Some(of) = next.fallback_of {
                fell_back.insert(of, next.channel);
            }
        }
    }
    let page = page.extend(|m| {
        let row = log_row(&t, m, fell_back.get(&m.id).copied());
        json!({
            "subject": row.subject,
            "outcome": (m.status == MessageStatus::Abandoned).then_some(row.status),
            "tone": row.tone,
        })
    });
    Ok(View::typed(LogPage {
        tx: texts::all(&lang),
        counts: LogCounts::of(&db).await?,
        messages: page,
    })
    .into_response())
}

/// The methods a type accepts, as `(code, label)` options of a select.
fn method_options(t: Text<'_>, methods: &[ProofMethod]) -> Vec<(String, String)> {
    methods
        .iter()
        .map(|m| {
            (
                m.as_str().to_owned(),
                t(&format!("proof_method.{}", m.as_str())),
            )
        })
        .collect()
}

/// The words a notice is called by: the request's label for the subject, else
/// `property.lease #7`.
fn notice_subject(notice: &StatutoryNotice) -> String {
    notice
        .subject_label
        .clone()
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| format!("{} #{}", notice.subject_type, notice.subject_id))
}

/// A date as people read it in the back office: `2026-09-01`.
fn day(at: DateTime) -> String {
    notices::local_date(at).format("%Y-%m-%d").to_string()
}

/// One notice of the banner a vertical page shows.
#[derive(Serialize, Debug, Clone)]
pub struct BannerItem {
    /// The notice's id (the sheet's id).
    pub id: i64,
    /// "Notice not delivered: Lease 7".
    pub message: String,
    /// "Since 2026-09-01".
    pub since: String,
    /// Where the proof is recorded.
    pub path: String,
    /// The proof methods the type accepts; empty hides the button.
    pub methods: Vec<(String, String)>,
}

/// What `wl-notify/_notice_banner.html` needs, as `notice_banner`.
#[derive(Serialize, Debug, Clone)]
pub struct NoticeBanner {
    /// The package texts of the sheet, as `tx['proof.method']`.
    pub tx: Texts,
    /// The notices not delivered; empty shows nothing.
    pub items: Vec<BannerItem>,
}

/// The "notice not delivered" banner of `subject` (A10): one item per
/// statutory notice that gave up and has neither a proof nor a delivery.
/// A vertical page puts it in its view data as `notice_banner` and includes
/// `wl-notify/_notice_banner.html`. The button shows only to holders of
/// `notifications.notices.record_proof`.
///
/// # Errors
///
/// Database errors.
pub async fn notice_banner(db: &Db, lang: &Lang, subject: &Subject) -> Result<NoticeBanner> {
    let t = texts_in(lang);
    let mut items = Vec::new();
    for notice in notices::undelivered(db, subject).await? {
        let methods = type_def(db, &notice.type_key).await?.proof_methods;
        items.push(BannerItem {
            id: notice.notice,
            message: t("notice.not_delivered").replace(":label", &notice.label),
            since: t("notice.since").replace(":date", &day(notice.gave_up_at)),
            path: notice.proof_path,
            methods: method_options(&t, &methods),
        });
    }
    Ok(NoticeBanner {
        tx: texts::all(lang),
        items,
    })
}

/// A proof once recorded.
#[derive(Serialize, Debug, Clone)]
pub struct RecordedProof {
    /// "hand delivery".
    pub method: String,
    /// The delivery date.
    pub delivered_on: String,
    /// The note, or empty.
    pub note: String,
    /// The attachment's name, or empty.
    pub attachment: String,
    /// Where the attachment is downloaded; empty without one.
    pub file_path: String,
}

/// What the proof page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-notify/notice_proof.html")]
pub(crate) struct NoticeProofPage {
    tx: Texts,
    notice: i64,
    subject: String,
    type_key: String,
    gave_up_on: String,
    methods: Vec<(String, String)>,
    notice_banner: NoticeBanner,
    proof: Option<RecordedProof>,
}

/// `GET /statutory-notices/{id}/proof`: the form offering the type's proof
/// methods, or the proof once recorded.
async fn proof_page(
    State(db): State<Db>,
    lang: Lang,
    Found(notice): Found<StatutoryNotice>,
) -> Result<View> {
    let t = texts_in(&lang);
    let methods = type_def(&db, &notice.type_key).await?.proof_methods;
    let proof = match (notice.proof_method, notice.delivered_on) {
        (Some(method), Some(delivered_on)) => Some(RecordedProof {
            method: t(&format!("proof_method.{}", method.as_str())),
            delivered_on: delivered_on.format("%Y-%m-%d").to_string(),
            note: notice.proof_note.clone().unwrap_or_default(),
            attachment: notice.attachment_name.clone().unwrap_or_default(),
            file_path: if notice.attachment_key.is_some() {
                format!("{}/file", notice.proof_path())
            } else {
                String::new()
            },
        }),
        _ => None,
    };
    // The page is where the proof is recorded, so its own banner has no button.
    let mut banner = notice_banner(&db, &lang, &notice.subject()).await?;
    banner.items.retain(|item| item.id == notice.id);
    for item in &mut banner.items {
        item.methods.clear();
    }
    Ok(View::typed(NoticeProofPage {
        tx: texts::all(&lang),
        notice: notice.id,
        subject: notice_subject(&notice),
        type_key: notice.type_key.clone(),
        gave_up_on: day(notice.gave_up_at),
        methods: method_options(&t, &methods),
        notice_banner: banner,
        proof,
    }))
}

/// The proof form (multipart).
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ProofForm {
    /// How the notice was delivered: `courier`, `registered_mail`,
    /// `hand_delivery` or `call`.
    #[serde(default)]
    pub method: Option<String>,
    /// The delivery date, `YYYY-MM-DD`.
    #[serde(default)]
    pub delivered_on: Option<String>,
    /// The note.
    #[validate(max = 2000)]
    #[serde(default)]
    pub note: Option<String>,
    /// The attachment: a signed note, a receipt or a photo.
    #[serde(default)]
    pub files: Vec<Upload>,
}

/// A 422 with `message` under `field`.
fn invalid(field: &str, message: String, input: &impl Serialize) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(input))
}

/// `POST /statutory-notices/{id}/proof`: records the proof and reloads the
/// page. A refused proof is a 422 under its field and stores nothing; a
/// notice that already has a proof, or was delivered, is a 409.
async fn proof_store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ProofForm>,
) -> Result<Response> {
    let delivered_on = match form.delivered_on.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(value) => match renox::chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
            Ok(date) => Some(date),
            Err(_) => {
                let message = texts::t(&lang, "wl_notify.proof.date_invalid");
                return Err(invalid("delivered_on", message, &form));
            }
        },
    };
    let Some(method) = form.method.as_deref().and_then(notices::parse_method) else {
        // An unknown notice is still a 404.
        StatutoryNotice::find(&state.db, id)
            .await?
            .ok_or(Error::NotFound)?;
        let refusal = ProofRefusal::MethodNotOffered;
        let message = texts::t(&lang, refusal.text_key());
        return Err(invalid(refusal.field(), message, &form));
    };
    // A browser sends an empty file part when none was chosen.
    let file = form.files.iter().find(|file| file.size() > 0);
    let input = ProofInput {
        method,
        delivered_on,
        note: form.note.clone().unwrap_or_default(),
        attachment: file.map(|file| ProofFile {
            name: file.file_name().trim().to_owned(),
            mime: file.sniffed_type().map_or_else(
                || file.content_type().trim().to_owned(),
                std::borrow::ToOwned::to_owned,
            ),
            size: file.size(),
        }),
    };
    match notices::record_proof(&state, user.id, id, input, file).await? {
        Ok(_) => Ok((
            Toast::success(texts::t(&lang, "wl_notify.proof.recorded")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(ProofError::Refused(refusal)) => {
            let message = texts::t(&lang, refusal.text_key());
            Err(invalid(refusal.field(), message, &form))
        }
        Err(ProofError::AlreadyRecorded) => Err(Error::Status(
            StatusCode::CONFLICT,
            texts::t(&lang, "wl_notify.proof.already_recorded"),
        )),
    }
}

/// `GET /statutory-notices/{id}/proof/file`: the attachment of the proof; 404
/// without one.
async fn proof_file(
    State(state): State<AppState>,
    Found(notice): Found<StatutoryNotice>,
) -> Result<Download> {
    let (Some(key), Some(name)) = (notice.attachment_key, notice.attachment_name) else {
        return Err(Error::NotFound);
    };
    Download::from_storage(&state.storage, &key, name).await
}

/// One channel's row on the send-windows page.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct WindowRow {
    /// The channel key, e.g. `sms`.
    pub channel: &'static str,
    /// "SMS".
    pub label: String,
    /// The opening time as `HH:MM`.
    pub start: String,
    /// The closing time as `HH:MM`.
    pub end: String,
    /// The hint under the times: the SMS limit, or the general rule.
    pub hint: String,
}

/// What the send-windows page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-notify/send_windows.html")]
pub(crate) struct SendWindowsPage {
    tx: Texts,
    windows: Vec<WindowRow>,
}

/// Minutes after midnight as `HH:MM`.
fn clock_text(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

/// `HH:MM` as minutes after midnight; `24:00` is the end of the day.
fn parse_clock(text: &str) -> Option<u32> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let (hours, minutes): (u32, u32) = (hours.parse().ok()?, minutes.parse().ok()?);
    (minutes < 60 && (hours < 24 || (hours == 24 && minutes == 0))).then_some(hours * 60 + minutes)
}

/// `GET /notify/send-windows`: each channel's window, the default where none
/// is stored.
async fn send_windows_index(State(db): State<Db>, lang: Lang) -> Result<View> {
    let t = texts_in(&lang);
    let mut windows = Vec::new();
    for channel in NotifyChannel::ALL {
        let current = SendWindow::for_channel(&db, *channel).await?;
        windows.push(WindowRow {
            channel: channel.as_str(),
            label: channel_label(&t, *channel),
            start: clock_text(current.start),
            end: clock_text(current.end),
            hint: t(if *channel == NotifyChannel::Sms {
                "send_windows.sms_hint"
            } else {
                "send_windows.hint"
            }),
        });
    }
    Ok(View::typed(SendWindowsPage {
        tx: texts::all(&lang),
        windows,
    }))
}

/// The send-window form: one channel, its opening and closing time.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct SendWindowForm {
    /// `email`, `sms` or `whatsapp`.
    pub channel: String,
    /// The opening time, `HH:MM`.
    pub start: String,
    /// The closing time, `HH:MM`.
    pub end: String,
}

/// `POST /notify/send-windows`: saves one channel's window. A window the
/// rules refuse (start not before end, an SMS window outside 07:00-21:00) is
/// a 422 with the message, and the stored window is unchanged.
async fn send_windows_update(
    State(db): State<Db>,
    lang: Lang,
    Form(form): Form<SendWindowForm>,
) -> Result<Response> {
    let channel = NotifyChannel::ALL
        .iter()
        .copied()
        .find(|c| c.as_str() == form.channel)
        .ok_or(Error::NotFound)?;
    let (Some(start), Some(end)) = (parse_clock(&form.start), parse_clock(&form.end)) else {
        return Err(invalid(
            "end",
            texts::t(&lang, "wl_notify.send_windows.bad_time"),
            &form,
        ));
    };
    match SendWindow::save_checked(&db, channel, Window { start, end }).await? {
        Ok(_) => Ok((
            Toast::success(texts::t(&lang, "wl_notify.send_windows.saved")),
            Redirect::route("wl_notify.send_windows", &[])?,
        )
            .into_response()),
        Err(refusal) => Err(invalid("end", refusal.message().to_owned(), &form)),
    }
}

/// The badge tone of a broadcast status.
fn broadcast_tone(status: BroadcastStatus) -> &'static str {
    match status {
        BroadcastStatus::PendingApproval => "warning",
        BroadcastStatus::Completed => "success",
        BroadcastStatus::Cancelled => "danger",
        BroadcastStatus::Draft | BroadcastStatus::Scheduled | BroadcastStatus::Sending => "info",
    }
}

/// The text key of a broadcast status.
fn broadcast_status_key(status: BroadcastStatus) -> &'static str {
    match status {
        BroadcastStatus::Draft => "broadcast.status.draft",
        BroadcastStatus::PendingApproval => "broadcast.status.pending_approval",
        BroadcastStatus::Scheduled => "broadcast.status.scheduled",
        BroadcastStatus::Sending => "broadcast.status.sending",
        BroadcastStatus::Completed => "broadcast.status.completed",
        BroadcastStatus::Cancelled => "broadcast.status.cancelled",
    }
}

/// Whether `user` may see and cancel `broadcast`: its requester, or a holder
/// of `notifications.marketing.send`.
fn may_handle(user: &AuthUser, broadcast: &Broadcast) -> bool {
    user.id == broadcast.requested_by || user.has_permission(permissions::MARKETING_SEND)
}

/// The broadcasts list.
#[derive(Serialize, ViewData)]
#[view(template = "wl-notify/broadcasts.html")]
pub(crate) struct BroadcastsPage {
    tx: Texts,
    broadcasts: GridPage<Broadcast>,
}

/// The broadcasts grid: search on the type, filters by status, channel and
/// send time. Only holders of `notifications.marketing.send` export.
fn broadcasts_grid(lang: &Lang, can_export: bool) -> Grid {
    let t = texts_in(lang);
    let statuses: Vec<(String, String)> = BroadcastStatus::ALL
        .iter()
        .map(|s| (s.as_str().to_owned(), t(broadcast_status_key(*s))))
        .collect();
    let channels: Vec<(String, String)> = NotifyChannel::ALL
        .iter()
        .map(|c| (c.as_str().to_owned(), channel_label(&t, *c)))
        .collect();
    let grid = Grid::new("notify-broadcasts")
        .title(&t("broadcasts.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(Column::number("id", &t("fields.id")).hidden())
        .column(
            Column::text("type_key", &t("fields.key"))
                .frozen()
                .mobile()
                .searchable()
                .link("/notify/broadcasts/{id}"),
        )
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("draft", "info"),
                    ("pending_approval", "warning"),
                    ("scheduled", "info"),
                    ("sending", "info"),
                    ("completed", "success"),
                    ("cancelled", "danger"),
                ])
                .mobile(),
        )
        .column(Column::select("channel", &t("fields.channel"), channels).mobile())
        .column(Column::number("estimate", &t("broadcasts.estimate")))
        .column(Column::datetime("send_at", &t("broadcasts.send_at")).mobile())
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .sort_by("-id")
        .row_url("/notify/broadcasts/{id}")
        .cards_on_mobile()
        .empty_state(&t("broadcasts.empty"), Some(&t("broadcasts.empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// `GET /notify/broadcasts`: newest first. A holder sees every broadcast,
/// anyone else only the ones they asked for.
async fn broadcasts_index(user: AuthUser, lang: Lang, request: GridRequest) -> Result<Response> {
    let holder = user.has_permission(permissions::MARKETING_SEND);
    let grid = broadcasts_grid(&lang, holder);
    if request.param("export").is_some() && !holder {
        return Err(Error::Forbidden);
    }
    let mut query = Broadcast::query();
    if !holder {
        query = query.where_(Broadcast::REQUESTED_BY.eq(user.id));
    }
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    Ok(View::typed(BroadcastsPage {
        tx: texts::all(&lang),
        broadcasts: grid.page(query, &request).await?,
    })
    .into_response())
}

/// A count with its label (an excluded reason, a result).
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CountRow {
    /// "No consent".
    pub label: String,
    /// How many.
    pub count: i64,
}

/// One broadcast's page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-notify/broadcast.html")]
pub(crate) struct BroadcastPage {
    tx: Texts,
    broadcast: Broadcast,
    status: String,
    tone: &'static str,
    channel: String,
    /// The template text of the type for the channel, or "—".
    template: String,
    /// Which audience and who asked, as words; never party names.
    rule: String,
    excluded: Vec<CountRow>,
    results: Vec<CountRow>,
    approvals: Vec<ApprovalCard>,
    can_cancel: bool,
    can_stop: bool,
}

/// The rule text: the segment of the requester, as words.
fn rule_text(t: Text<'_>, b: &Broadcast) -> String {
    t("broadcasts.rule")
        .replace(":segment", &b.segment_id.to_string())
        .replace(
            ":requester",
            &format!("{} #{}", b.requester_type, b.requester_id),
        )
}

/// The broadcast `id` for `user`: 404 when it does not exist, 403 when the
/// user neither asked for it nor holds the permission.
async fn broadcast_for(db: &Db, user: &AuthUser, id: i64) -> Result<Broadcast> {
    let broadcast = Broadcast::find(db, id).await?.ok_or(Error::NotFound)?;
    if may_handle(user, &broadcast) {
        Ok(broadcast)
    } else {
        Err(Error::Forbidden)
    }
}

/// `GET /notify/broadcasts/{id}`: the estimate, the rule, the excluded
/// counts, the results, the approval card and the cancel button.
async fn broadcast_show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let t = texts_in(&lang);
    let db = state.db.clone();
    let b = broadcast_for(&db, &user, id).await?;
    let template = broadcasts::find_type(&db, &b.type_key)
        .await?
        .and_then(|def| {
            def.template_for(b.channel, &lang.locale)
                .map(|tpl| tpl.body.clone())
        })
        .unwrap_or_else(|| "—".to_owned());
    let counts = broadcast_results::excluded(&db, b.id, &b.excluded.0).await?;
    let excluded = counts
        .0
        .iter()
        .map(|(reason, count)| {
            let key = format!("broadcast.excluded.{reason}");
            let found = t(&key);
            CountRow {
                label: if found == format!("wl_notify.{key}") {
                    reason.clone()
                } else {
                    found
                },
                count: *count,
            }
        })
        .collect();
    let r = &b.results.0;
    let results = [
        ("sent", r.sent),
        ("delivered", r.delivered),
        ("failed", r.failed),
        ("opted_out", r.opted_out),
    ]
    .into_iter()
    .map(|(key, count)| CountRow {
        label: t(&format!("broadcasts.results.{key}")),
        count,
    })
    .collect();
    let approvals = approvals::cards_for(&db, &b.broadcast_id().subject(), &user, &lang).await?;
    let can_cancel = matches!(
        b.status,
        BroadcastStatus::PendingApproval | BroadcastStatus::Scheduled
    );
    let can_stop = stale::may_stop(&db, &user, &b).await?;
    Ok(View::typed(BroadcastPage {
        tx: texts::all(&lang),
        status: t(broadcast_status_key(b.status)),
        tone: broadcast_tone(b.status),
        channel: channel_label(&t, b.channel),
        template,
        rule: rule_text(&t, &b),
        excluded,
        results,
        approvals,
        can_cancel,
        can_stop,
        broadcast: b,
    }))
}

/// The cancel form: an optional reason.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct BroadcastCancelForm {
    /// Why it is cancelled.
    #[serde(default)]
    pub reason: String,
}

/// `POST /notify/broadcasts/{id}/cancel`: cancels a broadcast that waits for
/// approval or for its send time.
async fn broadcast_cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BroadcastCancelForm>,
) -> Result<Response> {
    broadcast_for(&state.db, &user, id).await?;
    broadcast_run::cancel(&state, id, &user, &form.reason).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_notify.broadcasts.cancelled")),
        Redirect::route("wl_notify.broadcast", &[&id])?,
    )
        .into_response())
}

/// `POST /notify/broadcasts/{id}/stop`: suppresses the messages of a sending
/// or completed broadcast that are not sent yet (`NTF.RUL.03`).
async fn broadcast_stop(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    broadcast_for(&state.db, &user, id).await?;
    stale::stop(&state, id, &user).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_notify.broadcasts.stopped")),
        Redirect::route("wl_notify.broadcast", &[&id])?,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_type_is_named_in_words_from_its_key() {
        assert_eq!(
            type_name("accounting.return_approval_withdrawn"),
            "Return approval withdrawn"
        );
        assert_eq!(type_name("lease.renewal"), "Renewal");
        assert_eq!(type_name("plain"), "Plain");
    }
    use renox::db::Json;
    use wl_kernel::notify::Category;
    use wl_kernel::notify::GiveUpReaction::{
        AlternativeRecipient, MarkContactInvalid, StaffTask, StatutoryNotice,
    };

    fn english(key: &str) -> String {
        texts::english(&format!("wl_notify.{key}"))
    }

    fn reminder() -> NotificationType {
        NotificationType {
            key: "invoice.reminder".into(),
            module: "invoicing".into(),
            category: Category::Reminder,
            give_up: Json(vec![AlternativeRecipient, MarkContactInvalid, StaffTask]),
            staff_role: Some("collector".into()),
            channels: Json(vec![NotifyChannel::Email, NotifyChannel::Sms]),
            ..Default::default()
        }
    }

    #[test]
    fn a_type_shows_statutory_reactions_in_run_order_and_the_staff_role() {
        let row = type_row(&english, &reminder());
        assert_eq!(row.statutory_label, "No");
        assert_eq!(
            row.give_up,
            "Mark contact invalid, Alternative recipient, Staff task"
        );
        assert_eq!(row.staff_role, "Collector");
        assert_eq!(row.channels, "Email → SMS");
        assert_eq!(row.category, "Reminder");

        let notice = NotificationType {
            statutory: true,
            give_up: Json(vec![StatutoryNotice, MarkContactInvalid]),
            staff_role: None,
            ..reminder()
        };
        let row = type_row(&english, &notice);
        assert_eq!(row.statutory_label, "Yes");
        assert_eq!(row.give_up, "Mark contact invalid, Statutory notice");
        assert_eq!(row.staff_role, "—");

        let quiet = NotificationType {
            give_up: Json(vec![GiveUpReaction::None]),
            ..reminder()
        };
        assert_eq!(type_row(&english, &quiet).give_up, "None");
    }

    #[test]
    fn the_form_keeps_run_order_and_the_reactions_it_does_not_offer() {
        let form = TypeForm {
            staff_task: true,
            mark_contact_invalid: true,
            ..Default::default()
        };
        assert_eq!(
            form.reactions(&[StatutoryNotice, MarkContactInvalid]),
            [MarkContactInvalid, StaffTask, StatutoryNotice]
        );
        assert_eq!(
            TypeForm::default().reactions(&[StaffTask]),
            [GiveUpReaction::None]
        );
    }

    #[test]
    fn the_edit_boxes_tick_the_stored_reactions_in_run_order() {
        let boxes = reaction_boxes(&english, &reminder());
        let names: Vec<_> = boxes.iter().map(|b| (b.name, b.checked)).collect();
        assert_eq!(
            names,
            [
                ("mark_contact_invalid", true),
                ("alternative_recipient", true),
                ("staff_task", true)
            ]
        );
        assert_eq!(boxes[2].label, "Staff task");
    }

    #[test]
    fn an_abandoned_message_never_reads_as_sent() {
        let (gave_up, _) = status_label(&english, MessageStatus::Abandoned, None);
        assert_eq!(gave_up, "Not delivered (gave up)");
        let (fell_back, _) =
            status_label(&english, MessageStatus::Abandoned, Some(NotifyChannel::Sms));
        assert_eq!(fell_back, "Not delivered (fell back to SMS)");
        for label in [gave_up, fell_back] {
            assert!(!label.contains("Sent") && !label.contains("Delivered"));
        }
        assert!(!counts_as_sent(MessageStatus::Abandoned));
        assert!(!counts_as_sent(MessageStatus::Failed));
        assert!(!counts_as_sent(MessageStatus::Suppressed));
        assert!(counts_as_sent(MessageStatus::Delivered));
    }

    #[test]
    fn the_counters_put_abandoned_messages_under_not_delivered() {
        let mut counts = LogCounts::default();
        for status in MessageStatus::ALL.iter().copied() {
            *counts.bucket(status) += 1;
        }
        assert_eq!(
            counts,
            LogCounts {
                sent: 3,
                pending: 3,
                not_delivered: 1,
                suppressed: 1,
            }
        );
    }

    #[test]
    fn a_log_row_shows_the_address_and_the_last_error() {
        let message = Message {
            id: 9,
            type_key: "invoice.reminder".into(),
            subject_type: "invoice".into(),
            subject_id: 12,
            channel: NotifyChannel::Email,
            address: Some("accounts@bluedunes.example".into()),
            status: MessageStatus::Abandoned,
            attempts: 1,
            last_error_code: Some("550".into()),
            last_error: Some("mailbox unknown".into()),
            ..Default::default()
        };
        let row = log_row(&english, &message, Some(NotifyChannel::Sms));
        assert_eq!(row.recipient, "accounts@bluedunes.example");
        assert_eq!(row.subject, "invoice #12");
        assert_eq!(row.channel, "Email");
        assert_eq!(row.status, "Not delivered (fell back to SMS)");
        assert_eq!(row.last_error, "550 mailbox unknown");
    }
}
