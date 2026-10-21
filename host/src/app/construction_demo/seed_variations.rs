//! The variations: recorded and priced by the QS, submitted, resubmitted,
//! claimed, abandoned (with the MD's approval) and withdrawn by the project
//! manager, decided by the client (the QS records the decision). Every state
//! is there, and two variations have waited for the client for more than 14
//! days, so the chase rule has something to remind about.

use construction::{BoqLine, Variation};
use renox::prelude::*;
use wl_kernel::approvals::ApprovalRequest;

use super::at;
use super::people::{email_of, login_of};
use super::roles::{ADMIN, PROJECT_MANAGER, QUANTITY_SURVEYOR};
use super::seed_ipas::after;
use super::seed_projects::plan_of;
use super::{Seed, day, text};

/// What the client did with a variation.
#[derive(Debug, Clone, Copy)]
pub enum End {
    /// Still being priced.
    Draft,
    /// Sent to the client, no answer yet.
    Waiting,
    /// Approved after this many days, for this amount (dirhams) or these days.
    Approved(i64, i64),
    /// Rejected after this many days, for this reason.
    Rejected(i64, &'static str),
    /// Rejected after this many days, then rejected revision resubmitted.
    Resubmitted(i64, &'static str),
    /// Rejected after this many days, then pursued as a claim after this many
    /// more.
    Claimed(i64, i64),
    /// Rejected after this many days, then abandoned after this many more
    /// (the MD approves the next day).
    Abandoned(i64, i64),
    /// Withdrawn by us after this many days.
    Withdrawn(i64),
}

/// A variation of the demo.
#[derive(Debug, Clone, Copy)]
pub struct Var {
    /// The project.
    pub code: &'static str,
    /// `addition`, `omission` or `time`.
    pub kind: &'static str,
    /// What it is.
    pub description: &'static str,
    /// The instruction it was executed on.
    pub instruction: &'static str,
    /// Whether the work was done before the approval.
    pub executed: bool,
    /// The price asked, in dirhams (an omission's size).
    pub priced: i64,
    /// The days of an extension of time asked.
    pub days: i64,
    /// The delay event of an extension of time.
    pub event: &'static str,
    /// The BOQ line an omission takes work out of, and the quantity.
    pub omit: Option<(&'static str, i64)>,
    /// The day it is recorded, days from today.
    pub recorded: i64,
    /// Days after it is recorded that it is submitted.
    pub submit: Option<i64>,
    /// How it ended.
    pub end: End,
}

const fn var(
    code: &'static str,
    kind: &'static str,
    description: &'static str,
    priced: i64,
    recorded: i64,
    submit: Option<i64>,
    end: End,
) -> Var {
    Var {
        code,
        kind,
        description,
        instruction: "",
        executed: false,
        priced,
        days: 0,
        event: "",
        omit: None,
        recorded,
        submit,
        end,
    }
}

/// The variations, in the order they are recorded.
pub const VARIATIONS: &[Var] = &[
    Var {
        instruction: "SI-TWR-009",
        executed: true,
        ..var(
            "TWR-01",
            "addition",
            "Additional fire-rated doors to the plant room",
            31_000,
            -110,
            Some(2),
            End::Abandoned(18, 5),
        )
    },
    var(
        "TWR-01",
        "addition",
        "Raised access floor to the trading room",
        240_000,
        -100,
        Some(2),
        End::Approved(24, 215_000),
    ),
    Var {
        omit: Some(("4.1", 1_500)),
        ..var(
            "TWR-01",
            "omission",
            "Omit painting of the shell areas on level 14",
            57_000,
            -95,
            Some(2),
            End::Approved(13, 57_000),
        )
    },
    Var {
        days: 14,
        event: "Late handover of the floor by the landlord's fit-out contractor",
        ..var(
            "TWR-01",
            "time",
            "Extension of time for the late handover of the floor",
            0,
            -92,
            Some(2),
            End::Approved(20, 10),
        )
    },
    Var {
        instruction: "SI-TWR-018",
        executed: true,
        ..var(
            "TWR-01",
            "addition",
            "Acoustic lining to the boardroom walls",
            92_000,
            -75,
            Some(3),
            End::Claimed(17, 5),
        )
    },
    Var {
        instruction: "SI-TWR-031",
        executed: true,
        ..var(
            "TWR-01",
            "addition",
            "Re-routing of the sprinkler mains above the new ceiling",
            64_000,
            -60,
            Some(2),
            End::Resubmitted(18, "Within the original scope per the client's consultant"),
        )
    },
    var(
        "TWR-01",
        "addition",
        "Relocation of the server room power outlets",
        85_000,
        -34,
        Some(2),
        End::Waiting,
    ),
    var(
        "TWR-01",
        "addition",
        "Additional glazed partition to the meeting rooms",
        118_000,
        -20,
        Some(2),
        End::Withdrawn(2),
    ),
    var(
        "TWR-01",
        "addition",
        "Feature wall cladding in the reception",
        76_000,
        -6,
        None,
        End::Draft,
    ),
    var(
        "MALL-02",
        "addition",
        "Additional display lighting tracks",
        150_000,
        -80,
        Some(2),
        End::Approved(20, 150_000),
    ),
    Var {
        days: 7,
        event: "Landlord access restrictions during the sales event",
        ..var(
            "MALL-02",
            "time",
            "Extension of time for the mall access restrictions",
            0,
            -32,
            Some(2),
            End::Waiting,
        )
    },
    var(
        "MALL-02",
        "addition",
        "Change of the fitting room finishes",
        70_000,
        -14,
        Some(2),
        End::Waiting,
    ),
    var(
        "CLN-03",
        "addition",
        "Lead lining to the X-ray room",
        185_000,
        -40,
        Some(2),
        End::Rejected(20, "Lead lining was included in the medical equipment contract"),
    ),
];

impl Seed {
    /// All the variations of the demo.
    pub(crate) async fn variations(&mut self) -> Result {
        for v in VARIATIONS {
            self.variation(v).await?;
        }
        Ok(())
    }

    /// The newest variation of project `code`.
    async fn latest_variation(&self, code: &str) -> Result<Variation> {
        let project = self.project(code).await?;
        Variation::query()
            .where_(Variation::PROJECT_ID.eq(project.id))
            .order_by_desc(Variation::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no variation of {code}").into())
    }

    /// One variation, from its record to its end.
    async fn variation(&mut self, v: &Var) -> Result {
        let qs = email_of(QUANTITY_SURVEYOR);
        let pm = email_of(PROJECT_MANAGER);
        let project = self.project(v.code).await?;
        let recorded = day(v.recorded);
        let priced = (v.priced * 100).to_string();
        let days = if v.days > 0 {
            v.days.to_string()
        } else {
            String::new()
        };
        let event_date = if v.days > 0 {
            text(day(v.recorded - 14))
        } else {
            String::new()
        };
        let (line, quantity) = match v.omit {
            Some((number, quantity)) => {
                let found = BoqLine::query()
                    .where_(BoqLine::PROJECT_ID.eq(project.id))
                    .where_(BoqLine::NUMBER.eq(number))
                    .first(self.db())
                    .await?
                    .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no BOQ line {number}"))?;
                (found.id.to_string(), quantity.to_string())
            }
            None => (String::new(), String::new()),
        };
        at!(self, recorded, {
            self.driver
                .post(
                    qs,
                    &format!("/construction/projects/{}/variations", project.id),
                    &[
                        ("kind", v.kind),
                        ("description", v.description),
                        ("site_instruction", v.instruction),
                        ("executed", if v.executed { "on" } else { "" }),
                        ("priced_amount", priced.as_str()),
                        ("days_asked", days.as_str()),
                        ("event", v.event),
                        ("event_date", event_date.as_str()),
                        ("boq_line_id", line.as_str()),
                        ("omitted_quantity", quantity.as_str()),
                    ],
                )
                .await
        })?;
        let made = self.latest_variation(v.code).await?;
        self.report.add("variations", 1);
        let Some(submit) = v.submit else {
            return Ok(());
        };
        let submitted = after(recorded, submit);
        if let End::Withdrawn(days) = v.end {
            // A variation can be withdrawn while it is still a draft or after
            // it was sent; this one was sent first.
            at!(self, submitted, {
                self.driver
                    .post(pm, &format!("/construction/variations/{}/submit", made.id), &[])
                    .await
            })?;
            at!(self, after(submitted, days), {
                self.driver
                    .post(pm, &format!("/construction/variations/{}/withdraw", made.id), &[])
                    .await
            })?;
            return Ok(());
        }
        at!(self, submitted, {
            self.driver
                .post(pm, &format!("/construction/variations/{}/submit", made.id), &[])
                .await
        })?;
        let reference = format!("{}-{}-{}", v.code, "CVR", made.id);
        match v.end {
            End::Draft | End::Waiting | End::Withdrawn(_) => {}
            End::Approved(days, value) => {
                let on = after(submitted, days);
                let (amount, granted) = if v.kind == "time" {
                    (String::new(), value.to_string())
                } else {
                    ((value * 100).to_string(), String::new())
                };
                at!(self, on, {
                    self.driver
                        .post(
                            qs,
                            &format!("/construction/variations/{}/approve", made.id),
                            &[
                                ("client_reference", reference.as_str()),
                                ("approved_amount", amount.as_str()),
                                ("days_granted", granted.as_str()),
                            ],
                        )
                        .await
                })?;
            }
            End::Rejected(days, reason)
            | End::Resubmitted(days, reason) => {
                let on = after(submitted, days);
                at!(self, on, {
                    self.driver
                        .post(
                            qs,
                            &format!("/construction/variations/{}/reject", made.id),
                            &[("reason", reason)],
                        )
                        .await
                })?;
                if matches!(v.end, End::Resubmitted(..)) {
                    at!(self, after(on, 3), {
                        self.driver
                            .post(
                                pm,
                                &format!("/construction/variations/{}/resubmit", made.id),
                                &[],
                            )
                            .await
                    })?;
                    self.report.add("variations", 1);
                }
            }
            End::Claimed(days, more) | End::Abandoned(days, more) => {
                let on = after(submitted, days);
                at!(self, on, {
                    self.driver
                        .post(
                            qs,
                            &format!("/construction/variations/{}/reject", made.id),
                            &[("reason", "The client says the work was not instructed")],
                        )
                        .await
                })?;
                let then = after(on, more);
                if matches!(v.end, End::Claimed(..)) {
                    at!(self, then, {
                        self.driver
                            .post(pm, &format!("/construction/variations/{}/claim", made.id), &[])
                            .await
                    })?;
                } else {
                    at!(self, then, {
                        self.driver
                            .post(
                                pm,
                                &format!("/construction/variations/{}/abandon", made.id),
                                &[],
                            )
                            .await
                    })?;
                    let request = ApprovalRequest::pending_about(self.db(), &made.subject())
                        .await?
                        .into_iter()
                        .next()
                        .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no abandon request"))?;
                    at!(self, after(then, 1), {
                        self.driver
                            .post(
                                login_of(ADMIN).email,
                                &format!("/approvals/{}/approve", request.id),
                                &[],
                            )
                            .await
                    })?;
                }
            }
        }
        let _ = plan_of(v.code);
        Ok(())
    }
}
