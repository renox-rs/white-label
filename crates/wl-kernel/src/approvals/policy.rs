//! Approval policies (wl-kernel §3 *Approval policy*): who must approve an
//! action, and when.
//!
//! Modules declare their default policies once, in `register()`:
//!
//! ```ignore
//! PolicyDefault::new("payments.refund")
//!     .amount_above(500_000) // AED 5,000.00, in fils
//!     .step(["property_manager"])
//!     .step(["finance", "owner"]) // one of several roles
//!     .step_permission("payments.approve_refund") // any holder of it
//!     .register();
//! ```
//!
//! `wl:install` and the seeder store the defaults the database lacks
//! ([`install`]); a policy already there (maybe changed in the admin) is
//! kept as it is.

use std::sync::{PoisonError, RwLock};

use renox::db::{Executor, Json};
use renox::prelude::*;
use renox::serde_json::Value;
use serde::{Deserialize, Serialize};

/// The input key an amount condition reads when the policy names none: an
/// integer amount in fils.
pub const DEFAULT_AMOUNT_KEY: &str = "amount";
/// The input key a percentage condition reads when the policy names none:
/// an integer percentage in basis points.
pub const DEFAULT_PERCENTAGE_KEY: &str = "percentage_bps";
/// The expiry of a default policy, in hours (`PLT.RUL.02`, M = 72).
pub const DEFAULT_EXPIRY_HOURS: i64 = 72;

/// How a policy's condition is stored.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConditionKind {
    /// Every run of the action needs approval.
    #[default]
    Always,
    /// Runs whose amount (in fils) is above the threshold.
    AmountAbove,
    /// Runs whose percentage (in basis points) is above the threshold.
    PercentageAbove,
}

/// When a policy applies to a run of its action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    /// Always.
    Always,
    /// The amount, in fils, is above this.
    AmountAbove(i64),
    /// The percentage, in basis points, is above this.
    PercentageAbove(i64),
}

impl Condition {
    /// The stored kind and threshold.
    pub fn parts(self) -> (ConditionKind, i64) {
        match self {
            Condition::Always => (ConditionKind::Always, 0),
            Condition::AmountAbove(fils) => (ConditionKind::AmountAbove, fils),
            Condition::PercentageAbove(bps) => (ConditionKind::PercentageAbove, bps),
        }
    }

    /// The condition of a stored kind and threshold.
    pub fn from_parts(kind: ConditionKind, threshold: i64) -> Self {
        match kind {
            ConditionKind::Always => Condition::Always,
            ConditionKind::AmountAbove => Condition::AmountAbove(threshold),
            ConditionKind::PercentageAbove => Condition::PercentageAbove(threshold),
        }
    }

    /// The input key the condition reads by default.
    fn default_key(self) -> Option<&'static str> {
        match self {
            Condition::Always => None,
            Condition::AmountAbove(_) => Some(DEFAULT_AMOUNT_KEY),
            Condition::PercentageAbove(_) => Some(DEFAULT_PERCENTAGE_KEY),
        }
    }

    /// Whether a run with this transition input matches. The value is read
    /// from `input[key]`, an integer; a missing or non-integer value counts
    /// as above the threshold, so an unknown amount is never let through
    /// without approval.
    pub fn matches(self, input: &Value, key: Option<&str>) -> bool {
        let threshold = match self {
            Condition::Always => return true,
            Condition::AmountAbove(t) | Condition::PercentageAbove(t) => t,
        };
        let key = key.or(self.default_key()).unwrap_or_default();
        input
            .get(key)
            .and_then(Value::as_i64)
            .is_none_or(|value| value > threshold)
    }

    /// How strict the condition is: of several matching policies, the
    /// strictest (the highest threshold) wins; `Always` is the least strict.
    pub fn rank(self) -> (u8, i64) {
        match self {
            Condition::Always => (0, 0),
            Condition::AmountAbove(t) | Condition::PercentageAbove(t) => (1, t),
        }
    }
}

/// One step of a policy: any one user holding one of these roles, or the
/// step's permission, in the subject's scope may decide it.
///
/// Steps are stored as JSON (in the policy and, copied, in each request), so
/// a step written before `permission` existed reads as one without it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Step {
    /// The roles, any one of which may decide the step.
    pub roles: Vec<String>,
    /// A permission whose holders (in the subject's scope) may decide the
    /// step, instead of or besides the roles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    /// A permission every approver of the step must also hold (in the
    /// subject's scope), besides having one of the roles (or the
    /// `permission`). Unlike `permission` it narrows, never widens, who may
    /// decide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_permission: Option<String>,
}

