//! The workshop: job cards in every state of the machine, each told by the
//! people who do it (the advisor checks in and sends the estimate, the
//! technician inspects, drafts the lines, issues and fits the parts, the
//! customer answers on the link, the workshop manager passes the QC), a
//! booking that is checked in, and the vehicles nobody collects.

use auto::job_cards::{self, Delivery, NewJobCard, NewLabour, NewPart, Promise};
use auto::job_stops::{self, StopRequest};
use auto::{JobCard, LabourLine, PartLine, WorkStatus, parts};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::payments::PaymentMethod;
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{Resource, ServiceType};

use super::super::demo_data::{at, day};
use super::shop::workshop;
use super::{NABIL, SAMI, Seed, TAREQ, WALEED};

/// A labour line: what, hours in thousandths, rate an hour in fils.
pub(crate) type Labour = (&'static str, i64, i64);
/// A part line: what, stock code (or none), quantity in thousandths, price.
pub(crate) type Part = (&'static str, &'static str, i64, i64);

/// The lines of a job card as they were drafted.
pub(crate) struct Lines {
    pub(crate) labour: Vec<LabourLine>,
    pub(crate) parts: Vec<PartLine>,
}

impl Seed {
    /// Checks the vehicle `car` in at the desk. The odometer reads 10 km more
    /// than the register.
    pub(crate) async fn check_in(
        &self,
        car: &str,
        ack: &str,
        complaint: &str,
        tech: &str,
        inspection_fee: i64,
        storage_fee: i64,
    ) -> Result<JobCard> {
        let vehicle = auto::Vehicle::find_or_404(self.db(), self.vehicle(car).id).await?;
        job_cards::check_in(
            &self.state,
            self.who(SAMI),
            NewJobCard::new(vehicle.id, complaint)
                .odometer(vehicle.odometer_km + 10)
                .fuel("half")
                .condition("Light scratches on the bumper, interior clean")
                .acknowledged_by(ack)
                .inspection_fee(inspection_fee)
                .storage_fee(storage_fee)
                .technician(self.user(tech).id)
                .workshop(workshop()),
        )
        .await
    }

    /// The technician inspects and drafts the lines of the estimate.
    pub(crate) async fn draft(
        &self,
        card: i64,
        tech: &str,
        labour: &[Labour],
        stock: &[Part],
    ) -> Result<Lines> {
        let by = self.who(tech);
        let current = JobCard::find_or_404(self.db(), card).await?;
        if current.status == auto::JobCardStatus::CheckedIn {
            job_cards::start_inspection(&self.state, by, card).await?;
        }
        let mut lines = Lines {
            labour: Vec::new(),
            parts: Vec::new(),
        };
        for (what, hours, rate) in labour {
            lines.labour.push(
                job_cards::add_labour(
                    &self.state,
                    by,
                    card,
                    NewLabour::hourly(what, *hours, *rate),
                )
                .await?,
            );
        }
        for (what, code, quantity, price) in stock {
            let mut new = NewPart::new(what, *quantity, *price);
            if !code.is_empty() {
                new = new.item(code);
            }
            lines
                .parts
                .push(job_cards::add_part(&self.state, by, card, new).await?);
        }
        Ok(lines)
    }

    /// The advisor sends the estimate: promised for tomorrow, 30 days of
    /// warranty.
    pub(crate) async fn send(&self, card: i64) -> Result<JobCard> {
        job_cards::send_estimate(
            &self.state,
            self.who(SAMI),
            card,
            Promise {
                promised_for: Some(wl_kernel::documents::today() + TimeDelta::days(1)),
                warranty_days: Some(30),
            },
        )
        .await
    }

    /// The customer approves on the link.
    pub(crate) async fn approve_link(&self, card: i64, name: &str) -> Result<JobCard> {
        let round = job_cards::pending_round(self.db(), card)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no estimate to approve"))?;
        job_cards::approve_by_link(&self.state, &round.token, name, None, true).await
    }

    /// The technician does the work: the labour done, the parts issued from
    /// the workshop's store and fitted.
    pub(crate) async fn work(
        &self,
        tech: &str,
        labour: &[&LabourLine],
        stock: &[&PartLine],
    ) -> Result {
        let by = self.who(tech);
        for line in labour {
            job_cards::record_work(&self.state, by, line.id, WorkStatus::Done, None).await?;
        }
        for line in stock {
            parts::issue(&self.state, by, line.id, None).await?;
            parts::fit(&self.state, by, line.id, None).await?;
        }
        Ok(())
    }

    /// The technician finishes the work; the workshop manager passes the QC.
    pub(crate) async fn quality(&self, card: i64, tech: &str, pass: bool) -> Result {
        job_cards::finish_work(&self.state, self.who(tech), card).await?;
        if pass {
            let all: Vec<String> = auto::job_card::QC_KEYS
                .iter()
                .map(|k| (*k).to_owned())
                .collect();
            job_cards::save_qc(&self.state, self.who(WALEED), card, &all).await?;
            job_cards::pass_qc(&self.state, self.who(WALEED), card).await?;
        }
        Ok(())
    }

    /// The customer collects the vehicle; the card with its invoice.
    pub(crate) async fn deliver(&self, card: i64, to: &str) -> Result<JobCard> {
        let current = JobCard::find_or_404(self.db(), card).await?;
        job_cards::deliver(
            &self.state,
            self.who(SAMI),
            card,
            Delivery {
                delivered_to: to.to_owned(),
                odometer_out: Some(current.odometer_in + 30),
                ..Delivery::default()
            },
        )
        .await
    }

    /// A visit from the check-in to the delivery and the payment of its
    /// invoice (card), on `from` and `to`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn visit(
        &mut self,
        key: &'static str,
        car: &str,
        ack: &str,
        complaint: &str,
        tech: &'static str,
        labour: &[Labour],
        stock: &[Part],
        from: i64,
        to: i64,
    ) -> Result<Lines> {
        let (card, lines) = at!(self, day(from), {
            let card = self.check_in(car, ack, complaint, tech, 0, 0).await?;
            let lines = self.draft(card.id, tech, labour, stock).await?;
            self.send(card.id).await?;
            self.approve_link(card.id, ack).await?;
            let done: Vec<&LabourLine> = lines.labour.iter().collect();
            let fitted: Vec<&PartLine> = lines
                .parts
                .iter()
                .filter(|p| p.item_code.is_some())
                .collect();
            self.work(tech, &done, &fitted).await?;
            Ok::<_, renox::Error>((card, lines))
        })?;
        let delivered = at!(self, day(to), {
            self.quality(card.id, tech, true).await?;
            self.deliver(card.id, ack).await
        })?;
        self.cards.insert(key, card.id);
        if let Some(invoice) = delivered.invoice_id {
            self.pay_invoice(invoice, PaymentMethod::Card, None, to, "")
                .await?;
        }
        Ok(lines)
    }

    /// The job cards, one in each state of the machine.
    pub(crate) async fn workshop(&mut self) -> Result {
        // Loyalty members: the points of a paid invoice come to them.
        at!(self, day(-120), {
            for (party, mobile) in [
                ("mona", "0501110001"),
                ("priya", "0501110004"),
                ("ahmed", "0501110007"),
            ] {
                self.driver
                    .post(
                        super::ADMIN_LOGIN,
                        "/loyalty/accounts",
                        &[
                            ("party_id", &self.party(party).0.to_string()),
                            ("mobile", mobile),
                            ("terms", "1"),
                        ],
                    )
                    .await?;
            }
            Ok::<(), renox::Error>(())
        })?;

        // Delivered, invoiced and paid: the periodic service of a Hilux, and
        // the repairs whose parts later fail under warranty.
        self.visit(
            "delivered_hilux",
            "hilux",
            "Mona Hassan",
            "Periodic service at 55,000 km",
            TAREQ,
            &[("Periodic service labour", 2_000, 15_000)],
            &[
                ("Oil filter", "OIL-F", 1_000, 4_500),
                ("Air filter", "AIR-F", 1_000, 6_000),
            ],
            -95,
            -93,
        )
        .await?;
        self.visit(
            "delivered_pajero",
            "pajero",
            "Salim Obaid",
            "Battery light on, engine will not charge",
            NABIL,
            &[("Replace alternator", 3_000, 15_000)],
            &[("Alternator", "ALT-9", 1_000, 62_000)],
            -92,
            -90,
        )
        .await?;
        self.visit(
            "delivered_pajero_battery",
            "pajero",
            "Salim Obaid",
            "Slow start in the morning",
            NABIL,
            &[("Replace battery", 500, 15_000)],
            &[("Battery 70Ah", "BAT-70", 1_000, 42_000)],
            -80,
            -80,
        )
        .await?;
        self.visit(
            "delivered_civic",
            "civic",
            "Hamdan Saeed",
            "Misfire when cold",
            TAREQ,
            &[("Replace spark plugs", 1_000, 15_000)],
            &[("Spark plug set (4)", "SPARK-4", 1_000, 9_000)],
            -70,
            -69,
        )
        .await?;

        // Checked in from a booking: the booking of the periodic service is
        // checked in at the desk and opens the job card.
        let service = ServiceType::query()
            .where_(ServiceType::CODE.eq("periodic_service"))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no periodic service type"))?;
        let bay = Resource::query()
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no bay"))?;
        let tucson = self.vehicle("tucson").clone();
        let owner = self.party("priya");
        let start = wl_kernel::BusinessCalendar::default()
            .from_local(day(-4).and_hms_opt(12, 0, 0).unwrap_or_default());
        let advisor = self.who(SAMI).clone();
        let booking = at!(self, day(-6), {
            service::book(
                &self.state,
                NewBooking::new(PartyId(owner.0), service.id, &[bay.id], start)
                    .for_subject(tucson.subject())
                    .at(workshop()),
                Actor::staff(advisor.id),
                start - TimeDelta::hours(1),
            )
            .await
            .map_err(|e| renox::anyhow::anyhow!("demo seed: booking refused: {e:?}"))
        })?;
        let card = at!(self, day(-4), {
            service::check_in(
                &self.state,
                booking.id,
                Actor::staff(advisor.id),
                renox::db::now(),
            )
            .await
            .map_err(|e| renox::anyhow::anyhow!("demo seed: check-in refused: {e:?}"))?;
            let card = JobCard::of_booking(self.db(), booking.id)
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the check-in opened no card"))?;
            job_cards::record_intake(
                &self.state,
                &advisor,
                card.id,
                job_cards::Intake {
                    complaint: "Periodic service at 30,000 km".to_owned(),
                    odometer_in: tucson.odometer_km + 20,
                    fuel_level: Some("three_quarters".to_owned()),
                    condition: "Clean, small chip on the windscreen".to_owned(),
                    acknowledged_by: Some("Priya Nair".to_owned()),
                    signature: None,
                    accident_damage: false,
                    police_report_no: None,
                    inspection_fee: 0,
                    storage_fee_daily: 0,
                },
            )
            .await?;
            job_cards::assign_technician(&self.state, &advisor, card.id, self.user(TAREQ).id)
                .await?;
            Ok::<JobCard, renox::Error>(card)
        })?;
        self.cards.insert("checked_in_booking", card.id);

        // Awaiting approval: the estimate is with the customer, who has the
        // link; a part and two labour lines.
        let sportage_card = at!(self, day(-2), {
            let card = self
                .check_in(
                    "sportage",
                    "Khalid Ibrahim",
                    "Brakes grind when stopping",
                    TAREQ,
                    0,
                    0,
                )
                .await?;
            self.draft(
                card.id,
                TAREQ,
                &[
                    ("Brake pads and discs service", 2_500, 15_000),
                    ("Brake fluid change", 500, 15_000),
                ],
                &[
                    ("Brake pad set", "BRK-PAD", 1_000, 14_000),
                    ("Oil filter", "OIL-F", 1_000, 4_500),
                ],
            )
            .await?;
            self.send(card.id).await
        })?;
        self.cards.insert("awaiting_approval", sportage_card.id);

        // In progress: approved, half of the work done, one part fitted.
        let l200 = at!(self, day(-5), {
            let card = self
                .check_in(
                    "l200",
                    "Fatima Al Zaabi",
                    "Noise from the front suspension",
                    NABIL,
                    0,
                    0,
                )
                .await?;
            let lines = self
                .draft(
                    card.id,
                    NABIL,
                    &[
                        ("Replace front shock absorbers", 3_000, 15_000),
                        ("Wheel alignment", 1_000, 15_000),
                    ],
                    &[
                        ("Brake pad set", "BRK-PAD", 1_000, 14_000),
                        ("Air filter", "AIR-F", 1_000, 6_000),
                    ],
                )
                .await?;
            self.send(card.id).await?;
            self.approve_link(card.id, "Fatima Al Zaabi").await?;
            self.work(NABIL, &[&lines.labour[0]], &[&lines.parts[0]])
                .await?;
            Ok::<JobCard, renox::Error>(card)
        })?;
        self.cards.insert("in_progress", l200.id);

        // Awaiting parts: the gearbox mount is imported; the legal supply
        // date is on the card.
        let lexus = at!(self, day(-7), {
            let card = self
                .check_in("lexus", "Layla Haddad", "Vibration at idle", TAREQ, 0, 0)
                .await?;
            self.draft(
                card.id,
                TAREQ,
                &[("Replace gearbox mount", 2_000, 15_000)],
                &[],
            )
            .await?;
            job_cards::add_part(
                &self.state,
                self.who(TAREQ),
                card.id,
                NewPart::new("Gearbox mount (imported)", 1_000, 86_000).from_source("import"),
            )
            .await?;
            self.send(card.id).await?;
            self.approve_link(card.id, "Layla Haddad").await?;
            job_cards::wait_parts(&self.state, self.who(TAREQ), card.id).await
        })?;
        self.cards.insert("awaiting_parts", lexus.id);

        // Quality check: the work is done, the QC waits for the manager.
        let corolla = at!(self, day(-3), {
            let card = self
                .check_in(
                    "corolla",
                    "Ahmed Raza",
                    "Free service at 15,000 km",
                    TAREQ,
                    0,
                    0,
                )
                .await?;
            let lines = self
                .draft(
                    card.id,
                    TAREQ,
                    &[("Free service labour", 1_500, 15_000)],
                    &[("Oil filter", "OIL-F", 1_000, 4_500)],
                )
                .await?;
            self.send(card.id).await?;
            self.approve_link(card.id, "Ahmed Raza").await?;
            self.work(TAREQ, &[&lines.labour[0]], &[&lines.parts[0]])
                .await?;
            self.quality(card.id, TAREQ, false).await?;
            Ok::<JobCard, renox::Error>(card)
        })?;
        self.cards.insert("quality_check", corolla.id);

        // Ready, and not collected: the storage fee is agreed at AED 30 a day.
        let accord = at!(self, day(-9), {
            let card = self
                .check_in(
                    "accord",
                    "Yousef Al Mansoori",
                    "Air conditioning does not cool",
                    NABIL,
                    0,
                    3_000,
                )
                .await?;
            let lines = self
                .draft(
                    card.id,
                    NABIL,
                    &[("Recharge air conditioning", 1_500, 15_000)],
                    &[("Air filter", "AIR-F", 1_000, 6_000)],
                )
                .await?;
            self.send(card.id).await?;
            self.approve_link(card.id, "Yousef Al Mansoori").await?;
            self.work(NABIL, &[&lines.labour[0]], &[&lines.parts[0]])
                .await?;
            Ok::<JobCard, renox::Error>(card)
        })?;
        at!(self, day(-8), {
            self.quality(accord.id, NABIL, true).await
        })?;
        self.cards.insert("ready", accord.id);

        // Cancelled after the work started: the customer stopped and signed;
        // the work done is billed, the oil filter issued and not fitted goes
        // back to the store, and the vehicle waits for its owner.
        let patrol = at!(self, day(-20), {
            let card = self
                .check_in(
                    "patrol",
                    "Rashed Salem",
                    "Engine overheats on the highway",
                    TAREQ,
                    0,
                    5_000,
                )
                .await?;
            let lines = self
                .draft(
                    card.id,
                    TAREQ,
                    &[
                        ("Replace radiator", 4_000, 15_000),
                        ("Flush cooling system", 1_000, 15_000),
                    ],
                    &[
                        ("Oil filter", "OIL-F", 1_000, 4_500),
                        ("Spark plug set (4)", "SPARK-4", 1_000, 9_000),
                    ],
                )
                .await?;
            self.send(card.id).await?;
            self.approve_link(card.id, "Rashed Salem").await?;
            self.work(TAREQ, &[&lines.labour[1]], &[&lines.parts[1]])
                .await?;
            parts::issue(&self.state, self.who(TAREQ), lines.parts[0].id, None).await?;
            Ok::<JobCard, renox::Error>(card)
        })?;
        at!(self, day(-16), {
            job_stops::cancel(
                &self.state,
                self.who(SAMI),
                patrol.id,
                StopRequest::new("customer_stopped").signed("Rashed Salem"),
            )
            .await
        })?;
        self.cards.insert("cancelled", patrol.id);

        // Declined: the customer found the estimate too dear and said so on
        // the link; the inspection fee agreed at check-in is invoiced.
        let sunny = at!(self, day(-36), {
            let card = self
                .check_in(
                    "sunny",
                    "Omar Saleh",
                    "Gearbox slips between gears",
                    TAREQ,
                    5_000,
                    3_000,
                )
                .await?;
            self.draft(
                card.id,
                TAREQ,
                &[("Rebuild gearbox", 12_000, 15_000)],
                &[("Clutch kit", "CLUTCH-K", 1_000, 78_000)],
            )
            .await?;
            self.send(card.id).await?;
            Ok::<JobCard, renox::Error>(card)
        })?;
        at!(self, day(-35), {
            let round = job_cards::pending_round(self.db(), sunny.id)
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no round to decline"))?;
            job_cards::decline_by_link(
                &self.state,
                &round.token,
                "Omar Saleh",
                "Too expensive, I will sell the car",
            )
            .await
        })?;
        self.cards.insert("declined", sunny.id);

        // Voided: opened for the wrong vehicle.
        let camry = at!(self, day(-4), {
            let card = self
                .check_in("camry", "Noor Khalid", "Squeak from the belt", NABIL, 0, 0)
                .await?;
            job_cards::void(
                &self.state,
                self.who(SAMI),
                card.id,
                "Opened for the wrong vehicle",
            )
            .await
        })?;
        self.cards.insert("voided", camry.id);
        Ok(())
    }

    /// The daily runs of the days gone by: the reminders to the owners of the
    /// vehicles nobody collects (3 and 7 days, and the branch manager's task
    /// at 30 days) and the storage fee they run up.
    pub(crate) async fn reminders(&self) -> Result {
        for on in [-32, -28, -13, -9, -5, -1] {
            at!(self, day(on), {
                auto::storage::process(&self.state, day(on)).await
            })?;
        }
        Ok(())
    }
}
