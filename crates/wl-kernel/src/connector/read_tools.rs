//! The read-only tools of the AI connector (BL-AGT-07): `search`, `fetch`,
//! `related`, `list_reports` and `run_report`. They read through the same
//! functions as the pages ([`crate::search`], [`crate::related`]) and the
//! same [`Access`], so an assistant sees what its user sees, nothing more;
//! the company's agent policy ([`crate::agent::access`]) narrows that
//! further per record type.
//!
//! | Tool | Arguments | Returns |
//! |---|---|---|
//! | `search` | `query`, optional `type`, `limit` (≤ 20) | `id` (`subject_type:id`), title, line, status, URL |
//! | `fetch` | `id` | the search document, the module's fields (masked as on screen, outsiders' text marked), the names of the actions, URL |
//! | `related` | `id`, optional `relation` | the related records in groups |
//! | `list_reports` | — | the reports the user may run |
//! | `run_report` | `key`, optional `filters` | rows (≤ 500) and totals |
//!
//! A record the user may not open answers "not found", as its page does. A
//! record of class `health` is never sent (decision 0006 point 7). Reading
//! a sensitive record (`personal`, `financial`) is audited like a person's
//! view, with the assistant: "Viewed by Budi via Claude".

use renox::prelude::*;
use renox::serde_json::{Map, Value};

use super::fetch::fields_of;
use super::reports;
use crate::agent::AgentCall;
use crate::agent::access::{self, Use};
use crate::agent::tools::{AgentTool, register_tool};
use crate::machines;
use crate::related;
use crate::search::{self, Access, SearchDocumentRow, SearchHit, identifiers_of, searchable_type};
use crate::subject::Subject;

/// The most results `search` returns.
pub const MAX_SEARCH: u64 = 20;

/// The results `search` returns when the assistant asks no number.
pub const DEFAULT_SEARCH: u64 = 10;

/// The audit action of an assistant's read of a sensitive record.
pub const SENSITIVE_VIEWED: &str = "agent.sensitive_viewed";

/// The answer to a record the user may not open, or that does not exist.
const NOT_FOUND: &str = "Not found.";

fn invalid(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

fn not_found() -> Error {
    Error::Status(StatusCode::NOT_FOUND, NOT_FOUND.to_owned())
}

/// The access of the call's user, read once per call.
async fn access_of(call: &AgentCall) -> Result<Access> {
    Access::of(&call.state.db, &call.user).await
}

/// `path` on the client's domain.
fn absolute(call: &AgentCall, path: &str) -> String {
    format!("{}{path}", call.state.config.url.trim_end_matches('/'))
}

/// Whether the company lets assistants read `subject_type`.
async fn may_read(call: &AgentCall, subject_type: &str) -> bool {
    access::check(call, Some(subject_type), Use::Read)
        .await
        .is_ok()
}

/// The `id` argument as a subject.
fn subject_arg(args: &Value) -> Result<Subject> {
    let text = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("`id` is missing; give it like invoicing.invoice:42."))?;
    text.trim()
        .parse()
        .map_err(|_| invalid("`id` is not like invoicing.invoice:42."))
}

fn hit_json(call: &AgentCall, hit: &SearchHit) -> Value {
    json!({
        "id": hit.subject.to_string(),
        "type": hit.subject.subject_type,
        "title": hit.title,
        "secondary": hit.secondary,
        "status": hit.status,
        "voided": hit.voided,
        "matched": hit.matched,
        "url": absolute(call, &hit.url),
    })
}

/// `search`: like the search page, the best match first.
async fn search_tool(call: AgentCall, args: Value) -> Result<Value> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    if !search::searchable_query(&query) {
        return Err(invalid(format!(
            "`query` needs at least {} characters.",
            search::MIN_QUERY_CHARS
        )));
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_SEARCH)
        .clamp(1, MAX_SEARCH);
    let access = access_of(&call).await?;
    let db = &call.state.db;
    let wanted = args.get("type").and_then(Value::as_str).map(str::to_owned);
    let hits: Vec<SearchHit> = match &wanted {
        Some(subject_type) => {
            if !may_read(&call, subject_type).await {
                Vec::new()
            } else {
                search::search_type(db, &access, &query, subject_type, 1, limit)
                    .await?
                    .map(|group| group.hits)
                    .unwrap_or_default()
            }
        }
        None => {
            let results = search::search(db, &access, &query, limit).await?;
            let mut hits = Vec::new();
            for group in results.groups {
                if may_read(&call, &group.subject_type).await {
                    hits.extend(group.hits);
                }
            }
            // The groups come best first; the identifier matches lead.
            hits.sort_by_key(|hit| !hit.identifier_match);
            hits
        }
    };
    let results: Vec<Value> = hits
        .iter()
        .take(usize::try_from(limit).unwrap_or(usize::MAX))
        .map(|hit| hit_json(&call, hit))
        .collect();
    Ok(json!({ "query": query, "count": results.len(), "results": results }))
}

