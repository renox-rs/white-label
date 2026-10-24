//! The workshop's job cards, each from a booking checked in at the bay
//! (wl-scheduling's service, as the service advisor) and then through the
//! app's own routes as the people who do it: the advisor records the intake
//! and assigns the technician, the technician inspects and prices the work
//! from the labour menu and the store, the advisor sends the estimate, the
//! customer approves on the link (or by phone), the technician works and
//! takes the parts from the store, the workshop manager passes the QC and the
//! advisor hands the car over, which raises the invoice. Each job card stops
//! at the stage the screens need.

use auto::JobCard;
use renox::chrono::Duration;
use renox::prelude::*;
use wl_kernel::{PartyId, Subject};
use wl_scheduling::ServiceType;
use wl_scheduling::service::{self, Actor, NewBooking};

use super::{ADVISOR_EMAIL, Seed, TECH_EMAIL, TECH2_EMAIL, WORKSHOP_EMAIL, day, workshop};

/// How far a job card goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Stage {
    /// Just checked in at the bay.
    CheckedIn,
    /// The technician is inspecting it.
    Inspecting,
    /// The estimate is with the customer.
    AwaitingApproval,
    /// Approved: the work runs.
    InProgress,
    /// The work is done: quality check.
    QualityCheck,
    /// Passed the QC: ready for collection.
    Ready,
    /// Handed over and invoiced.
    Delivered,
}

/// One job card: key, vehicle, service type, complaint, odometer in,
/// labour (menu code, hours), parts (code, name, quantity, unit price),
/// technician, stage.
pub(crate) struct Job {
    pub key: &'static str,
    pub vehicle: &'static str,
    pub service: &'static str,
    pub complaint: &'static str,
    pub odometer: &'static str,
    pub labour: &'static [(&'static str, &'static str)],
    pub parts: &'static [(&'static str, &'static str, &'static str, &'static str)],
    pub technician: &'static str,
    pub stage: Stage,
}

const JOBS: &[Job] = &[
    Job {
        key: "lc",
        vehicle: "lc",
        service: "periodic_service",
        complaint: "60,000 km service; brake noise at low speed",
        odometer: "61,870",
        labour: &[("SVC-MAJOR", "3"), ("BRK-FRONT", "1.5")],
        parts: &[
            ("OIL-5W30-4L", "Engine oil 5W-30, 4 L", "2", "95"),
            ("FLT-OIL-TY", "Oil filter, Toyota", "1", "32"),
            ("FLT-AIR-TY", "Air filter, Toyota", "1", "58"),
            ("BRK-PAD-TY", "Front brake pad set, Toyota", "1", "185"),
        ],
        technician: TECH_EMAIL,
        stage: Stage::Delivered,
    },
    Job {
        key: "patrol",
        vehicle: "patrol",
        service: "periodic_service",
        complaint: "Rear suspension knocking on rough roads",
        odometer: "118,460",
        labour: &[("DIAG", "1"), ("SVC-MINOR", "1")],
        parts: &[
            ("SHK-RR-PT", "Rear shock absorber, Patrol", "2", "640"),
            ("OIL-5W30-4L", "Engine oil 5W-30, 4 L", "2", "95"),
        ],
        technician: TECH2_EMAIL,
        stage: Stage::Ready,
    },
    Job {
        key: "hilux",
        vehicle: "hilux",
        service: "periodic_service",
        complaint: "Company pickup: minor service and wipers",
        odometer: "96,240",
        labour: &[("SVC-MINOR", "1")],
        parts: &[
            ("OIL-5W30-4L", "Engine oil 5W-30, 4 L", "2", "95"),
            ("FLT-OIL-TY", "Oil filter, Toyota", "1", "32"),
            ("WPR-BLD-24", "Wiper blade 24 in", "2", "35"),
        ],
        technician: TECH_EMAIL,
        stage: Stage::QualityCheck,
    },
    Job {
        key: "accord",
        vehicle: "accord",
        service: "diagnosis",
        complaint: "Battery warning light, slow cranking in the morning",
        odometer: "74,355",
        labour: &[("DIAG", "1.5")],
        parts: &[("BAT-70AH", "Battery 70 Ah", "1", "445")],
        technician: TECH2_EMAIL,
        stage: Stage::InProgress,
    },
    Job {
        key: "camry",
        vehicle: "camry",
        service: "periodic_service",
        complaint: "First major service; spark plugs due",
        odometer: "28,940",
        labour: &[("SVC-MAJOR", "3")],
        parts: &[
            ("SPK-PLG-IR", "Iridium spark plug", "4", "58"),
            ("OIL-5W30-4L", "Engine oil 5W-30, 4 L", "2", "95"),
        ],
        technician: TECH_EMAIL,
        stage: Stage::AwaitingApproval,
    },
    Job {
        key: "corolla",
        vehicle: "corolla",
        service: "diagnosis",
        complaint: "Vibration in the steering above 100 km/h",
        odometer: "131,300",
        labour: &[],
        parts: &[],
        technician: TECH2_EMAIL,
        stage: Stage::Inspecting,
    },
    Job {
        key: "altima",
        vehicle: "altima",
        service: "periodic_service",
        complaint: "Air conditioning not cooling enough",
        odometer: "57,430",
        labour: &[],
        parts: &[],
        technician: "",
        stage: Stage::CheckedIn,
    },
];

