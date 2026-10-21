//! Retention campaigns (wl-sales §4.6 `activate` and `end`, §5.10;
//! BL-SAL-25): a campaign that sends one marketing message to a segment
//! through `wl-notify`.
//!
//! **Activate.** When a retention campaign becomes `active` (by a user or on
//! its start date) a listener asks `Notifier::broadcast` for one broadcast
//! at the send time (one already past goes at once). The requester is the
//! user who last set the segment, template, channel or send time
//! (`retention_set_by`), so that person cannot approve it (`NTF.ACL.04`),
//! also when the campaign activates by itself. `wl-notify` owns the estimate,
//! the approval above its limit, the send window, the frequency cap and the
//! consent checks; this module only asks.
//!
//! **Not sent.** A broadcast `wl-notify` refuses, rejects, cancels or lets
//! expire leaves the campaign `active` with the reason shown as *not sent*;
//! staff fix the fields and `request_broadcast` asks again. A campaign has
//! at most one open request and at most one completed broadcast, so a
//! `reopen` never sends again.
//!
//! **End.** Ending the campaign cancels a broadcast still waiting or
//! scheduled (`Notifier::cancel_broadcast`); `wl-notify` also asks
//! `CampaignRequesters` when the broadcast starts, and cancels it if the
//! campaign or a vertical's guard (05 `CLN.ACL.13`, `CampaignGuard`) no
//! longer allows the send.
//!
//! **Report.** `report` shows recipients, delivered, opted out, the members
//! left out by reason (counts only, never names) and the recipients whose
//! message was sent who bought within the response days after the send, with
//! their value before VAT, against the campaign's cost.
//!
//! Decisions:
//! - A sale counts on the Asia/Dubai day it completed, so a purchase on the
//!   day of the send counts as after it.
//! - A recipient in two campaigns counts in both.

use std::collections::BTreeMap;
use std::sync::Arc;

use renox::chrono::TimeDelta;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::Transitioned;
use serde::{Deserialize, Serialize};
use wl_kernel::broadcast::{
    BroadcastCancelled, BroadcastCompleted, BroadcastId, BroadcastRequest, BroadcastRequesters,
    RequesterCheck, RequesterFuture,
};
use wl_kernel::documents::dubai_date;
use wl_kernel::notify::{self, NotifyChannel};
use wl_kernel::sales::CAMPAIGN_SUBJECT_TYPE;
use wl_kernel::{SegmentId, Subject};

use crate::campaign::{Campaign, CampaignStatus, Purpose};
use crate::config::config_of;
use crate::costs;
use crate::machine::{ACTIVATE, ACTIVATE_ON_START, END, END_ON_END_DATE};
use crate::segments::{RuleFuture, spend_between};
use crate::texts::english;

/// Why a broadcast was cancelled when the campaign ended.
pub const CAMPAIGN_ENDED: &str = "campaign_ended";

/// How a request for a broadcast stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BroadcastState {
    /// Asked for; waiting for approval, scheduled, sending.
    #[default]
    Requested,
    /// Every recipient has a message or an exclusion.
    Completed,
    /// Rejected, withdrawn, cancelled, expired or its campaign ended.
    Cancelled,
    /// `wl-notify` refused the request.
    Failed,
}

/// A request of a campaign for a broadcast, and how it ended.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_campaign_broadcasts",
    index(campaign_id),
    index(broadcast_id)
)]
pub struct CampaignBroadcast {
    /// The primary key.
    pub id: i64,
    /// The campaign.
    #[model(references = "wl_sales_campaigns")]
    pub campaign_id: i64,
    /// The `wl-notify` broadcast; empty when the request was refused.
    pub broadcast_id: Option<i64>,
    /// The user named as the requester (`NTF.ACL.04`).
    pub requested_by: i64,
    /// How the request stands.
    pub status: BroadcastState,
    /// Why it did not go: the refusal, or the reason it was cancelled.
    pub reason: Option<String>,
    /// The send time asked for.
    pub send_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CampaignBroadcast {
    /// The requests of `campaign`, oldest first.
    pub async fn of_campaign(db: &Db, campaign: i64) -> Result<Vec<CampaignBroadcast>> {
        CampaignBroadcast::query()
            .where_(CampaignBroadcast::CAMPAIGN_ID.eq(campaign))
            .order_by(CampaignBroadcast::ID)
            .get(db)
            .await
    }

    /// The request that became the `wl-notify` broadcast `broadcast`.
    pub async fn of_broadcast(db: &Db, broadcast: i64) -> Result<Option<CampaignBroadcast>> {
        CampaignBroadcast::query()
            .where_(CampaignBroadcast::BROADCAST_ID.eq(broadcast))
            .first(db)
            .await
    }
}

/// Why a broadcast was not requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionRefused {
    /// There is no such campaign.
    NotFound,
    /// Only a retention campaign sends a message.
    NotRetention,
    /// The campaign is not `active`.
    NotActive,
    /// A retention field is missing.
    Incomplete,
    /// A request is still open: wait for it or cancel it.
    Open,
    /// The campaign has a completed broadcast; a new campaign is needed.
    AlreadySent,
}

