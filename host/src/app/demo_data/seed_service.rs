//! Making the maintenance side: the ticket categories of the trades, the
//! vendors' and buildings' papers, the contracts, the planned visits and the
//! tickets.

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;
use wl_kernel::tickets::{Assignee, OpenTicket, Requester, TicketChannel};
use wl_kernel::{PartyId, Subject};
use wl_ticketing::{
    Category, ContractStatus, NewContract, Priority, ServiceContract, ServiceContracts, SlaPolicy,
    Ticket, TicketStatus, Tickets,
};

use super::people::login_of;
use super::roles::{ADMIN, COORDINATOR, TECHNICIAN};
use super::seed_leases::text;
use super::service::{CONTRACTS, ContractPlan, ContractStanding, Standing, TICKETS, TicketPlan, Worker};
use super::{Seed, day};

/// The trades the demo adds to the ticket categories: key, English name,
/// Arabic name.
const TRADES: &[(&str, &str, &str)] = &[
    ("ac_hvac", "AC and chillers", "التكييف والمبردات"),
    ("plumbing", "Plumbing", "السباكة"),
    ("electrical", "Electrical", "الكهرباء"),
    ("pest_control", "Pest control", "مكافحة الحشرات"),
    ("cleaning", "Cleaning", "النظافة"),
    ("security", "Security and access", "الأمن والدخول"),
];