impl Seed {
    /// A booking of `service` for vehicle `vehicle` at the bay, `hours_ago`
    /// hours ago, checked in ten minutes after; the job card it opened.
    pub(crate) async fn check_in(
        &mut self,
        key: &'static str,
        vehicle: &str,
        service_code: &str,
        hours_ago: i64,
    ) -> Result<i64> {
        let advisor = self.user(ADVISOR_EMAIL).await?.id;
        let service_type = ServiceType::query()
            .where_(ServiceType::CODE.eq(service_code))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("the service type {service_code}"))?;
        let vehicle_id = self.vehicles[vehicle];
        let owner = auto::Vehicle::find_or_404(self.db(), vehicle_id)
            .await?
            .owner_party_id;
        let start = renox::db::now() - Duration::hours(hours_ago);
        let booking = service::book(
            &self.state,
            NewBooking::new(PartyId(owner), service_type.id, &[self.bay], start)
                .for_subject(Subject::new(auto::seed::VEHICLE_SUBJECT_TYPE, vehicle_id))
                .at(workshop()),
            Actor::staff(advisor),
            start - Duration::days(2),
        )
        .await
        .map_err(|e| renox::anyhow::anyhow!("demo seed: the booking of {key}: {e:?}"))?;
        service::check_in(
            &self.state,
            booking.id,
            Actor::staff(advisor),
            start + Duration::minutes(10),
        )
        .await
        .map_err(|e| renox::anyhow::anyhow!("demo seed: the check-in of {key}: {e:?}"))?;
        let card = JobCard::of_booking(self.db(), booking.id)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the job card of {key}"))?;
        self.jobs.insert(key, card.id);
        Ok(card.id)
    }

    pub(crate) async fn on_job(
        &self,
        who: &str,
        id: i64,
        action: &str,
        form: &[(&str, &str)],
    ) -> Result {
        self.driver
            .post(who, &format!("/auto/job-cards/{id}/{action}"), form)
            .await?;
        Ok(())
    }

    /// The ids of the rows of `table` of job card `id`, oldest first.
    pub(crate) async fn lines(&self, table: &str, id: i64) -> Result<Vec<i64>> {
        let rows = renox::db::sql(format!(
            "select id from {table} where job_card_id = {id} order by id"
        ))
        .fetch_all(self.db())
        .await?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.try_get::<i64>("id")?);
        }
        Ok(ids)
    }

    /// The advisor records the intake of job card `id`.
    pub(crate) async fn intake(
        &self,
        id: i64,
        complaint: &str,
        odometer: &str,
        by: &str,
    ) -> Result {
        self.on_job(
            ADVISOR_EMAIL,
            id,
            "intake",
            &[
                ("complaint", complaint),
                ("odometer", odometer),
                ("fuel", "half"),
                (
                    "condition",
                    "Minor scratches on the rear bumper; no warning lights",
                ),
                ("acknowledged_by", by),
            ],
        )
        .await
    }

    /// Job card `job.key` up to `job.stage`.
    async fn job(&mut self, job: &Job, hours_ago: i64) -> Result {
        let id = self
            .check_in(job.key, job.vehicle, job.service, hours_ago)
            .await?;
        let owner = auto::Vehicle::find_or_404(self.db(), self.vehicles[job.vehicle])
            .await?
            .owner_party_id;
        let owner_name = wl_parties::Party::find(self.db(), owner)
            .await?
            .map(|p| p.legal_name_en)
            .unwrap_or_default();
        self.intake(id, job.complaint, job.odometer, &owner_name)
            .await?;
        if job.stage == Stage::CheckedIn {
            return Ok(());
        }
        let technician = self.user(job.technician).await?;
        self.on_job(
            ADVISOR_EMAIL,
            id,
            "technician",
            &[("technician", &technician.id.to_string())],
        )
        .await?;
        self.on_job(job.technician, id, "inspect", &[]).await?;
        if job.stage == Stage::Inspecting {
            self.on_job(
                job.technician,
                id,
                "findings",
                &[(
                    "findings",
                    "Front wheels out of balance; checking the lower arm bushes",
                )],
            )
            .await?;
            return Ok(());
        }
        for (operation, hours) in job.labour {
            self.on_job(
                job.technician,
                id,
                "labour",
                &[("operation", operation), ("hours", hours)],
            )
            .await?;
        }
        for (code, name, quantity, price) in job.parts {
            self.on_job(
                job.technician,
                id,
                "parts",
                &[
                    ("item_code", code),
                    ("description", name),
                    ("quantity", quantity),
                    ("unit_price", price),
                    ("condition", "new"),
                ],
            )
            .await?;
        }
        self.on_job(
            ADVISOR_EMAIL,
            id,
            "send",
            &[
                ("promised_for", &day(1).to_string()),
                ("warranty_days", "90"),
            ],
        )
        .await?;
        if job.stage == Stage::AwaitingApproval {
            return Ok(());
        }
        // The customer approves on the link of the estimate.
        let round = auto::job_cards::pending_round(self.db(), id)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the estimate of {}", job.key))?;
        self.driver
            .post(
                ADVISOR_EMAIL,
                &format!("/auto/approve/{}", round.token),
                &[
                    ("name", &owner_name),
                    ("signature", &owner_name),
                    ("consent", "on"),
                ],
            )
            .await?;
        let labour = self.lines("auto_job_labour_lines", id).await?;
        let parts = self.lines("auto_job_part_lines", id).await?;
        for line in &parts {
            self.on_job(job.technician, id, &format!("parts/{line}/issue"), &[])
                .await?;
        }
        if job.stage == Stage::InProgress {
            if let Some(line) = labour.first() {
                self.on_job(
                    job.technician,
                    id,
                    &format!("labour/{line}/work"),
                    &[("work", "part_done"), ("hours", "0.5")],
                )
                .await?;
            }
            return Ok(());
        }
        for line in &parts {
            self.on_job(job.technician, id, &format!("parts/{line}/fit"), &[])
                .await?;
        }
        for line in &labour {
            self.on_job(
                job.technician,
                id,
                &format!("labour/{line}/work"),
                &[("work", "done")],
            )
            .await?;
        }
        self.on_job(job.technician, id, "finish", &[]).await?;
        if job.stage == Stage::QualityCheck {
            return Ok(());
        }
        let checks: Vec<(&str, &str)> = auto::job_card::QC_KEYS
            .iter()
            .map(|key| ("done", *key))
            .collect();
        self.on_job(WORKSHOP_EMAIL, id, "qc", &checks).await?;
        self.on_job(WORKSHOP_EMAIL, id, "pass-qc", &[]).await?;
        if job.stage == Stage::Ready {
            return Ok(());
        }
        self.on_job(
            ADVISOR_EMAIL,
            id,
            "deliver",
            &[
                ("delivered_to", &owner_name),
                ("odometer", job.odometer),
                ("next_service_date", &day(180).to_string()),
                ("next_service_odometer", "71,870"),
            ],
        )
        .await?;
        Ok(())
    }

    /// The job cards of the workshop's day.
    pub(crate) async fn workshop(&mut self) -> Result {
        let mut hours_ago = 30;
        for job in JOBS {
            self.job(job, hours_ago).await?;
            hours_ago -= 4;
        }
        Ok(())
    }
}
