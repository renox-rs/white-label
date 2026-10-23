//! The admissions side: four campaigns (one ended, two running with their
//! costs, one still a draft) and the leads the counsellor works: new ones,
//! some with no follow-up for a week, qualified, lost, reopened; the leads of
//! students who enrolled are won by their enrolment (`SAL.CRM.01`), so they
//! are made before the enrolments.

use renox::chrono::Duration;
use renox::prelude::*;
use wl_sales::{Campaign, LeadSource, NewLead};

use super::super::demo_data::{at, day};
use super::people::{AMAL, ARJUN, PRIYA, YOUSEF};
use super::{ADMIN_LOGIN, COUNSELLOR_LOGIN, Seed};

/// A campaign: code, name, channel, source, first and last day, budget in
/// dirhams, costs, whether it was started, whether it ended.
type CampaignPlan = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i64,
    i64,
    i64,
    &'static [(&'static str, i64)],
    bool,
    bool,
);

const CAMPAIGNS: &[CampaignPlan] = &[
    (
        "SUMKIDS26",
        "Summer kids camp 2026",
        "meta",
        "social",
        -120,
        -70,
        9_000,
        &[("Social boosts", 3_400), ("Flyers for schools", 1_250)],
        true,
        true,
    ),
    (
        "BTS26",
        "Back to school 2026",
        "meta",
        "social",
        -40,
        10,
        6_000,
        &[("Instagram reels", 2_100), ("Sponsored posts", 1_900)],
        true,
        false,
    ),
    (
        "IELTS-OCT",
        "IELTS October intake",
        "google",
        "paid_ads",
        -25,
        20,
        4_000,
        &[("Search ads, first fortnight", 1_800)],
        true,
        false,
    ),
    (
        "RAM27",
        "Ramadan courses 2027",
        "email",
        "website",
        120,
        150,
        3_000,
        &[],
        false,
        false,
    ),
];

/// What became of a lead.
#[derive(Clone, Copy)]
enum Fate {
    New,
    Contacted,
    Qualified,
    Lost(&'static str),
    Reopened,
}

/// A lead: name, source, campaign, interest, age in days, fate.
type LeadPlan = (
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static str,
    i64,
    Fate,
);

const LEADS: &[LeadPlan] = &[
    (
        "Tamer Said",
        "walk_in",
        None,
        "IELTS preparation",
        1,
        Fate::New,
    ),
    (
        "Joanna Dsouza",
        "website",
        Some("BTS26"),
        "Young Coders for her son",
        2,
        Fate::New,
    ),
    (
        "Omnia Fahmy",
        "social",
        Some("BTS26"),
        "Arabic for kids",
        3,
        Fate::New,
    ),
    (
        "Bilal Chaudhry",
        "referral",
        None,
        "Python for beginners",
        9,
        Fate::Contacted,
    ),
    (
        "Svetlana Orlova",
        "website",
        Some("IELTS-OCT"),
        "IELTS, evening class",
        10,
        Fate::Contacted,
    ),
    (
        "Mahmoud Darwish Jr",
        "walk_in",
        None,
        "Excel for his team of six",
        11,
        Fate::Contacted,
    ),
    (
        "Cheryl Tan",
        "paid_ads",
        Some("IELTS-OCT"),
        "IELTS, weekend class",
        6,
        Fate::Qualified,
    ),
    (
        "Faris Al Nuaimi",
        "event",
        None,
        "GMAT, next intake",
        7,
        Fate::Qualified,
    ),
    (
        "Ines Moreau",
        "partner",
        None,
        "French A1 for adults",
        20,
        Fate::Lost("price"),
    ),
    (
        "Dev Malhotra",
        "social",
        Some("BTS26"),
        "Python for his daughter",
        18,
        Fate::Lost("no_response"),
    ),
    (
        "Hala Yassin",
        "walk_in",
        None,
        "General English",
        32,
        Fate::Reopened,
    ),
];

/// The leads of the students who enrol later: name index, source, age.
const STUDENT_LEADS: &[(usize, &str, i64)] = &[
    (AMAL, "walk_in", 75),
    (ARJUN, "website", 60),
    (PRIYA, "referral", 66),
    (YOUSEF, "social", 50),
];

impl Seed {
    async fn campaign(&self, plan: &CampaignPlan) -> Result {
        let (code, name, channel, source, from, to, budget, costs, started, ended) = *plan;
        let source = LeadSource::by_code(self.db(), source)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no lead source {source}"))?;
        let owner = self.user(ADMIN_LOGIN).id.to_string();
        let source = source.id.to_string();
        let budget = budget.to_string();
        let start = day(from).to_string();
        let end = day(to).to_string();
        let planned = (from - 7).min(-3);
        self.act(
            planned,
            ADMIN_LOGIN,
            "/campaigns",
            &[
                ("name", name),
                ("code", code),
                ("purpose", "acquisition"),
                ("marketing_channel", channel),
                ("lead_source_id", source.as_str()),
                ("owner_id", owner.as_str()),
                ("start_date", start.as_str()),
                ("end_date", end.as_str()),
                ("budget", budget.as_str()),
            ],
        )
        .await?;
        let campaign = Campaign::by_code(self.db(), code)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no campaign {code}"))?;
        let path = format!("/campaigns/{}", campaign.id);
        if started {
            self.act(
                from.min(-1),
                ADMIN_LOGIN,
                &format!("{path}/transitions/activate"),
                &[],
            )
            .await?;
        }
        for (index, (description, amount)) in costs.iter().enumerate() {
            let when = (from + 6 + index as i64 * 14).min(-1);
            let entry = day(when).to_string();
            let amount = amount.to_string();
            self.act(
                when,
                ADMIN_LOGIN,
                &format!("{path}/costs"),
                &[
                    ("entry_date", entry.as_str()),
                    ("amount", amount.as_str()),
                    ("description", description),
                ],
            )
            .await?;
        }
        if ended {
            self.act(to + 1, ADMIN_LOGIN, &format!("{path}/transitions/end"), &[])
                .await?;
        }
        Ok(())
    }

