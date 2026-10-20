//! Changing the source or campaign of a party acquisition or a won lead
//! (`SAL.ACL.09`, wl-sales §5.8 step 3, issue #139).
//!
//! [`request`] stores an [`AcquisitionChange`] with its reason. A change of an
//! acquisition that has a first sale, or of a won lead, waits for another
//! holder of `campaigns.manage` (the setting
//! [`acquisition_change_approval`](crate::config::SalesConfig::acquisition_change_approval)
//! can switch that off); any other change is applied at once. [`approve`]
//! refuses the requester, [`reject`] leaves the attribution as it was. A
//! change that is applied is audited (`sales.acquisition_changed`) and an
//! acquisition's emits `CustomerAcquisitionChanged` after the commit.
//!
//! The status moves only through the machine: `apply` (system, when no
//! approval is needed), `approve` and `reject` (system, run by these services
//! for the user named in the run's input). The approval is this module's own
//! second-person rule on the change, not a kernel approval request: the
//! approver is any other user with `campaigns.manage`, which the page checks
//! before it calls [`approve`].

use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::sales::{CampaignId, CustomerAcquisitionChanged};

use crate::acquisition::PartyAcquisition;
use crate::campaign::Campaign;
use crate::config::config_of;
use crate::lead::{Lead, LeadStatus};
use crate::lead_source::LeadSource;
use crate::texts::english;

/// The audit action of an applied change.
pub const AUDIT_ACTION: &str = "sales.acquisition_changed";

/// `pending → applied`: no approval is needed.
pub const APPLY: &str = "apply";
/// `pending → applied`: another holder of `campaigns.manage` approved.
pub const APPROVE: &str = "approve";
/// `pending → rejected`.
pub const REJECT: &str = "reject";

/// Where a change stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChangeStatus {
    /// Waiting for a second person.
    #[default]
    Pending,
    /// Applied to the acquisition or the lead.
    Applied,
    /// Rejected; nothing changed.
    Rejected,
}

/// A requested change of source or campaign.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_acquisition_changes",
    state_machine,
    index(status)
)]
pub struct AcquisitionChange {
    /// The primary key.
    pub id: i64,
    /// `acquisition` or `lead`.
    pub kind: String,
    /// The acquisition changed.
    pub acquisition_id: Option<i64>,
    /// The lead changed.
    pub lead_id: Option<i64>,
    /// The source before.
    pub old_source_id: Option<i64>,
    /// The source after.
    pub new_source_id: i64,
    /// The campaign before.
    pub old_campaign_id: Option<i64>,
    /// The campaign after.
    pub new_campaign_id: Option<i64>,
    /// Why.
    pub reason: String,
    /// Who asked.
    pub requested_by: i64,
    /// Who approved or rejected.
    pub decided_by: Option<i64>,
    /// The reason given with a rejection.
    pub decision_reason: Option<String>,
    /// Where it stands; changed only by the machine.
    pub status: ChangeStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What a change is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeTarget {
    /// A party acquisition.
    Acquisition(i64),
    /// A lead.
    Lead(i64),
}

/// What a user asks for.
#[derive(Debug, Clone)]
pub struct ChangeRequest {
    /// The acquisition or lead.
    pub target: ChangeTarget,
    /// The lead source code it should have.
    pub source: String,
    /// The campaign code it should have, if any.
    pub campaign: Option<String>,
    /// Why; required.
    pub reason: String,
    /// The user who asks.
    pub requested_by: i64,
}

/// Why a change could not be requested or decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeRefused {
    /// The reason is blank.
    ReasonRequired,
    /// There is no such acquisition, lead or change.
    NotFound,
    /// There is no such lead source, or it is switched off.
    UnknownSource,
    /// There is no such campaign.
    UnknownCampaign,
    /// The source needs a campaign (`SAL.CRM.02`).
    CampaignRequired,
    /// Source and campaign are those it has.
    Unchanged,
    /// The requester cannot approve their own change (`SAL.ACL.09`).
    SamePerson,
    /// The change is no longer pending.
    NotPending,
}

