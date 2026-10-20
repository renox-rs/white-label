//! Onboarding templates (wl-hr §3 *Onboarding template*, *Onboarding template
//! step*, §7, issue #216): the steps every new employee in a role goes
//! through. A template applies to the job roles or trades it names, or to all
//! staff, and to the employment types it names (all when none). Its version
//! counts the changes of the template and its steps; an onboarding keeps the
//! version it was started from (the steps are copied).

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::employee::EmploymentType;

/// What a template step asks for.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StepKind {
    /// Marked done by an assignee, with evidence when the step needs it.
    #[default]
    Task,
    /// Done when its checklist run (`WRK.CHK.01`) passes.
    Checklist,
    /// Done when an accepted document of the kind is on the employee.
    Document,
    /// Done when the employee has acknowledged the SOPs of the role.
    SopAcknowledgement,
}

impl StepKind {
    /// The stored name.
    pub fn name(self) -> &'static str {
        self.as_str()
    }
}

/// An onboarding template.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_onboarding_templates")]
pub struct OnboardingTemplate {
    /// The primary key.
    pub id: i64,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: String,
    /// Applies to every employee.
    pub all_staff: bool,
    /// Comma-separated job role or trade keys, lower case.
    pub job_roles: String,
    /// Comma-separated employment types; empty for every type.
    pub employment_types: String,
    /// Activation also needs the required steps of this template.
    pub blocks_activation: bool,
    /// Counts the changes of the template and its steps.
    pub version: i64,
    /// Whether new employees get it.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One step of a template.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_onboarding_template_steps",
    unique(template_id, position)
)]
pub struct TemplateStep {
    /// The primary key.
    pub id: i64,
    /// The template.
    #[model(references = "wl_hr_onboarding_templates")]
    pub template_id: i64,
    /// The order in the template, from 1.
    pub position: i64,
    /// The title in English.
    pub title_en: String,
    /// The title in Arabic.
    pub title_ar: String,
    /// `hr`, `manager`, `employee` or `role:<name>`.
    pub owner: String,
    /// Days after the hire date.
    pub due_days: i64,
    /// Whether the onboarding needs it done or waived to complete.
    pub required: bool,
    /// What the step asks for.
    pub kind: StepKind,
    /// The checklist template key, for a `checklist` step.
    pub checklist_template: Option<String>,
    /// The document kind, for a `document` step.
    pub document_kind: Option<String>,
    /// Whether a `task` step needs an evidence file or note.
    pub evidence_required: bool,
    /// Help for whoever does the step.
    pub help_text: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The owners a step can have besides a named role.
pub const OWNER_HR: &str = "hr";
/// The employee's manager.
pub const OWNER_MANAGER: &str = "manager";
/// The employee.
pub const OWNER_EMPLOYEE: &str = "employee";

/// Whether `owner` is a valid step owner: `hr`, `manager`, `employee` or
/// `role:<name>`.
pub fn is_valid_owner(owner: &str) -> bool {
    matches!(owner, OWNER_HR | OWNER_MANAGER | OWNER_EMPLOYEE)
        || owner
            .strip_prefix("role:")
            .is_some_and(|name| !name.trim().is_empty())
}

/// A comma-separated list, trimmed and lower case, without blanks.
fn list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect()
}

impl OnboardingTemplate {
    /// The job roles or trades the template names.
    pub fn roles(&self) -> Vec<String> {
        list(&self.job_roles)
    }

    /// Whether the template applies to `job_role` with `employment_type`:
    /// an all-staff template or one that names the role, and one that names
    /// the type or no type at all.
    pub fn applies_to(&self, job_role: &str, employment_type: EmploymentType) -> bool {
        let role = job_role.trim().to_lowercase();
        let by_role = self.all_staff || self.roles().contains(&role);
        let types = list(&self.employment_types);
        by_role && (types.is_empty() || types.iter().any(|t| t == employment_type.as_str()))
    }