impl Step {
    /// A step decided by any holder of one of `roles`.
    pub fn one_of<S: Into<String>>(roles: impl IntoIterator<Item = S>) -> Self {
        Step {
            roles: roles.into_iter().map(Into::into).collect(),
            permission: None,
            required_permission: None,
        }
    }

    /// A step decided by any holder of `permission`, whatever their role.
    pub fn holders_of(permission: impl Into<String>) -> Self {
        Step {
            roles: Vec::new(),
            permission: Some(permission.into()),
            required_permission: None,
        }
    }

    /// The step, also decided by any holder of `permission`.
    pub fn or_permission(mut self, permission: impl Into<String>) -> Self {
        self.permission = Some(permission.into());
        self
    }

    /// The step, decided only by approvers who also hold `permission` in the
    /// subject's scope (e.g. `Step::one_of(["finance_manager"])
    /// .requiring("parties.block_exception")`).
    pub fn requiring(mut self, permission: impl Into<String>) -> Self {
        self.required_permission = Some(permission.into());
        self
    }

    /// Whether `role` may decide this step.
    pub fn allows(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// Whether the step names nobody: no role and no permission.
    pub fn is_empty(&self) -> bool {
        self.roles.is_empty() && self.permission.is_none()
    }
}

/// Who must approve an action, and when.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_approval_policies", unique(action, condition, threshold))]
pub struct ApprovalPolicy {
    /// The primary key.
    pub id: i64,
    /// The action key, e.g. `payments.refund`.
    pub action: String,
    /// The kind of condition.
    pub condition: ConditionKind,
    /// The condition's threshold: fils for an amount, basis points for a
    /// percentage, 0 for `always`.
    pub threshold: i64,
    /// The transition input's key the condition reads; `None` for the
    /// default (`amount`, `percentage_bps`).
    pub input_key: Option<String>,
    /// The ordered steps.
    pub steps: Json<Vec<Step>>,
    /// Segregation of duties: the requester may not approve, and nobody
    /// approves two steps of one request.
    pub sod: bool,
    /// Hours a request may wait before it can expire; `None` for never.
    pub expiry_hours: Option<i64>,
    /// Whether the policy is in force.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ApprovalPolicy {
    /// The policy's condition.
    pub fn condition(&self) -> Condition {
        Condition::from_parts(self.condition, self.threshold)
    }

    /// Whether the policy applies to a run with this transition input.
    pub fn matches(&self, input: &Value) -> bool {
        self.active && self.condition().matches(input, self.input_key.as_deref())
    }

    /// The policy in force for a run of `action` with `input`: of the
    /// active policies that match, the one with the highest threshold.
    /// `None` when the run needs no approval.
    pub async fn for_action<'c>(
        db: impl Executor<'c>,
        action: &str,
        input: &Value,
    ) -> Result<Option<ApprovalPolicy>> {
        let policies = ApprovalPolicy::query()
            .where_(ApprovalPolicy::ACTION.eq(action))
            .where_(ApprovalPolicy::ACTIVE.eq(true))
            .get(db)
            .await?;
        Ok(strictest(policies, input))
    }
}

/// Of `policies`, the strictest one that matches `input`.
fn strictest(policies: Vec<ApprovalPolicy>, input: &Value) -> Option<ApprovalPolicy> {
    policies
        .into_iter()
        .filter(|p| p.matches(input))
        .max_by_key(|p| p.condition().rank())
}

/// A module's default policy, stored by [`install`] when the database has
/// no policy of the same action and condition.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "a default policy is only installed once registered"]
pub struct PolicyDefault {
    action: String,
    condition: Condition,
    input_key: Option<String>,
    steps: Vec<Step>,
    sod: bool,
    expiry_hours: Option<i64>,
}

impl PolicyDefault {
    /// Every run of `action` needs approval, with segregation of duties and
    /// the default expiry; add the steps with [`PolicyDefault::step`].
    pub fn new(action: impl Into<String>) -> Self {
        PolicyDefault {
            action: action.into(),
            condition: Condition::Always,
            input_key: None,
            steps: Vec::new(),
            sod: true,
            expiry_hours: Some(DEFAULT_EXPIRY_HOURS),
        }
    }

    /// The action key.
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Only runs whose amount (in fils) is above `fils`.
    pub fn amount_above(mut self, fils: i64) -> Self {
        self.condition = Condition::AmountAbove(fils);
        self
    }

    /// Only runs whose percentage (in basis points) is above `bps`.
    pub fn percentage_above(mut self, bps: i64) -> Self {
        self.condition = Condition::PercentageAbove(bps);
        self
    }

