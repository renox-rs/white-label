//! The HTML screens of the Manpower vertical: the record pages of a client
//! contract and of a manpower request, and what every screen of the vertical
//! shares. This is the pattern the next screens build in (deployments, absence
//! cases, rate cards, timesheets, the portal, the approvals inbox):
//!
//! - **One file per page** in this folder; a page is a `#[derive(Serialize,
//!   ViewData)]` struct with `tx` (every text of `manpower.screen.*`, read in
//!   the template as `tx['contract.title']`) and the data in words: dates as
//!   `01 Nov 2026` ([`date_text`]), amounts as `AED 12,500.00` ([`aed`]), a
//!   role as `Security guard` ([`role_text`]), a status as the word of the
//!   group `status` ([`crate::texts::word`]) with its badge tone. The
//!   template never formats a raw key.
//! - **The routes stay where they are** (`pages.rs`): a `GET` answers the page
//!   when [`wl_kernel::negotiate::wants_html`] and the same JSON as before
//!   otherwise; a `POST` runs the existing service and answers
//!   [`done`]: a toast and a 303 back to the page.
//! - **A refusal** is the service's 422 (`Error::Status`). The host's
//!   `public/forms.js` shows its message in words inside the sheet that sent
//!   the form, with the input kept; a list of failing checks is one line each.
//! - **An action** is an [`Act`]: hidden when the person may not run it, a
//!   disabled button with the reason when a guard fails now, else a sheet
//!   (`rx-action-sheet`) with its fields.
//! - **The templates** are compiled in ([`VIEWS`]) and an app replaces one by a
//!   file of the same name under its own `resources/views/manpower/`.

pub(crate) mod absence_page;
pub(crate) mod contract_page;
pub(crate) mod deployment_page;
pub(crate) mod lists;
pub(crate) mod portal_page;
pub(crate) mod request_page;

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;

use crate::pages::back;
use crate::texts::{t, word};

/// The texts a template reads as `tx['contract.title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The templates of the screens, compiled in. An app replaces one with a file
/// of the same name under its views directory.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "manpower/contract.html",
        include_str!("../../views/manpower/contract.html"),
    ),
    (
        "manpower/absence_cases.html",
        include_str!("../../views/manpower/absence_cases.html"),
    ),
    (
        "manpower/absence_case.html",
        include_str!("../../views/manpower/absence_case.html"),
    ),
    (
        "manpower/deployment.html",
        include_str!("../../views/manpower/deployment.html"),
    ),
    (
        "manpower/request.html",
        include_str!("../../views/manpower/request.html"),
    ),
    (
        "manpower/portal_requests.html",
        include_str!("../../views/manpower/portal_requests.html"),
    ),
    (
        "manpower/portal_deployments.html",
        include_str!("../../views/manpower/portal_deployments.html"),
    ),
    (
        "manpower/portal_timesheets.html",
        include_str!("../../views/manpower/portal_timesheets.html"),
    ),
];

