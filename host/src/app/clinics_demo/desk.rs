//! The front desk of the demo clinic: the practitioners, rooms and equipment
//! with their opening hours, the service types the clinic adds to the seeded
//! ones, and the appointments in every state a booking reaches (booked,
//! confirmed, checked in, in progress, completed, cancelled with and without
//! a deposit, no-show), a waiting-list entry and the bookings a sick
//! practitioner puts in conflict.
//!
//! Bookings, check-ins, starts, completions, cancellations and no-shows are
//! wl-scheduling's own services; the clinic's appointment, visit, charge
//! capture and billing follow from its events. What the front desk records on
//! the appointment page and what the practitioner captures on the visit are
//! requests to the app's router as those users. A step of the past runs on
//! the seed's clock set to the visit's time, so the services stamp the day of
//! the visit.

use clinics::seed::REQUIRED_QUALIFICATIONS;
use renox::chrono::{Duration, NaiveDateTime, Timelike};
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::BusinessCalendar;
use wl_kernel::payments::{PaymentMethod, RecordPayment, payment_recorder};
use wl_scheduling::service::{self, Actor, CancelRequest, NewBooking};
use wl_scheduling::waiting::{self, NewEntry};
use wl_scheduling::{AvailabilityRule, Booking, Resource, ResourceType, ServiceType, resource};

use super::super::demo_data::day;
use super::people::{
    AHMED, DANIEL, ELENA, FATIMA, JOHN, KHALID, LAYAN, MOHAMMED, PRIYA, SOFIA, YOUSEF,
};
use super::{BUSINESS_BAY, JUMEIRAH, Seed, branch};

/// The resources and service types of the clinic, by their ids.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Kit {
    pub(crate) nadia: i64,
    pub(crate) yara: i64,
    pub(crate) samir: i64,
    pub(crate) room_physio: i64,
    pub(crate) room_dental: i64,
    pub(crate) xray: i64,
    pub(crate) consultation: i64,
    pub(crate) follow_up: i64,
    pub(crate) procedure: i64,
    pub(crate) dental: i64,
    pub(crate) physio: i64,
    pub(crate) dental_xray: i64,
}

/// What a story of an appointment ends with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum End {
    /// Booked and left there.
    Booked,
    /// Confirmed by the patient.
    Confirmed,
    /// The patient is at the desk.
    CheckedIn,
    /// The practitioner has started.
    InProgress,
    /// The visit is over and billed.
    Completed,
    /// The patient cancelled less than the forfeit window before the start.
    CancelledLate,
    /// The patient cancelled in good time.
    CancelledInTime,
    /// The clinic cancelled.
    CancelledByClinic,
    /// The patient did not come.
    NoShow,
}

/// When an appointment starts.
#[derive(Debug, Clone, Copy)]
pub(crate) enum When {
    /// On a day (from today) at a wall time.
    Day(i64, u32, u32),
    /// So many minutes from now (negative: before), for today's stories.
    Minutes(i64),
}

/// One appointment of the story.
#[derive(Debug, Clone)]
pub(crate) struct Story {
    pub(crate) patient: usize,
    pub(crate) service: i64,
    pub(crate) resources: Vec<i64>,
    pub(crate) branch: i64,
    pub(crate) when: When,
    /// Whether the appointment is for the patient's policy.
    pub(crate) insured: bool,
    pub(crate) end: End,
    /// What the practitioner captures: `(code, units, consumable)`.
    pub(crate) lines: Vec<(&'static str, i64, bool)>,
    /// The doctor's login.
    pub(crate) doctor: &'static str,
    /// The receptionist's login.
    pub(crate) desk: &'static str,
}

const NADIA_LOGIN: &str = "nadia@palmgrove.example";
const YARA_LOGIN: &str = "yara@palmgrove.example";
const SAMIR_LOGIN: &str = "samir@palmgrove.example";
const JUMEIRAH_DESK: &str = "reception.jumeirah@palmgrove.example";
const BAY_DESK: &str = "reception.bay@palmgrove.example";

/// The opening hours of every resource: every day 08:00 to 20:00.
const OPEN: (i64, i64) = (8 * 60, 20 * 60);

impl Seed {
    /// The instant of an Asia/Dubai wall time.
    pub(crate) fn utc(&self, local: NaiveDateTime) -> DateTime {
        BusinessCalendar::default().from_local(local)
    }