impl RetentionRefused {
    /// The English explanation.
    pub fn message(self) -> String {
        let key = match self {
            RetentionRefused::NotFound => "wl_sales.retention.problem.not_found",
            RetentionRefused::NotRetention => "wl_sales.retention.problem.not_retention",
            RetentionRefused::NotActive => "wl_sales.retention.problem.not_active",
            RetentionRefused::Incomplete => "wl_sales.retention.problem.incomplete",
            RetentionRefused::Open => "wl_sales.retention.problem.open",
            RetentionRefused::AlreadySent => "wl_sales.retention.problem.already_sent",
        };
        english(key, &[])
    }

    /// The refusal as an HTTP 422.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message())
    }
}

/// A rule of a vertical that stops a campaign sending (05 `CLN.ACL.13`: an
/// advertising permit for a campaign that promotes services). It is asked
/// when `wl-notify` takes the request and again when the broadcast starts.
pub trait CampaignGuard: Send + Sync + 'static {
    /// A code for why `campaign` may not send now, or `None` when it may.
    fn check<'a>(
        &'a self,
        state: &'a AppState,
        campaign: &'a Campaign,
    ) -> RuleFuture<'a, Option<String>>;
}

/// The vertical's campaign guards, provided by the module.
#[derive(Clone, Default)]
pub struct CampaignGuards(pub Vec<Arc<dyn CampaignGuard>>);

/// Answers `wl-notify` whether a campaign may still start a broadcast
/// (`NTF.ACL.04`): it is `active` and no vertical guard objects.
#[derive(Debug, Clone, Copy, Default)]
pub struct CampaignRequesters;

impl BroadcastRequesters for CampaignRequesters {
    fn check<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
        _at: DateTime,
    ) -> RequesterFuture<'a, RequesterCheck> {
        Box::pin(async move {
            if subject.subject_type != CAMPAIGN_SUBJECT_TYPE {
                return Ok(RequesterCheck::Ended);
            }
            let Some(campaign) = Campaign::find(&state.db, subject.subject_id).await? else {
                return Ok(RequesterCheck::Ended);
            };
            if campaign.status != CampaignStatus::Active {
                return Ok(RequesterCheck::Ended);
            }
            let guards = state
                .provided::<CampaignGuards>()
                .map(|g| (*g).clone())
                .unwrap_or_default();
            for guard in &guards.0 {
                if let Some(code) = guard.check(state, &campaign).await? {
                    return Ok(RequesterCheck::GuardFailed(code));
                }
            }
            Ok(RequesterCheck::Active)
        })
    }
}