/// A date as `01 Nov 2026`.
pub(crate) fn date_text(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

/// A date, or `fallback` when there is none.
pub(crate) fn date_or(date: Option<NaiveDate>, fallback: &str) -> String {
    date.map_or_else(|| fallback.to_owned(), date_text)
}

/// A period as `01 Jan 2026 - 31 Dec 2026`, or `15 Nov 2026 - Open-ended`.
pub(crate) fn period_text(lang: &Lang, start: NaiveDate, end: Option<NaiveDate>) -> String {
    let end = end.map_or_else(|| t(lang, "manpower.screen.open_ended"), date_text);
    format!("{} - {end}", date_text(start))
}

/// An amount in fils as `AED 12,500.00`.
pub(crate) fn aed(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let fils = fils.abs();
    let whole = (fils / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}AED {grouped}.{:02}", fils % 100)
}

/// A job role as words: `security_guard` is `Security guard`.
pub(crate) fn role_text(role: &str) -> String {
    let words = role.replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A document kind as words: `residence_visa` is `Residence visa`,
/// `emirates_id` is `Emirates ID`.
pub(crate) fn kind_label(kind: &str) -> String {
    let words: Vec<String> = kind
        .split('_')
        .map(|word| {
            if matches!(word, "id" | "sira") {
                word.to_uppercase()
            } else {
                word.to_owned()
            }
        })
        .collect();
    let text = words.join(" ");
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The name of a party, else `fallback`.
pub(crate) async fn party_text(
    state: &AppState,
    lang: &Lang,
    id: Option<i64>,
    fallback: &str,
) -> String {
    let Some(id) = id else {
        return fallback.to_owned();
    };
    wl_kernel::parties::directory(state)
        .display_name(&state.db, PartyId(id), &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| fallback.to_owned())
}

/// The name of a user, else `fallback` (the system, a user who is gone).
pub(crate) async fn user_text(state: &AppState, id: Option<i64>, fallback: &str) -> String {
    let Some(id) = id else {
        return fallback.to_owned();
    };
    User::find(&state.db, id)
        .await
        .ok()
        .flatten()
        .map_or_else(|| fallback.to_owned(), |user| user.name)
}

/// The word of a state transition in the activity list (`activate` is
/// `Activated`); the name in words when it has none.
pub(crate) fn transition_text(lang: &Lang, name: &str) -> String {
    word(lang, "transition", name)
}

/// What an action does now, for the person looking at the page.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub(crate) struct Act {
    /// Whether the person may run it at all; a button nobody may press is not
    /// drawn.
    pub shown: bool,
    /// While not empty, a guard fails now: the button is drawn disabled with
    /// this reason in words.
    pub blocked: String,
}

impl Act {
    /// Not offered to this person.
    pub(crate) fn hidden() -> Self {
        Self::default()
    }

    /// Offered and ready.
    pub(crate) fn ready() -> Self {
        Act {
            shown: true,
            blocked: String::new(),
        }
    }

    /// Offered when `shown`, ready.
    pub(crate) fn when(shown: bool) -> Self {
        Act {
            shown,
            blocked: String::new(),
        }
    }

    /// Offered but a guard fails: disabled with `reason`.
    pub(crate) fn blocked(reason: String) -> Self {
        Act {
            shown: true,
            blocked: reason,
        }
    }

    /// Offered, disabled with `reason` when there is one.
    pub(crate) fn unless(reason: Option<String>) -> Self {
        match reason {
            Some(reason) => Act::blocked(reason),
            None => Act::ready(),
        }
    }
}

/// A form post that worked: the toast `key` and a 303 back to the page `url`.
pub(crate) fn done(lang: &Lang, url: &str, key: &str) -> Response {
    (Toast::success(t(lang, key)), back(url)).into_response()
}

/// The options of a select as `(value, label)` pairs from `(value, text key)`
/// pairs: `words("manpower.screen.hold_reason", &["overdue", ...])`.
pub(crate) fn options(lang: &Lang, group: &str, values: &[&str]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|value| ((*value).to_owned(), word(lang, group, value)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tx['name']` a template reads is a text of the app: a missing
    /// one would show as nothing, or as a raw key, on the page.
    #[test]
    fn every_text_a_template_reads_exists() {
        let all: Vec<&str> = crate::texts::screen_text_keys();
        for (name, source) in VIEWS {
            let mut rest = *source;
            while let Some(at) = rest.find("tx['") {
                rest = &rest[at + 4..];
                let end = rest.find('\'').expect("a closed key");
                let key = &rest[..end];
                assert!(
                    all.contains(&key),
                    "{name} reads `{key}`, which has no text"
                );
            }
        }
    }

    #[test]
    fn amounts_dates_and_roles_are_in_words() {
        assert_eq!(aed(1_250_000), "AED 12,500.00");
        assert_eq!(aed(5), "AED 0.05");
        assert_eq!(aed(-150_000), "-AED 1,500.00");
        assert_eq!(
            date_text(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()),
            "01 Nov 2026"
        );
        assert_eq!(role_text("security_guard"), "Security guard");
        assert_eq!(role_text(""), "");
    }
}
