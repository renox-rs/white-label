//! A message from a template through an AI assistant (BL-AGT-10, decision
//! 0006 point 9, `PLT.AGT.03`): "send Al Ali a reminder about invoice 0004 on
//! WhatsApp". The assistant previews the message rendered from an approved
//! template, the user confirms, and it goes out through this module's queue,
//! within the template's own rules (consent, opt-out, quiet hours, the cap
//! per party), and is kept on the record with who sent it and through which
//! assistant.
//!
//! The action is `notify.message.send_message` (a [`wl_kernel::agent`]
//! action, so it goes through `preview_action` and `confirm_action`). Its
//! arguments are the record (`invoicing.invoice:4`), the template (a
//! notification type key) and the channel, plus the party when several are
//! linked to the record. **There is no body argument**: the text is always
//! the template's, filled from the record, so the model cannot write free
//! text; a call carrying `body`, `text` or `message` is refused.
//!
//! **Recipients**: only parties linked to the record by a declared relation
//! to `parties.party` ([`wl_kernel::related`], e.g. an invoice's customer),
//! at the party's own contact point for the channel; no other address is
//! accepted.
//!
//! **Refusals, with the channels allowed instead**: the party opted out of
//! the channel (its address is on the suppression list as an opt-out or a
//! complaint), the party asked to stop all messages, the party has no address
//! for the channel, the template has no text for it, or no provider sends on
//! it. Marketing types are not sent this way (broadcasts, BL-NTF-07).
//!
//! The placeholders a template can use: `number`, `title` and `record` (the
//! record's search title, e.g. the invoice number), `secondary`, `status`,
//! `party` and `name` (the party's legal name); a module adds more for its
//! record type with [`register_record_data`].

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::{Map, Value};
use wl_kernel::agent::{ActionKind, AgentAction, AgentCall, Change, Confirmed, Outcome, Planned};
use wl_kernel::machines::BoxFuture;
use wl_kernel::notify::{Category, NotificationTypeDef, NotifyChannel, NotifyRecipient};
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::search::Access;
use wl_kernel::{PartyId, Subject};

use crate::message::Message;
use crate::recipients::{Resolved, Route};
use crate::send::SendMessage;
use crate::suppressions::{self, SuppressionReason};

/// The action's record type.
pub const SUBJECT_TYPE: &str = "notify.message";

/// The action's name.
pub const SEND_MESSAGE: &str = "send_message";

/// The arguments that would carry free text, always refused.
pub const FREE_TEXT_ARGUMENTS: &[&str] = &["body", "text", "message", "content", "subject"];

/// More placeholders for records of one type: `(state, record id)`.
pub type RecordData = Arc<dyn Fn(AppState, i64) -> BoxFuture<'static, Result<Value>> + Send + Sync>;

static DATA: RwLock<BTreeMap<String, RecordData>> = RwLock::new(BTreeMap::new());

/// Adds placeholders for messages about records of `subject_type`; the
/// object returned is merged over the default ones.
pub fn register_record_data<F>(subject_type: &str, data: F)
where
    F: Fn(AppState, i64) -> BoxFuture<'static, Result<Value>> + Send + Sync + 'static,
{
    DATA.write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.to_owned(), Arc::new(data));
}

fn data_of(subject_type: &str) -> Option<RecordData> {
    DATA.read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(subject_type)
        .cloned()
}

/// The channel in words.
pub fn channel_words(channel: NotifyChannel) -> &'static str {
    match channel {
        NotifyChannel::Email => "e-mail",
        NotifyChannel::Sms => "SMS",
        NotifyChannel::Whatsapp => "WhatsApp",
    }
}

fn refuse(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The arguments of a send.
struct SendArgs {
    record: Subject,
    template: String,
    channel: NotifyChannel,
    party: Option<i64>,
}

impl SendArgs {
    fn read(args: &Value) -> Result<SendArgs> {
        if let Some(key) = FREE_TEXT_ARGUMENTS.iter().find(|k| args.get(**k).is_some()) {
            return Err(refuse(format!(
                "`{key}` is not accepted: a message is always an approved template, filled \
                 from the record. Pick a template instead."
            )));
        }
        let record: Subject = args
            .get("record")
            .and_then(Value::as_str)
            .and_then(|r| r.trim().parse().ok())
            .ok_or_else(|| refuse("`record` is not like invoicing.invoice:4."))?;
        let template = args
            .get("template")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| refuse("`template` (the message type) is missing."))?
            .to_owned();
        let channel_text = args
            .get("channel")
            .and_then(Value::as_str)
            .map(|c| c.trim().to_ascii_lowercase())
            .ok_or_else(|| refuse("`channel` (email, sms or whatsapp) is missing."))?;
        let channel = NotifyChannel::ALL
            .iter()
            .copied()
            .find(|c| c.as_str() == channel_text)
            .ok_or_else(|| refuse("`channel` must be email, sms or whatsapp."))?;
        let party = args.get("party").and_then(Value::as_i64);
        Ok(SendArgs {
            record,
            template,
            channel,
            party,
        })
    }
}