    /// The transition input's key the condition reads.
    pub fn input_key(mut self, key: impl Into<String>) -> Self {
        self.input_key = Some(key.into());
        self
    }

    /// Adds a step, decided by any holder of one of `roles`.
    pub fn step<S: Into<String>>(mut self, roles: impl IntoIterator<Item = S>) -> Self {
        self.steps.push(Step::one_of(roles));
        self
    }

    /// Adds a ready-made step, e.g. one built with [`Step::requiring`].
    pub fn with_step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    /// Adds a step, decided by any holder of `permission` in the subject's
    /// scope.
    pub fn step_permission(mut self, permission: impl Into<String>) -> Self {
        self.steps.push(Step::holders_of(permission));
        self
    }

    /// Adds a step, decided by any holder of one of `roles` or of
    /// `permission` in the subject's scope.
    pub fn step_or_permission<S: Into<String>>(
        mut self,
        roles: impl IntoIterator<Item = S>,
        permission: impl Into<String>,
    ) -> Self {
        self.steps
            .push(Step::one_of(roles).or_permission(permission));
        self
    }

    /// Turns segregation of duties off (it is on by default).
    pub fn without_sod(mut self) -> Self {
        self.sod = false;
        self
    }

    /// Hours a request may wait before it can expire; `None` for never.
    pub fn expiry_hours(mut self, hours: Option<i64>) -> Self {
        self.expiry_hours = hours;
        self
    }

    /// The row [`install`] stores.
    fn to_policy(&self) -> ApprovalPolicy {
        let (condition, threshold) = self.condition.parts();
        ApprovalPolicy {
            action: self.action.clone(),
            condition,
            threshold,
            input_key: self.input_key.clone(),
            steps: Json(self.steps.clone()),
            sod: self.sod,
            expiry_hours: self.expiry_hours,
            active: true,
            ..Default::default()
        }
    }

    /// Adds this default to the process-wide list [`install`] stores,
    /// replacing one of the same action and condition.
    ///
    /// # Panics
    ///
    /// When the policy has no step, or a step names no role and no
    /// permission: a programming error, found at boot.
    pub fn register(self) {
        assert!(
            !self.steps.is_empty(),
            "the approval policy of `{}` has no step",
            self.action
        );
        assert!(
            !self.steps.iter().any(Step::is_empty),
            "the approval policy of `{}` has a step nobody may decide",
            self.action
        );
        let mut defaults = DEFAULTS.write().unwrap_or_else(PoisonError::into_inner);
        defaults.retain(|d| (d.action.as_str(), d.condition) != (&*self.action, self.condition));
        defaults.push(self);
    }
}

static DEFAULTS: RwLock<Vec<PolicyDefault>> = RwLock::new(Vec::new());

