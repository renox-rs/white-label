//! The onboarding templates page (wl-hr §3, issue #216).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /hr/onboarding-templates` | `wl_hr.templates` | `onboarding.view` |
//!
//! It lists the templates with the roles they apply to, their version and
//! their steps. Editing a template comes with the template editor of the
//! roadmap; [`crate::template::create`] is the service until then.

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::Serialize;

use crate::permissions;
use crate::template::{OnboardingTemplate, StepKind};
use crate::texts;

/// The texts a template reads as `tx['templates.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The route of the page.
pub fn routes() -> Routes {
    Routes::new()
        .get("/hr/onboarding-templates", index)
        .name("wl_hr.templates")
        .require_permission(permissions::ONBOARDING_VIEW)
}

/// One step as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct StepRow {
    /// The order.
    pub position: i64,
    /// The title.
    pub title: String,
    /// Who does it.
    pub owner: String,
    /// Days after the start.
    pub due_days: i64,
    /// What it asks for.
    pub kind: String,
    /// Required or optional.
    pub required: String,
}

/// One template as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct TemplateRow {
    /// The primary key.
    pub id: i64,
    /// The name.
    pub name: String,
    /// What it applies to.
    pub applies_to: String,
    /// Its version.
    pub version: i64,
    /// Active or not, as text.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// Whether it blocks activation.
    pub blocks: bool,
    /// Its steps.
    pub steps: Vec<StepRow>,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/templates.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    templates: Vec<TemplateRow>,
}

/// The owner of a step in the page's language.
pub(crate) fn owner_label(lang: &Lang, owner: &str) -> String {
    match owner.strip_prefix("role:") {
        Some(role) => texts::t_with(lang, "wl_hr.owner.role", &[("name", &role)]),
        None => texts::t(lang, &format!("wl_hr.owner.{owner}")),
    }
}

/// The kind of a step in the page's language.
pub(crate) fn kind_label(lang: &Lang, kind: StepKind) -> String {
    texts::t(lang, &format!("wl_hr.kind.{}", kind.name()))
}

/// `GET /hr/onboarding-templates`.
async fn index(State(db): State<Db>, lang: Lang) -> Result<View> {
    let mut templates = Vec::new();
    let rows = OnboardingTemplate::query()
        .order_by(OnboardingTemplate::ID)
        .get(&db)
        .await?;
    for template in rows {
        let steps = template
            .steps(&db)
            .await?
            .into_iter()
            .map(|step| StepRow {
                position: step.position,
                title: if lang.locale == "ar" && !step.title_ar.trim().is_empty() {
                    step.title_ar.clone()
                } else {
                    step.title_en.clone()
                },
                owner: owner_label(&lang, &step.owner),
                due_days: step.due_days,
                kind: kind_label(&lang, step.kind),
                required: texts::t(
                    &lang,
                    if step.required {
                        "wl_hr.templates.required"
                    } else {
                        "wl_hr.templates.optional"
                    },
                ),
            })
            .collect();
        let applies_to = if template.all_staff {
            texts::t(&lang, "wl_hr.templates.all_staff")
        } else {
            template.roles().join(", ")
        };
        templates.push(TemplateRow {
            id: template.id,
            name: if lang.locale == "ar" && !template.name_ar.trim().is_empty() {
                template.name_ar.clone()
            } else {
                template.name_en.clone()
            },
            applies_to,
            version: template.version,
            status: texts::t(
                &lang,
                if template.active {
                    "wl_hr.templates.active"
                } else {
                    "wl_hr.templates.inactive"
                },
            ),
            status_kind: if template.active {
                "success"
            } else {
                "warning"
            },
            blocks: template.blocks_activation,
            steps,
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        templates,
    }))
}
