//! The compliance report (wl-ticketing §6, issue #148): the open compliance
//! flags by subject, reason and age.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /compliance-flags` | `wl_ticketing.compliance_flags` | `tickets.view` (`?subject_type=&subject_id=` lists one subject's flags) |
//!
//! There is no report library yet; this page is the report and is what the
//! dashboard of a vertical links to. The template is compiled in and can be
//! replaced by the app (`resources/views/wl-ticketing/compliance_flags.html`).

use std::collections::BTreeMap;

use renox::axum::extract::Query;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::config::Category;
use crate::flags::ComplianceFlag;
use crate::permissions;
use crate::texts;
use crate::ticket::Ticket;

/// The texts a template reads as `tx['flags.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes, behind `tickets.view`.
pub fn routes() -> Routes {
    // Anywhere: a manager sees the flags of their buildings.
    wl_kernel::scopes::anywhere(
        Routes::new()
            .get("/compliance-flags", index)
            .name("wl_ticketing.compliance_flags"),
        permissions::VIEW,
    )
}

/// The query of the report.
#[derive(Deserialize, Debug, Default)]
pub struct FlagQuery {
    /// Only the flags on this subject type…
    #[serde(default)]
    pub subject_type: Option<String>,
    /// …and this id.
    #[serde(default)]
    pub subject_id: Option<i64>,
}

/// One open flag as the report shows it.
#[derive(Serialize, Debug, Clone)]
pub struct FlagRow {
    /// The subject, `property.building_system #4`.
    pub subject: String,
    /// The category's name.
    pub category: String,
    /// The reason's label.
    pub reason: String,
    /// The reason's key.
    pub reason_key: String,
    /// When it was raised, Asia/Dubai.
    pub raised: String,
    /// Whole days open, e.g. `3 days`.
    pub age: String,
    /// The number of the visit ticket of a `visit_missed` flag.
    pub visit: String,
    /// The page of that visit, when there is one.
    pub visit_id: Option<i64>,
    /// The service contract, when the flag is about one.
    pub contract: String,
    /// The page of that contract, when there is one.
    pub contract_id: Option<i64>,
}

/// What the report needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-ticketing/compliance_flags.html")]
pub(crate) struct FlagsPage {
    tx: Texts,
    flags: Vec<FlagRow>,
}

/// `GET /compliance-flags`: the open flags, oldest first.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<FlagQuery>,
) -> Result<View> {
    let reach = wl_kernel::scopes::reach(&state.db, &user, permissions::VIEW).await?;
    let subject = match (query.subject_type, query.subject_id) {
        (Some(kind), Some(id)) if !kind.trim().is_empty() => Some(Subject::new(kind.trim(), id)),
        _ => None,
    };
    let flags = match &subject {
        Some(subject) => ComplianceFlag::open_for(&state.db, subject).await?,
        None => ComplianceFlag::open(&state.db).await?,
    };
    let today = wl_kernel::documents::dubai_date(renox::db::now());
    let mut categories: BTreeMap<i64, String> = BTreeMap::new();
    let mut rows = Vec::new();
    for flag in flags {
        // Only the flags on the subjects of the user's scope.
        if !reach
            .contains(
                &state.db,
                &Subject::new(&flag.subject_type, flag.subject_id),
            )
            .await?
        {
            continue;
        }
        if let std::collections::btree_map::Entry::Vacant(slot) = categories.entry(flag.category_id)
        {
            let name = Category::find(&state.db, flag.category_id)
                .await?
                .map(|c| c.name(&lang.locale).to_owned())
                .unwrap_or_default();
            slot.insert(name);
        }
        let visit = match flag.ticket_id {
            Some(id) => Ticket::find(&state.db, id).await?.map(|t| t.number),
            None => None,
        };
        let age = (today - wl_kernel::documents::dubai_date(flag.raised_at)).num_days();
        rows.push(FlagRow {
            subject: format!("{} #{}", flag.subject_type, flag.subject_id),
            category: categories
                .get(&flag.category_id)
                .cloned()
                .unwrap_or_default(),
            reason: texts::t(
                &lang,
                &format!("wl_ticketing.flags.reason_{}", flag.reason.as_str()),
            ),
            reason_key: flag.reason.as_str().to_owned(),
            raised: wl_kernel::calendar::calendar(&state)
                .local(flag.raised_at)
                .format("%d %b %Y")
                .to_string(),
            age: texts::t(&lang, "wl_ticketing.flags.days").replace(":count", &age.to_string()),
            visit: visit.unwrap_or_default(),
            visit_id: flag.ticket_id,
            contract: flag
                .contract_id
                .map(|c| format!("#{c}"))
                .unwrap_or_default(),
            contract_id: flag.contract_id,
        });
    }
    Ok(View::typed(FlagsPage {
        tx: texts::all(&lang),
        flags: rows,
    }))
}
