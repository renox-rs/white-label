//! The Campaign state machine (wl-sales §4.6, #138 A7).
//!
//! ```text
//! [*] draft --activate / activate_on_start--> active --end / end_on_end_date--> ended
//!                                             active <--------reopen----------- ended
//! ```
//!
//! `activate` and `end` each have a user transition (`campaigns.manage`) and
//! a system twin, because a Renox transition is either user or system. The
//! twins share the guards and effects. The system twins also check their
//! date, so a run on the wrong day is refused.
//!
//! The date the effects and guards call "today" is the run's metadata
//! `{"today": "YYYY-MM-DD"}` (the daily run gives its date), else today in
//! Asia/Dubai. The events and the owner's notice follow the commit
//! ([`crate::events`]).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_date;

use crate::campaign::{Campaign, CampaignStatus, Purpose};
use crate::permissions;
use crate::promo_code::PromoCode;

/// A manager makes a draft campaign active.
pub const ACTIVATE: &str = "activate";
/// The system activates a draft whose start date is reached.
pub const ACTIVATE_ON_START: &str = "activate_on_start";
/// A manager ends an active campaign.
pub const END: &str = "end";
/// The system ends an active campaign past its end date.
pub const END_ON_END_DATE: &str = "end_on_end_date";
/// A manager runs an ended campaign again until a new end date.
pub const REOPEN: &str = "reopen";

/// What `reopen` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReopenInput {
    /// The new last day; after today.
    pub end_date: NaiveDate,
}

/// The run's metadata that tells the machine the date.
pub fn today_meta(today: NaiveDate) -> renox::serde_json::Value {
    renox::serde_json::json!({ "today": today.to_string() })
}

/// The date of the run: the metadata's `today`, else today in Asia/Dubai.
fn today_of(ctx: &TransitionCtx) -> NaiveDate {
    ctx.meta()
        .get("today")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(wl_kernel::documents::today)
}

impl Campaign {
    /// Why the campaign cannot be activated yet, if it cannot (§4.6 `activate`).
    pub fn activation_problem(&self) -> Option<&'static str> {
        if self.name.trim().is_empty() {
            return Some("The campaign needs its name.");
        }
        if self.code.trim().is_empty() {
            return Some("The campaign needs its code.");
        }
        if self.marketing_channel.is_none() {
            return Some("The campaign needs its marketing channel.");
        }
        let Some(purpose) = self.purpose else {
            return Some("The campaign needs its purpose.");
        };
        if self.owner_id.is_none() {
            return Some("The campaign needs its owner.");
        }
        let Some(start) = self.start_date else {
            return Some("The campaign needs its start date.");
        };
        if purpose == Purpose::Retention {
            if self.segment_id.is_none() {
                return Some("A retention campaign needs its segment.");
            }
            if self
                .template_key
                .as_deref()
                .is_none_or(|t| t.trim().is_empty())
            {
                return Some("A retention campaign needs its notification template.");
            }
            if self
                .message_channel
                .as_deref()
                .is_none_or(|c| c.trim().is_empty())
            {
                return Some("A retention campaign needs its message channel.");
            }
            let Some(send_at) = self.send_at else {
                return Some("A retention campaign needs its send time.");
            };
            let day = dubai_date(send_at);
            if day < start || self.end_date.is_some_and(|end| day > end) {
                return Some("The send time must fall between the start and the end date.");
            }
        }
        None
    }
}

impl StateMachine for Campaign {
    type State = CampaignStatus;
    const COLUMN: renox::db::Col<Campaign, CampaignStatus> = Campaign::STATUS;

    fn machine() -> Machine<Self> {
        use CampaignStatus::{Active, Draft, Ended};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Active)
            .tone("info")
            .state(Ended)
            .tone("neutral")
            .transition(activate(ACTIVATE, false))
            .transition(activate(ACTIVATE_ON_START, true))
            .transition(end(END, false))
            .transition(end(END_ON_END_DATE, true))
            .transition(reopen())
    }
}

/// `activate` (user) or `activate_on_start` (system): draft → active once the
/// fields are set; the promo code becomes valid to the end date.
fn activate(name: &'static str, system: bool) -> Transition<Campaign> {
    let t = Transition::new(name)
        .label("Activate")
        .tone("info")
        .from([CampaignStatus::Draft])
        .to(CampaignStatus::Active)
        .guard("fields_set", |c: &Campaign, _ctx| {
            let problem = c.activation_problem();
            Box::pin(async move { problem.map_or(Ok(()), |p| Err(p.to_owned())) })
        })
        .effect(|c: &Campaign, _ctx, tx| {
            let (id, end) = (c.id, c.end_date);
            Box::pin(async move { PromoCode::set_valid_to(tx, id, end).await })
        });
    if system {
        t.system().guard("start_reached", |c: &Campaign, ctx| {
            let reached = c.start_date.is_some_and(|s| s <= today_of(ctx));
            Box::pin(async move {
                if reached {
                    Ok(())
                } else {
                    Err("The start date is not reached.".to_owned())
                }
            })
        })
    } else {
        t.permission(permissions::CAMPAIGNS_MANAGE)
    }
}