    /// The wall time `hour:minute` of the day `n` days from today.
    pub(crate) fn local(&self, n: i64, hour: u32, minute: u32) -> NaiveDateTime {
        day(n).and_hms_opt(hour, minute, 0).unwrap_or_default()
    }

    /// The Asia/Dubai wall time of an instant.
    pub(crate) fn wall(&self, at: DateTime) -> NaiveDateTime {
        (at + Duration::seconds(i64::from(wl_kernel::documents::DUBAI_OFFSET_SECONDS))).naive_utc()
    }

    async fn resource(
        &self,
        name: &str,
        kind: ResourceType,
        at: i64,
        employee: Option<i64>,
        role: Option<&str>,
    ) -> Result<i64> {
        let mut resource = Resource::new(name, kind).at(&branch(at));
        if let Some(employee) = employee {
            resource = resource.employee(employee);
        }
        if let Some(role) = role {
            resource = resource.role(role);
        }
        resource.save(self.db()).await?;
        for weekday in 1..=7 {
            AvailabilityRule::new(resource.id, weekday, OPEN.0, OPEN.1)
                .save(self.db())
                .await?;
        }
        Ok(resource.id)
    }

    async fn service_type(&self, code: &str) -> Result<ServiceType> {
        ServiceType::query()
            .where_(ServiceType::CODE.eq(code))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("the service type {code} is not seeded").into())
    }

    /// A service type of the clinic added to the seeded ones: it checks the
    /// patient in, tracks the visit and needs the licence and the privileges
    /// like they do.
    async fn extra_service(
        &self,
        code: &str,
        name: &str,
        minutes: i64,
        needs: &str,
    ) -> Result<i64> {
        let mut service = ServiceType::new_in(&self.state, code, name, minutes);
        service.check_in_required = true;
        service.track_in_progress = true;
        service.required_qualifications = REQUIRED_QUALIFICATIONS.to_owned();
        service.required_types = needs.to_owned();
        service.cancel_cutoff_hours = 2;
        service.reschedule_cutoff_hours = 2;
        service.save(self.db()).await?;
        Ok(service.id)
    }

    fn employee_of(&self, email: &str) -> i64 {
        self.employees
            .iter()
            .find(|(known, _)| *known == email)
            .map(|(_, employee)| employee.0)
            .unwrap_or_default()
    }

    /// The practitioners, rooms and equipment, and the service types.
    pub(crate) async fn desk_setup(&mut self) -> Result {
        let mut kit = Kit {
            nadia: self
                .resource(
                    "Dr Nadia Rahman",
                    ResourceType::Person,
                    JUMEIRAH,
                    Some(self.employee_of(NADIA_LOGIN)),
                    Some("practitioner"),
                )
                .await?,
            yara: self
                .resource(
                    "Dr Yara Kassab",
                    ResourceType::Person,
                    JUMEIRAH,
                    Some(self.employee_of(YARA_LOGIN)),
                    Some("practitioner"),
                )
                .await?,
            samir: self
                .resource(
                    "Dr Samir Joshi",
                    ResourceType::Person,
                    BUSINESS_BAY,
                    Some(self.employee_of(SAMIR_LOGIN)),
                    Some("practitioner"),
                )
                .await?,
            room_physio: self
                .resource("Physio studio", ResourceType::Room, JUMEIRAH, None, None)
                .await?,
            room_dental: self
                .resource(
                    "Dental surgery 1",
                    ResourceType::Room,
                    BUSINESS_BAY,
                    None,
                    None,
                )
                .await?,
            xray: self
                .resource(
                    "Dental X-ray unit",
                    ResourceType::Equipment,
                    BUSINESS_BAY,
                    None,
                    None,
                )
                .await?,
            ..Kit::default()
        };
        // The seeded service types: Dr Nadia delivers all three. A procedure
        // takes a deposit (AED 200), refunded unless the patient cancels
        // late or does not come.
        for code in ["consultation", "follow_up", "procedure"] {
            let mut service = self.service_type(code).await?;
            if code == "procedure" {
                service.deposit_amount = 20_000;
                service.save(self.db()).await?;
                kit.procedure = service.id;
            } else if code == "consultation" {
                kit.consultation = service.id;
            } else {
                kit.follow_up = service.id;
            }
            resource::offer(self.db(), service.id, kit.nadia).await?;
        }
        resource::offer(self.db(), kit.follow_up, kit.yara).await?;
        kit.dental = self
            .extra_service("dental_checkup", "Dental check-up", 30, "person,room")
            .await?;
        resource::offer(self.db(), kit.dental, kit.samir).await?;
        resource::offer(self.db(), kit.dental, kit.room_dental).await?;
        kit.physio = self
            .extra_service("physio_session", "Physiotherapy session", 45, "person,room")
            .await?;
        resource::offer(self.db(), kit.physio, kit.yara).await?;
        resource::offer(self.db(), kit.physio, kit.room_physio).await?;
        kit.dental_xray = self
            .extra_service("dental_xray", "Dental X-ray", 15, "person,equipment")
            .await?;
        resource::offer(self.db(), kit.dental_xray, kit.samir).await?;
        resource::offer(self.db(), kit.dental_xray, kit.xray).await?;
        // A consulting room and an ultrasound unit complete the Jumeirah floor.
        self.resource(
            "Consultation room 1",
            ResourceType::Room,
            JUMEIRAH,
            None,
            None,
        )
        .await?;
        self.resource(
            "Ultrasound unit",
            ResourceType::Equipment,
            JUMEIRAH,
            None,
            None,
        )
        .await?;
        self.kit = kit;
        Ok(())
    }