/// Asks `wl-notify` for the broadcast of the retention campaign `campaign`,
/// as the user who last set its retention fields. A request `wl-notify`
/// refuses is kept as `failed` with the refusal as the reason, and the
/// campaign stays `active`.
///
/// # Errors
///
/// The database's errors. A refusal that does not need `wl-notify` is the
/// inner `Err`.
pub async fn request_broadcast(
    state: &AppState,
    campaign: i64,
) -> Result<std::result::Result<CampaignBroadcast, RetentionRefused>> {
    let Some(campaign) = Campaign::find(&state.db, campaign).await? else {
        return Ok(Err(RetentionRefused::NotFound));
    };
    if campaign.purpose != Some(Purpose::Retention) {
        return Ok(Err(RetentionRefused::NotRetention));
    }
    if campaign.status != CampaignStatus::Active {
        return Ok(Err(RetentionRefused::NotActive));
    }
    let earlier = CampaignBroadcast::of_campaign(&state.db, campaign.id).await?;
    if earlier
        .iter()
        .any(|b| b.status == BroadcastState::Completed)
    {
        return Ok(Err(RetentionRefused::AlreadySent));
    }
    if earlier
        .iter()
        .any(|b| b.status == BroadcastState::Requested)
    {
        return Ok(Err(RetentionRefused::Open));
    }
    let (Some(by), Some(segment), Some(key), Some(channel), Some(send_at)) = (
        campaign.retention_set_by,
        campaign.segment_id,
        campaign.template_key.clone(),
        campaign.message_channel.clone(),
        campaign.send_at,
    ) else {
        return Ok(Err(RetentionRefused::Incomplete));
    };
    // A send time already past sends at once.
    let when = send_at.max(now());
    let mut row = CampaignBroadcast::create(
        &state.db,
        CampaignBroadcast {
            campaign_id: campaign.id,
            requested_by: by,
            send_at: Some(when),
            ..CampaignBroadcast::default()
        },
    )
    .await?;
    let asked = match channel.parse::<NotifyChannel>() {
        Ok(channel) => {
            let request = BroadcastRequest::new(
                &key,
                campaign.campaign_id().subject(),
                by,
                channel,
                SegmentId(segment),
                when,
            );
            notify::notifier(state)
                .broadcast(state, request)
                .await
                .map_err(|e| e.to_string())
        }
        Err(_) => Err(english("wl_sales.retention.problem.channel", &[])),
    };
    match asked {
        Ok(id) => {
            row.broadcast_id = Some(id.0);
            CampaignBroadcast::query()
                .where_(CampaignBroadcast::ID.eq(row.id))
                .update(
                    &state.db,
                    &[("broadcast_id", &Some(id.0)), ("updated_at", &now())],
                )
                .await?;
        }
        Err(reason) => {
            row.status = BroadcastState::Failed;
            row.reason = Some(reason.clone());
            CampaignBroadcast::query()
                .where_(CampaignBroadcast::ID.eq(row.id))
                .update(
                    &state.db,
                    &[
                        ("status", &BroadcastState::Failed),
                        ("reason", &Some(reason)),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
    }
    Ok(Ok(row))
}

/// The campaign ended: a broadcast still waiting for approval or scheduled is
/// cancelled, and the request is marked.
async fn cancel_open(state: &AppState, campaign: &Campaign) -> Result {
    let rows = CampaignBroadcast::of_campaign(&state.db, campaign.id).await?;
    for row in rows
        .iter()
        .filter(|r| r.status == BroadcastState::Requested)
    {
        let Some(broadcast) = row.broadcast_id else {
            continue;
        };
        let cancelled = notify::notifier(state)
            .cancel_broadcast(state, BroadcastId(broadcast), CAMPAIGN_ENDED)
            .await?;
        if cancelled {
            mark_cancelled(state, broadcast, CAMPAIGN_ENDED).await?;
        }
    }
    Ok(())
}

/// Marks the request of `broadcast` cancelled with `reason`, once.
async fn mark_cancelled(state: &AppState, broadcast: i64, reason: &str) -> Result {
    CampaignBroadcast::query()
        .where_(CampaignBroadcast::BROADCAST_ID.eq(broadcast))
        .where_(CampaignBroadcast::STATUS.eq(BroadcastState::Requested))
        .update(
            &state.db,
            &[
                ("status", &BroadcastState::Cancelled),
                ("reason", &Some(reason.to_owned())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// A campaign became active or ended.
async fn on_campaign(state: &AppState, event: &Transitioned<Campaign>) -> Result {
    let name = event.transition.as_str();
    let activated = name == ACTIVATE || name == ACTIVATE_ON_START;
    let ended = name == END || name == END_ON_END_DATE;
    if !activated && !ended {
        return Ok(());
    }
    let Some(campaign) = Campaign::find(&state.db, event.key).await? else {
        return Ok(());
    };
    if campaign.purpose != Some(Purpose::Retention) {
        return Ok(());
    }
    if activated {
        // A refusal (an incomplete campaign cannot be active) asks nothing.
        let _asked = request_broadcast(state, campaign.id).await?;
        Ok(())
    } else {
        cancel_open(state, &campaign).await
    }
}

/// `wl-notify` completed a broadcast of a campaign.
async fn on_completed(state: &AppState, event: &BroadcastCompleted) -> Result {
    if event.requester.subject_type != CAMPAIGN_SUBJECT_TYPE {
        return Ok(());
    }
    CampaignBroadcast::query()
        .where_(CampaignBroadcast::BROADCAST_ID.eq(event.broadcast.0))
        .where_(CampaignBroadcast::STATUS.eq(BroadcastState::Requested))
        .update(
            &state.db,
            &[
                ("status", &BroadcastState::Completed),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// `wl-notify` cancelled a broadcast of a campaign: it shows *not sent* with
/// the approver's comment, or the code of the reason.
async fn on_cancelled(state: &AppState, event: &BroadcastCancelled) -> Result {
    if event.requester.subject_type != CAMPAIGN_SUBJECT_TYPE {
        return Ok(());
    }
    let stored = notify::notifier(state)
        .broadcast_report(state, event.broadcast)
        .await?
        .and_then(|r| r.cancel_reason)
        .filter(|r| !r.trim().is_empty());
    let reason = stored.unwrap_or_else(|| event.reason.as_str().to_owned());
    mark_cancelled(state, event.broadcast.0, &reason).await
}

/// How a retention campaign stands (§5.10 step 4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetentionReport {
    /// The state of the latest request, if the campaign asked for one.
    pub state: Option<BroadcastState>,
    /// Why nothing was sent: the refusal or the reason it was cancelled.
    pub not_sent: Option<String>,
    /// The `wl-notify` status of the broadcast.
    pub status: Option<String>,
    /// The messages queued; `None` before the send.
    pub recipients: Option<i64>,
    /// The messages delivered.
    pub delivered: i64,
    /// The recipients who opted out.
    pub opted_out: i64,
    /// The members left out, by reason: counts only.
    pub excluded: BTreeMap<String, i64>,
    /// The recipients whose message was sent who bought within the response
    /// days after the send.
    pub returned: i64,
    /// What they spent in those days before VAT, in fils.
    pub returned_value: i64,
    /// The campaign's approved cost, in fils.
    pub cost: i64,
    /// The response days in force.
    pub response_days: i64,
}

/// The report of the campaign `campaign` now.
///
/// # Errors
///
/// The database's errors.
pub async fn report(state: &AppState, campaign: &Campaign) -> Result<RetentionReport> {
    let rows = CampaignBroadcast::of_campaign(&state.db, campaign.id).await?;
    let current = rows
        .iter()
        .rev()
        .find(|r| r.status == BroadcastState::Completed)
        .or_else(|| rows.last());
    let mut out = RetentionReport {
        state: current.map(|r| r.status),
        response_days: config_of(state).retention_response_days,
        cost: costs::totals(&state.db, campaign.id, None).await?.approved,
        ..RetentionReport::default()
    };
    let Some(row) = current else {
        return Ok(out);
    };
    if matches!(
        row.status,
        BroadcastState::Cancelled | BroadcastState::Failed
    ) {
        out.not_sent = Some(row.reason.clone().unwrap_or_default());
    }
    let Some(id) = row.broadcast_id else {
        return Ok(out);
    };
    let Some(sent) = notify::notifier(state)
        .broadcast_report(state, BroadcastId(id))
        .await?
    else {
        return Ok(out);
    };
    out.status = Some(sent.status.clone());
    out.recipients = sent.recipients;
    out.delivered = sent.delivered;
    out.opted_out = sent.opted_out;
    out.excluded = sent.excluded.clone();
    if row.status == BroadcastState::Completed && !sent.reached.is_empty() {
        let from = dubai_date(sent.send_at);
        let to = from + TimeDelta::days(out.response_days);
        let spend = spend_between(&state.db, from, to).await?;
        for party in &sent.reached {
            if let Some(value) = spend.get(&party.0) {
                out.returned += 1;
                out.returned_value += value;
            }
        }
    }
    Ok(out)
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    wl_kernel::broadcast::provide_requesters(app, CampaignRequesters);
    wl_kernel::segments::provide_segments(app, crate::segments::SalesSegments);
    app.listen(
        |event: Transitioned<Campaign>, state: AppState| async move {
            on_campaign(&state, &event).await
        },
    );
    app.listen(|event: BroadcastCompleted, state: AppState| async move {
        on_completed(&state, &event).await
    });
    app.listen(|event: BroadcastCancelled, state: AppState| async move {
        on_cancelled(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_are_stored_in_snake_case() {
        let names: Vec<_> = BroadcastState::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(names, ["requested", "completed", "cancelled", "failed"]);
    }

    #[test]
    fn every_refusal_has_a_message() {
        for refused in [
            RetentionRefused::NotFound,
            RetentionRefused::NotRetention,
            RetentionRefused::NotActive,
            RetentionRefused::Incomplete,
            RetentionRefused::Open,
            RetentionRefused::AlreadySent,
        ] {
            assert!(!refused.message().starts_with("wl_sales."));
        }
    }
}