    /// The active templates that apply to `job_role` with `employment_type`,
    /// the all-staff ones first, then by id.
    pub async fn applicable(
        db: &Db,
        job_role: &str,
        employment_type: EmploymentType,
    ) -> Result<Vec<OnboardingTemplate>> {
        let mut found: Vec<OnboardingTemplate> = OnboardingTemplate::query()
            .where_(OnboardingTemplate::ACTIVE.eq(true))
            .order_by(OnboardingTemplate::ID)
            .get(db)
            .await?
            .into_iter()
            .filter(|t| t.applies_to(job_role, employment_type))
            .collect();
        found.sort_by_key(|t| (!t.all_staff, t.id));
        Ok(found)
    }

    /// The steps of the template, in order.
    pub async fn steps(&self, db: &Db) -> Result<Vec<TemplateStep>> {
        TemplateStep::query()
            .where_(TemplateStep::TEMPLATE_ID.eq(self.id))
            .order_by(TemplateStep::POSITION)
            .get(db)
            .await
    }
}

/// What a template step is made from.
#[derive(Debug, Clone)]
pub struct StepInput {
    /// The title in English.
    pub title_en: String,
    /// The title in Arabic.
    pub title_ar: String,
    /// `hr`, `manager`, `employee` or `role:<name>`.
    pub owner: String,
    /// Days after the hire date.
    pub due_days: i64,
    /// What it asks for.
    pub kind: StepKind,
    /// Optional steps do not hold the completion.
    pub required: bool,
    /// A `task` step needs an evidence file or note.
    pub evidence_required: bool,
    /// The checklist template key, for a `checklist` step.
    pub checklist_template: Option<String>,
    /// The document kind, for a `document` step.
    pub document_kind: Option<String>,
}

impl StepInput {
    /// A required `task` step with no evidence.
    pub fn task(title: &str, owner: &str, due_days: i64) -> Self {
        StepInput {
            title_en: title.to_owned(),
            title_ar: String::new(),
            owner: owner.to_owned(),
            due_days,
            kind: StepKind::Task,
            required: true,
            evidence_required: false,
            checklist_template: None,
            document_kind: None,
        }
    }

    /// A `checklist` step on the checklist template `template`.
    pub fn checklist(title: &str, owner: &str, due_days: i64, template: &str) -> Self {
        StepInput {
            kind: StepKind::Checklist,
            checklist_template: Some(template.to_owned()),
            ..Self::task(title, owner, due_days)
        }
    }

    /// A `document` step done by an accepted document of `document_kind`.
    pub fn document(title: &str, owner: &str, due_days: i64, document_kind: &str) -> Self {
        StepInput {
            kind: StepKind::Document,
            document_kind: Some(document_kind.to_owned()),
            ..Self::task(title, owner, due_days)
        }
    }

    /// An `sop_acknowledgement` step.
    pub fn sops(title: &str, owner: &str, due_days: i64) -> Self {
        StepInput {
            kind: StepKind::SopAcknowledgement,
            ..Self::task(title, owner, due_days)
        }
    }

    /// Optional: it does not hold the completion.
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    /// A `task` step that needs an evidence file or note.
    pub fn evidence(mut self) -> Self {
        self.evidence_required = true;
        self
    }
}

/// What a template is made from.
#[derive(Debug, Clone, Default)]
pub struct TemplateInput {
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: String,
    /// Applies to every employee.
    pub all_staff: bool,
    /// The job roles or trades it applies to.
    pub job_roles: Vec<String>,
    /// The employment types it applies to; none for every type.
    pub employment_types: Vec<EmploymentType>,
    /// Activation also needs its required steps.
    pub blocks_activation: bool,
    /// The steps, in order.
    pub steps: Vec<StepInput>,
}

impl TemplateInput {
    /// A template for every employee.
    pub fn all_staff(name: &str) -> Self {
        TemplateInput {
            name_en: name.to_owned(),
            all_staff: true,
            ..Self::default()
        }
    }

    /// A template for the job role or trade `role`.
    pub fn for_role(name: &str, role: &str) -> Self {
        TemplateInput {
            name_en: name.to_owned(),
            job_roles: vec![role.to_owned()],
            ..Self::default()
        }
    }

    /// With a step.
    pub fn step(mut self, step: StepInput) -> Self {
        self.steps.push(step);
        self
    }
}