    /// The start of `story` as a wall time.
    fn start_of(&self, story: &Story) -> NaiveDateTime {
        match story.when {
            When::Day(n, hour, minute) => self.local(n, hour, minute),
            When::Minutes(minutes) => {
                // Rounded to the minute, so the booking is the same minute.
                let at = renox::db::now() + Duration::minutes(minutes);
                let wall = self.wall(at);
                wall.date()
                    .and_hms_opt(wall.hour(), wall.minute(), 0)
                    .unwrap_or(wall)
            }
        }
    }

    /// Makes the booking of `story`: a day or two before the visit for the
    /// past and today, now for the future.
    async fn book(&self, story: &Story) -> Result<Booking> {
        let start = self.utc(self.start_of(story));
        let now = renox::db::now();
        let at = if start - Duration::days(1) > now {
            now
        } else {
            start - Duration::days(2)
        };
        let desk = self.user(story.desk).id;
        let booked = service::book(
            &self.state,
            NewBooking::new(
                self.patients[story.patient],
                story.service,
                &story.resources,
                start,
            )
            .for_subject(self.patients[story.patient].subject())
            .at(branch(story.branch)),
            Actor::staff(desk),
            at,
        )
        .await;
        let booking = self
            .refused(
                &format!("the booking of patient {} at {start}", story.patient),
                booked,
            )
            .await?;
        // The desk took the deposit of the service on the card terminal; the
        // payment is recorded against the booking, so a refund can follow it.
        let deposit = ServiceType::find_or_404(self.db(), story.service)
            .await?
            .deposit_amount;
        if deposit > 0 {
            let payment = RecordPayment::new(
                self.patients[story.patient],
                self.entity,
                PaymentMethod::Card,
                deposit,
            )
            .reference(&format!("POS-{}", booking.number))
            .prepaid(booking.subject())
            .collected_by(desk);
            payment_recorder(&self.state)
                .record(&self.state, payment, Some(desk))
                .await?;
        }
        Ok(booking)
    }

    async fn refused<T>(
        &self,
        what: &str,
        result: std::result::Result<T, wl_scheduling::Refusal>,
    ) -> Result<T> {
        result.map_err(|refusal| {
            renox::anyhow::anyhow!(
                "demo seed: {what} was refused: {}",
                refusal.message(&self.state.lang("en"))
            )
            .into()
        })
    }

    /// Attaches `policy` to the appointment of booking `id`.
    async fn attach(&self, id: i64, policy: i64, desk: &str) -> Result {
        self.driver
            .post(
                desk,
                &format!("/clinics/appointments/{id}/policy"),
                &[("policy", &policy.to_string())],
            )
            .await?;
        Ok(())
    }

