//! The rate card pages (09 Manpower `MPW.ACL.16`, BL-MPW-26): the card of a
//! contract with its versions (`GET /rate-cards/{id}`) and one version with its
//! lines, the client's acceptance, the MD's approval and the price preview
//! (`GET /rate-card-versions/{id}`). The routes are those of
//! [`crate::rate_card_pages`]: a `GET` answers the page when the request asks
//! for HTML ([`wl_kernel::negotiate::wants_html`]) and the same JSON as before
//! otherwise; the buttons post to the existing routes, which call
//! [`crate::rate_cards`] and answer 303 to the page with a toast.
//!
//! Decisions:
//!
//! - **Status in words.** The machine has four states; the page tells where a
//!   version stands today: *Pending* (the client has not accepted), *Waiting
//!   for the MD* (accepted, dated in the past), *Accepted* (it starts in the
//!   future), *In force* (it prices today), *Superseded* (a later accepted
//!   version prices today) and *Not approved* (the MD said no).
//! - **The acceptance date** is the day the acceptance is recorded: the service
//!   takes no other, so the sheet asks for the document only.
//! - **The preview** prices the sample hours by the version in force on the
//!   work date, not by the version being looked at, because a pending version
//!   prices nothing. A month rate is shown split by version (pro rata days).
//! - **Rates** are on these pages only; the routes need `rate_cards.view`
//!   (`MPW.ACL.04`), which the supervisor does not hold.
//! - **Approval.** The MD approves and rejects with the kernel's approval
//!   routes; the requester (the user who accepted) is refused there (403).

use std::collections::BTreeMap;

use renox::chrono::{Datelike, NaiveDate};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{ApprovalRequest, ensure_approver};

use crate::permissions;
use crate::rate_card::{RateCard, RateCardLine, RateCardVersion, Unit, VersionStatus};
use crate::rate_card_pages::hundredths;
use crate::rate_cards::today;
use crate::rate_pricing::{self, MonthWork, OVERTIME, PUBLIC_HOLIDAY, Work, version_in_force};
use crate::texts;

/// The texts a template reads as `tx['screen.accept']`.
type Texts = BTreeMap<String, String>;

/// The page of a card.
pub(crate) fn card_url(id: i64) -> String {
    format!("/rate-cards/{id}")
}

/// The page of a version.
pub(crate) fn version_url(id: i64) -> String {
    format!("/rate-card-versions/{id}")
}

/// The package text `key` (below `wl_invoicing.rate_card.`).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_invoicing.rate_card.{key}"))
}

/// The text `key` with `{{ name }}` filled.
fn filled(lang: &Lang, key: &str, values: &[(&str, &str)]) -> String {
    let mut out = text(lang, key);
    for (name, value) in values {
        out = out.replace(&format!("{{{{ {name} }}}}"), value);
    }
    out
}

/// The texts of the screens, keyed without the prefix: `screen.accept`.
fn screen_texts(lang: &Lang) -> Texts {
    texts::all(lang)
        .into_iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("rate_card.")
                .map(|rest| (rest.to_owned(), value))
        })
        .collect()
}

/// A date as `01 Nov 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

/// An amount in fils as `AED 12,500.00`.
fn aed(fils: i64) -> String {
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

/// A role or shift key as words: `security_guard` is `Security guard`.
fn words(key: &str) -> String {
    let spaced = key.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Hundredths as `1.25`.
fn hundredths_text(value: i64) -> String {
    format!("{}.{:02}", value / 100, value % 100)
}

/// Thousandths of an hour as `8.00 h`.
fn hours_text(milli: i64) -> String {
    format!("{}.{:02} h", milli / 1000, (milli % 1000) / 10)
}

/// Where a version stands today, as the key of its word.
fn standing(version: &RateCardVersion, all: &[RateCardVersion], day: NaiveDate) -> &'static str {
    match version.status {
        VersionStatus::Pending => "pending",
        VersionStatus::AwaitingApproval => "waiting_md",
        VersionStatus::Rejected => "rejected",
        VersionStatus::Accepted => {
            if version.effective_from > day {
                "accepted"
            } else if all.iter().any(|other| {
                other.status == VersionStatus::Accepted
                    && other.effective_from > version.effective_from
                    && other.effective_from <= day
            }) {
                "superseded"
            } else {
                "in_force"
            }
        }
    }
}

/// The tone of the badge of a standing.
fn kind_of(standing: &str) -> &'static str {
    match standing {
        "waiting_md" => "warning",
        "accepted" | "in_force" => "success",
        "rejected" => "error",
        _ => "info",
    }
}

