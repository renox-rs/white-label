//! The insurance claims, one in each state of the machine: registered,
//! awaiting the survey, estimate submitted, approved, supplementary pending,
//! rejected, withdrawn by the customer, total loss, invoiced, settled and
//! voided. The insurance coordinator talks to the insurers, the technician
//! drafts the lines, the workshop manager accepts the police report, the
//! accountant bills (a payer bill for the insurer, an invoice for the excess).

use auto::claims::{self, Answer, NewClaim, Withdrawal};
use auto::job_cards::{self, NewLabour};
use auto::{InsuranceClaim, LabourLine};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::upload::Upload;
use wl_documents::{Documents, NewDocument};
use wl_kernel::documents::today;
use wl_kernel::payer_bills::{PayerBillId, Remittance, RemittanceLine, payer_bills};
use wl_kernel::payments::PaymentMethod;

use super::super::demo_data::{at, day};
use super::{AISHA, CARLA, NABIL, SAMI, Seed, WALEED};

/// How far a claim goes in its story.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Registered,
    Survey,
    Submitted,
    Approved,
    Supplementary,
    Rejected,
    Withdrawn,
    TotalLoss,
    Invoiced,
    Settled,
    Voided,
}

/// A claim of the story: key, insurer key, vehicle, the customer who signs,
/// day of the accident report, how far it goes, police report number.
struct Case {
    key: &'static str,
    insurer: &'static str,
    car: &'static str,
    customer: &'static str,
    day: i64,
    stage: Stage,
    report: &'static str,
}

const CASES: &[Case] = &[
    Case {
        key: "registered",
        insurer: "falcon",
        car: "cl1",
        customer: "Mona Hassan",
        day: -12,
        stage: Stage::Registered,
        report: "DXB-PR-6101",
    },
    Case {
        key: "survey",
        insurer: "falcon",
        car: "cl2",
        customer: "Priya Nair",
        day: -11,
        stage: Stage::Survey,
        report: "DXB-PR-6102",
    },
    Case {
        key: "submitted",
        insurer: "falcon",
        car: "cl3",
        customer: "Yousef Al Mansoori",
        day: -10,
        stage: Stage::Submitted,
        report: "SHJ-PR-6103",
    },
    Case {
        key: "approved",
        insurer: "falcon",
        car: "cl4",
        customer: "Khalid Ibrahim",
        day: -14,
        stage: Stage::Approved,
        report: "DXB-PR-6104",
    },
    Case {
        key: "supplementary",
        insurer: "falcon",
        car: "cl5",
        customer: "Fatima Al Zaabi",
        day: -13,
        stage: Stage::Supplementary,
        report: "DXB-PR-6105",
    },
    Case {
        key: "rejected",
        insurer: "crescent",
        car: "cl6",
        customer: "Ahmed Raza",
        day: -26,
        stage: Stage::Rejected,
        report: "DXB-PR-6106",
    },
    Case {
        key: "withdrawn",
        insurer: "falcon",
        car: "cl7",
        customer: "Layla Haddad",
        day: -30,
        stage: Stage::Withdrawn,
        report: "AJM-PR-6107",
    },
    Case {
        key: "total_loss",
        insurer: "falcon",
        car: "cl8",
        customer: "Omar Saleh",
        day: -25,
        stage: Stage::TotalLoss,
        report: "DXB-PR-6108",
    },
    Case {
        key: "invoiced",
        insurer: "falcon",
        car: "cl9",
        customer: "Noor Khalid",
        day: -22,
        stage: Stage::Invoiced,
        report: "DXB-PR-6109",
    },
    Case {
        key: "settled",
        insurer: "crescent",
        car: "cl10",
        customer: "Hamdan Saeed",
        day: -60,
        stage: Stage::Settled,
        report: "SHJ-PR-6110",
    },
    Case {
        key: "voided",
        insurer: "crescent",
        car: "cl11",
        customer: "Salim Obaid",
        day: -15,
        stage: Stage::Voided,
        report: "DXB-PR-6111",
    },
];

