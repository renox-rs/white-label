//! The notices of a recall (wl-inventory §4 *Recall* `send_notices`, §5.7
//! step 5, issue #235): the recall owner picks affected lines and a notice
//! template and the parties are told through wl-notify, in English and
//! Arabic, one message per line.
//!
//! - [`send_notices`] (`recalls.manage`): `traced` to `traced`. The guards
//!   need a template (`customer`, `patient` or `internal`) and at least one
//!   line; the lines must belong to the recall, be `pending` and have a party.
//!   The transition marks them `sent`; the notices go out right after it.
//! - [`waive`] (`recalls.manage`, a reason): the owner decides a party need
//!   not be told. A line with no party (unknown, or a count of walk-in sales)
//!   cannot be sent and is waived to close the recall.
//! - `MessageGaveUp` ([`on_gave_up`]): a notice that could not be delivered
//!   after every retry returns its line to `pending` and gives the recall
//!   owner a task, one open task per line. A `pending` line can be sent again,
//!   with another template (another channel).
//!
//! Decisions: the message is about the line (`inventory.recall_line:7`), so
//! the give-up finds its line without a message table; if sending fails after
//! the lines were marked, the lines not yet told go back to `pending` and the
//! error is returned, so a party is never marked told without a notice (a
//! notice that went out twice is the lesser harm); the recall owner is the
//! user who opened the recall; the authority notice is not built (AU-20), the
//! recall report is the evidence.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::inventory::RECALL_LINE_SUBJECT_TYPE;
use wl_kernel::notify::{
    self, Category, MessageGaveUp, NotificationTypeDef, NotifyChannel, NotifyRequest,
};
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY};
use wl_kernel::{PartyId, Subject};

use crate::item::Item;
use crate::lot::Lot;
use crate::permissions;
use crate::recall::{NoticeInput, Recall, RecallStatus, SEND_NOTICES};
use crate::recall_line::{NoticeStatus, RecallLine};
use crate::texts::{english, said};

/// The templates, by the kind of party told.
pub const TEMPLATES: [&str; 3] = ["customer", "patient", "internal"];

/// The start of the notification type key of every recall notice.
pub const NOTICE_PREFIX: &str = "inventory.recall_notice_";

/// Whether `name` is one of [`TEMPLATES`].
pub fn is_template(name: &str) -> bool {
    TEMPLATES.contains(&name)
}

/// The notification type key of template `template`:
/// `inventory.recall_notice_patient`.
pub fn notice_key(template: &str) -> String {
    format!("{NOTICE_PREFIX}{template}")
}

/// The Arabic subject of every notice.
const SUBJECT_AR: &str = "إشعار سلامة: سحب المنتج {{ number }}";

/// The Arabic body of the notice to a customer.
const CUSTOMER_AR: &str = "نقوم بسحب المنتج {{ item }} (الدفعة {{ lot }}) الذي استلمتموه. {{ reason }}. يرجى التوقف عن استخدامه والتواصل معنا.";

/// The Arabic body of the notice to a patient.
const PATIENT_AR: &str = "يتم سحب منتج استُخدم في رعايتكم بتاريخ {{ date }} ({{ item }}، الدفعة {{ lot }}). {{ reason }}. يرجى التواصل مع العيادة لترتيب فحص.";

/// The Arabic body of the notice to staff.
const INTERNAL_AR: &str = "سحب المنتج {{ number }}: {{ item }}، الدفعة {{ lot }}، استُخدم في {{ subject }} بتاريخ {{ date }}. {{ reason }}. يرجى المراجعة واتخاذ الإجراء.";

/// The notification types of the three templates, English and Arabic.
pub fn notice_types() -> Vec<NotificationTypeDef> {
    let english_body = |template: &str| match template {
        "customer" => english("wl_inventory.recall.notice_body_customer"),
        "patient" => english("wl_inventory.recall.notice_body_patient"),
        _ => english("wl_inventory.recall.notice_body_internal"),
    };
    let arabic_body = |template: &str| match template {
        "customer" => CUSTOMER_AR,
        "patient" => PATIENT_AR,
        _ => INTERNAL_AR,
    };
    TEMPLATES
        .iter()
        .map(|template| {
            NotificationTypeDef::new(&notice_key(template), "inventory")
                .group("recalls")
                .category(Category::Transactional)
                .template(
                    NotifyChannel::Email,
                    "en",
                    Some(english("wl_inventory.recall.notice_subject")),
                    english_body(template),
                )
                .template(
                    NotifyChannel::Email,
                    "ar",
                    Some(SUBJECT_AR),
                    arabic_body(template),
                )
        })
        .collect()
}