/// The name of user `id`, or `fallback`.
async fn user_name(db: &Db, id: Option<i64>, fallback: &str) -> String {
    let Some(id) = id else {
        return fallback.to_owned();
    };
    User::find(db, id)
        .await
        .ok()
        .flatten()
        .map_or_else(|| fallback.to_owned(), |user| user.name)
}

/// The back link to the contract the card prices, when its module has a page.
fn source_link(card: &RateCard) -> Option<String> {
    wl_kernel::approvals::subject_url(&card.source_type, card.source_id)
}

/// Whether `user` makes versions and lines of `card` (in the scope of its contract).
async fn can_manage(state: &AppState, user: &AuthUser, card: &RateCard) -> Result<bool> {
    let scope = wl_kernel::scopes::scope_of(&state.db, &card.source()).await?;
    Ok(
        user.has_permission_in(permissions::RATE_CARDS_MANAGE, &scope)
            || user.has_permission(permissions::RATE_CARDS_MANAGE),
    )
}

/// A version in the list of a card.
#[derive(Serialize)]
struct VersionRowView {
    url: String,
    version: String,
    effective: String,
    status: String,
    status_kind: &'static str,
    accepted: String,
    lines: usize,
}

/// The page of a card.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/rate_card.html")]
pub(crate) struct CardPage {
    tx: Texts,
    id: i64,
    /// *Rate card of Gulf Tower Facilities LLC*.
    title: String,
    back: String,
    back_label: String,
    can_manage: bool,
    today: String,
    versions: Vec<VersionRowView>,
}

/// `GET /rate-cards/{id}` as a page.
pub(crate) async fn card_page(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    card: &RateCard,
) -> Result<View> {
    let day = today();
    let versions = RateCardVersion::of_card(&state.db, card.id).await?;
    let mut rows = Vec::new();
    for version in &versions {
        let now = standing(version, &versions, day);
        let accepted = match version.accepted_on {
            Some(on) => match version
                .accepted_document
                .as_deref()
                .filter(|d| !d.is_empty())
            {
                Some(document) => format!("{}, {document}", date_text(on)),
                None => date_text(on),
            },
            None => text(lang, "screen.not_yet"),
        };
        rows.push(VersionRowView {
            url: version_url(version.id),
            version: format!("v{}", version.version),
            effective: date_text(version.effective_from),
            status: text(lang, &format!("screen.status_{now}")),
            status_kind: kind_of(now),
            accepted,
            lines: version.lines(&state.db).await?.len(),
        });
    }
    let back = source_link(card);
    let client = wl_kernel::parties::directory(state)
        .display_name(&state.db, wl_kernel::PartyId(card.party_id), &lang.locale)
        .await
        .ok()
        .flatten();
    let title = match client {
        Some(client) => filled(lang, "screen.card_title_of", &[("client", &client)]),
        None => text(lang, "screen.card_title"),
    };
    Ok(View::typed(CardPage {
        tx: screen_texts(lang),
        id: card.id,
        title,
        back_label: if back.is_some() {
            text(lang, "screen.back_contract")
        } else {
            String::new()
        },
        back: back.unwrap_or_default(),
        can_manage: can_manage(state, user, card).await?,
        today: day.to_string(),
        versions: rows,
    }))
}

/// A fact of the version.
#[derive(Serialize)]
struct Fact {
    label: String,
    value: String,
}

/// A line of a version.
#[derive(Serialize)]
struct LineView {
    id: i64,
    role: String,
    role_key: String,
    shift: String,
    shift_key: String,
    rate: String,
    unit: String,
    unit_key: &'static str,
    overtime: String,
    holiday: String,
    minimum: String,
    rate_value: String,
    overtime_value: String,
    holiday_value: String,
    minimum_value: String,
}

