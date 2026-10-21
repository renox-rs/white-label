//! One URL, two answers: a page for a browser and JSON for everything else.
//!
//! Several back-office routes began as JSON endpoints (the closing screen of
//! a project, the suppliers of an item, ...) and a client of the API relies
//! on that. A person who opens the same address in a browser needs a page. A
//! handler asks [`wants_html`] and answers the page only when the request
//! names `text/html` in its `Accept` header, which every browser does and
//! no API client does by accident; without it the answer is the JSON it
//! always was, byte for byte.
//!
//! ```ignore
//! async fn show(headers: HeaderMap, /* ... */) -> Result<Response> {
//!     if wl_kernel::negotiate::wants_html(&headers) {
//!         return Ok(View::typed(page).into_response());
//!     }
//!     json_response(&view)
//! }
//! ```

use renox::axum::http::HeaderMap;
use renox::axum::http::header::ACCEPT;

/// Whether the request asks for a page: its `Accept` header lists `text/html`
/// (with any quality other than 0), or it is an `hx-boost` navigation, which
/// replaces the page and sends `Accept: */*`.
pub fn wants_html(headers: &HeaderMap) -> bool {
    let boosted = headers
        .get("hx-boosted")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    if boosted {
        return true;
    }
    headers
        .get_all(ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|range| {
            let mut parts = range.split(';');
            let media = parts.next().unwrap_or_default().trim();
            let refused = parts.any(|parameter| {
                let parameter = parameter.trim();
                parameter
                    .strip_prefix("q=")
                    .and_then(|quality| quality.trim().parse::<f32>().ok())
                    .is_some_and(|quality| quality <= 0.0)
            });
            media.eq_ignore_ascii_case("text/html") && !refused
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(name: &'static str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(name, value.parse().unwrap());
        headers
    }

    #[test]
    fn a_browser_gets_the_page() {
        let browser = with(
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        );
        assert!(wants_html(&browser));
        assert!(wants_html(&with("accept", "TEXT/HTML; q=0.5")));
    }

    #[test]
    fn everything_else_gets_json() {
        assert!(!wants_html(&HeaderMap::new()));
        assert!(!wants_html(&with("accept", "*/*")));
        assert!(!wants_html(&with("accept", "application/json")));
        assert!(!wants_html(&with("accept", "text/html;q=0")));
        assert!(!wants_html(&with("accept", "text/htmlx")));
    }

    #[test]
    fn a_boosted_navigation_gets_the_page() {
        let mut headers = with("accept", "*/*");
        headers.insert("hx-boosted", "true".parse().unwrap());
        assert!(wants_html(&headers));
    }
}
