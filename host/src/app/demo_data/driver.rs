//! The demo seeder's way into the app: requests to the app's own router,
//! signed in as a demo user with an API token, so every step goes through the
//! same routes, permissions, guards and state machines as a person in the
//! browser. A request that is refused stops the seeding with the page's own
//! message, which is how a rule that a demo record breaks shows up.

use std::collections::BTreeMap;

use renox::axum::Router;
use renox::axum::body::Body;
use renox::axum::http::{HeaderMap, Method, Request, StatusCode};
use renox::prelude::*;
use tower::ServiceExt;

/// What a request answered.
#[derive(Debug, Clone)]
pub struct Reply {
    /// The status.
    pub status: StatusCode,
    /// The headers.
    pub headers: HeaderMap,
    /// The body as text.
    pub body: String,
}

impl Reply {
    /// Whether the request went through (a page, a redirect or a refresh).
    pub fn ok(&self) -> bool {
        self.status.is_success() || self.status.is_redirection()
    }

    /// The path a redirect (or `HX-Redirect`) names.
    pub fn location(&self) -> Option<String> {
        ["hx-redirect", "location"]
            .iter()
            .find_map(|name| self.headers.get(*name))
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    /// The id at the end of the redirect's path, e.g. `/property/leases/12`.
    pub fn redirected_id(&self) -> Option<i64> {
        self.location()?
            .trim_end_matches('/')
            .rsplit('/')
            .next()?
            .parse()
            .ok()
    }

    /// The message of a refusal, without markup.
    pub fn message(&self) -> String {
        let mut text = String::new();
        let mut in_tag = false;
        for c in self.body.chars() {
            match c {
                '<' => in_tag = true,
                '>' => {
                    in_tag = false;
                    text.push(' ');
                }
                c if !in_tag => text.push(c),
                _ => {}
            }
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        text.chars().take(4000).collect()
    }
}

/// The router and the tokens of the demo users.
pub struct Driver {
    router: Router,
    tokens: BTreeMap<String, String>,
}

fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

impl Driver {
    /// A driver over `router`.
    pub fn new(router: Router) -> Self {
        Self {
            router,
            tokens: BTreeMap::new(),
        }
    }

    /// Lets requests be sent as `user`, under the name `who`.
    pub async fn sign_in(&mut self, db: &Db, who: &str, user: &User) -> Result {
        let token = user.create_token(db, "demo seed", None).await?;
        self.tokens.insert(who.to_owned(), token.plain);
        Ok(())
    }

    /// Whether `who` can be sent as.
    pub fn knows(&self, who: &str) -> bool {
        self.tokens.contains_key(who)
    }

    async fn send(
        &self,
        who: &str,
        method: Method,
        path: &str,
        form: &[(&str, &str)],
    ) -> Result<Reply> {
        let token = self
            .tokens
            .get(who)
            .ok_or_else(|| renox::anyhow::anyhow!("the demo user `{who}` is not signed in"))?;
        let body = form
            .iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let mut request = Request::builder()
            .method(method.clone())
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("hx-request", "true");
        if method == Method::POST {
            request = request.header("content-type", "application/x-www-form-urlencoded");
        }
        let response = self
            .router
            .clone()
            .oneshot(request.body(Body::from(body))?)
            .await
            .map_err(|e| renox::anyhow::anyhow!("the router failed: {e}"))?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .map_err(|e| renox::anyhow::anyhow!("the body failed: {e}"))?
            .to_bytes();
        Ok(Reply {
            status,
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }

    /// A GET as `who`.
    pub async fn get(&self, who: &str, path: &str) -> Result<Reply> {
        self.send(who, Method::GET, path, &[]).await
    }

    /// A POST of a form as `who`; whatever the app answers.
    pub async fn try_post(&self, who: &str, path: &str, form: &[(&str, &str)]) -> Result<Reply> {
        self.send(who, Method::POST, path, form).await
    }

    /// A POST of a form as `who`, which must go through: a refusal stops the
    /// seeding with its path, the user and the page's message.
    pub async fn post(&self, who: &str, path: &str, form: &[(&str, &str)]) -> Result<Reply> {
        let reply = self.try_post(who, path, form).await?;
        if !reply.ok() {
            return Err(renox::anyhow::anyhow!(
                "demo seed: POST {path} as {who} was refused ({}): {}",
                reply.status,
                reply.message()
            )
            .into());
        }
        Ok(reply)
    }
}
