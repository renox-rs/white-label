//! Service contracts in the ticketing harness: a world with the staff who
//! work with contracts, the categories of a property client (fire safety and
//! lifts are compliance-critical), the events the contract flows emit, and
//! helpers that make a contract and its planned visits.

#![allow(dead_code)]

use renox::auth::permissions;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::{
    Assignee, ComplianceFlagCleared, ComplianceFlagRaised, OpenTicket, ServiceContractActivated,
    ServiceContractEnded, VendorNoShow,
};
use wl_kernel::{PartyId, Subject};
use wl_ticketing::{
    Category, NewContract, Priority, ServiceContract, ServiceContracts, Ticket, Tickets,
    WlTicketing, permissions as perm,
};

use std::sync::Arc;

use wl_kernel::documents::DocumentGuard;

use super::documents::Documents;
use super::{Booted, CLIENT, Recorded, boot_app, dubai, record};

/// "Tower A lifts".
pub fn tower_a_lifts() -> Subject {
    Subject::new("property.building_system", 4)
}

/// "Tower A cleaning".
pub fn tower_a_cleaning() -> Subject {
    Subject::new("property.building_system", 5)
}

/// "Tower A fire alarm".
pub fn tower_a_fire_alarm() -> Subject {
    Subject::new("property.building_system", 6)
}