/// The approval waiting about the version.
#[derive(Serialize)]
struct ApprovalView {
    id: i64,
    can_decide: bool,
}

/// One priced row of the preview.
#[derive(Serialize)]
struct PreviewRow {
    what: String,
    billed: String,
    rate: String,
    amount: String,
}

/// The answer of the preview.
#[derive(Serialize)]
struct PreviewResult {
    heading: String,
    rows: Vec<PreviewRow>,
    total: String,
}

/// The preview form with what was asked and what came of it.
#[derive(Serialize)]
struct PreviewView {
    options: Vec<(String, String)>,
    selected: String,
    date: String,
    regular: String,
    overtime: String,
    holiday: String,
    asked: bool,
    message: String,
    result: Vec<PreviewResult>,
}

/// What the preview form sends.
#[derive(Deserialize, Default, Debug)]
pub(crate) struct PreviewQuery {
    line: Option<String>,
    date: Option<String>,
    regular: Option<String>,
    overtime: Option<String>,
    holiday: Option<String>,
}

/// The page of a version.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/rate_card_version.html")]
pub(crate) struct VersionPage {
    tx: Texts,
    id: i64,
    title: String,
    card_url: String,
    back_label: String,
    status: String,
    status_kind: &'static str,
    note: String,
    facts: Vec<Fact>,
    lines: Vec<LineView>,
    editable: bool,
    can_add_line: bool,
    can_accept: bool,
    accept_blocked: String,
    approval: Vec<ApprovalView>,
    units: Vec<(String, String)>,
    preview: PreviewView,
}

fn line_view(lang: &Lang, line: &RateCardLine) -> LineView {
    let (unit_key, unit) = match line.unit {
        Unit::Hour => ("hour", "screen.unit_hour"),
        Unit::Day => ("day", "screen.unit_day"),
        Unit::Month => ("month", "screen.unit_month"),
    };
    LineView {
        id: line.id,
        role: words(&line.role),
        role_key: line.role.clone(),
        shift: words(&line.shift_type),
        shift_key: line.shift_type.clone(),
        rate: aed(line.rate),
        unit: text(lang, unit),
        unit_key,
        overtime: format!("{}x", hundredths_text(line.ot_multiplier)),
        holiday: format!("{}x", hundredths_text(line.holiday_multiplier)),
        minimum: if line.min_hours_milli == 0 {
            text(lang, "screen.no_minimum")
        } else {
            hours_text(line.min_hours_milli)
        },
        rate_value: hundredths_text(line.rate),
        overtime_value: hundredths_text(line.ot_multiplier),
        holiday_value: hundredths_text(line.holiday_multiplier),
        minimum_value: hundredths_text(line.min_hours_milli / 10),
    }
}

/// Hours typed as `10` or `2.5` in thousandths; empty is none.
fn milli_of(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return Some(0);
    }
    hundredths(text).map(|h| h * 10)
}

