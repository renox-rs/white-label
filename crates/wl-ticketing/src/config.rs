//! Ticketing configuration rows: SLA policies and their targets, hold
//! reasons and categories (wl-ticketing §3, §7). The seeder ([`crate::seed`])
//! stores the defaults; a manager changes them afterwards.

use renox::db::{DateTime, Executor, Json};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::priority::Priority;

/// An SLA policy, e.g. `standard`.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_ticketing_sla_policies", unique(key))]
pub struct SlaPolicy {
    /// The primary key.
    pub id: i64,
    /// The key, e.g. `standard`.
    pub key: String,
    /// The English name.
    pub name_en: String,
    /// The Arabic name.
    pub name_ar: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SlaPolicy {
    /// The policy `key`, if any.
    pub async fn by_key<'c>(db: impl Executor<'c>, key: &str) -> Result<Option<SlaPolicy>> {
        SlaPolicy::query()
            .where_(SlaPolicy::KEY.eq(key))
            .first(db)
            .await
    }

    /// The policy's target for `priority`, if it has one.
    pub async fn target(&self, db: &Db, priority: Priority) -> Result<Option<SlaTarget>> {
        SlaTarget::query()
            .where_(SlaTarget::POLICY_ID.eq(self.id))
            .where_(SlaTarget::PRIORITY.eq(priority))
            .first(db)
            .await
    }

    /// The policy's targets, most urgent first.
    pub async fn targets(&self, db: &Db) -> Result<Vec<SlaTarget>> {
        let mut targets = SlaTarget::query()
            .where_(SlaTarget::POLICY_ID.eq(self.id))
            .get(db)
            .await?;
        targets.sort_by_key(|t| {
            Priority::ALL
                .iter()
                .position(|p| *p == t.priority)
                .unwrap_or(usize::MAX)
        });
        Ok(targets)
    }
}

/// The response and resolution targets of a policy for one priority, in
/// working minutes (one SLA day is nine working hours).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_ticketing_sla_targets", unique(policy_id, priority))]
pub struct SlaTarget {
    /// The primary key.
    pub id: i64,
    /// The policy.
    #[model(references = "wl_ticketing_sla_policies")]
    pub policy_id: i64,
    /// The priority it applies to.
    pub priority: Priority,
    /// Working minutes from the first assignment to the first response.
    pub response_minutes: i64,
    /// Working minutes from the first assignment to resolution.
    pub resolution_minutes: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A reason a ticket may be put on hold.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_ticketing_hold_reasons", unique(key))]
pub struct HoldReason {
    /// The primary key.
    pub id: i64,
    /// The key, e.g. `parts`.
    pub key: String,
    /// The English name.
    pub name_en: String,
    /// The Arabic name.
    pub name_ar: String,
    /// Whether the SLA clock stops while the ticket is on hold for it.
    pub pauses_sla: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl HoldReason {
    /// The hold reason `key`, if any.
    pub async fn by_key<'c>(db: impl Executor<'c>, key: &str) -> Result<Option<HoldReason>> {
        HoldReason::query()
            .where_(HoldReason::KEY.eq(key))
            .first(db)
            .await
    }

    /// Every hold reason, by key.
    pub async fn ordered(db: &Db) -> Result<Vec<HoldReason>> {
        HoldReason::query().order_by(HoldReason::KEY).get(db).await
    }
}

/// When a category makes the invoice draft of a chargeable ticket (§4.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InvoiceOn {
    /// At `resolve` (the default).
    #[default]
    Resolve,
    /// At close: `confirm`, `close` or the auto-close. At `resolve` the
    /// requester only sees the amount.
    Close,
}

/// A ticket category.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_ticketing_categories", unique(key))]
pub struct Category {
    /// The primary key.
    pub id: i64,
    /// The key, e.g. `general`.
    pub key: String,
    /// The English name.
    pub name_en: String,
    /// The Arabic name.
    pub name_ar: String,
    /// The priority a new ticket of the category gets.
    pub default_priority: Priority,
    /// The SLA policy of its tickets.
    #[model(references = "wl_ticketing_sla_policies")]
    pub sla_policy_id: Option<i64>,
    /// Whether the requester confirms the work before the ticket closes.
    pub requires_signoff: bool,
    /// Whether only `tickets.confidential.view` holders see its tickets.
    pub confidential: bool,
    /// Whether its tickets can carry charges.
    pub chargeable: bool,
    /// When its chargeable tickets are invoiced: at `resolve` or at close.
    pub invoice_on: InvoiceOn,
    /// Whether new tickets can use it.
    pub active: bool,
    /// Whether a covered subject that is not compliant for it gets a
    /// compliance flag (01: fire and life safety, lifts), set by the vertical.
    pub compliance_critical: bool,
    /// Whether the platform owns it (the categories other modules open
    /// tickets in): the client may rename it, not delete it or turn it off.
    pub platform: bool,
    /// The role whose holders look after its tickets, e.g. `hr`.
    pub default_assignee_role: Option<String>,
    /// The custom fields of its tickets.
    pub custom_fields: Json<Vec<CustomField>>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What kind of value a custom field takes.
#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// Free text.
    #[default]
    Text,
    /// A date.
    Date,
    /// One of the field's choices.
    Choice,
}

/// One custom field of a category's tickets (wl-ticketing §3 *custom fields
/// schema*). The schema is stored with the category; the values come with
/// the kernel's custom fields.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct CustomField {
    /// The key, e.g. `forum`.
    pub key: String,
    /// The English label.
    pub label_en: String,
    /// The Arabic label.
    pub label_ar: String,
    /// What it takes.
    pub kind: FieldKind,
    /// Whether a ticket needs it filled.
    pub required: bool,
    /// The choices of a `choice` field.
    pub choices: Vec<String>,
}

impl Category {
    /// The category `key`, if any.
    pub async fn by_key<'c>(db: impl Executor<'c>, key: &str) -> Result<Option<Category>> {
        Category::query()
            .where_(Category::KEY.eq(key))
            .first(db)
            .await
    }

    /// The name in `locale`: Arabic for `ar` when there is one, else English.
    pub fn name(&self, locale: &str) -> &str {
        if locale == "ar" && !self.name_ar.is_empty() {
            &self.name_ar
        } else {
            &self.name_en
        }
    }

    /// The category's SLA policy, if it has one.
    pub async fn sla_policy(&self, db: &Db) -> Result<Option<SlaPolicy>> {
        match self.sla_policy_id {
            Some(id) => SlaPolicy::find(db, id).await,
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_category_name_falls_back_to_english() {
        let mut category = Category {
            name_en: "General".to_owned(),
            ..Category::default()
        };
        assert_eq!(category.name("ar"), "General");
        category.name_ar = "عام".to_owned();
        assert_eq!(category.name("ar"), "عام");
        assert_eq!(category.name("en"), "General");
    }
}