/// `end` (user) or `end_on_end_date` (system): active → ended; the promo
/// code is valid to today.
fn end(name: &'static str, system: bool) -> Transition<Campaign> {
    let t = Transition::new(name)
        .label("End")
        .tone("neutral")
        .from([CampaignStatus::Active])
        .to(CampaignStatus::Ended)
        .effect(|c: &Campaign, ctx, tx| {
            let (id, today) = (c.id, today_of(ctx));
            Box::pin(async move { PromoCode::set_valid_to(tx, id, Some(today)).await })
        });
    if system {
        t.system().guard("end_date_passed", |c: &Campaign, ctx| {
            let passed = c.end_date.is_some_and(|e| e < today_of(ctx));
            Box::pin(async move {
                if passed {
                    Ok(())
                } else {
                    Err("The end date has not passed.".to_owned())
                }
            })
        })
    } else {
        t.permission(permissions::CAMPAIGNS_MANAGE)
    }
}

/// `reopen`: ended → active with a new end date after today; the promo code
/// is valid again to it unless its uses are used up.
fn reopen() -> Transition<Campaign> {
    Transition::new(REOPEN)
        .label("Reopen")
        .tone("info")
        .permission(permissions::CAMPAIGNS_MANAGE)
        .from([CampaignStatus::Ended])
        .to(CampaignStatus::Active)
        .guard("new_end_date", |c: &Campaign, ctx| {
            let verdict = match ctx.input_as::<ReopenInput>() {
                Err(_) => Err("Give the new end date.".to_owned()),
                Ok(input) if input.end_date <= today_of(ctx) => {
                    Err("The new end date must be after today.".to_owned())
                }
                Ok(input) if c.start_date.is_some_and(|s| input.end_date < s) => {
                    Err("The end date cannot be before the start date.".to_owned())
                }
                Ok(_) => Ok(()),
            };
            Box::pin(async move { verdict })
        })
        .effect(|c: &Campaign, ctx, tx| {
            let id = c.id;
            let end = ctx.input_as::<ReopenInput>().ok().map(|i| i.end_date);
            Box::pin(async move {
                let Some(end) = end else { return Ok(()) };
                Campaign::query()
                    .where_(Campaign::ID.eq(id))
                    .update(&mut *tx, &[("end_date", &Some(end))])
                    .await?;
                PromoCode::reopen(tx, id, end).await
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete() -> Campaign {
        Campaign {
            name: "Spring".into(),
            code: "SPRING26".into(),
            purpose: Some(Purpose::Acquisition),
            marketing_channel: Some(crate::campaign::MarketingChannel::Meta),
            owner_id: Some(1),
            start_date: NaiveDate::from_ymd_opt(2026, 3, 1),
            ..Campaign::default()
        }
    }

    #[test]
    fn a_complete_acquisition_campaign_can_be_activated() {
        assert_eq!(complete().activation_problem(), None);
    }

    #[test]
    fn each_missing_field_stops_the_activation() {
        let cases: [fn(&mut Campaign); 6] = [
            |c| c.name = " ".into(),
            |c| c.code = String::new(),
            |c| c.purpose = None,
            |c| c.marketing_channel = None,
            |c| c.owner_id = None,
            |c| c.start_date = None,
        ];
        for case in cases {
            let mut c = complete();
            case(&mut c);
            assert!(c.activation_problem().is_some());
        }
    }

    #[test]
    fn a_retention_campaign_needs_its_fields_and_a_send_time_in_its_period() {
        let mut c = complete();
        c.purpose = Some(Purpose::Retention);
        c.end_date = NaiveDate::from_ymd_opt(2026, 3, 31);
        assert!(c.activation_problem().is_some());
        c.segment_id = Some(1);
        c.template_key = Some("promo".into());
        c.message_channel = Some("email".into());
        assert!(c.activation_problem().is_some(), "no send time");
        let at = |m, d| {
            NaiveDate::from_ymd_opt(2026, m, d)
                .unwrap()
                .and_hms_opt(8, 0, 0)
                .unwrap()
                .and_utc()
        };
        c.send_at = Some(at(4, 2));
        assert!(c.activation_problem().is_some(), "after the end");
        c.send_at = Some(at(2, 20));
        assert!(c.activation_problem().is_some(), "before the start");
        c.send_at = Some(at(3, 10));
        assert_eq!(c.activation_problem(), None);
    }

    #[test]
    fn the_machine_has_user_and_system_twins() {
        let machine = Campaign::machine();
        for name in [ACTIVATE, END, REOPEN] {
            assert!(!machine.find(name).unwrap().is_system(), "{name}");
        }
        for name in [ACTIVATE_ON_START, END_ON_END_DATE] {
            assert!(machine.find(name).unwrap().is_system(), "{name}");
        }
        assert_eq!(machine.initial_states(), ["draft"]);
    }
}
