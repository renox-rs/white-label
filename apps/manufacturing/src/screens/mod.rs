//! The HTML screens of the Manufacturing vertical: the work order list and the
//! record page (the pattern of `apps/manpower/src/screens`):
//!
//! - **One file per page** in this folder; a page is a `#[derive(Serialize,
//!   ViewData)]` struct with `tx` (every text of `manufacturing.screen.*`, read
//!   in the template as `tx['order.title']`) and the data in words: dates as
//!   `01 Nov 2026`, amounts as `AED 12,500.00`, quantities as `12.5`, a status
//!   as the word of the group `status` with its badge tone.
//! - **The routes stay in `pages.rs`**: a `GET` answers the page when
//!   [`wl_kernel::negotiate::wants_html`] and JSON otherwise; a `POST` runs the
//!   service and answers [`done`]: a toast and a 303 back to the page.
//! - **A refusal** is the service's 422; the host's `forms.js` shows it in
//!   words inside the sheet that sent the form.
//! - **An action** is an [`Act`]: hidden when the person may not run it, a
//!   disabled button with the reason when a guard fails now, else a sheet.
//! - **On a phone (390 px)** the pages are a stack of cards and tables inside
//!   `rx-table` wrappers; the columns that do not fit are `hide-narrow`.

pub(crate) mod ncr_page;
pub(crate) mod ncrs_page;
pub(crate) mod work_order_page;
pub(crate) mod work_orders_page;

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;

use crate::pages::back;
use crate::texts::{t, word};

/// The texts a template reads as `tx['order.title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The templates of the screens, compiled in. An app replaces one with a file
/// of the same name under its views directory.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "manufacturing/work_orders.html",
        include_str!("../../views/manufacturing/work_orders.html"),
    ),
    (
        "manufacturing/work_order.html",
        include_str!("../../views/manufacturing/work_order.html"),
    ),
    (
        "manufacturing/ncrs.html",
        include_str!("../../views/manufacturing/ncrs.html"),
    ),
    (
        "manufacturing/portal_invoices.html",
        include_str!("../../views/manufacturing/portal_invoices.html"),
    ),
    (
        "manufacturing/ncr.html",
        include_str!("../../views/manufacturing/ncr.html"),
    ),
];

/// A date as `01 Nov 2026`.
pub(crate) fn date_text(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

/// An instant as `02 Nov 2026 08:00` in Asia/Dubai.
pub(crate) fn time_text(at: renox::db::DateTime) -> String {
    at.with_timezone(&wl_kernel::BusinessCalendar::offset())
        .format("%d %b %Y %H:%M")
        .to_string()
}

/// A period as `01 Nov 2026 - 05 Nov 2026`.
pub(crate) fn period_text(lang: &Lang, start: Option<NaiveDate>, end: Option<NaiveDate>) -> String {
    match (start, end) {
        (Some(start), Some(end)) => format!("{} - {}", date_text(start), date_text(end)),
        (Some(start), None) => date_text(start),
        (None, Some(end)) => date_text(end),
        (None, None) => t(lang, "manufacturing.screen.not_set"),
    }
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

/// A quantity in thousandths as `500` or `12.5`.
pub(crate) fn quantity_text(milli: i64) -> String {
    let sign = if milli < 0 { "-" } else { "" };
    let milli = milli.abs();
    let (whole, part) = (milli / 1_000, milli % 1_000);
    if part == 0 {
        format!("{sign}{whole}")
    } else {
        let part = format!("{part:03}");
        format!("{sign}{whole}.{}", part.trim_end_matches('0'))
    }
}

/// A quantity typed as `500` or `12.5` in thousandths; `None` when it is not
/// a number with at most three decimals.
pub(crate) fn parse_quantity(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (whole, part) = text.split_once('.').unwrap_or((text, ""));
    if part.len() > 3
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !part.chars().all(|c| c.is_ascii_digit())
        || whole.is_empty()
    {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let part: i64 = format!("{part:0<3}").parse().ok()?;
    whole.checked_mul(1_000)?.checked_add(part)
}

/// The name of a user, else `fallback`.
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
        .display_name(&state.db, wl_kernel::PartyId(id), &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| fallback.to_owned())
}

/// The word of a state transition in the activity list.
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

/// The options of a select as `(value, label)` pairs.
pub(crate) fn options(lang: &Lang, group: &str, values: &[&str]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|value| ((*value).to_owned(), word(lang, group, value)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tx['name']` a template reads is a text of the app.
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
    fn quantities_and_amounts_are_in_words() {
        assert_eq!(quantity_text(500_000), "500");
        assert_eq!(quantity_text(12_500), "12.5");
        assert_eq!(quantity_text(1_050), "1.05");
        assert_eq!(parse_quantity("12.5"), Some(12_500));
        assert_eq!(parse_quantity("500"), Some(500_000));
        assert_eq!(parse_quantity("1.2345"), None);
        assert_eq!(parse_quantity("abc"), None);
        assert_eq!(parse_quantity(""), None);
        assert_eq!(aed(150_000), "AED 1,500.00");
        assert_eq!(aed(-20_000), "-AED 200.00");
    }
}