/// Registers the notification types with the kernel registry.
pub(crate) fn register_types() {
    for def in notice_types() {
        notify::register_type(def);
    }
}

/// The source key of the owner's task about line `line_id`.
pub fn task_key(line_id: i64) -> String {
    format!("inventory.recall_notice:{line_id}")
}

/// The notification asked for about `line` with `template`.
async fn request_for(
    state: &AppState,
    recall: &Recall,
    line: &RecallLine,
    template: &str,
) -> Result<NotifyRequest> {
    let item = match line.lot_id {
        Some(lot_id) => {
            let lot = Lot::find_or_404(&state.db, lot_id).await?;
            let item = Item::find_or_404(&state.db, lot.item_id).await?;
            (item.name_en.clone(), lot.number)
        }
        None => (String::new(), String::new()),
    };
    let number = recall.number.clone().unwrap_or_default();
    let party = PartyId(line.party_id.unwrap_or_default());
    Ok(NotifyRequest::new(&notice_key(template), line.subject())
        .subject_label(&number)
        .to_party(party)
        .data(renox::serde_json::json!({
            "number": number,
            "reason": recall.reason,
            "item": item.0,
            "lot": item.1,
            "subject": line.used_by().map(|s| s.to_string()).unwrap_or_default(),
            "date": line.occurred_on.map(|d| d.to_string()).unwrap_or_default(),
        })))
}