    /// Takes one appointment through its story.
    pub(crate) async fn story_of(&self, story: &Story) -> Result<Booking> {
        let booking = self.book(story).await?;
        let id = booking.id;
        let actor = Actor::staff(self.user(story.desk).id);
        let start = self.start_of(story);
        let state = &self.state;
        let policy = if story.insured {
            self.policy_of(story.patient)
        } else {
            None
        };
        match story.end {
            End::Booked | End::Confirmed => {
                if let Some(policy) = policy {
                    self.attach(id, policy, story.desk).await?;
                }
                if story.end == End::Confirmed {
                    let done =
                        service::confirm(state, id, Actor::party(None), renox::db::now()).await;
                    self.refused("a confirmation", done).await?;
                }
                return Booking::find_or_404(self.db(), id).await;
            }
            End::CancelledLate | End::CancelledInTime | End::CancelledByClinic => {
                let (request, when) = match story.end {
                    End::CancelledLate => (
                        CancelRequest::new("party_request"),
                        start - Duration::hours(3),
                    ),
                    End::CancelledInTime => (
                        CancelRequest::new("party_request"),
                        start - Duration::days(2),
                    ),
                    _ => (
                        CancelRequest::new("equipment_failure")
                            .note("The ultrasound unit was out of service for the day."),
                        start - Duration::days(1),
                    ),
                };
                let at = self.utc(when);
                let cancelled = self
                    .clock
                    .at_time(when, async {
                        service::cancel(state, id, request, actor, at).await
                    })
                    .await;
                self.refused("a cancellation", cancelled).await?;
                return Booking::find_or_404(self.db(), id).await;
            }
            End::NoShow => {
                let when = start + Duration::minutes(30);
                let at = self.utc(when);
                let marked = self
                    .clock
                    .at_time(when, async {
                        service::mark_no_show(state, id, actor, at).await
                    })
                    .await;
                self.refused("a no-show", marked).await?;
                return Booking::find_or_404(self.db(), id).await;
            }
            End::CheckedIn | End::InProgress | End::Completed => {}
        }
        // The patient arrives: coverage first, then the check-in.
        let arrives = start - Duration::minutes(10);
        self.clock
            .at_time(arrives, async {
                if let Some(policy) = policy {
                    self.attach(id, policy, story.desk).await?;
                    self.driver
                        .post(
                            story.desk,
                            &format!("/clinics/appointments/{id}/eligibility"),
                            &[
                                ("result", "positive"),
                                ("reference", &format!("ELG-{id:05}")),
                            ],
                        )
                        .await?;
                }
                let done = service::check_in(state, id, actor, self.utc(arrives)).await;
                self.refused("a check-in", done).await.map(|_| ())
            })
            .await?;
        if story.end == End::CheckedIn {
            return Booking::find_or_404(self.db(), id).await;
        }
        let started = start + Duration::minutes(5);
        self.clock
            .at_time(started, async {
                let done = service::start(state, id, actor, self.utc(started)).await;
                self.refused("a start", done).await.map(|_| ())
            })
            .await?;
        if story.end == End::InProgress {
            return Booking::find_or_404(self.db(), id).await;
        }
        let over = start + Duration::minutes(25);
        self.clock
            .at_time(over, async {
                let visit = clinics::Visit::of_booking(self.db(), id)
                    .await?
                    .ok_or_else(|| {
                        renox::anyhow::anyhow!("demo seed: no visit for booking {id}")
                    })?;
                if story.lines.is_empty() {
                    self.driver
                        .post(
                            story.doctor,
                            &format!("/clinics/visits/{}/none", visit.id),
                            &[],
                        )
                        .await?;
                }
                for (line, units, consumable) in &story.lines {
                    // `CODE@LOT` names the lot of a tracked consumable.
                    let (code, lot) = line.split_once('@').unwrap_or((line, ""));
                    let kind = if *consumable { "consumable" } else { "service" };
                    let who = if *consumable {
                        story.desk
                    } else {
                        story.doctor
                    };
                    self.driver
                        .post(
                            who,
                            &format!("/clinics/visits/{}/charges", visit.id),
                            &[
                                ("code", code),
                                ("units", &units.to_string()),
                                ("kind", kind),
                                ("lot", lot),
                            ],
                        )
                        .await?;
                }
                let done = service::complete(state, id, actor, self.utc(over)).await;
                self.refused("a completion", done).await.map(|_| ())
            })
            .await?;
        Booking::find_or_404(self.db(), id).await
    }

