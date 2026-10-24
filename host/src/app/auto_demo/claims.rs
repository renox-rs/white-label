//! The insurance repairs: Sara Al Mansoori's Fortuner, hit in a car park, its
//! claim registered with the police report, the estimate submitted and
//! approved by Gulf Shield line by line with the excess, the body work
//! running; and Khalid Obaid's Tiida, whose estimate is with Desert Falcon.
//! Also the two job cards that went another way: the Sunny's estimate the
//! customer declined, and the Pajero waiting for an alternator ordered from
//! the supplier.

use renox::prelude::*;
use wl_documents::{Document, DocumentKind, DocumentStatus};

use super::{ADVISOR_EMAIL, INSURANCE_EMAIL, Seed, TECH_EMAIL, TECH2_EMAIL, WORKSHOP_EMAIL, day};

impl Seed {
    /// The insurer record of the party `key`.
    async fn insurer(&self, key: &str) -> Result<i64> {
        let party = self.parties[key].0;
        let rows = renox::db::sql(format!(
            "select id from auto_insurers where party_id = {party}"
        ))
        .fetch_all(self.db())
        .await?;
        let row = rows
            .first()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the insurer {key}"))?;
        Ok(row.try_get::<i64>("id")?)
    }

    /// The police accident report of job card `job`, uploaded by the
    /// advisor and accepted by the workshop manager (the second user,
    /// `AUT.ACL.20`). Nothing when the document kinds are not installed.
    async fn police_report(&self, job: i64, number: &str) -> Result {
        let Some(kind) = DocumentKind::by_key(self.db(), auto::seed::POLICE_REPORT_KIND).await?
        else {
            return Ok(());
        };
        let card = auto::JobCard::find_or_404(self.db(), job).await?;
        let subject = card.subject();
        Document::create(
            self.db(),
            Document {
                subject_type: subject.subject_type.clone(),
                subject_id: subject.subject_id,
                kind_id: kind.id,
                title: "Police accident report".into(),
                number: Some(number.to_owned()),
                issue_date: Some(day(-4)),
                issuing_authority: Some("Dubai Police".into()),
                current_version: 1,
                status: DocumentStatus::Accepted,
                uploaded_by: Some(self.user(ADVISOR_EMAIL).await?.id),
                reviewed_by: Some(self.user(WORKSHOP_EMAIL).await?.id),
                reviewed_at: Some(renox::db::now()),
                accepted_at: Some(renox::db::now()),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// An accident job card: checked in with the police report, inspected
    /// and priced by `technician`.
    async fn accident_job(
        &mut self,
        key: &'static str,
        technician: &str,
        hours_ago: i64,
        police: &str,
        lines: &[(&str, &str, &str, &str)],
        hours: &str,
    ) -> Result<i64> {
        let id = self
            .check_in(key, key, "accident_estimate", hours_ago)
            .await?;
        let vehicle = auto::Vehicle::find_or_404(self.db(), self.vehicles[key]).await?;
        let owner = wl_parties::Party::find(self.db(), vehicle.owner_party_id)
            .await?
            .map(|p| p.legal_name_en)
            .unwrap_or_default();
        self.on_job(
            ADVISOR_EMAIL,
            id,
            "intake",
            &[
                (
                    "complaint",
                    "Accident damage to the front, insurance repair",
                ),
                ("odometer", &vehicle.odometer_km.to_string()),
                ("fuel", "quarter"),
                ("condition", "Front bumper cracked, left headlamp broken"),
                ("acknowledged_by", &owner),
                ("accident", "on"),
                ("police_report", police),
            ],
        )
        .await?;
        let tech = self.user(technician).await?;
        self.on_job(
            ADVISOR_EMAIL,
            id,
            "technician",
            &[("technician", &tech.id.to_string())],
        )
        .await?;
        self.police_report(id, police).await?;
        self.on_job(technician, id, "inspect", &[]).await?;
        self.on_job(
            technician,
            id,
            "labour",
            &[("operation", "BODY-PAINT"), ("hours", hours)],
        )
        .await?;
        for (code, name, quantity, price) in lines {
            self.on_job(
                technician,
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
        Ok(id)
    }

    /// The claim of job card `job` with insurer `insurer`, registered by the
    /// insurance coordinator, the estimate sent and submitted; its id.
    async fn claim(
        &self,
        job: i64,
        vehicle: &str,
        insurer: &str,
        numbers: (&str, &str, &str),
        excess: &str,
    ) -> Result<i64> {
        let (policy, claim_no, police) = numbers;
        let made = self
            .driver
            .post(
                INSURANCE_EMAIL,
                "/auto/claims",
                &[
                    ("insurer", &self.insurer(insurer).await?.to_string()),
                    ("vehicle", &self.vehicles[vehicle].to_string()),
                    ("job_card", &job.to_string()),
                    ("policy_no", policy),
                    ("claim_no", claim_no),
                    ("accident_date", &day(-4).to_string()),
                    ("police_report_no", police),
                    ("police_report_date", &day(-4).to_string()),
                    ("repair_type", "non_agency"),
                    ("excess", excess),
                ],
            )
            .await?;
        let id = made
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the claim of {vehicle}"))?;
        // The advisor sends the estimate; the claim submits what was sent.
        self.on_job(
            ADVISOR_EMAIL,
            job,
            "send",
            &[
                ("promised_for", &day(5).to_string()),
                ("warranty_days", "180"),
            ],
        )
        .await?;
        self.driver
            .post(INSURANCE_EMAIL, &format!("/auto/claims/{id}/submit"), &[])
            .await?;
        Ok(id)
    }

    /// The insurance repairs, the declined estimate and the job waiting for a
    /// part.
    pub(crate) async fn claims(&mut self) -> Result {
        // Fortuner: approved by Gulf Shield, the body work running.
        let fortuner = self
            .accident_job(
                "fortuner",
                TECH_EMAIL,
                52,
                "DXB-PR-2026-118734",
                &[
                    ("BMP-FR-FT", "Front bumper, Fortuner", "1", "1,780"),
                    ("HDL-LH-FT", "Headlamp left, Fortuner", "1", "1,320"),
                ],
                "6",
            )
            .await?;
        let claim = self
            .claim(
                fortuner,
                "fortuner",
                "shield",
                ("GS-MTR-0048821", "GS-CLM-26-07713", "DXB-PR-2026-118734"),
                "500",
            )
            .await?;
        let mut answer: Vec<(String, String)> = vec![
            ("reference".to_owned(), "GS-APR-26-07713".to_owned()),
            (
                "document".to_owned(),
                "Approval letter GS-APR-26-07713".to_owned(),
            ),
            ("excess".to_owned(), "500".to_owned()),
        ];
        for line in self.lines("auto_job_labour_lines", fortuner).await? {
            answer.push((format!("line_labour_{line}"), "1,440".to_owned()));
        }
        for (line, amount) in self
            .lines("auto_job_part_lines", fortuner)
            .await?
            .into_iter()
            .zip(["1,780", "1,250"])
        {
            answer.push((format!("line_part_{line}"), amount.to_owned()));
        }
        let pairs: Vec<(&str, &str)> = answer
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.driver
            .post(
                INSURANCE_EMAIL,
                &format!("/auto/claims/{claim}/approve"),
                &pairs,
            )
            .await?;
        for line in self.lines("auto_job_part_lines", fortuner).await? {
            self.on_job(TECH_EMAIL, fortuner, &format!("parts/{line}/issue"), &[])
                .await?;
        }

        // Tiida: the estimate is with Desert Falcon.
        let tiida = self
            .accident_job(
                "tiida",
                TECH2_EMAIL,
                38,
                "DXB-PR-2026-121905",
                &[("WPR-BLD-24", "Wiper blade 24 in", "2", "35")],
                "4",
            )
            .await?;
        self.claim(
            tiida,
            "tiida",
            "falcon",
            ("DF-MOT-77310", "DF-26-04452", "DXB-PR-2026-121905"),
            "350",
        )
        .await?;

        // Sunny: the customer declined the clutch estimate.
        let sunny = self.check_in("sunny", "sunny", "diagnosis", 75).await?;
        self.intake(sunny, "Clutch slipping on hills", "189,760", "Ravi Kumar")
            .await?;
        let tech = self.user(TECH2_EMAIL).await?;
        self.on_job(
            ADVISOR_EMAIL,
            sunny,
            "technician",
            &[("technician", &tech.id.to_string())],
        )
        .await?;
        self.on_job(TECH2_EMAIL, sunny, "inspect", &[]).await?;
        self.on_job(
            TECH2_EMAIL,
            sunny,
            "labour",
            &[("operation", "CLUTCH"), ("hours", "4")],
        )
        .await?;
        self.on_job(
            TECH2_EMAIL,
            sunny,
            "parts",
            &[
                ("item_code", "CLT-KIT-NS"),
                ("description", "Clutch kit, Nissan Sunny"),
                ("quantity", "1"),
                ("unit_price", "980"),
                ("condition", "new"),
            ],
        )
        .await?;
        self.on_job(
            ADVISOR_EMAIL,
            sunny,
            "send",
            &[
                ("promised_for", &day(1).to_string()),
                ("warranty_days", "90"),
            ],
        )
        .await?;
        self.on_job(
            ADVISOR_EMAIL,
            sunny,
            "decline",
            &[(
                "reason",
                "Customer will sell the car; asked for the old clutch to stay",
            )],
        )
        .await?;

        // Pajero: the alternator is on special order from the supplier.
        let pajero = self.check_in("pajero", "pajero", "diagnosis", 80).await?;
        self.intake(
            pajero,
            "Battery light on, battery drains overnight",
            "152,640",
            "Omar Siddiqui",
        )
        .await?;
        self.on_job(
            ADVISOR_EMAIL,
            pajero,
            "technician",
            &[("technician", &tech.id.to_string())],
        )
        .await?;
        self.on_job(TECH2_EMAIL, pajero, "inspect", &[]).await?;
        self.on_job(
            TECH2_EMAIL,
            pajero,
            "labour",
            &[("operation", "DIAG"), ("hours", "2")],
        )
        .await?;
        self.on_job(
            TECH2_EMAIL,
            pajero,
            "parts",
            &[
                ("item_code", "ALT-PJ-90A"),
                ("description", "Alternator 90 A, Pajero"),
                ("quantity", "1"),
                ("unit_price", "2,150"),
                ("condition", "new"),
                ("special_order", "on"),
                ("source", "local_supplier"),
            ],
        )
        .await?;
        self.on_job(
            ADVISOR_EMAIL,
            pajero,
            "send",
            &[
                ("promised_for", &day(3).to_string()),
                ("warranty_days", "180"),
            ],
        )
        .await?;
        self.on_job(
            ADVISOR_EMAIL,
            pajero,
            "approve",
            &[
                ("name", "Omar Siddiqui"),
                ("method", "recorded"),
                ("signature", ""),
            ],
        )
        .await?;
        if let Some(line) = self.lines("auto_job_part_lines", pajero).await?.first() {
            self.on_job(
                ADVISOR_EMAIL,
                pajero,
                &format!("parts/{line}/expected"),
                &[("expected", &day(3).to_string())],
            )
            .await?;
        }
        self.on_job(TECH2_EMAIL, pajero, "wait-parts", &[]).await?;
        Ok(())
    }
}