/// Everything a send needs, found without writing.
struct Plan {
    def: NotificationTypeDef,
    record: Subject,
    record_title: String,
    party: PartyId,
    party_name: String,
    channel: NotifyChannel,
    address: String,
    contact_point: i64,
    language: String,
    data: Value,
    subject: Option<String>,
    body: String,
}

impl Plan {
    fn fingerprint(&self) -> String {
        renox::webhook::sha256_hex(format!(
            "{}|{}|{}|{}|{}|{}",
            self.def.key,
            self.channel.as_str(),
            self.address,
            self.language,
            self.subject.as_deref().unwrap_or_default(),
            self.body
        ))
    }
}

/// The parties linked to `record` by a relation to `parties.party`.
async fn linked_parties(db: &Db, record: &Subject) -> Result<Vec<PartyId>> {
    let mut found = Vec::new();
    for relation in wl_kernel::related::registered() {
        if relation.from_type() != record.subject_type || relation.to_type() != PARTY_SUBJECT_TYPE {
            continue;
        }
        for (_, to) in relation
            .load(db, &record.subject_type, &[record.subject_id])
            .await?
        {
            let party = PartyId(to.subject_id);
            if !found.contains(&party) {
                found.push(party);
            }
        }
    }
    Ok(found)
}

/// Whether the party may get messages on `channel` at `address`; the reason
/// when not.
async fn blocked_reason(
    state: &AppState,
    party_name: &str,
    channel: NotifyChannel,
    address: &str,
) -> Result<Option<String>> {
    let words = channel_words(channel);
    Ok(
        match suppressions::reason_for(&state.db, channel, address).await? {
            Some(SuppressionReason::OptOut) => {
                Some(format!("{party_name} opted out of {words} messages."))
            }
            Some(SuppressionReason::Complaint) => Some(format!(
                "{party_name} complained about {words} messages and gets none."
            )),
            Some(SuppressionReason::HardBounce | SuppressionReason::Invalid) => Some(format!(
                "{party_name}'s {words} address does not work (it bounced or is invalid)."
            )),
            None => None,
        },
    )
}