    /// The appointments of the clinic, in their states.
    pub(crate) async fn desk(&mut self) -> Result {
        let k = self.kit;
        let consult = |patient, when, insured, end, lines: Vec<(&'static str, i64, bool)>| Story {
            patient,
            service: k.consultation,
            resources: vec![k.nadia],
            branch: JUMEIRAH,
            when,
            insured,
            end,
            lines,
            doctor: NADIA_LOGIN,
            desk: JUMEIRAH_DESK,
        };
        let mut stories = vec![
            // The past: completed and billed visits (the claims follow).
            consult(
                AHMED,
                When::Day(-12, 9, 0),
                true,
                End::Completed,
                vec![("consultation", 1, false), ("GLOVES-M", 1, true)],
            ),
            consult(
                PRIYA,
                When::Day(-10, 10, 30),
                true,
                End::Completed,
                vec![("consultation", 1, false)],
            ),
            consult(
                LAYAN,
                When::Day(-24, 11, 0),
                true,
                End::Completed,
                vec![("consultation", 1, false)],
            ),
            consult(
                MOHAMMED,
                When::Day(-5, 9, 30),
                false,
                End::Completed,
                vec![],
            ),
            consult(
                YOUSEF,
                When::Day(-3, 10, 0),
                true,
                End::Completed,
                vec![("consultation", 1, false)],
            ),
            Story {
                service: k.follow_up,
                ..consult(
                    KHALID,
                    When::Day(-9, 15, 0),
                    true,
                    End::Completed,
                    vec![("follow_up", 1, false), ("SYRINGE-5ML", 1, true)],
                )
            },
            Story {
                service: k.dental,
                resources: vec![k.samir, k.room_dental],
                branch: BUSINESS_BAY,
                doctor: SAMIR_LOGIN,
                desk: BAY_DESK,
                ..consult(
                    FATIMA,
                    When::Day(-11, 11, 0),
                    true,
                    End::Completed,
                    vec![
                        ("dental_checkup", 1, false),
                        ("ANAES-CART@L-A2199", 1, true),
                    ],
                )
            },
            Story {
                service: k.dental_xray,
                resources: vec![k.samir, k.xray],
                branch: BUSINESS_BAY,
                doctor: SAMIR_LOGIN,
                desk: BAY_DESK,
                ..consult(
                    FATIMA,
                    When::Day(-2, 16, 0),
                    true,
                    End::Completed,
                    vec![("dental_xray", 1, false)],
                )
            },
            Story {
                service: k.physio,
                resources: vec![k.yara, k.room_physio],
                doctor: YARA_LOGIN,
                ..consult(SOFIA, When::Day(-8, 14, 0), false, End::Completed, vec![])
            },
            // Cancelled and no-show.
            Story {
                service: k.dental,
                resources: vec![k.samir, k.room_dental],
                branch: BUSINESS_BAY,
                doctor: SAMIR_LOGIN,
                desk: BAY_DESK,
                ..consult(DANIEL, When::Day(-4, 11, 0), false, End::NoShow, vec![])
            },
            Story {
                service: k.procedure,
                ..consult(
                    ELENA,
                    When::Day(-3, 13, 0),
                    false,
                    End::CancelledLate,
                    vec![],
                )
            },
            Story {
                service: k.procedure,
                ..consult(
                    JOHN,
                    When::Day(-2, 12, 0),
                    false,
                    End::CancelledByClinic,
                    vec![],
                )
            },
            consult(
                MOHAMMED,
                When::Day(-1, 9, 0),
                false,
                End::CancelledInTime,
                vec![],
            ),
            // Today.
            consult(PRIYA, When::Minutes(-25), true, End::CheckedIn, vec![]),
            Story {
                service: k.follow_up,
                resources: vec![k.yara],
                doctor: YARA_LOGIN,
                ..consult(AHMED, When::Minutes(-40), true, End::InProgress, vec![])
            },
            consult(FATIMA, When::Minutes(90), true, End::Confirmed, vec![]),
            Story {
                service: k.physio,
                resources: vec![k.yara, k.room_physio],
                doctor: YARA_LOGIN,
                ..consult(SOFIA, When::Minutes(180), false, End::Booked, vec![])
            },
            Story {
                service: k.dental,
                resources: vec![k.samir, k.room_dental],
                branch: BUSINESS_BAY,
                doctor: SAMIR_LOGIN,
                desk: BAY_DESK,
                ..consult(JOHN, When::Minutes(120), false, End::Booked, vec![])
            },
            Story {
                service: k.dental_xray,
                resources: vec![k.samir, k.xray],
                branch: BUSINESS_BAY,
                doctor: SAMIR_LOGIN,
                desk: BAY_DESK,
                ..consult(LAYAN, When::Minutes(240), true, End::Booked, vec![])
            },
            // Tomorrow and after.
            Story {
                service: k.dental,
                resources: vec![k.samir, k.room_dental],
                branch: BUSINESS_BAY,
                doctor: SAMIR_LOGIN,
                desk: BAY_DESK,
                ..consult(PRIYA, When::Day(1, 10, 0), true, End::Booked, vec![])
            },
            Story {
                service: k.procedure,
                ..consult(KHALID, When::Day(1, 11, 0), true, End::Booked, vec![])
            },
            Story {
                service: k.procedure,
                ..consult(PRIYA, When::Day(2, 10, 0), true, End::Booked, vec![])
            },
            consult(ELENA, When::Day(2, 15, 0), false, End::Booked, vec![]),
            Story {
                service: k.physio,
                resources: vec![k.yara, k.room_physio],
                doctor: YARA_LOGIN,
                ..consult(SOFIA, When::Day(3, 9, 0), false, End::Booked, vec![])
            },
            consult(DANIEL, When::Day(3, 12, 0), false, End::Booked, vec![]),
            Story {
                service: k.follow_up,
                ..consult(YOUSEF, When::Day(4, 10, 0), true, End::Booked, vec![])
            },
            Story {
                service: k.dental_xray,
                resources: vec![k.samir, k.xray],
                branch: BUSINESS_BAY,
                doctor: SAMIR_LOGIN,
                desk: BAY_DESK,
                ..consult(JOHN, When::Day(5, 11, 0), false, End::Booked, vec![])
            },
            consult(AHMED, When::Day(7, 9, 30), true, End::Confirmed, vec![]),
            Story {
                service: k.procedure,
                ..consult(FATIMA, When::Day(8, 14, 0), true, End::Booked, vec![])
            },
            Story {
                service: k.dental,
                resources: vec![k.samir, k.room_dental],
                branch: BUSINESS_BAY,
                doctor: SAMIR_LOGIN,
                desk: BAY_DESK,
                ..consult(PRIYA, When::Day(10, 10, 0), true, End::Booked, vec![])
            },
            // Dr Yara's licence ends in 20 days: this is the last day she can
            // be booked on.
            Story {
                service: k.physio,
                resources: vec![k.yara, k.room_physio],
                doctor: YARA_LOGIN,
                ..consult(SOFIA, When::Day(19, 9, 0), false, End::Booked, vec![])
            },
        ];
        // Oldest first, so the numbers and the days of the history agree.
        stories.sort_by_key(|story| self.start_of(story));
        for story in &stories {
            let booking = self.story_of(story).await?;
            self.bookings.push((story.patient, booking.id, story.end));
        }
        // The waiting list: Daniel waits for a consultation with Dr Nadia in
        // the next two days.
        let from = self.utc(self.local(1, 8, 0));
        let until = self.utc(self.local(2, 20, 0));
        let joined = waiting::join(
            &self.state,
            NewEntry::new(self.patients[DANIEL], k.consultation)
                .with(k.nadia)
                .between(from, until),
        )
        .await;
        self.refused("the waiting-list entry", joined).await?;
        Ok(())
    }
}