/// The preview of the hours asked for.
async fn preview(
    state: &AppState,
    lang: &Lang,
    card: &RateCard,
    query: &PreviewQuery,
    options: Vec<(String, String)>,
) -> Result<PreviewView> {
    let asked = query.date.is_some() || query.line.is_some();
    let mut view = PreviewView {
        selected: query
            .line
            .clone()
            .filter(|line| options.iter().any(|(value, _)| value == line))
            .or_else(|| options.first().map(|(value, _)| value.clone()))
            .unwrap_or_default(),
        options,
        date: query.date.clone().unwrap_or_else(|| today().to_string()),
        regular: query.regular.clone().unwrap_or_default(),
        overtime: query.overtime.clone().unwrap_or_default(),
        holiday: query.holiday.clone().unwrap_or_default(),
        asked,
        message: String::new(),
        result: Vec::new(),
    };
    if !asked {
        return Ok(view);
    }
    let parsed = (
        view.date.trim().parse::<NaiveDate>().ok(),
        milli_of(&view.regular),
        milli_of(&view.overtime),
        milli_of(&view.holiday),
    );
    let (Some(date), Some(regular), Some(overtime), Some(holiday)) = parsed else {
        view.message = text(lang, "screen.preview_bad");
        return Ok(view);
    };
    let Some((role, shift)) = view.selected.split_once('|') else {
        view.message = text(lang, "screen.preview_no_line");
        return Ok(view);
    };
    let Some(in_force) = version_in_force(&state.db, card.id, date).await? else {
        view.message = text(lang, "screen.preview_none");
        return Ok(view);
    };
    let Some(line) = in_force
        .lines(&state.db)
        .await?
        .into_iter()
        .find(|line| line.role == role && line.shift_type == shift)
    else {
        view.message = text(lang, "screen.preview_no_line");
        return Ok(view);
    };
    let heading = filled(
        lang,
        "screen.preview_result",
        &[
            ("date", &date_text(date)),
            ("version", &in_force.version.to_string()),
        ],
    );
    if line.unit == Unit::Month {
        let pieces = rate_pricing::price_month(
            &state.db,
            &MonthWork {
                source: card.source(),
                role: role.to_owned(),
                shift_type: shift.to_owned(),
                year: date.year(),
                month: date.month(),
            },
        )
        .await?;
        let total = pieces.iter().map(|piece| piece.amount).sum();
        let rows = pieces
            .iter()
            .map(|piece| PreviewRow {
                what: filled(
                    lang,
                    "screen.preview_split",
                    &[
                        ("from", &date_text(piece.from)),
                        ("to", &date_text(piece.to)),
                        ("days", &piece.days.to_string()),
                        ("month_days", &piece.month_days.to_string()),
                    ],
                ),
                billed: format!("v{}", piece.version.version),
                rate: aed(piece.rate),
                amount: aed(piece.amount),
            })
            .collect();
        view.result.push(PreviewResult {
            heading,
            rows,
            total: aed(total),
        });
        return Ok(view);
    }
    let mut rows = Vec::new();
    let mut total = 0;
    for (category, key, milli) in [
        ("normal", "screen.preview_category_normal", regular),
        (OVERTIME, "screen.preview_category_overtime", overtime),
        (PUBLIC_HOLIDAY, "screen.preview_category_holiday", holiday),
    ] {
        if milli <= 0 {
            continue;
        }
        let work = Work::new(card.source(), role, shift, date, category, milli);
        if let Some(priced) = rate_pricing::price(&state.db, &work).await? {
            total += priced.amount;
            rows.push(PreviewRow {
                what: text(lang, key),
                billed: hours_text(priced.billed_milli),
                rate: aed(priced.line.rate),
                amount: aed(priced.amount),
            });
        }
    }
    if rows.is_empty() {
        view.message = text(lang, "screen.preview_bad");
    } else {
        view.result.push(PreviewResult {
            heading,
            rows,
            total: aed(total),
        });
    }
    Ok(view)
}