/// Why a template could not be saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateRefused {
    /// The name is blank.
    NameRequired,
    /// It names no role and is not for all staff.
    NoRole,
    /// It has no step.
    NoSteps,
    /// A step has no title, an unknown owner, a negative due date or lacks
    /// what its kind needs; the text says which.
    BadStep(String),
}

impl TemplateRefused {
    /// The English explanation.
    pub fn message(&self) -> String {
        match self {
            TemplateRefused::NameRequired => {
                crate::texts::english("wl_hr.templates.name_required", &[])
            }
            TemplateRefused::NoRole => crate::texts::english("wl_hr.templates.no_role", &[]),
            TemplateRefused::NoSteps => crate::texts::english("wl_hr.templates.no_steps", &[]),
            TemplateRefused::BadStep(text) => text.clone(),
        }
    }
}

/// Checks a step.
fn check_step(position: usize, step: &StepInput) -> Option<String> {
    let at = position + 1;
    if step.title_en.trim().is_empty() {
        return Some(crate::texts::english(
            "wl_hr.templates.step_title",
            &[("n", &at)],
        ));
    }
    if !is_valid_owner(&step.owner) {
        return Some(crate::texts::english(
            "wl_hr.templates.step_owner",
            &[("n", &at)],
        ));
    }
    if step.due_days < 0 {
        return Some(crate::texts::english(
            "wl_hr.templates.step_due",
            &[("n", &at)],
        ));
    }
    let missing = match step.kind {
        StepKind::Checklist => step
            .checklist_template
            .as_deref()
            .is_none_or(|t| t.trim().is_empty()),
        StepKind::Document => step
            .document_kind
            .as_deref()
            .is_none_or(|k| k.trim().is_empty()),
        _ => false,
    };
    missing.then(|| crate::texts::english("wl_hr.templates.step_kind", &[("n", &at)]))
}

