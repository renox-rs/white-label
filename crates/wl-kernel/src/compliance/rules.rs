//! The stored side of the compliance-critical flag (wl-kernel §3, §7): who
//! is alerted for a rule, and since when it is watched for a missed run.
//!
//! The flag itself is code ([`super::CriticalRule`]): `install` writes
//! `critical` back to yes for every registered rule, so no client change
//! turns it off. The owner role is editable: the row's, else the one the
//! module named, else the deployment's business owner role.

use std::sync::Arc;

use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::registry::{all, rule};
use crate::config::ClientConfig;
use crate::scopes::COMPANY_ADMIN_ROLE;

/// One compliance-critical rule as stored (`wl_compliance_rules`).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_compliance_rules", unique(rule))]
pub struct ComplianceRule {
    /// The primary key.
    pub id: i64,
    /// The rule id, e.g. `HRM.RUL.03`.
    pub rule: String,
    /// The role alerted besides the company admin; `None` for the module's,
    /// else the deployment's business owner role.
    pub owner_role: Option<String>,
    /// Whether the rule is critical. Always written back to yes.
    pub critical: bool,
    /// Occurrences planned before this are not checked for a missed run:
    /// they happened before the platform watched the rule.
    pub watched_since: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ComplianceRule {
    /// The stored row of `rule`.
    pub async fn of(db: &Db, rule: &str) -> Result<Option<ComplianceRule>> {
        ComplianceRule::query()
            .where_(ComplianceRule::RULE.eq(rule))
            .first(db)
            .await
    }
}

/// Stores a row for each registered rule the database lacks, and writes
/// `critical` back to yes on every row of a registered rule (a client cannot
/// turn the flag off). A new row starts being watched now. Returns how many
/// rows were created.
pub async fn install(db: &Db) -> Result<usize> {
    let mut created = 0;
    for critical in all() {
        match ComplianceRule::of(db, critical.rule_id()).await? {
            Some(row) => {
                if !row.critical {
                    ComplianceRule::where_eq("id", row.id)
                        .update(db, &[("critical", &true), ("updated_at", &Some(now()))])
                        .await?;
                }
            }
            None => {
                ComplianceRule::create(
                    db,
                    ComplianceRule {
                        rule: critical.rule_id().to_owned(),
                        owner_role: critical.owner_role_name().map(str::to_owned),
                        critical: true,
                        watched_since: Some(now()),
                        ..Default::default()
                    },
                )
                .await?;
                created += 1;
            }
        }
    }
    Ok(created)
}

/// The business owner role of this deployment: `[roles] owner`, else the
/// company admin role.
pub fn business_owner_role(state: &AppState) -> String {
    state
        .provided::<Arc<ClientConfig>>()
        .map(|config| config.owner_role().to_owned())
        .unwrap_or_else(|| COMPANY_ADMIN_ROLE.to_owned())
}

/// The role alerted for `rule_id` besides the company admin: the stored
/// row's, else the module's, else the business owner role.
pub async fn owner_role(state: &AppState, rule_id: &str) -> Result<String> {
    if let Some(row) = ComplianceRule::of(&state.db, rule_id).await?
        && let Some(role) = row.owner_role.filter(|r| !r.trim().is_empty())
    {
        return Ok(role);
    }
    if let Some(role) = rule(rule_id).and_then(|r| r.owner_role_name().map(str::to_owned)) {
        return Ok(role);
    }
    Ok(business_owner_role(state))
}