impl ChangeRefused {
    /// The English explanation.
    pub fn message(self) -> String {
        let key = match self {
            ChangeRefused::ReasonRequired => "wl_sales.changes.reason_required",
            ChangeRefused::NotFound => "wl_sales.changes.not_found",
            ChangeRefused::UnknownSource => "wl_sales.changes.unknown_source",
            ChangeRefused::UnknownCampaign => "wl_sales.changes.unknown_campaign",
            ChangeRefused::CampaignRequired => "wl_sales.changes.campaign_required",
            ChangeRefused::Unchanged => "wl_sales.changes.unchanged",
            ChangeRefused::SamePerson => "wl_sales.changes.same_person",
            ChangeRefused::NotPending => "wl_sales.changes.not_pending",
        };
        english(key, &[])
    }

    /// The refusal as an HTTP 422.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message())
    }
}

/// What a decision run carries: the user who decides.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// The user who approves or rejects.
    pub by: i64,
}

fn decision_of(ctx: &TransitionCtx) -> Option<Decision> {
    ctx.input_as::<Decision>().ok()
}

/// Sets non-status columns of change `id`.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    AcquisitionChange::query()
        .where_(AcquisitionChange::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// Writes the change on its acquisition or lead.
async fn write_through(tx: &mut renox::db::Transaction, change: &AcquisitionChange) -> Result {
    if let Some(id) = change.acquisition_id {
        PartyAcquisition::query()
            .where_(PartyAcquisition::ID.eq(id))
            .update(
                &mut *tx,
                &[
                    ("source_id", &change.new_source_id),
                    ("campaign_id", &change.new_campaign_id),
                    ("source_missing", &false),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    if let Some(id) = change.lead_id {
        Lead::query()
            .where_(Lead::ID.eq(id))
            .update(
                &mut *tx,
                &[
                    ("source_id", &change.new_source_id),
                    ("campaign_id", &change.new_campaign_id),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    Ok(())
}

impl StateMachine for AcquisitionChange {
    type State = ChangeStatus;
    const COLUMN: renox::db::Col<AcquisitionChange, ChangeStatus> = AcquisitionChange::STATUS;

    fn machine() -> Machine<Self> {
        use ChangeStatus::{Applied, Pending, Rejected};
        Machine::new()
            .initial([Pending])
            .state(Pending)
            .tone("warning")
            .state(Applied)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("danger")
            .terminal()
            .transition(
                Transition::new(APPLY)
                    .label("Apply")
                    .system()
                    .from([Pending])
                    .to(Applied)
                    .effect(|c: &AcquisitionChange, _ctx, tx| {
                        let change = c.clone();
                        Box::pin(async move {
                            write_through(tx, &change).await?;
                            set(tx, change.id, &[("decided_by", &Some(change.requested_by))]).await
                        })
                    }),
            )
            .transition(
                Transition::new(APPROVE)
                    .label("Approve")
                    .system()
                    .from([Pending])
                    .to(Applied)
                    .guard("another_person", |c: &AcquisitionChange, ctx| {
                        let verdict = match decision_of(ctx) {
                            Some(d) if d.by != c.requested_by => Ok(()),
                            Some(_) => Err(english("wl_sales.changes.same_person", &[])),
                            None => Err(english("wl_sales.changes.no_decider", &[])),
                        };
                        Box::pin(async move { verdict })
                    })
                    .effect(|c: &AcquisitionChange, ctx, tx| {
                        let change = c.clone();
                        let by = decision_of(ctx).map(|d| d.by);
                        Box::pin(async move {
                            write_through(tx, &change).await?;
                            set(tx, change.id, &[("decided_by", &by)]).await
                        })
                    }),
            )
            .transition(
                Transition::new(REJECT)
                    .label("Reject")
                    .system()
                    .from([Pending])
                    .to(Rejected)
                    .guard("decider_given", |_c: &AcquisitionChange, ctx| {
                        let given = decision_of(ctx).is_some();
                        Box::pin(async move {
                            if given {
                                Ok(())
                            } else {
                                Err(english("wl_sales.changes.no_decider", &[]))
                            }
                        })
                    })
                    .effect(|c: &AcquisitionChange, ctx, tx| {
                        let id = c.id;
                        let by = decision_of(ctx).map(|d| d.by);
                        let why = ctx
                            .reason()
                            .map(str::trim)
                            .filter(|r| !r.is_empty())
                            .map(str::to_owned);
                        Box::pin(async move {
                            set(tx, id, &[("decided_by", &by), ("decision_reason", &why)]).await
                        })
                    }),
            )
    }
}

/// Whether changing `target` needs a second person: it has a first sale, or
/// it is a won lead, and the setting is on.
async fn needs_approval(state: &AppState, target: ChangeTarget) -> Result<Option<bool>> {
    let on = config_of(state).acquisition_change_approval;
    Ok(match target {
        ChangeTarget::Acquisition(id) => PartyAcquisition::find(&state.db, id)
            .await?
            .map(|a| on && a.first_sale().is_some()),
        ChangeTarget::Lead(id) => Lead::find(&state.db, id)
            .await?
            .map(|l| on && l.status == LeadStatus::Won),
    })
}

/// Stores the change `request` asks for. Pending when it needs a second
/// person, else applied at once. Refused without a reason, for an unknown
/// source or campaign, for a source that needs a campaign, and when nothing
/// would change.
pub async fn request(
    state: &AppState,
    request: ChangeRequest,
) -> Result<std::result::Result<AcquisitionChange, ChangeRefused>> {
    if request.reason.trim().is_empty() {
        return Ok(Err(ChangeRefused::ReasonRequired));
    }
    let Some(needs) = needs_approval(state, request.target).await? else {
        return Ok(Err(ChangeRefused::NotFound));
    };
    let source = match LeadSource::by_code(&state.db, &LeadSource::normalize_code(&request.source)).await? {
        Some(source) if source.active => source,
        _ => return Ok(Err(ChangeRefused::UnknownSource)),
    };
    let campaign = match request.campaign.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(code) => match Campaign::by_code(&state.db, code).await? {
            Some(campaign) => Some(campaign),
            None => return Ok(Err(ChangeRefused::UnknownCampaign)),
        },
        None => None,
    };
    if campaign.is_none() && config_of(state).campaign_required_for(&source.code) {
        return Ok(Err(ChangeRefused::CampaignRequired));
    }
    let (old_source, old_campaign, acquisition_id, lead_id, kind) = match request.target {
        ChangeTarget::Acquisition(id) => {
            let Some(a) = PartyAcquisition::find(&state.db, id).await? else {
                return Ok(Err(ChangeRefused::NotFound));
            };
            (a.source_id, a.campaign_id, Some(id), None, "acquisition")
        }
        ChangeTarget::Lead(id) => {
            let Some(l) = Lead::find(&state.db, id).await? else {
                return Ok(Err(ChangeRefused::NotFound));
            };
            (l.source_id, l.campaign_id, None, Some(id), "lead")
        }
    };
    let new_campaign = campaign.as_ref().map(|c| c.id);
    if old_source == source.id && old_campaign == new_campaign {
        return Ok(Err(ChangeRefused::Unchanged));
    }
    let change = AcquisitionChange::create(
        &state.db,
        AcquisitionChange {
            kind: kind.to_owned(),
            acquisition_id,
            lead_id,
            old_source_id: Some(old_source),
            new_source_id: source.id,
            old_campaign_id: old_campaign,
            new_campaign_id: new_campaign,
            reason: request.reason.trim().to_owned(),
            requested_by: request.requested_by,
            ..AcquisitionChange::default()
        },
    )
    .await?;
    if !needs {
        let _outcome = change
            .transition(APPLY)
            .by_system()
            .run(state)
            .await
            .map_err(|e| e.into_error())?;
    }
    Ok(Ok(AcquisitionChange::find_or_404(&state.db, change.id).await?))
}

/// Approves pending change `id` as user `by`, who must be another user than
/// the requester (`SAL.ACL.09`); the caller checks `campaigns.manage`.
pub async fn approve(
    state: &AppState,
    id: i64,
    by: i64,
) -> Result<std::result::Result<AcquisitionChange, ChangeRefused>> {
    let Some(change) = AcquisitionChange::find(&state.db, id).await? else {
        return Ok(Err(ChangeRefused::NotFound));
    };
    if change.status != ChangeStatus::Pending {
        return Ok(Err(ChangeRefused::NotPending));
    }
    if change.requested_by == by {
        return Ok(Err(ChangeRefused::SamePerson));
    }
    let _outcome = change
        .transition(APPROVE)
        .by_system()
        .input(&Decision { by })
        .meta(json!({ "ran_by": by }))
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(Ok(AcquisitionChange::find_or_404(&state.db, id).await?))
}

/// Rejects pending change `id` as user `by`; nothing changes.
pub async fn reject(
    state: &AppState,
    id: i64,
    by: i64,
    reason: &str,
) -> Result<std::result::Result<AcquisitionChange, ChangeRefused>> {
    let Some(change) = AcquisitionChange::find(&state.db, id).await? else {
        return Ok(Err(ChangeRefused::NotFound));
    };
    if change.status != ChangeStatus::Pending {
        return Ok(Err(ChangeRefused::NotPending));
    }
    let mut run = change
        .transition(REJECT)
        .by_system()
        .input(&Decision { by })
        .meta(json!({ "ran_by": by }));
    if !reason.trim().is_empty() {
        run = run.reason(reason.trim());
    }
    let _outcome = run.run(state).await.map_err(|e| e.into_error())?;
    Ok(Ok(AcquisitionChange::find_or_404(&state.db, id).await?))
}

/// After the commit of an applied change: the audit record and, for an
/// acquisition, `CustomerAcquisitionChanged`.
async fn applied(state: &AppState, id: i64) -> Result {
    let Some(change) = AcquisitionChange::find(&state.db, id).await? else {
        return Ok(());
    };
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new(AUDIT_ACTION)
            .user(change.decided_by.unwrap_or(change.requested_by))
            .subject(AcquisitionChange::TABLE, change.id)
            .data(json!({
                "kind": change.kind,
                "acquisition_id": change.acquisition_id,
                "lead_id": change.lead_id,
                "old_source_id": change.old_source_id,
                "new_source_id": change.new_source_id,
                "old_campaign_id": change.old_campaign_id,
                "new_campaign_id": change.new_campaign_id,
                "reason": change.reason,
                "requested_by": change.requested_by,
                "approved_by": change.decided_by,
            })),
    )
    .await?;
    let Some(acquisition_id) = change.acquisition_id else {
        return Ok(());
    };
    let Some(acquisition) = PartyAcquisition::find(&state.db, acquisition_id).await? else {
        return Ok(());
    };
    let code = |id: Option<i64>| async move {
        match id {
            Some(id) => Ok::<_, Error>(LeadSource::find(&state.db, id).await?.map(|s| s.code)),
            None => Ok(None),
        }
    };
    let old_source = code(change.old_source_id).await?;
    let new_source = code(Some(change.new_source_id)).await?;
    state
        .emit(CustomerAcquisitionChanged {
            party: PartyId(acquisition.party_id),
            old_source,
            new_source,
            old_campaign: change.old_campaign_id.map(CampaignId),
            new_campaign: change.new_campaign_id.map(CampaignId),
            old_first_sale: acquisition.first_sale(),
            new_first_sale: acquisition.first_sale(),
            reason: change.reason.clone(),
        })
        .await
}

/// Registers the listener of applied changes.
pub(crate) fn register(app: &mut Registry) {
    renox::state_machine::register::<AcquisitionChange>();
    app.listen(
        |event: Transitioned<AcquisitionChange>, state: AppState| async move {
            let name = event.transition.as_str();
            if ![APPLY, APPROVE].contains(&name) {
                return Ok(());
            }
            applied(&state, event.key).await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_three_system_transitions_out_of_pending() {
        let machine = AcquisitionChange::machine();
        assert_eq!(machine.transitions().len(), 3);
        for (name, to) in [(APPLY, "applied"), (APPROVE, "applied"), (REJECT, "rejected")] {
            let t = machine.find(name).expect(name);
            assert!(t.is_system(), "{name}");
            assert_eq!(t.from_states(), ["pending"], "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        assert_eq!(machine.initial_states(), ["pending"]);
    }

    #[renox::test]
    async fn the_requester_cannot_approve() {
        let machine = AcquisitionChange::machine();
        let change = AcquisitionChange {
            requested_by: 7,
            ..AcquisitionChange::default()
        };
        let ctx = |by: i64| {
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(Decision { by }).unwrap())
        };
        let approve = machine.find(APPROVE).unwrap();
        assert!(approve.check_guards(&change, &ctx(7)).await.is_err());
        assert!(approve.check_guards(&change, &ctx(8)).await.is_ok());
        assert!(approve
            .check_guards(&change, &TransitionCtx::new(None))
            .await
            .is_err());
    }
}
