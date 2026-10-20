//! Making campaigns, leads and enquiries: the marketing manager's campaigns
//! with their costs (one over budget, waiting for a manager), the leads they
//! brought in, worked by marketing, and the enquiries staff recorded.

use super::real_today as today;
use property::enquiry::{Enquiry, NewEnquiry};
use renox::chrono::Duration;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::sales::{LeadId, lead_conversion};
use wl_sales::{Campaign, Lead, LeadSource, NewLead};

use super::people::login_of;
use super::roles::{LEASING_AGENT, MARKETING, MARKETING_MANAGER};
use super::sales::{CAMPAIGNS, CampaignPlan, ENQUIRIES, LEADS, LeadFate, LeadPlan};
use super::seed_leases::{text, upto};
use super::{Seed, day};

impl Seed {
    /// Campaigns, leads and enquiries.
    pub(crate) async fn sales(&mut self) -> Result {
        for plan in CAMPAIGNS {
            self.campaign(plan).await?;
        }
        for plan in LEADS {
            self.lead(plan).await?;
        }
        self.enquiries().await
    }

    async fn campaign(&mut self, plan: &CampaignPlan) -> Result {
        let manager = login_of(MARKETING_MANAGER).email;
        let source = LeadSource::by_code(self.db(), plan.source)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no lead source {}", plan.source))?;
        let owner = self.user_id(manager).await?.to_string();
        let source_id = source.id.to_string();
        let budget = plan.budget.to_string();
        let start = text(day(plan.start));
        let end = text(day(plan.end));
        // Planned a week before it starts (a campaign still to start, this
        // week).
        let planned = upto(day(plan.start - 7)).min(today() - Duration::days(3));
        at!(self, planned, {
            self.driver
                .post(
                    manager,
                    "/campaigns",
                    &[
                        ("name", plan.name),
                        ("code", plan.code),
                        ("purpose", "acquisition"),
                        ("marketing_channel", plan.channel),
                        ("lead_source_id", source_id.as_str()),
                        ("owner_id", owner.as_str()),
                        ("start_date", start.as_str()),
                        ("end_date", end.as_str()),
                        ("budget", budget.as_str()),
                    ],
                )
                .await
        })?;
        let campaign = Campaign::by_code(self.db(), plan.code)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no campaign {}", plan.code))?;
        self.report.add("campaigns", 1);
        let path = format!("/campaigns/{}", campaign.id);
        if plan.standing != "draft" {
            at!(self, upto(day(plan.start)), {
                self.driver
                    .post(manager, &format!("{path}/transitions/activate"), &[])
                    .await
            })?;
        }
        for (index, (description, amount)) in plan.costs.iter().enumerate() {
            let when = upto(day(plan.start + 6 + index as i64 * 14));
            at!(self, when, {
                self.driver
                    .post(
                        manager,
                        &format!("{path}/costs"),
                        &[
                            ("entry_date", text(when).as_str()),
                            ("amount", amount.to_string().as_str()),
                            ("description", description),
                        ],
                    )
                    .await
            })?;
            self.report.add("campaign costs", 1);
        }
        if plan.standing == "ended" {
            at!(self, upto(day(plan.end + 1)), {
                self.driver
                    .post(manager, &format!("{path}/transitions/end"), &[])
                    .await
            })?;
        }
        Ok(())
    }