/// Saves a new, active template with its steps in one transaction, at
/// version 1.
pub async fn create(
    db: &Db,
    input: TemplateInput,
) -> Result<std::result::Result<OnboardingTemplate, TemplateRefused>> {
    if input.name_en.trim().is_empty() {
        return Ok(Err(TemplateRefused::NameRequired));
    }
    if !input.all_staff && input.job_roles.iter().all(|r| r.trim().is_empty()) {
        return Ok(Err(TemplateRefused::NoRole));
    }
    if input.steps.is_empty() {
        return Ok(Err(TemplateRefused::NoSteps));
    }
    for (position, step) in input.steps.iter().enumerate() {
        if let Some(text) = check_step(position, step) {
            return Ok(Err(TemplateRefused::BadStep(text)));
        }
    }
    let mut tx = db.begin().await?;
    let template = OnboardingTemplate::create(
        &mut tx,
        OnboardingTemplate {
            name_en: input.name_en.trim().to_owned(),
            name_ar: input.name_ar.trim().to_owned(),
            all_staff: input.all_staff,
            job_roles: input
                .job_roles
                .iter()
                .map(|r| r.trim().to_lowercase())
                .filter(|r| !r.is_empty())
                .collect::<Vec<_>>()
                .join(","),
            employment_types: input
                .employment_types
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(","),
            blocks_activation: input.blocks_activation,
            version: 1,
            active: true,
            ..OnboardingTemplate::default()
        },
    )
    .await?;
    for (position, step) in input.steps.iter().enumerate() {
        TemplateStep::create(
            &mut tx,
            TemplateStep {
                template_id: template.id,
                position: i64::try_from(position).unwrap_or(0) + 1,
                title_en: step.title_en.trim().to_owned(),
                title_ar: step.title_ar.trim().to_owned(),
                owner: step.owner.clone(),
                due_days: step.due_days,
                required: step.required,
                kind: step.kind,
                checklist_template: step.checklist_template.clone(),
                document_kind: step.document_kind.clone(),
                evidence_required: step.evidence_required,
                ..TemplateStep::default()
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Ok(template))
}

/// The *all staff* induction template of §7: welcome and introductions
/// (manager, day 1), system account and access (HR, day 1), workplace and
/// safety induction (manager, checklist, day 3), uniform or PPE handed over
/// (HR, checklist with the employee's signature, day 3), SOPs acknowledged
/// (employee, day 7). Every step required, none needs evidence.
pub fn induction() -> TemplateInput {
    TemplateInput::all_staff("All staff induction")
        .step(StepInput::task(
            "Welcome and introductions",
            OWNER_MANAGER,
            1,
        ))
        .step(StepInput::task("System account and access", OWNER_HR, 1))
        .step(StepInput::checklist(
            "Workplace and safety induction",
            OWNER_MANAGER,
            3,
            "workplace_induction",
        ))
        .step(StepInput::checklist(
            "Uniform or PPE handed over",
            OWNER_HR,
            3,
            "ppe_handover",
        ))
        .step(StepInput::sops("SOPs acknowledged", OWNER_EMPLOYEE, 7))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(all_staff: bool, roles: &str, types: &str) -> OnboardingTemplate {
        OnboardingTemplate {
            all_staff,
            job_roles: roles.into(),
            employment_types: types.into(),
            active: true,
            ..OnboardingTemplate::default()
        }
    }

    #[test]
    fn a_template_applies_by_role_and_type() {
        let all = template(true, "", "");
        assert!(all.applies_to("anyone", EmploymentType::Freelance));
        let drivers = template(false, "driver, rider", "");
        assert!(drivers.applies_to(" Driver ", EmploymentType::FullTime));
        assert!(drivers.applies_to("rider", EmploymentType::Temporary));
        assert!(!drivers.applies_to("cleaner", EmploymentType::FullTime));
        let employed = template(true, "", "full_time,part_time");
        assert!(employed.applies_to("x", EmploymentType::PartTime));
        assert!(!employed.applies_to("x", EmploymentType::Freelance));
    }

    #[test]
    fn owners_are_hr_manager_employee_or_a_named_role() {
        for owner in ["hr", "manager", "employee", "role:supervisor"] {
            assert!(is_valid_owner(owner), "{owner}");
        }
        for owner in ["", "boss", "role:", "role: "] {
            assert!(!is_valid_owner(owner), "{owner}");
        }
    }

    #[test]
    fn the_seeded_induction_has_the_five_steps_of_the_spec() {
        let template = induction();
        assert!(template.all_staff);
        let steps: Vec<(&str, i64, StepKind)> = template
            .steps
            .iter()
            .map(|s| (s.owner.as_str(), s.due_days, s.kind))
            .collect();
        assert_eq!(
            steps,
            [
                ("manager", 1, StepKind::Task),
                ("hr", 1, StepKind::Task),
                ("manager", 3, StepKind::Checklist),
                ("hr", 3, StepKind::Checklist),
                ("employee", 7, StepKind::SopAcknowledgement),
            ]
        );
        assert!(
            template
                .steps
                .iter()
                .all(|s| s.required && !s.evidence_required)
        );
        for (n, step) in template.steps.iter().enumerate() {
            assert_eq!(check_step(n, step), None);
        }
    }

    #[test]
    fn a_step_needs_its_title_owner_and_what_its_kind_names() {
        let ok = StepInput::task("Welcome", "hr", 1);
        assert_eq!(check_step(0, &ok), None);
        assert!(
            check_step(
                0,
                &StepInput {
                    title_en: " ".into(),
                    ..ok.clone()
                }
            )
            .is_some()
        );
        assert!(
            check_step(
                0,
                &StepInput {
                    owner: "boss".into(),
                    ..ok.clone()
                }
            )
            .is_some()
        );
        assert!(
            check_step(
                0,
                &StepInput {
                    due_days: -1,
                    ..ok.clone()
                }
            )
            .is_some()
        );
        assert!(
            check_step(
                0,
                &StepInput {
                    kind: StepKind::Checklist,
                    ..ok.clone()
                }
            )
            .is_some()
        );
        assert!(
            check_step(
                0,
                &StepInput {
                    kind: StepKind::Document,
                    ..ok
                }
            )
            .is_some()
        );
    }
}
