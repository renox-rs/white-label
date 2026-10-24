//! The agent tools over JSON, for a signed-in user (a session or an API
//! token): `GET /agent/tools` lists them, `POST /agent/tools/{tool}` calls
//! one with the arguments as the JSON body. The assistant's name comes in the
//! `X-Agent-Client` header (it only labels the history: "via Claude"); the
//! MCP endpoint (BL-AGT-07) calls the same [`super::tools::call`] with the
//! OAuth client's name instead.
//!
//! Every call is refused while the company's agent policy is off
//! ([`super::access`]).

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use renox::serde_json::Value;

use super::{AgentCall, access, tools};

/// The header that names the assistant.
pub const CLIENT_HEADER: &str = "x-agent-client";

/// The name of the tool call route.
pub const CALL_ROUTE: &str = "wl_kernel.agent.call";

/// The name of the tool list route.
pub const LIST_ROUTE: &str = "wl_kernel.agent.tools";

/// The routes, for signed-in users.
pub fn routes() -> Routes {
    Routes::new()
        .get("/agent/tools", list)
        .name(LIST_ROUTE)
        .post("/agent/tools/{tool}", call)
        .name(CALL_ROUTE)
        .require_auth()
}

fn client_of(headers: &HeaderMap) -> String {
    headers
        .get(CLIENT_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// `GET /agent/tools`: the tools, with their schemas.
async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let call = AgentCall::new(state, user, &client_of(&headers));
    access::check(&call, None, access::Use::Read).await?;
    let list: Vec<Value> = tools::tools()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
                "writes": t.writes,
            })
        })
        .collect();
    Ok(Json(json!({ "tools": list })))
}

/// `POST /agent/tools/{tool}`: calls it with the JSON body.
async fn call(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Path(tool): Path<String>,
    Json(args): Json<Value>,
) -> Result<Json<Value>> {
    let call = AgentCall::new(state, user, &client_of(&headers));
    let answer = tools::call(&call, &tool, args).await?;
    Ok(Json(answer))
}