/// A calendar date written `2026-12-31`.
pub fn date(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// The app, its staff and the events of the contract flows.
pub struct World {
    /// The booted app.
    pub t: Booted,
    /// `ComplianceFlagRaised`.
    pub raised: Recorded<ComplianceFlagRaised>,
    /// `ComplianceFlagCleared`.
    pub cleared: Recorded<ComplianceFlagCleared>,
    /// `ServiceContractActivated`.
    pub activated: Recorded<ServiceContractActivated>,
    /// `ServiceContractEnded`.
    pub ended: Recorded<ServiceContractEnded>,
    /// `VendorNoShow`.
    pub no_shows: Recorded<VendorNoShow>,
    /// Manages contracts and categories, assigns and works tickets.
    pub admin: User,
    /// The maintenance coordinator.
    pub coordinator: User,
    /// The property manager in scope.
    pub manager: User,
    /// A staff technician, whom a visit can be reassigned to.
    pub imran: User,
}

/// The world with the default module settings (the property manager is the
/// manager in scope).
pub async fn world() -> World {
    world_with(WlTicketing::new().compliance_manager_role("property_manager")).await
}

/// The world with 01's Civil Defence approval guarding `resolve` and contract
/// `activate` for fire and life safety, and the table of the vendors'
/// approvals.
pub async fn world_with_documents() -> (World, Documents) {
    let documents = Documents::civil_defence();
    let guard: Arc<dyn DocumentGuard> = Arc::new(documents.clone());
    let world = world_customized(
        WlTicketing::new().compliance_manager_role("property_manager"),
        move |app| app.provide(guard),
    )
    .await;
    (world, documents)
}

/// The world with `module`.
pub async fn world_with(module: WlTicketing) -> World {
    world_customized(module, |app| app).await
}

/// The world with `module`, and `customize` adding to the app.
pub async fn world_customized(module: WlTicketing, customize: impl FnOnce(App) -> App) -> World {
    let raised = Recorded::new();
    let cleared = Recorded::new();
    let activated = Recorded::new();
    let ended = Recorded::new();
    let no_shows = Recorded::new();
    let (a, b, c, d, e) = (
        raised.clone(),
        cleared.clone(),
        activated.clone(),
        ended.clone(),
        no_shows.clone(),
    );
    let t = boot_app(CLIENT, module, move |app| {
        let app = record(record(record(record(record(app, &a), &b), &c), &d), &e);
        customize(app)
    })
    .await;
    permissions::define_role(
        t.db(),
        "coordinator",
        &[
            perm::VIEW,
            perm::ASSIGN,
            perm::WORK,
            perm::SERVICE_CONTRACTS_VIEW,
        ],
    )
    .await
    .unwrap();
    for role in ["property_manager", "admin"] {
        permissions::define_role(t.db(), role, &[perm::VIEW])
            .await
            .unwrap();
    }
    let staff = |name: &'static str, role: &'static str| {
        let t = &t;
        async move {
            let email = format!("{}@example.com", name.to_lowercase());
            let user = User::register(t.db(), name, &email, "password123")
                .await
                .unwrap();
            user.assign_role(t.db(), role).await.unwrap();
            user
        }
    };
    let coordinator = staff("Cora", "coordinator").await;
    let manager = staff("Pam", "property_manager").await;
    let admin = t
        .user(
            "Alia",
            &[
                perm::VIEW,
                perm::ASSIGN,
                perm::WORK,
                perm::MANAGE,
                perm::SERVICE_CONTRACTS_VIEW,
                perm::SERVICE_CONTRACTS_MANAGE,
                perm::MAINTENANCE_PLANS_MANAGE,
            ],
        )
        .await;
    let imran = t.technician("Imran").await;
    for (key, name, critical) in [
        ("lifts", "Lifts", true),
        ("fire_safety", "Fire and life safety", true),
        ("cleaning", "Cleaning", false),
        ("plumbing", "Plumbing", false),
    ] {
        Category::create(
            t.db(),
            Category {
                key: key.into(),
                name_en: name.into(),
                default_priority: Priority::Normal,
                compliance_critical: critical,
                active: true,
                ..Category::default()
            },
        )
        .await
        .unwrap();
    }
    World {
        t,
        raised,
        cleared,
        activated,
        ended,
        no_shows,
        admin,
        coordinator,
        manager,
        imran,
    }
}

impl World {
    /// A contract of `vendor` from `start` to `end` covering `covers`
    /// (subject and category key), activated by the admin through the page.
    pub async fn contract(
        &self,
        vendor: PartyId,
        start: &str,
        end: &str,
        covers: &[(Subject, &str)],
    ) -> ServiceContract {
        let draft = self.draft(vendor, start, end, covers).await;
        self.activate(&draft).await
    }

    /// A `draft` contract of `vendor` covering `covers`.
    pub async fn draft(
        &self,
        vendor: PartyId,
        start: &str,
        end: &str,
        covers: &[(Subject, &str)],
    ) -> ServiceContract {
        let contract = ServiceContracts::create_draft(
            self.t.state(),
            NewContract {
                vendor,
                title: "Annual maintenance".into(),
                start_date: date(start),
                end_date: date(end),
                renewal_notice_days: 60,
                sla_policy_id: None,
            },
        )
        .await
        .expect("the draft is made");
        for (subject, category) in covers {
            ServiceContracts::add_coverage(self.t.db(), &contract, subject.clone(), category)
                .await
                .expect("the coverage is added");
        }
        contract
    }

    /// The admin activates `contract` through its page.
    pub async fn activate(&self, contract: &ServiceContract) -> ServiceContract {
        self.try_activate(contract).await.assert_ok();
        self.reload(contract).await
    }

    /// The admin activates `contract` through its page; the response.
    pub async fn try_activate(&self, contract: &ServiceContract) -> renox::testing::TestResponse {
        self.t.app.acting_as(&self.admin);
        self.t
            .app
            .htmx()
            .post(
                &format!("/service-contracts/{}/activate", contract.id),
                &[("x", "y")],
            )
            .await
    }

    /// The admin terminates `contract` for `reason`, ending on `date`.
    pub async fn terminate(
        &self,
        contract: &ServiceContract,
        reason: &str,
        date: &str,
    ) -> renox::testing::TestResponse {
        self.t.app.acting_as(&self.admin);
        self.t
            .app
            .htmx()
            .post(
                &format!("/service-contracts/{}/terminate", contract.id),
                &[("reason", reason), ("termination_date", date)],
            )
            .await
    }

    /// `contract`, as stored now.
    pub async fn reload(&self, contract: &ServiceContract) -> ServiceContract {
        ServiceContract::find_or_404(self.t.db(), contract.id)
            .await
            .unwrap()
    }

    /// A planned visit of `contract` in `category` on `subject`, planned
    /// for `due`, assigned to `assignee` when there is one.
    pub async fn visit(
        &self,
        contract: &ServiceContract,
        category: &str,
        subject: Subject,
        due: &str,
        assignee: Option<Assignee>,
    ) -> Ticket {
        let mut request = OpenTicket::new(subject, category, "Planned visit")
            .planned_due(date(due))
            .under_contract(contract.contract_id());
        if let Some(assignee) = assignee {
            request = request.assign_to(assignee);
        }
        let opened = Tickets::open_at(self.t.state(), request, dubai("2026-10-25", 8, 0))
            .await
            .expect("the visit opens");
        self.t.ticket(opened.id.0).await
    }

    /// The coordinator starts `visit` (for its vendor, which has no login).
    pub async fn start(&self, visit: &Ticket) -> Ticket {
        self.t
            .run(&self.coordinator, visit, "start", "2026-12-05 09:00", &[])
            .await
            .assert_ok();
        self.t.reload(visit).await
    }

    /// How many notices of `type_key` went to the user `user`.
    pub fn told(&self, type_key: &str, user: &User) -> usize {
        self.t
            .notices
            .recipients(type_key)
            .into_iter()
            .filter(|to| matches!(to, NotifyRecipient::User { id } if *id == user.id))
            .count()
    }

    /// How many notices of `type_key` went to the party `party`.
    pub fn told_party(&self, type_key: &str, party: PartyId) -> usize {
        self.t
            .notices
            .recipients(type_key)
            .into_iter()
            .filter(|to| matches!(to, NotifyRecipient::Party { id } if *id == party))
            .count()
    }
}