/// Puts the lines `ids` back to `pending` after a notice failed.
async fn revert(state: &AppState, ids: &[i64]) -> Result {
    let pending = NoticeStatus::Pending.as_str().to_owned();
    for id in ids {
        RecallLine::query()
            .where_(RecallLine::ID.eq(*id))
            .where_(RecallLine::NOTICE_STATUS.eq(NoticeStatus::Sent))
            .update(
                &state.db,
                &[
                    ("notice_status", &pending),
                    ("template", &None::<String>),
                    ("sent_at", &None::<DateTime>),
                    ("sent_by", &None::<i64>),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    Ok(())
}

/// `user` (a holder of `recalls.manage`) sends the notice `template` to the
/// parties of `line_ids` of recall `recall_id` (§4 `send_notices`).
///
/// # Errors
///
/// `Forbidden` without the permission; `NotFrom` unless the recall is
/// `traced`; `Guard` (422) without a known template, without lines, or for a
/// line of another recall, one that is not `pending` or one with no party; the
/// notifier's error.
pub async fn send_notices(
    state: &AppState,
    user: &AuthUser,
    recall_id: i64,
    template: &str,
    line_ids: Vec<i64>,
) -> std::result::Result<Recall, TransitionError> {
    let recall = Recall::find_or_404(&state.db, recall_id).await?;
    let template = template.trim().to_lowercase();
    let mut ids: Vec<i64> = Vec::new();
    for id in line_ids {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let mut lines = Vec::new();
    let mut problem = None;
    for id in &ids {
        match RecallLine::find(&state.db, *id).await? {
            Some(line) if line.recall_id == recall.id => lines.push(line),
            _ => problem = Some(said("wl_inventory.recall.line_unknown", &[])),
        }
    }
    for line in &lines {
        if line.notice_status != NoticeStatus::Pending {
            problem = Some(said("wl_inventory.recall.line_not_pending", &[]));
        } else if line.party_id.is_none() {
            problem = Some(said("wl_inventory.recall.line_no_party", &[]));
        }
    }
    let input = NoticeInput {
        template: template.clone(),
        lines: ids.clone(),
        by: Some(user.id),
        problem,
    };
    let _outcome = recall
        .transition(SEND_NOTICES)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    for (position, line) in lines.iter().enumerate() {
        let sent = async {
            let request = request_for(state, &recall, line, &template).await?;
            notify::notifier(state).notify(state, request).await?;
            Ok::<(), Error>(())
        }
        .await;
        if let Err(error) = sent {
            let untold: Vec<i64> = lines[position..].iter().map(|line| line.id).collect();
            revert(state, &untold).await?;
            return Err(TransitionError::Failed(error));
        }
    }
    Ok(Recall::find_or_404(&state.db, recall_id).await?)
}

/// `user` (a holder of `recalls.manage`) decides the party of line `line_id`
/// need not be told, with `reason`.
///
/// # Errors
///
/// `Forbidden` without the permission; 422 without a reason, for a recall
/// that is not `traced` or a line that is not `pending`; `NotFound`.
pub async fn waive(
    state: &AppState,
    user: &AuthUser,
    line_id: i64,
    reason: &str,
) -> Result<RecallLine> {
    if !user.has_permission(permissions::RECALLS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let line = RecallLine::find_or_404(&state.db, line_id).await?;
    let recall = Recall::find_or_404(&state.db, line.recall_id).await?;
    let refused =
        |message: String| Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message);
    let reason = reason.trim().to_owned();
    if reason.is_empty() {
        return Err(refused(said("wl_inventory.recall.waive_reason", &[])));
    }
    if recall.status != RecallStatus::Traced {
        return Err(refused(said("wl_inventory.recall.not_traced", &[])));
    }
    if line.notice_status != NoticeStatus::Pending {
        return Err(refused(said("wl_inventory.recall.line_not_pending", &[])));
    }
    let now = renox::db::now();
    RecallLine::query()
        .where_(RecallLine::ID.eq(line.id))
        .update(
            &state.db,
            &[
                ("notice_status", &NoticeStatus::Waived.as_str().to_owned()),
                ("waive_reason", &Some(reason)),
                ("waived_by", &Some(user.id)),
                ("waived_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    RecallLine::find_or_404(&state.db, line.id).await
}

/// A notice of a recall line could not be delivered: the line is `pending`
/// again and the recall owner gets a task (one open task per line).
///
/// # Errors
///
/// A database error, or the task creator's.
pub async fn on_gave_up(state: &AppState, event: &MessageGaveUp) -> Result {
    if event.subject.subject_type != RECALL_LINE_SUBJECT_TYPE
        || !event.type_key.starts_with(NOTICE_PREFIX)
    {
        return Ok(());
    }
    let Some(line) = RecallLine::find(&state.db, event.subject.subject_id).await? else {
        return Ok(());
    };
    if line.notice_status != NoticeStatus::Sent {
        return Ok(());
    }
    revert(state, &[line.id]).await?;
    let recall = Recall::find_or_404(&state.db, line.recall_id).await?;
    let key = task_key(line.id);
    let creator = tickets::task_creator(state);
    if creator.find_open(state, &key).await?.is_some() {
        return Ok(());
    }
    let number = recall.number.clone().unwrap_or_default();
    let used_by: Option<Subject> = line.used_by();
    let title = said("wl_inventory.recall.notice_task_title", &[("number", &number)]);
    let description = said(
        "wl_inventory.recall.notice_task_body",
        &[
            ("subject", &used_by.map(|s| s.to_string()).unwrap_or_default()),
            ("party", &line.party_id.unwrap_or_default().to_string()),
        ],
    );
    let mut ticket = OpenTicket::new(recall.subject(), TASK_CATEGORY, &title)
        .description(&description)
        .source_key(&key);
    if let Some(owner) = recall.opened_by {
        ticket = ticket.assign_to(Assignee::user(owner));
    }
    creator.open(state, ticket).await?;
    Ok(())
}

/// Registers the listener of `MessageGaveUp`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: MessageGaveUp, state: AppState| async move {
        on_gave_up(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_templates_are_the_three_party_types_with_both_languages() {
        for template in TEMPLATES {
            assert!(is_template(template));
        }
        assert!(!is_template("press"));
        assert_eq!(notice_key("patient"), "inventory.recall_notice_patient");
        let types = notice_types();
        assert_eq!(types.len(), 3);
    }

    #[test]
    fn the_task_is_keyed_by_the_line() {
        assert_eq!(task_key(7), "inventory.recall_notice:7");
    }
}
