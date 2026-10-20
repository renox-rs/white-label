//! Onboarding that blocks activation (wl-hr §3 *Onboarding*, §4 *Employee*
//! `activate`, §7 *Onboarding blocks activation*, issue #219).
//!
//! Onboarding blocks nothing by default. A template with *blocks activation*
//! makes `activate` need every **required** step that comes from that template
//! `done` or `waived`; open steps of other templates in the same onboarding do
//! not block, optional steps do not block, and an onboarding started by hand
//! (trigger `manual`) never blocks. The setting is read from the template when
//! `activate` runs, so switching it off lifts the block at once. The statutory
//! items of `activate` are not touched here.
//!
//! The refusal is a 422 whose message lists the open steps and their
//! assignees.

use renox::db::Transaction;
use renox::prelude::*;

use crate::onboarding::{
    AppliedTemplate, Onboarding, OnboardingStatus, OnboardingStep, StepAssignee, StepStatus,
};
use crate::template::OnboardingTemplate;
use crate::texts::english;

/// An open required step that blocks `activate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    /// The step.
    pub step_id: i64,
    /// Its title.
    pub title: String,
    /// The template it comes from.
    pub template: String,
    /// The names of its assignees; empty when it is recorded in person.
    pub assignees: Vec<String>,
}

/// The open required steps, from templates that block activation, of the
/// `hire` onboardings of `employee_id` that are in progress; read inside
/// `tx`, so the check and the activation see the same state.
pub async fn blockers_in(tx: &mut Transaction, employee_id: i64) -> Result<Vec<Blocker>> {
    let mut blockers = Vec::new();
    let running = Onboarding::query()
        .where_(Onboarding::EMPLOYEE_ID.eq(employee_id))
        .where_(Onboarding::START_TRIGGER.eq("hire"))
        .where_(Onboarding::STATUS.eq(OnboardingStatus::InProgress))
        .order_by(Onboarding::ID)
        .get(&mut *tx)
        .await?;
    for onboarding in running {
        let mut blocking: Vec<OnboardingTemplate> = Vec::new();
        for applied in AppliedTemplate::query()
            .where_(AppliedTemplate::ONBOARDING_ID.eq(onboarding.id))
            .order_by(AppliedTemplate::ID)
            .get(&mut *tx)
            .await?
        {
            if let Some(template) = OnboardingTemplate::find(&mut *tx, applied.template_id).await?
                && template.blocks_activation
            {
                blocking.push(template);
            }
        }
        if blocking.is_empty() {
            continue;
        }
        let open = OnboardingStep::query()
            .where_(OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
            .where_(OnboardingStep::STATUS.eq(StepStatus::Open))
            .where_(OnboardingStep::REQUIRED.eq(true))
            .order_by(OnboardingStep::POSITION)
            .get(&mut *tx)
            .await?;
        for step in open {
            let Some(template) = blocking.iter().find(|t| t.id == step.template_id) else {
                continue;
            };
            let mut names = Vec::new();
            for assignee in StepAssignee::query()
                .where_(StepAssignee::STEP_ID.eq(step.id))
                .order_by(StepAssignee::USER_ID)
                .get(&mut *tx)
                .await?
            {
                names.push(
                    User::find(&mut *tx, assignee.user_id)
                        .await?
                        .map_or_else(|| format!("#{}", assignee.user_id), |u| u.name),
                );
            }
            blockers.push(Blocker {
                step_id: step.id,
                title: step.title_en.clone(),
                template: template.name_en.clone(),
                assignees: names,
            });
        }
    }
    Ok(blockers)
}

/// The refusal text: one line per open step with its template and assignees.
pub fn refusal_message(blockers: &[Blocker]) -> String {
    let mut lines = vec![english("wl_hr.activation.blocked", &[])];
    for blocker in blockers {
        let who = if blocker.assignees.is_empty() {
            english("wl_hr.steps.nobody", &[])
        } else {
            blocker.assignees.join(", ")
        };
        lines.push(english(
            "wl_hr.activation.line",
            &[
                ("step", &blocker.title),
                ("template", &blocker.template),
                ("who", &who),
            ],
        ));
    }
    lines.join("\n")
}

/// The refusal as an HTTP 422.
pub fn refusal(blockers: &[Blocker]) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, refusal_message(blockers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_lists_every_step_with_its_assignees() {
        let blockers = [
            Blocker {
                step_id: 1,
                title: "Safety briefing".into(),
                template: "Site induction".into(),
                assignees: vec!["Hana".into(), "Hiba".into()],
            },
            Blocker {
                step_id: 2,
                title: "PPE handed over".into(),
                template: "Site induction".into(),
                assignees: vec![],
            },
        ];
        let text = refusal_message(&blockers);
        assert!(text.starts_with("Activation waits"), "{text}");
        assert!(text.contains("Safety briefing"));
        assert!(text.contains("Hana, Hiba"));
        assert!(text.contains("PPE handed over (Site induction): In person"));
        assert_eq!(text.lines().count(), 3);
    }
}