/// Finds what a send would do, writing nothing.
async fn plan(call: &AgentCall, args: &Value) -> Result<Plan> {
    let args = SendArgs::read(args)?;
    let state = &call.state;
    let db = &state.db;
    // The user must be able to open the record, as its page.
    let access = Access::of(db, &call.user).await?;
    let Some(record_title) = wl_kernel::related::can_open(db, &access, &args.record).await? else {
        return Err(Error::NotFound);
    };
    wl_kernel::agent::access::check(
        call,
        Some(&args.record.subject_type),
        wl_kernel::agent::Use::Act,
    )
    .await?;
    let def = crate::notifier::type_def(db, &args.template)
        .await
        .map_err(|_| refuse(format!("There is no message template `{}`.", args.template)))?;
    if def.category == Category::Marketing {
        return Err(refuse(
            "Marketing messages are sent as broadcasts, not through an assistant.",
        ));
    }
    let linked = linked_parties(db, &args.record).await?;
    let party = match args.party {
        Some(wanted) if linked.contains(&PartyId(wanted)) => PartyId(wanted),
        Some(_) => {
            return Err(refuse(
                "That party is not linked to the record: a message goes only to a party of the \
                 record.",
            ));
        }
        None => match linked.as_slice() {
            [only] => *only,
            [] => return Err(refuse("No party is linked to this record.")),
            many => {
                let ids: Vec<String> = many.iter().map(|p| p.0.to_string()).collect();
                return Err(refuse(format!(
                    "Several parties are linked to this record ({}): say which with `party`.",
                    ids.join(", ")
                )));
            }
        },
    };
    let directory = wl_kernel::parties::directory(state);
    let party_name = directory
        .billing_identity(db, party)
        .await?
        .map(|p| p.legal_name_en)
        .unwrap_or_else(|| format!("party #{}", party.0));
    let contacts = directory.contacts(db, party).await?;
    let language = contacts
        .as_ref()
        .map(|c| c.language.trim().to_owned())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| crate::recipients::default_language(state));
    let open = crate::notifier::open_channels(state);
    let stopped = crate::stopped::is_stopped(db, &NotifyRecipient::Party { id: party }).await?;
    if stopped {
        return Err(refuse(format!(
            "{party_name} asked to receive no messages at all."
        )));
    }
    // The channels this party may be reached on with this template.
    let mut allowed = Vec::new();
    for &channel in &def.channels {
        let Some(point) = contacts.as_ref().and_then(|c| c.point_for(channel)) else {
            continue;
        };
        if def.template_for(channel, &language).is_none() || !open.contains(&channel) {
            continue;
        }
        if blocked_reason(state, &party_name, channel, &point.address)
            .await?
            .is_none()
        {
            allowed.push(channel_words(channel));
        }
    }
    let offer = if allowed.is_empty() {
        "No other channel is allowed for this message.".to_owned()
    } else {
        format!("Channels allowed: {}.", allowed.join(", "))
    };
    let words = channel_words(args.channel);
    if !def.channels.contains(&args.channel) || def.template_for(args.channel, &language).is_none()
    {
        return Err(refuse(format!(
            "The template `{}` has no {words} text. {offer}",
            def.key
        )));
    }
    let Some(point) = contacts
        .as_ref()
        .and_then(|c| c.point_for(args.channel))
        .cloned()
    else {
        return Err(refuse(format!(
            "{party_name} has no {words} address. {offer}"
        )));
    };
    if let Some(reason) = blocked_reason(state, &party_name, args.channel, &point.address).await? {
        return Err(refuse(format!("{reason} {offer}")));
    }
    if !open.contains(&args.channel) {
        return Err(refuse(format!(
            "No {words} provider is set up for this company. {offer}"
        )));
    }
    let data = placeholders(state, &args.record, &record_title, &party_name).await?;
    let rendered = crate::render::render(&def, args.channel, &language, &data);
    Ok(Plan {
        def,
        record: args.record,
        record_title,
        party,
        party_name,
        channel: args.channel,
        address: point.address.trim().to_owned(),
        contact_point: point.id,
        language,
        data,
        subject: rendered.subject,
        body: rendered.body,
    })
}

/// The template's placeholders for `record`.
async fn placeholders(
    state: &AppState,
    record: &Subject,
    title: &str,
    party_name: &str,
) -> Result<Value> {
    let mut data = Map::new();
    let document = wl_kernel::search::document_of(&state.db, record).await?;
    let title = document
        .as_ref()
        .map_or(title, |d| d.title.as_str())
        .to_owned();
    for key in ["number", "title", "record"] {
        data.insert(key.to_owned(), Value::String(title.clone()));
    }
    if let Some(document) = &document {
        data.insert(
            "secondary".into(),
            Value::String(document.secondary.clone()),
        );
        if let Some(status) = &document.status {
            data.insert("status".into(), Value::String(status.clone()));
        }
    }
    for key in ["party", "name"] {
        data.insert(key.to_owned(), Value::String(party_name.to_owned()));
    }
    if let Some(loader) = data_of(&record.subject_type)
        && let Value::Object(more) = loader(state.clone(), record.subject_id).await?
    {
        data.extend(more);
    }
    Ok(Value::Object(data))
}

/// Who sent a message through an assistant: "sent by Budi via Claude", for
/// the record's Activity; `None` for any other message.
pub fn sent_by(message: &Message) -> Option<String> {
    let agent = message.data.0.get("agent")?;
    let by = agent.get("by").and_then(Value::as_str)?;
    let via = agent.get("client").and_then(Value::as_str)?;
    Some(format!("sent by {by} via {via}"))
}

