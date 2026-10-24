//! Accident job cards and the police report (a legal block on the approval),
//! and a vehicle incident in the workshop decided by the branch manager.

use auto::incidents::{self, NewIncident};
use auto::job_cards::{self, NewJobCard};
use auto::test_drives::{self, NewDrive};
use renox::prelude::*;
use renox::upload::Upload;
use wl_documents::{Documents, NewDocument};

use super::super::demo_data::{at, day};
use super::shop::workshop;
use super::{BASMA, NABIL, SAMI, Seed, TAREQ, WALEED};

impl Seed {
    /// An accident job card with the report number typed at check-in, an
    /// estimate sent and the police report uploaded by the advisor.
    async fn accident_card(&self, car: &str, ack: &str, report: &str) -> Result<auto::JobCard> {
        let vehicle = auto::Vehicle::find_or_404(self.db(), self.vehicle(car).id).await?;
        let sami = self.who(SAMI);
        let card = job_cards::check_in(
            &self.state,
            sami,
            NewJobCard::new(vehicle.id, "Hit from behind at a junction")
                .odometer(vehicle.odometer_km + 10)
                .fuel("quarter")
                .condition("Rear bumper and boot lid damaged")
                .acknowledged_by(ack)
                .technician(self.user(NABIL).id)
                .workshop(workshop())
                .accident(report),
        )
        .await?;
        self.draft(
            card.id,
            NABIL,
            &[("Boot lid and bumper repair", 4_000, 15_000)],
            &[],
        )
        .await?;
        let card = self.send(card.id).await?;
        Documents::attach(
            &self.state,
            Some(sami),
            NewDocument::new(
                card.subject(),
                "police_report",
                Upload::new(
                    "police-report.pdf",
                    "application/pdf",
                    format!("%PDF-1.7 police report {report}").into_bytes(),
                ),
            )
            .number(report),
        )
        .await?
        .expect("the report is attached");
        Ok(card)
    }

    pub(crate) async fn stories(&mut self) -> Result {
        // The report waits for review: the approval is refused until the
        // workshop manager accepts it.
        let waiting = at!(self, day(-3), {
            self.accident_card("accident1", "Mona Hassan", "DXB-PR-7001")
                .await
        })?;
        self.cards.insert("police_waiting", waiting.id);
        // The report is accepted by a second user and the customer approves.
        let accepted = at!(self, day(-6), {
            self.accident_card("accident2", "Ahmed Raza", "DXB-PR-7002")
                .await
        })?;
        at!(self, day(-5), {
            let report = wl_documents::Document::query()
                .where_(wl_documents::Document::SUBJECT_ID.eq(accepted.id))
                .where_(wl_documents::Document::SUBJECT_TYPE.eq(accepted.subject().subject_type))
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no police report"))?;
            Documents::accept(&self.state, self.who(WALEED), &report).await?;
            job_cards::approve_recorded(
                &self.state,
                self.who(SAMI),
                accepted.id,
                "Ahmed Raza",
                "recorded",
                None,
            )
            .await
        })?;
        self.cards.insert("police_accepted", accepted.id);

        // The technician took the Patrol for a test drive and scratched a
        // door: the incident is the branch manager's to decide (workshop:
        // an internal job card for the car).
        let card = self.cards["in_progress"];
        let incident = at!(self, day(-4), {
            let drive =
                test_drives::record(&self.state, self.who(NABIL), card, NewDrive::out(120_320))
                    .await?;
            test_drives::finish(&self.state, self.who(NABIL), drive.id, 120_331, None).await?;
            incidents::report(
                &self.state,
                self.who(NABIL),
                card,
                NewIncident::new("Scratch on the right door while parking after the test drive")
                    .road_accident(),
            )
            .await
        })?;
        at!(self, day(-3), {
            incidents::decide(
                &self.state,
                self.who(BASMA),
                incident.id,
                "workshop",
                "The technician hit the post in the yard",
            )
            .await
        })?;
        // A second incident is still with the branch manager.
        let quality = self.cards["quality_check"];
        at!(self, day(-2), {
            incidents::report(
                &self.state,
                self.who(TAREQ),
                quality,
                NewIncident::new("Dent on the left mirror found after the wash"),
            )
            .await
        })?;
        Ok(())
    }
}
