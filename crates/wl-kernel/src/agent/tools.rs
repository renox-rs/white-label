//! The agent tools: what the MCP connector (BL-AGT-07, `/mcp`) serves.
//!
//! A tool is a name, a description, a JSON schema of its arguments and a
//! handler taking the [`AgentCall`] and the arguments. The kernel registers
//! the BL-AGT-08 and BL-AGT-09 tools at boot:
//!
//! | Tool | Arguments | Returns |
//! |---|---|---|
//! | `list_actions` | optional `type` | the registered agent actions |
//! | `preview_action` | `action`, `record` (`invoicing.invoice:42` or 42), `arguments` | the change, `requires_approval`, a token valid 10 minutes |
//! | `confirm_action` | `token` | the result, the same on a repeated confirm |
//! | `inbox` | optional `since` (RFC 3339) | the five groups |
//! | `mark_read` | `ids` | how many were marked, the bell's count |
//!
//! Messages (BL-AGT-10) are an agent action (`<type>.send_message`,
//! registered by wl-notify), so they go through `preview_action` and
//! `confirm_action` like any other.
//!
//! **For the connector (#547):** list [`tools`] next to the read-only tools
//! and answer a call with [`call`] (it checks the agent policy first); a
//! tool's `writes` flag says whether it can change data, for the MCP tool
//! annotations (`readOnlyHint`). Another module may add a tool with
//! [`register_tool`].

use std::sync::{Arc, PoisonError, RwLock};

use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::Value;

use super::access::{self, Use};
use super::{AgentCall, actions, inbox};
use crate::machines::BoxFuture;
use crate::subject::Subject;

/// A tool's handler.
pub type ToolFn = Arc<dyn Fn(AgentCall, Value) -> BoxFuture<'static, Result<Value>> + Send + Sync>;

/// One tool.
#[derive(Clone)]
pub struct AgentTool {
    /// Its name, e.g. `preview_action`.
    pub name: String,
    /// What it does, for the assistant.
    pub description: String,
    /// The JSON schema of its arguments.
    pub input_schema: Value,
    /// Whether it can change data (only through a confirm).
    pub writes: bool,
    handler: ToolFn,
}

impl AgentTool {
    /// A tool called `name`.
    pub fn new<F>(name: &str, description: &str, input_schema: Value, handler: F) -> Self
    where
        F: Fn(AgentCall, Value) -> BoxFuture<'static, Result<Value>> + Send + Sync + 'static,
    {
        AgentTool {
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema,
            writes: false,
            handler: Arc::new(handler),
        }
    }

    /// Marks it as a tool that can change data.
    pub fn writes(mut self) -> Self {
        self.writes = true;
        self
    }

    /// Runs it, without the policy check ([`call`] does that).
    pub async fn run(&self, call: AgentCall, args: Value) -> Result<Value> {
        (self.handler)(call, args).await
    }
}

static TOOLS: RwLock<Vec<AgentTool>> = RwLock::new(Vec::new());

/// Adds `tool`, replacing one of the same name.
pub fn register_tool(tool: AgentTool) {
    let mut tools = TOOLS.write().unwrap_or_else(PoisonError::into_inner);
    match tools.iter_mut().find(|t| t.name == tool.name) {
        Some(existing) => *existing = tool,
        None => tools.push(tool),
    }
}

/// Every registered tool, in the order registered.
pub fn tools() -> Vec<AgentTool> {
    TOOLS.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The tool `name`, if registered.
pub fn tool(name: &str) -> Option<AgentTool> {
    TOOLS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|t| t.name == name)
        .cloned()
}