/// The stored document of `subject` when the user may open it and the
/// company lets assistants read its type; "not found" otherwise.
async fn readable(
    call: &AgentCall,
    access: &Access,
    subject: &Subject,
) -> Result<SearchDocumentRow> {
    let searchable = searchable_type(&subject.subject_type).ok_or_else(not_found)?;
    if !may_read(call, &subject.subject_type).await {
        return Err(not_found());
    }
    let document = search::document_of(&call.state.db, subject)
        .await?
        .filter(|d| access.sees_document(&searchable, d))
        .ok_or_else(not_found)?;
    // Health data is never sent to an assistant (decision 0006 point 7).
    if document.sensitivity == "health" {
        return Err(not_found());
    }
    Ok(document)
}

/// The names of the transitions record `subject` may take from its state
/// (the screen's buttons), apart from the system's; this connector runs
/// none of them.
async fn action_names(db: &Db, subject: &Subject) -> Result<Vec<String>> {
    let Some(machine) = machines::get(&subject.subject_type) else {
        return Ok(Vec::new());
    };
    let Some(state) = machine.current_state(db, subject.subject_id).await? else {
        return Ok(Vec::new());
    };
    Ok(machine
        .transitions()
        .iter()
        .filter(|t| !t.system && t.from.iter().any(|from| *from == state))
        .filter(|t| !machines::is_system_name(&t.name))
        .map(|t| t.name.clone())
        .collect())
}

/// Records that the call's user read the sensitive record `subject` through
/// the assistant.
async fn audit_sensitive(call: &AgentCall, subject: &Subject, sensitivity: &str) -> Result {
    let text = format!("Viewed by {} via {}", call.user.name, call.client);
    renox::audit::record(
        &call.state.db,
        renox::audit::Entry::new(SENSITIVE_VIEWED)
            .user(call.user.id)
            .subject(&subject.subject_type, subject.subject_id)
            .data(json!({
                "client": call.client,
                "via": call.client,
                "sensitivity": sensitivity,
                "text": text,
            })),
    )
    .await
}

/// `fetch`: the record as its page shows it to the user.
async fn fetch_tool(call: AgentCall, args: Value) -> Result<Value> {
    let subject = subject_arg(&args)?;
    let access = access_of(&call).await?;
    let document = readable(&call, &access, &subject).await?;
    let db = &call.state.db;
    let searchable = searchable_type(&subject.subject_type).ok_or_else(not_found)?;
    // Only the identifiers kept in clear: a sensitive one is a hash.
    let identifiers: Vec<Value> = identifiers_of(db, &subject)
        .await?
        .into_iter()
        .filter(|i| !i.hashed)
        .map(|i| json!({ "kind": i.kind, "value": i.value }))
        .collect();
    let fields = fields_of(db, &subject.subject_type, subject.subject_id)
        .await?
        .map(|f| f.fields().to_vec())
        .unwrap_or_default();
    let actions = action_names(db, &subject).await?;
    if document.sensitivity != "normal" {
        audit_sensitive(&call, &subject, &document.sensitivity).await?;
    }
    Ok(json!({
        "id": subject.to_string(),
        "type": subject.subject_type,
        "type_label": searchable.label(),
        "title": document.title,
        "secondary": document.secondary,
        "status": document.status,
        "voided": document.voided,
        "sensitivity": document.sensitivity,
        "identifiers": identifiers,
        "fields": fields,
        "actions": actions,
        "actions_note": "Action names only: this connector reads, it changes nothing.",
        "url": absolute(&call, &searchable.url_of(subject.subject_id)),
    }))
}