impl Seed<'_> {
    /// The service side of the demo.
    pub(crate) async fn service(&mut self) -> Result {
        self.trades().await?;
        self.vendor_papers().await?;
        let contracts = self.contracts().await?;
        self.planned_visits(&contracts).await?;
        self.tickets().await?;
        self.compliance_rules().await
    }

    /// The categories of the trades, as the Property install makes the others.
    async fn trades(&mut self) -> Result {
        let policy = SlaPolicy::by_key(self.db(), wl_ticketing::seed::STANDARD_POLICY).await?;
        let mut made = 0;
        for (key, en, ar) in TRADES {
            if Category::by_key(self.db(), key).await?.is_some() {
                continue;
            }
            Category::create(
                self.db(),
                Category {
                    key: (*key).to_owned(),
                    name_en: (*en).to_owned(),
                    name_ar: (*ar).to_owned(),
                    default_priority: Priority::Normal,
                    sla_policy_id: policy.as_ref().map(|p| p.id),
                    active: true,
                    ..Category::default()
                },
            )
            .await?;
            made += 1;
        }
        self.report.add("ticket categories", made);
        Ok(())
    }

    /// The papers of vendors and buildings: the Civil Defence approval of the
    /// fire vendor, and the safety and lift certificates of the buildings,
    /// two of them close to their end.
    async fn vendor_papers(&mut self) -> Result {
        let fire = self.party("Safeguard Fire Systems LLC").clone();
        self.document(
            Subject::new(wl_kernel::parties::PARTY_SUBJECT_TYPE, fire.id),
            "civil_defence_approval",
            "Civil Defence approval",
            Some("DEMO-CD-0000351".to_owned()),
            Some(day(25)),
            Some(day(-340)),
        )
        .await?;
        for (code, safety, lift) in [
            ("JVT-A", 210, Some(140)),
            ("JVT-B", 12, Some(60)),
            ("BBY", 300, Some(35)),
            ("AQZ", 280, None),
            ("ADR", 150, None),
            ("SHJ", 95, None),
        ] {
            let building = self.building(code).clone();
            let subject = Subject::new(property::building::BUILDING_SUBJECT_TYPE, building.id);
            self.document(
                subject.clone(),
                "building_safety_certificate",
                "Building safety certificate",
                Some(format!("DEMO-BSC-{:05}", building.id * 113)),
                Some(day(safety)),
                Some(day(safety - 365)),
            )
            .await?;
            if let Some(lift) = lift {
                self.document(
                    subject,
                    "lift_certificate",
                    "Lift certificate",
                    Some(format!("DEMO-LFT-{:05}", building.id * 71)),
                    Some(day(lift)),
                    Some(day(lift - 365)),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// The contracts, each taken to its standing; their ids by title.
    async fn contracts(&mut self) -> Result<Vec<(&'static ContractPlan, i64)>> {
        let admin = login_of(ADMIN).email;
        let mut made = Vec::new();
        for plan in CONTRACTS {
            let vendor = PartyId(self.party(plan.vendor).id);
            let contract = ServiceContracts::create_draft(
                &self.state,
                NewContract {
                    vendor,
                    title: plan.title.to_owned(),
                    start_date: day(plan.start),
                    end_date: day(plan.end),
                    renewal_notice_days: 60,
                    sla_policy_id: None,
                },
            )
            .await?;
            for code in plan.buildings {
                let building = self.building(code).id;
                ServiceContracts::add_coverage(
                    self.db(),
                    &contract,
                    Subject::new(property::building::BUILDING_SUBJECT_TYPE, building),
                    plan.category,
                )
                .await?;
            }
            self.report.add("service contracts", 1);
            made.push((plan, contract.id));
            if plan.standing == ContractStanding::Draft {
                continue;
            }
            self.driver
                .post(admin, &format!("/service-contracts/{}/activate", contract.id), &[])
                .await?;
            let active = ServiceContract::find_or_404(self.db(), contract.id).await?;
            match plan.standing {
                ContractStanding::Expiring => {
                    ServiceContracts::flag_expiring(&self.state, &active, today())
                        .await
                        .map_err(renox::state_machine::TransitionError::into_error)?;
                }
                ContractStanding::Expired => {
                    ServiceContracts::flag_expiring(&self.state, &active, today())
                        .await
                        .map_err(renox::state_machine::TransitionError::into_error)?;
                    let expiring = ServiceContract::find_or_404(self.db(), contract.id).await?;
                    ServiceContracts::expire(&self.state, &expiring, today())
                        .await
                        .map_err(renox::state_machine::TransitionError::into_error)?;
                }
                ContractStanding::Terminated => {
                    self.driver
                        .post(
                            admin,
                            &format!("/service-contracts/{}/terminate", contract.id),
                            &[
                                ("reason", "The vendor missed three monthly visits"),
                                ("termination_date", text(today()).as_str()),
                            ],
                        )
                        .await?;
                }
                _ => {}
            }
        }
        let _ = ContractStatus::Active;
        Ok(made)
    }

    /// The planned visits of the fire and lift contracts: done with the
    /// vendor's service report, one coming, one missed.
    async fn planned_visits(&mut self, contracts: &[(&'static ContractPlan, i64)]) -> Result {
        let coordinator = login_of(COORDINATOR).email;
        for (plan, id) in contracts {
            if !matches!(plan.category, "fire_life_safety" | "lifts")
                || plan.standing == ContractStanding::Terminated
            {
                continue;
            }
            let vendor = PartyId(self.party(plan.vendor).id);
            for (index, code) in plan.buildings.iter().take(2).enumerate() {
                let building = self.building(code).id;
                let subject = Subject::new(property::building::BUILDING_SUBJECT_TYPE, building);
                // Two done quarters, the next one coming, and (for the first
                // building) the last one missed.
                for (quarter, due) in [(1, -190), (2, -95), (3, 14), (4, -6)] {
                    if quarter == 4 && !(index == 0 && plan.category == "fire_life_safety") {
                        continue;
                    }
                    let due_on = day(due);
                    let mut request = OpenTicket::new(
                        subject.clone(),
                        plan.category,
                        &format!("Planned visit, quarter {quarter}: {}", plan.title),
                    )
                    .planned_due(due_on)
                    .assign_to(Assignee::party(vendor));
                    request.contract = Some(wl_kernel::tickets::ServiceContractId(*id));
                    let opened = Tickets::open_at(
                        &self.state,
                        request,
                        renox::db::now() - Duration::days((-due).max(0)),
                    )
                    .await?;
                    self.report.add("planned visits", 1);
                    if due < -50 {
                        let ticket = format!("/tickets/{}", opened.id.0);
                        self.driver
                            .post(
                                coordinator,
                                &format!("{ticket}/service-report"),
                                &[("reference", &format!("SR-DEMO-{}-{quarter}", opened.id.0))],
                            )
                            .await?;
                        self.driver
                            .post(coordinator, &format!("{ticket}/actions/start"), &[])
                            .await?;
                        self.driver
                            .post(coordinator, &format!("{ticket}/actions/resolve"), &[])
                            .await?;
                        self.driver
                            .post(coordinator, &format!("{ticket}/actions/close"), &[])
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The tickets, each taken to its standing.
    async fn tickets(&mut self) -> Result {
        for plan in TICKETS {
            self.ticket(plan).await?;
        }
        Ok(())
    }

    async fn ticket(&mut self, plan: &TicketPlan) -> Result {
        let coordinator = login_of(COORDINATOR).email;
        let technician = login_of(TECHNICIAN);
        let subject = match plan.place.1 {
            Some(number) => Subject::new(
                property::unit::UNIT_SUBJECT_TYPE,
                self.unit(plan.place.0, number).id,
            ),
            None => Subject::new(
                property::building::BUILDING_SUBJECT_TYPE,
                self.building(plan.place.0).id,
            ),
        };
        let mut request = OpenTicket::new(subject, plan.category, plan.title).channel(
            if plan.tenant.is_some() {
                TicketChannel::Portal
            } else {
                TicketChannel::Staff
            },
        );
        if let Some(tenant) = plan.tenant {
            request = request
                .requester(Requester::Party {
                    id: PartyId(self.party(tenant).id),
                })
                .description(&format!("Reported by {tenant}."));
        }
        let (worker_user, assignee) = match plan.worker {
            Worker::Nobody => (coordinator, None),
            Worker::Vendor(name) => (
                coordinator,
                Some(Assignee::party(PartyId(self.party(name).id))),
            ),
            Worker::Technician => (
                technician.email,
                Some(Assignee::user(self.user_id(technician.email).await?)),
            ),
        };
        if let Some(assignee) = assignee {
            request = request.assign_to(assignee);
        }
        let at = renox::db::now() - Duration::days(plan.age);
        let opened = Tickets::open_at(&self.state, request, at).await?;
        self.report.add("tickets", 1);
        let path = format!("/tickets/{}", opened.id.0);
        let action = |name: &str| format!("{path}/actions/{name}");
        match plan.standing {
            Standing::Open | Standing::Assigned => {}
            Standing::Done => {
                Tickets::complete(&self.state, opened.id, "Work done and signed off by the building")
                    .await?;
            }
            Standing::InProgress => {
                self.driver.post(worker_user, &action("start"), &[]).await?;
            }
            Standing::OnHold(reason) => {
                self.driver.post(worker_user, &action("start"), &[]).await?;
                self.driver
                    .post(worker_user, &format!("{path}/hold"), &[("hold_reason", reason)])
                    .await?;
            }
            Standing::Resolved => {
                self.driver.post(worker_user, &action("start"), &[]).await?;
                self.driver.post(worker_user, &action("resolve"), &[]).await?;
            }
            Standing::Closed => {
                self.driver.post(worker_user, &action("start"), &[]).await?;
                self.driver.post(worker_user, &action("resolve"), &[]).await?;
                self.driver.post(coordinator, &action("close"), &[]).await?;
            }
            Standing::Charged => {
                self.driver
                    .post(
                        coordinator,
                        &format!("{path}/charges"),
                        &[
                            ("description", "Replacement part and labour"),
                            ("quantity", "1"),
                            ("unit_price", &format!("{}.00", 180 + plan.age * 5)),
                            ("vat_category", "standard"),
                        ],
                    )
                    .await?;
                self.driver.post(worker_user, &action("start"), &[]).await?;
                self.driver.post(worker_user, &action("resolve"), &[]).await?;
                self.driver.post(coordinator, &action("close"), &[]).await?;
            }
            Standing::Cancelled(reason) => {
                self.driver
                    .post(coordinator, &format!("{path}/cancel"), &[("reason", reason)])
                    .await?;
            }
        }
        Ok(())
    }

    /// The daily rules of the service side: planned visits that were not done
    /// within the grace days raise the compliance flag of their building.
    async fn compliance_rules(&mut self) -> Result {
        wl_ticketing::visit_check::run(&self.state, today()).await?;
        let _ = (NaiveDate::MIN, text, Ticket::default, TicketStatus::Open);
        Ok(())
    }
}
