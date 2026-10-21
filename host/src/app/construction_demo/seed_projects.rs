//! The projects: awarded through `POST /construction/projects/award` by the
//! project manager and started by the project manager. What the award form
//! does not carry (the budget, the retention terms, the advance, the site's
//! emirate and the project manager) is set on the project afterwards,
//! because the setup screen that edits them comes with the BOQ issue.

use super::at;
use construction::Project;
use renox::prelude::*;
use wl_inventory::Location;

use super::people::email_of;
use super::roles::PROJECT_MANAGER;
use super::{Seed, day, text};

/// One project of the demo.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    /// The code.
    pub code: &'static str,
    /// The name.
    pub name: &'static str,
    /// The client, by English name.
    pub client: &'static str,
    /// The contract value, in dirhams.
    pub value: i64,
    /// The budget, in dirhams.
    pub budget: i64,
    /// The advance paid, in dirhams.
    pub advance: i64,
    /// The day it is awarded, days from today.
    pub awarded: i64,
    /// The day work starts, if it does.
    pub started: Option<i64>,
    /// The contract completion date, days from today.
    pub due: i64,
    /// Whether the site works on Saturdays.
    pub saturdays: bool,
    /// The retention withheld, in basis points.
    pub retention_bps: i64,
}

/// The projects, in the order they are awarded.
pub const PLANS: &[Plan] = &[
    Plan {
        code: "OFC-06",
        name: "Business Bay Tower 3, Levels 8 to 9 office fit-out",
        client: super::people::GULF_HORIZON,
        value: 3_300_000,
        budget: 2_650_000,
        advance: 330_000,
        awarded: -340,
        started: Some(-330),
        due: -135,
        saturdays: false,
        retention_bps: 1_000,
    },
    Plan {
        code: "TRM-07",
        name: "Dubai South showroom fit-out",
        client: super::people::FESTIVAL_PLAZA,
        value: 2_200_000,
        budget: 1_800_000,
        advance: 220_000,
        awarded: -150,
        started: Some(-138),
        due: 120,
        saturdays: false,
        retention_bps: 1_000,
    },
    Plan {
        code: super::seed_hr::TWR,
        name: "Marina Tower level 14 office fit-out",
        client: super::people::GULF_HORIZON,
        value: 6_400_000,
        budget: 5_300_000,
        advance: 640_000,
        awarded: -148,
        started: Some(-140),
        due: 150,
        saturdays: false,
        retention_bps: 1_000,
    },
    Plan {
        code: super::seed_hr::MALL,
        name: "Festival Plaza unit L2-14 fashion store fit-out",
        client: super::people::FESTIVAL_PLAZA,
        value: 2_850_000,
        budget: 2_350_000,
        advance: 285_000,
        awarded: -135,
        started: Some(-125),
        due: 90,
        saturdays: false,
        retention_bps: 1_000,
    },
    Plan {
        code: super::seed_hr::CLN,
        name: "Al Noor medical clinic fit-out, Business Bay",
        client: super::people::AL_NOOR,
        value: 4_100_000,
        budget: 3_500_000,
        advance: 410_000,
        awarded: -110,
        started: Some(-100),
        due: 140,
        saturdays: false,
        retention_bps: 1_000,
    },
    Plan {
        code: super::seed_hr::RST,
        name: "Saffron Garden restaurant fit-out, Dubai Marina",
        client: super::people::FESTIVAL_PLAZA,
        value: 1_900_000,
        budget: 1_560_000,
        advance: 190_000,
        awarded: -75,
        started: Some(-68),
        due: 100,
        saturdays: false,
        retention_bps: 1_000,
    },
    Plan {
        code: "VIL-05",
        name: "Palm villa show apartment interior fit-out",
        client: super::people::GULF_HORIZON,
        value: 920_000,
        budget: 740_000,
        advance: 0,
        awarded: -4,
        started: None,
        due: 200,
        saturdays: false,
        retention_bps: 1_000,
    },
];

/// The plan of project `code`.
pub fn plan_of(code: &str) -> &'static Plan {
    PLANS
        .iter()
        .find(|plan| plan.code == code)
        .unwrap_or_else(|| panic!("the demo project {code}"))
}

impl Seed {
    /// The project with this code.
    pub(crate) async fn project(&self, code: &str) -> Result<Project> {
        Project::query()
            .where_(Project::CODE.eq(code))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no project {code}").into())
    }

    /// Awards project `code` on its day (the project manager) and sets what
    /// the form does not carry.
    pub(crate) async fn award(&mut self, code: &str) -> Result {
        let plan = plan_of(code);
        let pm = email_of(PROJECT_MANAGER);
        let client = self.party(plan.client).id.to_string();
        let value = (plan.value * 100).to_string();
        let document = format!("LPO-{}.pdf", plan.code);
        at!(self, day(plan.awarded), {
            self.driver
                .post(
                    pm,
                    "/construction/projects/award",
                    &[
                        ("client_party_id", client.as_str()),
                        ("code", plan.code),
                        ("name", plan.name),
                        ("contract_value", value.as_str()),
                        ("award_document", document.as_str()),
                    ],
                )
                .await
        })?;
        let manager = Some(self.user_id(pm).await?);
        let project = self.project(code).await?;
        let budget = plan.budget * 100;
        let advance = plan.advance * 100;
        // The cap of the retention is 5% of the contract value.
        let cap = Some(plan.value * 100 / 20);
        let due = Some(day(plan.due));
        Project::query()
            .where_(Project::ID.eq(project.id))
            .update(
                self.db(),
                &[
                    ("budget", &budget),
                    ("advance_paid", &advance),
                    ("retention_bps", &plan.retention_bps),
                    ("retention_cap", &cap),
                    ("completion_due_on", &due),
                    ("works_saturdays", &plan.saturdays),
                    ("manager_user_id", &manager),
                ],
            )
            .await?;
        self.report.add("projects", 1);
        // The store of the project: its goods are received and counted there.
        let mut store = Location::new(&format!("STORE-{code}"), &format!("{code} site store"))
            .in_scope(&project.subject());
        store.save(self.db()).await?;
        self.report.add("stores", 1);
        Ok(())
    }

    /// Starts project `code` on its day (the project manager).
    pub(crate) async fn start(&mut self, code: &str) -> Result {
        let plan = plan_of(code);
        let Some(started) = plan.started else {
            return Ok(());
        };
        let pm = email_of(PROJECT_MANAGER);
        let project = self.project(code).await?;
        at!(self, day(started), {
            self.driver
                .post(pm, &format!("/demo/projects/{}/start", project.id), &[])
                .await
        })?;
        let _ = text(day(started));
        Ok(())
    }
}