/// The registered default policies.
pub fn defaults() -> Vec<PolicyDefault> {
    DEFAULTS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Stores the registered default policies the database lacks (by action,
/// condition and threshold). Policies already there are left as they are,
/// so changes made in the admin are kept. Returns how many were created.
pub async fn install(db: &Db) -> Result<usize> {
    let mut tx = db.begin().await?;
    let mut created = 0;
    for default in defaults() {
        let row = default.to_policy();
        let known = ApprovalPolicy::query()
            .where_(ApprovalPolicy::ACTION.eq(row.action.as_str()))
            .where_(ApprovalPolicy::CONDITION.eq(row.condition))
            .where_(ApprovalPolicy::THRESHOLD.eq(row.threshold))
            .exists(&mut tx)
            .await?;
        if known {
            continue;
        }
        ApprovalPolicy::create(&mut tx, row).await?;
        created += 1;
    }
    tx.commit().await?;
    Ok(created)
}

#[cfg(test)]
mod tests {
    use renox::serde_json::json;

    use super::*;

    fn policy(condition: Condition) -> ApprovalPolicy {
        PolicyDefault::new("payments.refund")
            .step(["finance"])
            .to_policy()
            .with_condition(condition)
    }

    impl ApprovalPolicy {
        fn with_condition(mut self, condition: Condition) -> Self {
            (self.condition, self.threshold) = condition.parts();
            self
        }
    }

    #[test]
    fn amount_conditions_read_the_input_in_fils() {
        let above_5000 = Condition::AmountAbove(500_000);
        assert!(above_5000.matches(&json!({ "amount": 500_001 }), None));
        assert!(!above_5000.matches(&json!({ "amount": 500_000 }), None));
        assert!(above_5000.matches(&json!({ "total": 900_000 }), Some("total")));
        assert!(!above_5000.matches(&json!({ "total": 100 }), Some("total")));
        assert!(Condition::Always.matches(&Value::Null, None));
    }

    #[test]
    fn an_unknown_amount_needs_approval() {
        let above = Condition::AmountAbove(500_000);
        assert!(above.matches(&json!({}), None));
        assert!(above.matches(&json!({ "amount": "a lot" }), None));
        assert!(Condition::PercentageAbove(1_000).matches(&Value::Null, None));
    }

    #[test]
    fn percentages_are_in_basis_points() {
        let above_10 = Condition::PercentageAbove(1_000);
        assert!(above_10.matches(&json!({ "percentage_bps": 1_001 }), None));
        assert!(!above_10.matches(&json!({ "percentage_bps": 1_000 }), None));
    }

    #[test]
    fn the_strictest_matching_policy_wins() {
        let policies = vec![
            policy(Condition::Always),
            policy(Condition::AmountAbove(1_000_000)),
            policy(Condition::AmountAbove(5_000_000)),
        ];
        let small = strictest(policies.clone(), &json!({ "amount": 500 })).unwrap();
        assert_eq!(small.condition(), Condition::Always);
        let large = strictest(policies.clone(), &json!({ "amount": 2_000_000 })).unwrap();
        assert_eq!(large.condition(), Condition::AmountAbove(1_000_000));
        let huge = strictest(policies, &json!({ "amount": 9_000_000 })).unwrap();
        assert_eq!(huge.condition(), Condition::AmountAbove(5_000_000));
    }

    #[test]
    fn an_inactive_policy_never_matches() {
        let mut inactive = policy(Condition::Always);
        inactive.active = false;
        assert!(strictest(vec![inactive], &Value::Null).is_none());
    }

    #[test]
    fn conditions_are_stored_as_kind_and_threshold() {
        for condition in [
            Condition::Always,
            Condition::AmountAbove(500_000),
            Condition::PercentageAbove(250),
        ] {
            let (kind, threshold) = condition.parts();
            assert_eq!(Condition::from_parts(kind, threshold), condition);
        }
        assert_eq!(ConditionKind::AmountAbove.as_str(), "amount_above");
    }

    #[test]
    fn a_default_has_sod_and_the_default_expiry() {
        let row = PolicyDefault::new("payments.refund")
            .amount_above(500_000)
            .step(["property_manager"])
            .step(["finance", "owner"])
            .to_policy();
        assert!(row.sod && row.active);
        assert_eq!(row.expiry_hours, Some(DEFAULT_EXPIRY_HOURS));
        assert_eq!(row.steps.len(), 2);
        assert!(row.steps[1].allows("owner"));
        assert!(!row.steps[0].allows("owner"));
    }

    #[test]
    fn a_step_may_name_a_permission() {
        let row = PolicyDefault::new("payments.refund")
            .step(["property_manager"])
            .step_permission("payments.approve_refund")
            .to_policy();
        assert_eq!(row.steps[0].permission, None);
        assert_eq!(row.steps[1], Step::holders_of("payments.approve_refund"));
        assert!(row.steps[1].roles.is_empty() && !row.steps[1].is_empty());
        let both = Step::one_of(["finance"]).or_permission("payments.approve_refund");
        assert!(both.allows("finance"));
        assert_eq!(both.permission.as_deref(), Some("payments.approve_refund"));
        assert!(Step::one_of(Vec::<String>::new()).is_empty());
    }

    #[test]
    fn steps_stored_without_a_permission_still_read() {
        let step: Step = renox::serde_json::from_value(json!({ "roles": ["finance"] })).unwrap();
        assert_eq!(step, Step::one_of(["finance"]));
        assert_eq!(step.required_permission, None);
        let strict = Step::one_of(["finance"]).requiring("parties.block_exception");
        let back: Step =
            renox::serde_json::from_value(renox::serde_json::to_value(&strict).unwrap()).unwrap();
        assert_eq!(back, strict);
        assert!(!strict.is_empty());
        assert_eq!(
            renox::serde_json::to_value(Step::one_of(["finance"])).unwrap(),
            json!({ "roles": ["finance"] })
        );
        assert_eq!(
            renox::serde_json::to_value(Step::holders_of("p")).unwrap(),
            json!({ "roles": [], "permission": "p" })
        );
    }

    #[test]
    #[should_panic(expected = "has a step nobody may decide")]
    fn a_step_naming_nobody_is_refused_at_boot() {
        PolicyDefault::new("tests.nobody")
            .step(Vec::<String>::new())
            .register();
    }
}