/// Queues the planned message, as [`crate::notifier::queue`] does for one
/// party, on the chosen channel at the party's own address.
async fn queue(call: &AgentCall, plan: &Plan) -> Result<Message> {
    let mut data = plan.data.clone();
    if let Value::Object(map) = &mut data {
        map.insert(
            "agent".into(),
            json!({ "by": call.user.name, "client": call.client, "user_id": call.user.id }),
        );
    }
    let resolved = Resolved {
        chosen: None,
        language: plan.language.clone(),
        route: Route {
            channel: plan.channel,
            address: Some(plan.address.clone()),
            contact_point: Some(plan.contact_point),
            suppress: None,
        },
    };
    let recipient = NotifyRecipient::Party { id: plan.party };
    let mut message =
        crate::notifier::compose(&plan.def, &plan.record, &recipient, &resolved, &data);
    message.subject_label = Some(plan.record_title.clone());
    // The text previewed is the text sent.
    message.rendered_subject.clone_from(&plan.subject);
    message.rendered_body.clone_from(&plan.body);
    message.data = Json(data);
    let state = &call.state;
    let mut tx = state.db.begin().await?;
    message.insert(&mut tx).await?;
    message.chain_id = Some(message.id);
    message.save(&mut tx).await?;
    state
        .queue
        .dispatch_in(
            &mut tx,
            SendMessage {
                message_id: message.id,
            },
        )
        .await?;
    tx.commit().await?;
    Ok(message)
}

/// Registers `notify.message.send_message`.
pub(crate) fn register() {
    AgentAction::custom(SUBJECT_TYPE, SEND_MESSAGE, ActionKind::Message)
        .label("Send a message from a template")
        .description(
            "Sends an approved message template about a record to a party linked to it (e.g. \
             an invoice's customer), on e-mail, SMS or WhatsApp, in the party's language. \
             There is no free text: pick the template. Consent, opt-outs and quiet hours apply.",
        )
        .needs_record(false)
        .arguments(json!({
            "type": "object",
            "required": ["record", "template", "channel"],
            "properties": {
                "record": { "type": "string", "description": "The record, e.g. invoicing.invoice:4" },
                "template": { "type": "string", "description": "The message type key, e.g. invoice.reminder" },
                "channel": { "type": "string", "enum": ["email", "sms", "whatsapp"] },
                "party": { "type": "integer", "description": "The party, when several are linked to the record" }
            },
            "additionalProperties": false
        }))
        .preview(|call, _record, args| {
            Box::pin(async move {
                let plan = plan(&call, &args).await?;
                let words = channel_words(plan.channel);
                let mut change = Change::new(format!(
                    "Send `{}` about {} to {} on {words}",
                    plan.def.key, plan.record_title, plan.party_name
                ))
                .line("To", &plan.party_name)
                .line("Channel", words)
                .line(
                    "Address",
                    crate::mask::address(plan.channel, &plan.address),
                )
                .line("Language", &plan.language);
                if let Some(subject) = &plan.subject {
                    change = change.line("Subject", subject);
                }
                let change = change.line("Message", &plan.body);
                let fingerprint = plan.fingerprint();
                Ok(Planned::new(change).fingerprint(fingerprint))
            })
        })
        .confirm(|call, _record, args, fingerprint| {
            Box::pin(async move {
                let plan = plan(&call, &args).await?;
                if plan.fingerprint() != fingerprint {
                    return Err(Error::Status(
                        StatusCode::CONFLICT,
                        wl_kernel::agent::actions::MOVED.to_owned(),
                    ));
                }
                let message = queue(&call, &plan).await?;
                let words = channel_words(plan.channel);
                Ok(Confirmed {
                    outcome: Outcome::Sent,
                    action: format!("{SUBJECT_TYPE}.{SEND_MESSAGE}"),
                    record: Some(plan.record.to_string()),
                    message: format!(
                        "The message to {} on {words} is queued; it goes out within the \
                         template's rules (quiet hours, limits).",
                        plan.party_name
                    ),
                    url: wl_kernel::agent::actions::url_of(
                        &plan.record.subject_type,
                        plan.record.subject_id,
                    ),
                    data: json!({ "message": message.id, "channel": plan.channel.as_str() }),
                })
            })
        })
        .register();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_text_is_refused() {
        let args = json!({
            "record": "invoicing.invoice:4",
            "template": "invoice.reminder",
            "channel": "whatsapp",
            "body": "Pay now or else"
        });
        assert!(SendArgs::read(&args).is_err());
    }

    #[test]
    fn the_arguments_are_read() {
        let args = json!({
            "record": "invoicing.invoice:4",
            "template": "invoice.reminder",
            "channel": "WhatsApp"
        });
        let read = SendArgs::read(&args).expect("read");
        assert_eq!(read.record, Subject::new("invoicing.invoice", 4));
        assert_eq!(read.channel, NotifyChannel::Whatsapp);
        assert_eq!(read.party, None);
    }
}
