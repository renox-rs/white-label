//! "By Budi via Claude" (BL-AGT-08, decision 0006 point 6): a transition an
//! agent ran carries the assistant's name in its history row's metadata
//! ([`super::AgentCall::meta`]); a record page draws its history with
//! [`timeline`] instead of `renox::state_machine::timeline` to show it.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{MachineInfo, StateTransition};

use super::META_KEY;

/// The assistant that ran the transition of `row`, if an agent did.
pub fn via(row: &StateTransition) -> Option<&str> {
    via_meta(&row.metadata)
}

/// The assistant named in transition metadata, if any.
pub fn via_meta(metadata: &Value) -> Option<&str> {
    metadata
        .get(META_KEY)
        .and_then(|agent| agent.get("client"))
        .and_then(Value::as_str)
}

/// "by Budi via Claude", "by Budi", "via Claude" or nothing.
pub fn by_line(actor: Option<&str>, client: Option<&str>) -> Option<String> {
    match (actor, client) {
        (Some(actor), Some(client)) => Some(format!("by {actor} via {client}")),
        (Some(actor), None) => Some(format!("by {actor}")),
        (None, Some(client)) => Some(format!("via {client}")),
        (None, None) => None,
    }
}

/// The items of `rows` as `renox::state_machine::timeline` gives them, each
/// body starting with who ran it ("by Budi via Claude"), then the reason.
pub async fn timeline(
    db: &Db,
    rows: &[StateTransition],
    machine: Option<&MachineInfo>,
) -> Result<Vec<Value>> {
    let names = names(db, rows).await?;
    let mut items = renox::state_machine::timeline(rows, machine);
    for (item, row) in items.iter_mut().zip(rows) {
        let actor = row.actor_id.and_then(|id| names.get(&id)).map(String::as_str);
        let Some(line) = by_line(actor, via(row)) else {
            continue;
        };
        let body = match row.reason.as_deref().filter(|r| !r.trim().is_empty()) {
            Some(reason) => format!("{line}: {reason}"),
            None => line,
        };
        item["body"] = Value::String(body);
    }
    Ok(items)
}

/// The names of the actors of `rows`.
async fn names(db: &Db, rows: &[StateTransition]) -> Result<BTreeMap<i64, String>> {
    let mut ids: Vec<i64> = rows.iter().filter_map(|r| r.actor_id).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let users = User::query().where_in("id", ids).get(db).await?;
    Ok(users.into_iter().map(|u| (u.id, u.name)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_names_the_person_and_the_assistant() {
        assert_eq!(
            by_line(Some("Budi"), Some("Claude")).as_deref(),
            Some("by Budi via Claude")
        );
        assert_eq!(by_line(Some("Budi"), None).as_deref(), Some("by Budi"));
        assert_eq!(by_line(None, None), None);
    }

    #[test]
    fn the_assistant_is_read_from_the_metadata() {
        let meta = json!({ "agent": { "client": "Claude", "user_id": 7 } });
        assert_eq!(via_meta(&meta), Some("Claude"));
        assert_eq!(via_meta(&json!({ "approval_request": 3 })), None);
        assert_eq!(via_meta(&Value::Null), None);
    }
}