/// `GET /rate-card-versions/{id}` as a page.
pub(crate) async fn version_page(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    version: &RateCardVersion,
    query: &PreviewQuery,
) -> Result<View> {
    let day = today();
    let card = RateCard::find_or_404(&state.db, version.rate_card_id).await?;
    let all = RateCardVersion::of_card(&state.db, card.id).await?;
    let now = standing(version, &all, day);
    let lines = version.lines(&state.db).await?;
    let none = text(lang, "screen.not_yet");

    let mut facts = vec![
        Fact {
            label: text(lang, "screen.effective"),
            value: date_text(version.effective_from),
        },
        Fact {
            label: text(lang, "screen.fact_made_on"),
            value: date_text(version.submitted_on),
        },
    ];
    if version.past_dated {
        facts.push(Fact {
            label: text(lang, "screen.fact_past_dated"),
            value: text(lang, "screen.yes"),
        });
    }
    if let Some(on) = version.accepted_on {
        facts.push(Fact {
            label: text(lang, "screen.fact_accepted_on"),
            value: date_text(on),
        });
        facts.push(Fact {
            label: text(lang, "screen.fact_accepted_by"),
            value: user_name(&state.db, version.accepted_by, &none).await,
        });
        if let Some(document) = version
            .accepted_document
            .as_deref()
            .filter(|d| !d.is_empty())
        {
            facts.push(Fact {
                label: text(lang, "screen.fact_document"),
                value: document.to_owned(),
            });
        }
    }
    if version.approved_by.is_some() {
        facts.push(Fact {
            label: text(lang, "screen.fact_approved_by"),
            value: user_name(&state.db, version.approved_by, &none).await,
        });
    }
    if let Some(reason) = version.rejected_reason.as_deref().filter(|r| !r.is_empty()) {
        facts.push(Fact {
            label: text(lang, "screen.fact_reason"),
            value: reason.to_owned(),
        });
    }

    let mut approval = Vec::new();
    if version.status == VersionStatus::AwaitingApproval {
        for request in ApprovalRequest::pending_about(&state.db, &version.subject()).await? {
            let can_decide = ensure_approver(&state.db, &request, user).await.is_ok();
            approval.push(ApprovalView {
                id: request.id,
                can_decide,
            });
        }
    }

    // The role and shift pairs of every version, for the preview.
    let mut pairs: Vec<(String, String)> = Vec::new();
    for other in &all {
        for line in other.lines(&state.db).await? {
            let value = format!("{}|{}", line.role, line.shift_type);
            if !pairs.iter().any(|(v, _)| *v == value) {
                pairs.push((
                    value,
                    format!(
                        "{}, {}",
                        words(&line.role),
                        line.shift_type.replace('_', " ")
                    ),
                ));
            }
        }
    }

    let can_manage = can_manage(state, user, &card).await?;
    let editable = version.status == VersionStatus::Pending;
    Ok(View::typed(VersionPage {
        tx: screen_texts(lang),
        id: version.id,
        title: filled(
            lang,
            "screen.version_title",
            &[("version", &version.version.to_string())],
        ),
        card_url: card_url(card.id),
        back_label: text(lang, "screen.back_card"),
        status: text(lang, &format!("screen.status_{now}")),
        status_kind: kind_of(now),
        note: text(lang, &format!("screen.note_{now}")),
        facts,
        lines: lines.iter().map(|line| line_view(lang, line)).collect(),
        editable,
        can_add_line: editable && can_manage,
        can_accept: editable && can_manage,
        accept_blocked: if lines.is_empty() {
            text(lang, "screen.accept_blocked")
        } else {
            String::new()
        },
        approval,
        units: [
            ("hour", "screen.unit_hour"),
            ("day", "screen.unit_day"),
            ("month", "screen.unit_month"),
        ]
        .iter()
        .map(|(value, key)| ((*value).to_owned(), text(lang, key)))
        .collect(),
        preview: preview(state, lang, &card, query, pairs).await?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tx['name']` a template reads is a text of the package: a missing
    /// one would show as nothing on the page.
    #[test]
    fn every_text_a_rate_card_template_reads_exists() {
        let keys: Vec<String> = crate::rate_card_texts::TEXTS
            .iter()
            .map(|(key, _)| key.trim_start_matches("wl_invoicing.rate_card.").to_owned())
            .collect();
        for (name, source) in [
            (
                "rate_card.html",
                include_str!("../views/wl-invoicing/rate_card.html"),
            ),
            (
                "rate_card_version.html",
                include_str!("../views/wl-invoicing/rate_card_version.html"),
            ),
        ] {
            let mut rest = source;
            while let Some(at) = rest.find("tx['") {
                rest = &rest[at + 4..];
                let end = rest.find('\'').expect("a closed key");
                assert!(
                    keys.iter().any(|key| key == &rest[..end]),
                    "{name} reads `{}`, which has no text",
                    &rest[..end]
                );
            }
        }
    }

    #[test]
    fn amounts_hours_and_roles_are_in_words() {
        assert_eq!(aed(2_800), "AED 28.00");
        assert_eq!(aed(1_250_000), "AED 12,500.00");
        assert_eq!(hours_text(8_000), "8.00 h");
        assert_eq!(hours_text(2_500), "2.50 h");
        assert_eq!(hundredths_text(125), "1.25");
        assert_eq!(words("security_guard"), "Security guard");
        assert_eq!(milli_of("10"), Some(10_000));
        assert_eq!(milli_of(""), Some(0));
        assert_eq!(milli_of("x"), None);
    }
}
