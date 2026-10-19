//! A campaign cost entry (wl-sales §4, #138 A11): money spent on a
//! campaign, counted in its budget only once `approved`. The status moves
//! only through the machine ([`crate::cost_machine`]).

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::sales::CAMPAIGN_COST_ENTRY_SUBJECT_TYPE;

/// Where a cost entry stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CostStatus {
    /// Being saved, or returned, withdrawn or expired; editable, never counted.
    #[default]
    Draft,
    /// Linked to a supplier bill that is not approved yet; never counted.
    AwaitingBill,
    /// Over the budget, waiting for a holder of `campaigns.costs.manage`.
    PendingApproval,
    /// Counted in the campaign's costs.
    Approved,
    /// An approver said no.
    Rejected,
    /// Entered in error, or its bill was voided.
    Voided,
}

/// A cost entry of a campaign.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_campaign_cost_entries",
    state_machine,
    index(campaign_id, status)
)]
pub struct CostEntry {
    /// The primary key.
    pub id: i64,
    /// The campaign.
    #[model(references = "wl_sales_campaigns")]
    pub campaign_id: i64,
    /// The day of the cost.
    pub entry_date: NaiveDate,
    /// Fils of AED; negative for a credit.
    pub amount: i64,
    /// What it was for.
    pub description: String,
    /// The supplier bill it comes from, if linked.
    pub supplier_bill_id: Option<i64>,
    /// The supplier bill line it comes from, if linked.
    pub supplier_bill_line_id: Option<i64>,
    /// Who linked the bill line.
    pub linked_by: Option<i64>,
    /// Who entered it.
    pub requested_by: i64,
    /// Who gave the last yes; empty when it was approved on save.
    pub approved_by: Option<i64>,
    /// The kernel approval request, while or after one was needed.
    pub approval_request_id: Option<i64>,
    /// Why it was rejected or voided.
    pub reason: Option<String>,
    /// The campaign's branch scope type, copied; empty for global.
    pub scope_type: Option<String>,
    /// The campaign's branch scope id, copied; empty for global.
    pub scope_id: Option<String>,
    /// Where it stands; changed only by the machine.
    pub status: CostStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CostEntry {
    /// The entry as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(CAMPAIGN_COST_ENTRY_SUBJECT_TYPE, self.id)
    }

    /// Where approvers are looked for: the campaign's branch, else global.
    pub fn scope(&self) -> Scope {
        match (&self.scope_type, &self.scope_id) {
            (Some(kind), Some(id)) => Scope::new(kind, id),
            _ => Scope::global(),
        }
    }
}

/// Why a cost entry could not be saved or edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostRefused {
    /// There is no such campaign.
    CampaignNotFound,
    /// There is no such entry.
    NotFound,
    /// The amount is zero.
    NoAmount,
    /// A negative entry needs `campaigns.costs.manage`.
    NegativeNeedsManager,
    /// Only the one who entered it edits it.
    NotRequester,
    /// An approved entry is never edited.
    Approved,
    /// The entry is in a state that cannot be edited.
    NotEditable,
}

impl CostRefused {
    /// The English explanation.
    pub fn message(self) -> String {
        let key = match self {
            CostRefused::CampaignNotFound => "wl_sales.costs.campaign_not_found",
            CostRefused::NotFound => "wl_sales.costs.not_found",
            CostRefused::NoAmount => "wl_sales.costs.no_amount",
            CostRefused::NegativeNeedsManager => "wl_sales.costs.negative_needs_manager",
            CostRefused::NotRequester => "wl_sales.costs.not_requester",
            CostRefused::Approved => "wl_sales.costs.approved",
            CostRefused::NotEditable => "wl_sales.costs.not_editable",
        };
        crate::texts::english(key, &[])
    }

    /// The refusal as an HTTP 422.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_entry_is_a_draft_in_the_global_scope() {
        let e = CostEntry::default();
        assert_eq!(e.status, CostStatus::Draft);
        assert!(e.scope().is_global());
    }

    #[test]
    fn the_stored_branch_is_the_approval_scope() {
        let e = CostEntry {
            scope_type: Some("property.site".into()),
            scope_id: Some("4".into()),
            ..CostEntry::default()
        };
        assert_eq!(e.scope(), Scope::new("property.site", 4));
        assert_eq!(e.subject().subject_type, "sales.campaign_cost_entry");
    }

    #[test]
    fn every_refusal_has_a_text() {
        for r in [
            CostRefused::CampaignNotFound,
            CostRefused::NotFound,
            CostRefused::NoAmount,
            CostRefused::NegativeNeedsManager,
            CostRefused::NotRequester,
            CostRefused::Approved,
            CostRefused::NotEditable,
        ] {
            assert!(!r.message().starts_with("wl_sales."), "{r:?}");
        }
    }
}