    async fn lead(&mut self, plan: &LeadPlan) -> Result {
        let marketing = login_of(MARKETING).email;
        let owner = self.user_id(login_of(LEASING_AGENT).email).await?;
        let mut new = if plan.campaign.is_some() {
            NewLead::web_form(None, plan.name)
        } else {
            NewLead::staff(None, plan.name, plan.source)
        }
        .interest(plan.interest)
        .owner(owner);
        if let Some(code) = plan.campaign {
            new = new.campaign_code(code);
        }
        let came = day(-plan.age);
        let captured = at!(self, came, {
            wl_sales::lead::capture(&self.state, new, came).await
        })?
        .map_err(|refused| refused.into_error())?;
        let lead = match captured {
            wl_sales::Captured::Created(lead) => lead,
            other => {
                return Err(renox::anyhow::anyhow!("demo seed: lead {other:?}").into());
            }
        };
        self.report.add("leads", 1);
        let path = format!("/leads/{}/transitions", lead.id);
        let note = "Called, wants to view this week";
        let later = |days: i64| upto(came + Duration::days(days));
        match plan.fate {
            LeadFate::New => {}
            LeadFate::Contacted => {
                at!(self, later(1), {
                    self.driver
                        .post(marketing, &format!("{path}/contact"), &[("reason", note)])
                        .await
                })?;
            }
            LeadFate::Qualified => {
                at!(self, later(1), {
                    self.driver
                        .post(marketing, &format!("{path}/contact"), &[("reason", note)])
                        .await
                })?;
                at!(self, later(4), {
                    self.driver
                        .post(marketing, &format!("{path}/qualify"), &[])
                        .await
                })?;
            }
            LeadFate::Won(tenant) => {
                at!(self, later(1), {
                    self.driver
                        .post(marketing, &format!("{path}/contact"), &[("reason", note)])
                        .await
                })?;
                at!(self, later(3), {
                    self.driver
                        .post(marketing, &format!("{path}/qualify"), &[])
                        .await
                })?;
                let party = self.party(tenant).id;
                let lease = property::Lease::query()
                    .where_(property::Lease::TENANT_PARTY_ID.eq(party))
                    .order_by_desc(property::Lease::ID)
                    .first(self.db())
                    .await?
                    .map(|l| l.id);
                if let Some(lease) = lease {
                    at!(self, later(12), {
                        lead_conversion(&self.state)
                            .win(
                                &self.state,
                                LeadId(lead.id),
                                Subject::new("property.lease", lease),
                            )
                            .await
                    })?;
                }
            }
            LeadFate::Lost(reason) => {
                at!(self, later(6), {
                    self.driver
                        .post(marketing, &format!("{path}/lose"), &[("reason", reason)])
                        .await
                })?;
            }
            LeadFate::Reopened => {
                at!(self, later(8), {
                    self.driver
                        .post(
                            marketing,
                            &format!("{path}/lose"),
                            &[("reason", "no_response")],
                        )
                        .await
                })?;
                at!(self, later(30), {
                    self.driver
                        .post(
                            marketing,
                            &format!("{path}/reopen"),
                            &[("reason", "The prospect called back")],
                        )
                        .await
                })?;
            }
        }
        let _ = Lead::default;
        Ok(())
    }

    /// The enquiries about units, recorded by staff on the day they came in;
    /// three converted into leases.
    async fn enquiries(&mut self) -> Result {
        let agent = self.user_id(login_of(LEASING_AGENT).email).await?;
        for plan in ENQUIRIES {
            let unit_id = plan.unit.map(|(code, number)| self.unit(code, number).id);
            let came = day(-plan.age);
            let enquiry = at!(self, came, {
                property::enquiry::record(
                    self.db(),
                    NewEnquiry {
                        prospect_name: plan.name.to_owned(),
                        contact: plan.contact.to_owned(),
                        unit_id,
                        source: plan.source.to_owned(),
                        campaign: plan.campaign.map(str::to_owned),
                        notes: plan.note.to_owned(),
                        recorded_by: Some(agent),
                    },
                )
                .await
            })?;
            self.report.add("enquiries", 1);
            if let Some(tenant) = plan.converted {
                let party = wl_kernel::PartyId(self.party(tenant).id);
                at!(self, upto(came + Duration::days(9)), {
                    property::enquiry::convert(&self.state, enquiry.id, party).await
                })?;
            }
        }
        let _ = Enquiry::default;
        Ok(())
    }
}