/// Calls tool `name` with `args` for `call`: refused (403) when the
/// company's agent policy is off, 404 for an unknown tool.
pub async fn call(call: &AgentCall, name: &str, args: Value) -> Result<Value> {
    access::check(call, None, Use::Read).await?;
    let Some(tool) = tool(name) else {
        return Err(Error::Status(
            StatusCode::NOT_FOUND,
            format!("There is no tool `{name}`."),
        ));
    };
    tool.run(call.clone(), args).await
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<Value> {
    renox::serde_json::to_value(value).map_err(|e| Error::from(renox::anyhow::anyhow!(e)))
}

/// The record of `preview_action`: `invoicing.invoice:42` (of the action's
/// type) or `42`.
pub fn record_id(action: &actions::AgentAction, record: Option<&Value>) -> Result<Option<i64>> {
    match record {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_i64()
            .map(Some)
            .ok_or_else(|| invalid("`record` is not a record id.")),
        Some(Value::String(text)) => {
            if let Ok(id) = text.trim().parse::<i64>() {
                return Ok(Some(id));
            }
            let subject: Subject = text
                .trim()
                .parse()
                .map_err(|_| invalid("`record` is not like invoicing.invoice:42."))?;
            if subject.subject_type != action.subject_type() {
                return Err(invalid(format!(
                    "`{}` acts on {}, not on {}.",
                    action.key(),
                    action.subject_type(),
                    subject.subject_type
                )));
            }
            Ok(Some(subject.subject_id))
        }
        Some(_) => Err(invalid("`record` is not a record id.")),
    }
}

/// `since` as an instant.
fn since_of(args: &Value) -> Result<Option<DateTime>> {
    match args.get("since").and_then(Value::as_str) {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => renox::chrono::DateTime::parse_from_rfc3339(text.trim())
            .map(|at| Some(at.with_timezone(&renox::chrono::Utc)))
            .map_err(|_| invalid("`since` is not a date and time like 2026-11-03T09:00:00+04:00.")),
    }
}

/// The kernel's tools.
pub(crate) fn register_kernel_tools() {
    register_tool(AgentTool::new(
        "list_actions",
        "The actions you may preview and confirm for the user (create a draft, issue, assign, \
         record, send a message from a template). Nothing else can be changed.",
        json!({
            "type": "object",
            "properties": { "type": { "type": "string", "description": "Only the actions on this record type, e.g. invoicing.invoice" } }
        }),
        |_call, args| {
            Box::pin(async move {
                let wanted = args.get("type").and_then(Value::as_str).map(str::to_owned);
                let list: Vec<_> = actions::registered()
                    .iter()
                    .filter(|a| wanted.as_deref().is_none_or(|t| a.subject_type() == t))
                    .map(|a| a.info())
                    .collect();
                Ok(json!({ "actions": list }))
            })
        },
    ));
    register_tool(
        AgentTool::new(
            "preview_action",
            "Shows what an action would do, writing nothing: the change in words, the amounts, \
             whether an approval will be requested, and a confirmation token valid 10 minutes. \
             Show it to the user and call confirm_action only when they agree.",
            json!({
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": { "type": "string", "description": "An action key from list_actions" },
                    "record": { "type": ["string", "integer"], "description": "The record, e.g. invoicing.invoice:42" },
                    "arguments": { "type": "object", "description": "The action's arguments" }
                }
            }),
            |call, args| {
                Box::pin(async move {
                    let key = args
                        .get("action")
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("`action` is missing."))?
                        .to_owned();
                    let action = actions::get(&key).ok_or_else(|| {
                        Error::Status(
                            StatusCode::NOT_FOUND,
                            format!("There is no agent action `{key}`; list_actions names them."),
                        )
                    })?;
                    let record = record_id(&action, args.get("record"))?;
                    let arguments = args.get("arguments").cloned().unwrap_or_else(|| json!({}));
                    let preview = actions::preview(&call, &key, record, arguments).await?;
                    to_json(&preview)
                })
            },
        )
        .writes(),
    );
    register_tool(
        AgentTool::new(
            "confirm_action",
            "Does the action a preview_action returned the token for, once. Confirming again \
             with the same token returns the same result. Refused when the record changed since \
             the preview: preview again then.",
            json!({
                "type": "object",
                "required": ["token"],
                "properties": { "token": { "type": "string" } }
            }),
            |call, args| {
                Box::pin(async move {
                    let token = args
                        .get("token")
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("`token` is missing."))?
                        .to_owned();
                    let confirmed = actions::confirm(&call, &token).await?;
                    to_json(&confirmed)
                })
            },
        )
        .writes(),
    );
    register_tool(AgentTool::new(
        "inbox",
        "What needs the user now: approvals waiting for them, overdue items, mentions, \
         notifications and failed automations, each with why, the due date (Asia/Dubai) and a \
         link. Approvals are decided by the user on their page, never by an assistant.",
        json!({
            "type": "object",
            "properties": { "since": { "type": "string", "description": "Only dated items from then on (RFC 3339)" } }
        }),
        |call, args| {
            Box::pin(async move {
                let since = since_of(&args)?;
                let found = inbox::inbox(&call, since).await?;
                to_json(&found)
            })
        },
    ));
    register_tool(
        AgentTool::new(
            "mark_read",
            "Marks the user's notifications read (ids from inbox, like notification:40).",
            json!({
                "type": "object",
                "required": ["ids"],
                "properties": { "ids": { "type": "array", "items": { "type": ["string", "integer"] } } }
            }),
            |call, args| {
                Box::pin(async move {
                    let ids: Vec<String> = args
                        .get("ids")
                        .and_then(Value::as_array)
                        .ok_or_else(|| invalid("`ids` is missing."))?
                        .iter()
                        .map(|id| match id {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        })
                        .collect();
                    let (marked, unread) = inbox::mark_read(&call, &ids).await?;
                    Ok(json!({ "marked": marked, "unread": unread }))
                })
            },
        )
        .writes(),
    );
}