/// `related`: the record's related records, grouped as on its panel.
async fn related_tool(call: AgentCall, args: Value) -> Result<Value> {
    let subject = subject_arg(&args)?;
    let access = access_of(&call).await?;
    let db = &call.state.db;
    if related::can_open(db, &access, &subject).await?.is_none()
        || !may_read(&call, &subject.subject_type).await
    {
        return Err(not_found());
    }
    let wanted = args
        .get("relation")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut groups = Vec::new();
    for group in related::related(db, &access, &subject, related::PANEL_DEPTH).await? {
        if wanted.as_deref().is_some_and(|name| name != group.name) {
            continue;
        }
        if !may_read(&call, &group.to_type).await {
            continue;
        }
        let records: Vec<Value> = group
            .records
            .iter()
            .map(|r| {
                json!({
                    "id": r.subject.to_string(),
                    "title": r.title,
                    "secondary": r.secondary,
                    "status": r.status,
                    "voided": r.voided,
                    "url": absolute(&call, &r.url),
                })
            })
            .collect();
        groups.push(json!({
            "relation": group.name,
            "label": group.label,
            "type": group.to_type,
            "count": group.count,
            "records": records,
        }));
    }
    Ok(json!({ "id": subject.to_string(), "groups": groups }))
}

/// `list_reports`: the reports the user holds the permission of.
async fn list_reports_tool(call: AgentCall, _args: Value) -> Result<Value> {
    let access = access_of(&call).await?;
    let list: Vec<Value> = reports::reports()
        .iter()
        .filter(|r| access.holds(r.permission()))
        .map(reports::AgentReport::info)
        .collect();
    Ok(json!({ "reports": list }))
}

/// `run_report`: one report's rows and totals, for the user's scopes.
async fn run_report_tool(call: AgentCall, args: Value) -> Result<Value> {
    let key = args
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("`key` is missing; list_reports names the reports."))?;
    let access = access_of(&call).await?;
    let report = reports::report(key)
        .filter(|r| access.holds(r.permission()))
        .ok_or_else(not_found)?;
    let filters: Map<String, Value> = args
        .get("filters")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    report.run(&call.state.db, &access, filters).await
}

/// Registers the read-only tools next to the agent tools.
pub(crate) fn register() {
    register_tool(AgentTool::new(
        "search",
        "Searches the records the user may open, like the app's search box: document numbers, \
         TRN, phone and plate numbers match exactly first, then words in English or Arabic. \
         Returns each record's id (like invoicing.invoice:42) for fetch and related.",
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "string", "description": "A number, a name or words" },
                "type": { "type": "string", "description": "Only this record type, e.g. invoicing.invoice" },
                "limit": { "type": "integer", "minimum": 1, "maximum": MAX_SEARCH }
            }
        }),
        |call, args| Box::pin(search_tool(call, args)),
    ));
    register_tool(AgentTool::new(
        "fetch",
        "Reads one record the user may open: its fields as on its page (sensitive values \
         masked; text written by outsiders comes marked as untrusted data), its status, the \
         names of its actions and its link.",
        json!({
            "type": "object",
            "required": ["id"],
            "properties": { "id": { "type": "string", "description": "A record id from search, like invoicing.invoice:42" } }
        }),
        |call, args| Box::pin(fetch_tool(call, args)),
    ));
    register_tool(AgentTool::new(
        "related",
        "The records that belong to a record (an invoice's customer, payments, credit notes, \
         documents...), in groups, as its Related panel shows them to the user.",
        json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": { "type": "string", "description": "A record id, like invoicing.invoice:42" },
                "relation": { "type": "string", "description": "Only this group, e.g. payments" }
            }
        }),
        |call, args| Box::pin(related_tool(call, args)),
    ));
    register_tool(AgentTool::new(
        "list_reports",
        "The reports the user may run, with their filters.",
        json!({ "type": "object", "properties": {} }),
        |call, args| Box::pin(list_reports_tool(call, args)),
    ));
    register_tool(AgentTool::new(
        "run_report",
        "Runs a report for the user's scopes: at most 500 rows and the totals. Amounts are in \
         the currency's minor unit (fils for AED).",
        json!({
            "type": "object",
            "required": ["key"],
            "properties": {
                "key": { "type": "string", "description": "A report key from list_reports" },
                "filters": { "type": "object", "description": "The report's filters" }
            }
        }),
        |call, args| Box::pin(run_report_tool(call, args)),
    ));
}