impl Seed {
    /// The police report on the claim: attached by the coordinator and
    /// accepted by the workshop manager, a second user (a legal block).
    async fn police_report(&self, claim: &InsuranceClaim, number: &str) -> Result {
        let report = Documents::attach(
            &self.state,
            Some(self.who(CARLA)),
            NewDocument::new(
                claim.subject(),
                "police_report",
                Upload::new(
                    "police-report.pdf",
                    "application/pdf",
                    format!("%PDF-1.7 police report {number}").into_bytes(),
                ),
            )
            .number(number),
        )
        .await?
        .expect("the report is attached");
        Documents::accept(&self.state, self.who(WALEED), &report).await?;
        Ok(())
    }

    /// The insurer's answer to the estimate, line by line at their price.
    fn answer(&self, reference: &str, lines: &[LabourLine]) -> Answer {
        let mut answer = Answer::new(reference, &format!("{reference}-approval-email"));
        for line in lines {
            answer = answer.labour(line.id, line.price());
        }
        answer
    }

    /// All the claims of the story.
    pub(crate) async fn claims(&mut self) -> Result {
        for case in CASES {
            self.claim_case(case).await?;
        }
        Ok(())
    }

    async fn claim_case(&mut self, case: &Case) -> Result {
        let insurer = self.insurers[case.insurer].clone();
        let vehicle = self.vehicle(case.car).clone();
        let carla = self.who(CARLA).clone();
        let sami = self.who(SAMI).clone();
        let nabil = self.who(NABIL).clone();
        let with_estimate = case.stage >= Stage::Submitted && case.stage != Stage::Voided;
        // Day 0 of the case: the vehicle comes in, the claim is registered,
        // the estimate is drafted and sent.
        let (card, claim, lines) = at!(self, day(case.day), {
            let card = self
                .check_in(
                    case.car,
                    case.customer,
                    "Accident damage on the right side",
                    NABIL,
                    0,
                    0,
                )
                .await?;
            let mut lines = Vec::new();
            if with_estimate {
                let drafted = self
                    .draft(
                        card.id,
                        NABIL,
                        &[
                            ("Door panel repair", 3_000, 12_000),
                            ("Respray and polish", 2_000, 12_000),
                        ],
                        &[],
                    )
                    .await?;
                lines = drafted.labour;
                self.send(card.id).await?;
            }
            let claim = claims::register(
                &self.state,
                &carla,
                NewClaim::new(
                    insurer.id,
                    vehicle.id,
                    &format!("POL-{}", 7_000 + vehicle.id),
                    &format!("{}-2026-{:03}", case.insurer.to_uppercase(), vehicle.id),
                    today() - TimeDelta::days(3),
                )
                .job_card(card.id)
                .police_report(case.report, Some(today() - TimeDelta::days(2)))
                .excess(20_000),
            )
            .await?;
            Ok::<_, renox::Error>((card, claim, lines))
        })?;
        self.cards.insert(case.key, card.id);
        self.claims.insert(case.key, claim.id);
        if case.stage == Stage::Registered {
            return Ok(());
        }
        if case.stage == Stage::Voided {
            at!(self, day(case.day + 1), {
                claims::void(&self.state, &carla, claim.id, "Registered twice by mistake").await
            })?;
            return Ok(());
        }

        // Survey booked, estimate submitted, the insurer answers.
        at!(self, day(case.day + 1), {
            self.police_report(&claim, case.report).await?;
            claims::book_survey(&self.state, &carla, claim.id).await
        })?;
        if case.stage == Stage::Survey {
            return Ok(());
        }
        at!(self, day(case.day + 2), {
            claims::submit_estimate(&self.state, &carla, claim.id).await
        })?;
        if case.stage == Stage::Submitted {
            return Ok(());
        }
        if case.stage == Stage::Rejected {
            at!(self, day(case.day + 4), {
                claims::reject(
                    &self.state,
                    &carla,
                    claim.id,
                    "The policy had lapsed on the day of the accident",
                )
                .await
            })?;
            return Ok(());
        }
        let reference = format!("LPO-{}", 8_000 + vehicle.id);
        at!(self, day(case.day + 4), {
            claims::approve(
                &self.state,
                &carla,
                claim.id,
                self.answer(&reference, &lines),
            )
            .await
        })?;
        if case.stage == Stage::Approved {
            // The job goes on: the first line is done.
            at!(self, day(case.day + 5), {
                job_cards::record_work(
                    &self.state,
                    &nabil,
                    lines[0].id,
                    auto::WorkStatus::Done,
                    None,
                )
                .await
            })?;
            return Ok(());
        }
        if case.stage == Stage::Supplementary {
            at!(self, day(case.day + 5), {
                job_cards::record_work(
                    &self.state,
                    &nabil,
                    lines[0].id,
                    auto::WorkStatus::Done,
                    None,
                )
                .await?;
                job_cards::add_labour(
                    &self.state,
                    &nabil,
                    card.id,
                    NewLabour::hourly("Straighten the frame rail", 3_000, 12_000),
                )
                .await?;
                job_cards::send_estimate(
                    &self.state,
                    &sami,
                    card.id,
                    auto::job_cards::Promise::default(),
                )
                .await?;
                claims::request_supplementary(
                    &self.state,
                    &sami,
                    claim.id,
                    "Frame rail bent behind the bumper",
                )
                .await
            })?;
            return Ok(());
        }
        if case.stage == Stage::Withdrawn {
            at!(self, day(case.day + 6), {
                job_cards::record_work(
                    &self.state,
                    &nabil,
                    lines[0].id,
                    auto::WorkStatus::Done,
                    None,
                )
                .await?;
                claims::withdraw(
                    &self.state,
                    &carla,
                    claim.id,
                    Withdrawal::new("Withdrawn by the customer: the car is sold")
                        .document("withdrawal-letter-107")
                        .stop(),
                )
                .await
            })?;
            return Ok(());
        }
        if case.stage == Stage::TotalLoss {
            at!(self, day(case.day + 6), {
                job_cards::record_work(
                    &self.state,
                    &nabil,
                    lines[0].id,
                    auto::WorkStatus::Done,
                    None,
                )
                .await?;
                claims::declare_total_loss(
                    &self.state,
                    &carla,
                    claim.id,
                    "falcon-total-loss-notice-108",
                )
                .await
            })?;
            return Ok(());
        }

        // The repair: the work, the QC (the claim completes by itself), the
        // collection, then the accountant bills the insurer and the customer.
        at!(self, day(case.day + 8), {
            for line in &lines {
                job_cards::record_work(&self.state, &nabil, line.id, auto::WorkStatus::Done, None)
                    .await?;
            }
            self.quality(card.id, NABIL, true).await?;
            self.deliver(card.id, case.customer).await
        })?;
        let invoiced = at!(self, day(case.day + 9), {
            claims::invoice(&self.state, self.who(AISHA), claim.id).await
        })?;
        if case.stage == Stage::Invoiced {
            return Ok(());
        }
        // Settled: the insurer's advice pays its part, the customer pays the
        // excess.
        let bill = invoiced
            .payer_bill_id
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no payer bill"))?;
        at!(self, day(case.day + 30), {
            let bills = payer_bills(&self.state);
            let mut advice = Remittance::new("CRS-ADV-1", today());
            for line in bills.lines(self.db(), PayerBillId(bill)).await? {
                advice = advice.line(RemittanceLine::paid(line.id, line.claimed));
            }
            bills
                .record_remittance(&self.state, PayerBillId(bill), advice, self.who(AISHA))
                .await
        })?;
        if let Some(invoice) = invoiced.customer_invoice_id {
            self.pay_invoice(invoice, PaymentMethod::Card, None, case.day + 31, "")
                .await?;
        }
        at!(self, day(case.day + 31), {
            claims::settle_if_paid(&self.state, claim.id).await
        })?;
        Ok(())
    }
}
