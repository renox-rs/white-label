//! The HTML pages of the construction screens: the projects list, an IPA, the
//! closing screen of a project and its variation report.
//!
//! The IPA, the closing screen and the variation report are routes that began
//! as JSON endpoints. They answer a page when the request names `text/html`
//! (a browser) and the same JSON as before otherwise
//! ([`wl_kernel::negotiate::wants_html`]). The templates are compiled into
//! the crate (`views/construction/`); an app replaces one with a file of the
//! same name under its own `resources/views/construction/`.
//!
//! Money in a template is fils, shown through the `money` filter; a link to a
//! record of another module is a path built here, so a page needs no route
//! name of a module the client does not run.

pub(crate) mod closing;
pub(crate) mod ipa;
pub(crate) mod projects;
pub(crate) mod variations;

use std::collections::BTreeMap;

use renox::prelude::*;
use wl_kernel::PartyId;

use crate::ipa::IpaStatus;
use crate::project::ProjectStatus;
use crate::texts::t;
use crate::variation::VariationStatus;

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "construction/projects.html",
        include_str!("../../views/construction/projects.html"),
    ),
    (
        "construction/ipa.html",
        include_str!("../../views/construction/ipa.html"),
    ),
    (
        "construction/closing.html",
        include_str!("../../views/construction/closing.html"),
    ),
    (
        "construction/variation_report.html",
        include_str!("../../views/construction/variation_report.html"),
    ),
];

/// The texts a template reads as `tx['projects.title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The texts named `names`, from the keys `construction.screen.<name>`.
pub(crate) fn tx(lang: &Lang, names: &[&'static str]) -> Texts {
    names
        .iter()
        .map(|name| (*name, t(lang, &format!("construction.screen.{name}"))))
        .collect()
}

/// The word for `value` in `group` (`status`, `kind`, `reason`); the value
/// itself when it has no text.
pub(crate) fn word(lang: &Lang, group: &str, value: &str) -> String {
    let key = format!("construction.screen.{group}.{value}");
    let found = t(lang, &key);
    if found == key {
        value.replace('_', " ")
    } else {
        found
    }
}

/// The badge colour of a project.
pub(crate) fn project_kind(status: ProjectStatus) -> &'static str {
    match status {
        ProjectStatus::InProgress | ProjectStatus::PracticalCompletion => "success",
        ProjectStatus::OnHold => "warning",
        ProjectStatus::Terminated => "error",
        ProjectStatus::Setup | ProjectStatus::Closed | ProjectStatus::Cancelled => "info",
    }
}

/// The badge colour of an IPA.
pub(crate) fn ipa_kind(status: IpaStatus) -> &'static str {
    match status {
        IpaStatus::Submitted => "warning",
        IpaStatus::Certified | IpaStatus::Invoiced => "success",
        IpaStatus::Draft | IpaStatus::Superseded => "info",
    }
}

/// The badge colour of a variation.
pub(crate) fn variation_kind(status: VariationStatus) -> &'static str {
    match status {
        VariationStatus::Submitted | VariationStatus::Claimed => "warning",
        VariationStatus::Approved => "success",
        VariationStatus::Rejected | VariationStatus::Abandoned => "error",
        VariationStatus::Draft | VariationStatus::Withdrawn => "info",
    }
}

/// The badge colour of a deduction's status (`undecided`, `accepted`,
/// `contested`).
pub(crate) fn deduction_kind(status: &str) -> &'static str {
    match status {
        "accepted" => "success",
        "contested" => "error",
        _ => "warning",
    }
}

/// The name of a party, else `#id`.
pub(crate) async fn party_name(state: &AppState, lang: &Lang, id: i64) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, PartyId(id), &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{id}"))
}

/// The page of an IPA.
pub(crate) fn ipa_url(id: i64) -> String {
    format!("/construction/ipas/{id}")
}

/// The closing screen of a project.
pub(crate) fn closing_url(id: i64) -> String {
    format!("/construction/projects/{id}/closing")
}

/// The variation report of a project.
pub(crate) fn variations_url(id: i64) -> String {
    format!("/construction/projects/{id}/variation-report")
}

/// The list of projects.
pub(crate) const PROJECTS_URL: &str = "/construction/projects";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_texts::SCREEN_TEXTS;

    /// Every `tx['name']` a template reads is a text of the crate, and every
    /// name a page hands over (`tx(lang, &[...])`) is read by its template or
    /// by none: a missing text would show as a raw key on the page.
    #[test]
    fn every_text_a_template_reads_exists() {
        for (name, source) in VIEWS {
            let mut rest = *source;
            while let Some(at) = rest.find("tx['") {
                rest = &rest[at + 4..];
                let end = rest.find('\'').expect("a closed key");
                let key = format!("construction.screen.{}", &rest[..end]);
                assert!(
                    SCREEN_TEXTS.iter().any(|(known, _)| *known == key),
                    "{name} reads `{key}`, which has no text"
                );
            }
        }
    }
}