    async fn lead(&self, plan: &LeadPlan) -> Result {
        let (name, source, campaign, interest, age, fate) = *plan;
        let owner = self.user(COUNSELLOR_LOGIN).id;
        let mut new = match campaign {
            Some(_) => NewLead::web_form(None, name),
            None => NewLead::staff(None, name, source),
        }
        .interest(interest)
        .owner(owner);
        if let Some(code) = campaign {
            new = new.campaign_code(code);
        }
        self.capture(new, age).await?;
        let lead = wl_sales::Lead::query()
            .order_by_desc(wl_sales::Lead::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the lead was not saved"))?;
        let path = format!("/leads/{}/transitions", lead.id);
        let note = "Called, wants to know the next intake";
        let later = |days: i64| -(age - days).max(0);
        match fate {
            Fate::New => {}
            Fate::Contacted => {
                self.act(
                    later(1),
                    COUNSELLOR_LOGIN,
                    &format!("{path}/contact"),
                    &[("reason", note)],
                )
                .await?;
            }
            Fate::Qualified => {
                self.act(
                    later(1),
                    COUNSELLOR_LOGIN,
                    &format!("{path}/contact"),
                    &[("reason", note)],
                )
                .await?;
                self.act(later(3), COUNSELLOR_LOGIN, &format!("{path}/qualify"), &[])
                    .await?;
            }
            Fate::Lost(reason) => {
                self.act(
                    later(6),
                    COUNSELLOR_LOGIN,
                    &format!("{path}/lose"),
                    &[("reason", reason)],
                )
                .await?;
            }
            Fate::Reopened => {
                self.act(
                    later(8),
                    COUNSELLOR_LOGIN,
                    &format!("{path}/lose"),
                    &[("reason", "no_response")],
                )
                .await?;
                self.act(
                    later(20),
                    COUNSELLOR_LOGIN,
                    &format!("{path}/reopen"),
                    &[("reason", "She called back after the summer")],
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Captures `new` `age` days ago.
    async fn capture(&self, new: NewLead, age: i64) -> Result {
        let came = day(-age);
        let captured = at!(self, came, {
            wl_sales::lead::capture(&self.state, new, came).await
        })?
        .map_err(|refused| refused.into_error())?;
        match captured {
            wl_sales::Captured::Created(_) => Ok(()),
            other => Err(renox::anyhow::anyhow!("demo seed: lead {other:?}").into()),
        }
    }

    pub(crate) async fn sales(&mut self) -> Result {
        for plan in CAMPAIGNS {
            self.campaign(plan).await?;
        }
        for plan in LEADS {
            self.lead(plan).await?;
        }
        // The leads of students who enrol later: the enrolment wins them.
        for (student, source, age) in STUDENT_LEADS {
            let party = self.students[*student];
            let name = super::people::STUDENTS[*student].name;
            let new = NewLead::staff(Some(party), name, source)
                .interest("Course enquiry")
                .owner(self.user(COUNSELLOR_LOGIN).id);
            self.capture(new, *age).await?;
            let lead = wl_sales::Lead::query()
                .order_by_desc(wl_sales::Lead::ID)
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the lead was not saved"))?;
            let path = format!("/leads/{}/transitions", lead.id);
            self.act(
                -(age - 1),
                COUNSELLOR_LOGIN,
                &format!("{path}/contact"),
                &[("reason", "Visited the centre, took the brochure")],
            )
            .await?;
            self.act(
                -(age - 3),
                COUNSELLOR_LOGIN,
                &format!("{path}/qualify"),
                &[],
            )
            .await?;
        }
        let _ = Duration::days(0);
        Ok(())
    }
}
