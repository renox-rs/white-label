//! Booting the Manpower vertical in a `TestApp` with auth and the kernel, and
//! fakes for what the vertical reaches through the kernel contracts: the party
//! directory, the rate cards, the workforce, the notifier and the portal. A
//! test cannot build an `AuthUser`, so a request is made through the
//! vertical's own routes as the user.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use manpower::permissions as perm;
use manpower::{
    ClientContract, ContractActivated, ContractStatus, Manpower, ManpowerRequest, RequestStatus,
    Site,
};
use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, provide_directory};
use wl_kernel::portal::{PortalFuture, PortalUser, PortalUsers, provide_portal_users};
use wl_kernel::rate_cards::{RateCardFuture, RateCardVersion, RateCards, provide_rate_cards};
use wl_kernel::{BusinessCalendar, PartyId, PartyKind, PartyStatus, PartySummary, Subject};

/// A manpower agency with the parties module standing in for the directory.
const CLIENT: &str = r#"
[client]
name = "Gulf Manpower Services LLC"
app = "manpower"

[modules]
enabled = ["parties"]

[[legal_entities]]
code = "GMS"
name_en = "Gulf Manpower Services LLC"
default = true
"#;

/// The first client company.
pub const TOWER: PartyId = PartyId(31);

/// The second client company.
pub const FOODS: PartyId = PartyId(32);

/// A date.
pub fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// The Asia/Dubai wall-clock time `hour:minute` on `date`.
pub fn dubai(date: NaiveDate, hour: u32, minute: u32) -> DateTime {
    BusinessCalendar::default().from_local(date.and_hms_opt(hour, minute, 0).unwrap())
}

/// An event log.
#[derive(Clone)]
pub struct Recorded<E>(pub Arc<Mutex<Vec<E>>>);

impl<E: Clone> Recorded<E> {
    /// What was recorded, oldest first.
    pub fn all(&self) -> Vec<E> {
        self.0.lock().unwrap().clone()
    }
}

impl<E> Default for Recorded<E> {
    fn default() -> Self {
        Recorded(Arc::new(Mutex::new(Vec::new())))
    }
}

/// Records `event`s of type `E` into `log`.
fn record<E: Event + Clone + Send + Sync + 'static>(app: &mut Registry, log: &Recorded<E>) {
    let log = log.0.clone();
    app.listen(move |event: E, _state: AppState| {
        let log = log.clone();
        async move {
            log.lock().unwrap().push(event);
            Ok(())
        }
    });
}

/// The client companies the party directory knows.
#[derive(Clone, Default)]
pub struct Parties(Arc<Mutex<BTreeMap<i64, String>>>);

impl Parties {
    /// Knows the party `id` as `name`.
    pub fn know(&self, id: PartyId, name: &str) {
        self.0.lock().unwrap().insert(id.0, name.to_owned());
    }
}

impl PartyDirectory for Parties {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&party.0)
                .map(|name| PartySummary {
                    id: party,
                    code: format!("P-{:05}", party.0),
                    kind: PartyKind::Company,
                    display_name: name.clone(),
                    status: PartyStatus::Active,
                    display_block_reason: None,
                }))
        })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        _code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().get(&party.0).cloned()) })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .contains_key(&party.0)
                .then_some(PartyStatus::Active))
        })
    }
}

/// The portal grants: who acts for which client company, with which roles.
#[derive(Clone, Default)]
pub struct Portal(Arc<Mutex<Vec<PortalUser>>>);

impl Portal {
    /// `user` acts for `party` with the portal `roles`.
    pub fn grant(&self, user: &User, party: PartyId, roles: &[&str]) {
        self.0.lock().unwrap().push(PortalUser {
            user_id: user.id,
            person: PartyId(9_000 + user.id),
            acting_for: party,
            roles: roles.iter().map(|r| (*r).to_owned()).collect(),
        });
    }
}

impl PortalUsers for Portal {
    fn users_for<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _on: NaiveDate,
    ) -> PortalFuture<'a, Vec<PortalUser>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|user| user.acting_for == party)
                .cloned()
                .collect())
        })
    }

    fn acting_for<'a>(
        &'a self,
        _db: &'a Db,
        user: i64,
        party: PartyId,
        _on: NaiveDate,
    ) -> PortalFuture<'a, Option<PortalUser>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|grant| grant.user_id == user && grant.acting_for == party)
                .cloned())
        })
    }
}

/// An accepted rate card version and the roles and shifts it prices.
#[derive(Clone)]
struct Version {
    source: Subject,
    effective_from: NaiveDate,
    lines: Vec<(String, String)>,
}

/// The rate cards of wl-invoicing, as the vertical asks for them.
#[derive(Clone, Default)]
pub struct Cards(Arc<Mutex<Vec<Version>>>);

impl Cards {
    /// The client accepted a version of the card of `source` effective from
    /// `effective_from`, pricing `lines` (role, shift type).
    pub fn accept(&self, source: &Subject, effective_from: NaiveDate, lines: &[(&str, &str)]) {
        self.0.lock().unwrap().push(Version {
            source: source.clone(),
            effective_from,
            lines: lines
                .iter()
                .map(|(role, shift)| ((*role).to_owned(), (*shift).to_owned()))
                .collect(),
        });
    }

    /// The accepted version in force on `on`: the latest effective on or
    /// before it.
    fn in_force(&self, source: &Subject, on: NaiveDate) -> Option<Version> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v.source == *source && v.effective_from <= on)
            .max_by_key(|v| v.effective_from)
            .cloned()
    }
}

impl RateCards for Cards {
    fn accepted_version<'a>(
        &'a self,
        _state: &'a AppState,
        source: &'a Subject,
        on: NaiveDate,
    ) -> RateCardFuture<'a, Option<RateCardVersion>> {
        Box::pin(async move {
            Ok(self.in_force(source, on).map(|v| RateCardVersion {
                rate_card: source.subject_id,
                version: 1,
                effective_from: v.effective_from,
            }))
        })
    }

    fn covers<'a>(
        &'a self,
        _state: &'a AppState,
        source: &'a Subject,
        role: &'a str,
        shift: &'a str,
        on: NaiveDate,
    ) -> RateCardFuture<'a, bool> {
        Box::pin(async move {
            Ok(self.in_force(source, on).is_some_and(|v| {
                v.lines
                    .iter()
                    .any(|(r, s)| r.eq_ignore_ascii_case(role) && s.eq_ignore_ascii_case(shift))
            }))
        })
    }
}

/// A notifier that records every request and queues nothing.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

impl Recording {
    /// Every request so far, oldest first.
    pub fn all(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }

    /// The requests of the type `key`.
    pub fn of_type(&self, key: &str) -> Vec<NotifyRequest> {
        self.all()
            .into_iter()
            .filter(|request| request.type_key == key)
            .collect()
    }
}

/// The users a request is for.
pub fn users_of(request: &NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|recipient| match recipient {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        self.0.lock().unwrap().push(request);
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Provides the fakes.
struct Fakes {
    parties: Parties,
    cards: Cards,
    portal: Portal,
    notices: Recording,
    activated: Recorded<ContractActivated>,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        provide_directory(app, self.parties.clone());
        provide_rate_cards(app, self.cards.clone());
        provide_portal_users(app, self.portal.clone());
        provide_notifier(app, self.notices.clone());
        record(app, &self.activated);
    }
}

/// The app under test with the fakes.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// The party directory.
    pub parties: Parties,
    /// The rate cards.
    pub cards: Cards,
    /// The portal grants.
    pub portal: Portal,
    /// What the notifier was asked for.
    pub notices: Recording,
    /// `ContractActivated`.
    pub activated: Recorded<ContractActivated>,
    /// Days the clock was moved forward from the real one.
    clock_days: std::sync::atomic::AtomicI64,
    /// The contracts made, for their references.
    made: std::sync::atomic::AtomicI64,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
}

impl Booted {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// A user with no role.
    pub async fn user(&self, name: &str) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        User::register(self.db(), name, &email, "password123")
            .await
            .unwrap()
    }

    /// A user holding the role `role` everywhere.
    pub async fn user_with(&self, name: &str, role: &str) -> User {
        let user = self.user(name).await;
        user.assign_role(self.db(), role).await.unwrap();
        user
    }

    /// An account manager of the client company `client` only.
    pub async fn account_manager(&self, name: &str, client: PartyId) -> User {
        let user = self.user(name).await;
        user.assign_role_in(
            self.db(),
            perm::ACCOUNT_MANAGER_ROLE,
            &perm::client_scope(client.0),
        )
        .await
        .unwrap();
        user
    }

    /// The MD.
    pub async fn md(&self, name: &str) -> User {
        self.user_with(name, perm::MD_ROLE).await
    }

    /// Runs `work` with the clock on `day` (Asia/Dubai) at midday, moved
    /// forward from the real one; days only go forward within a test.
    pub async fn on<F: std::future::Future>(&self, day: NaiveDate, work: F) -> F::Output {
        use std::sync::atomic::Ordering;
        let real = wl_kernel::documents::dubai_date(renox::db::now());
        let wanted = (day - real).num_days();
        let moved = self.clock_days.load(Ordering::SeqCst);
        assert!(wanted >= moved, "the clock cannot go back to {day}");
        if wanted > moved {
            let seconds = u64::try_from(wanted - moved).unwrap() * 24 * 3600;
            self.app.travel(std::time::Duration::from_secs(seconds));
            self.clock_days.store(wanted, Ordering::SeqCst);
        }
        self.app.at_travelled_time(work).await
    }

    /// A contract of `client` in `status`, from 1 January to 31 December
    /// 2027, inserted as a fixture, managed by `manager`.
    pub async fn contract(
        &self,
        client: PartyId,
        status: ContractStatus,
        manager: Option<&User>,
    ) -> ClientContract {
        let n = self.made.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let mut contract = ClientContract {
            reference: format!("CC-{n:04}"),
            client_party_id: client.0,
            start_date: date(2027, 1, 1),
            end_date: date(2027, 12, 31),
            payment_terms_days: 30,
            notice_period_days: 30,
            account_manager_user_id: manager.map(|user| user.id),
            status,
            ..ClientContract::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        contract
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        contract
    }

    /// A client user of the portal who acts for `party` with the portal
    /// `roles` (`owner`, `approver`, `viewer`...).
    pub async fn client_user(&self, name: &str, party: PartyId, roles: &[&str]) -> User {
        let user = self.user(name).await;
        self.portal.grant(&user, party, roles);
        user
    }

    /// A site of `client`, in use.
    pub async fn site(&self, client: PartyId, name: &str) -> Site {
        Site::create(
            self.db(),
            Site {
                client_party_id: client.0,
                name: name.to_owned(),
                active: true,
                ..Site::default()
            },
        )
        .await
        .unwrap()
    }

    /// A request for `headcount` guards on the day shift at `site`, from 1
    /// February 2027 to 31 July 2027, in `status` (a fixture).
    pub async fn request(
        &self,
        contract: &ClientContract,
        site: &Site,
        headcount: i64,
        status: RequestStatus,
    ) -> ManpowerRequest {
        let mut request = ManpowerRequest {
            client_contract_id: contract.id,
            client_party_id: contract.client_party_id,
            site_id: site.id,
            role: "guard".into(),
            headcount,
            shift_type: "day".into(),
            shift_start: Some("07:00".into()),
            shift_end: Some("19:00".into()),
            start_date: date(2027, 2, 1),
            end_date: Some(date(2027, 7, 31)),
            status,
            ..ManpowerRequest::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        request
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        request
    }

    /// The request `id` as it is now.
    pub async fn reload_request(&self, id: i64) -> ManpowerRequest {
        ManpowerRequest::find_or_404(self.db(), id).await.unwrap()
    }

    /// The contract `id` as it is now.
    pub async fn reload_contract(&self, id: i64) -> ClientContract {
        ClientContract::find_or_404(self.db(), id).await.unwrap()
    }

    /// A POST as `by`.
    pub async fn post(&self, by: &User, path: &str, form: &[(&str, &str)]) -> TestResponse {
        self.app.acting_as(by);
        self.app.htmx().post(path, form).await
    }

    /// A GET as `by`.
    pub async fn get(&self, by: &User, path: &str) -> TestResponse {
        self.app.acting_as(by);
        self.app.get(path).await
    }

    /// A GET as `by` the way a browser sends it.
    pub async fn browse(&self, by: &User, path: &str) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .request()
            .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
            .get(path)
            .await
    }
}

/// Boots the app.
pub async fn boot() -> Booted {
    let parties = Parties::default();
    parties.know(TOWER, "Gulf Tower LLC");
    parties.know(FOODS, "Desert Foods LLC");
    let cards = Cards::default();
    let portal = Portal::default();
    let notices = Recording::default();
    let activated = Recorded::default();
    let fakes = Fakes {
        parties: parties.clone(),
        cards: cards.clone(),
        portal: portal.clone(),
        notices: notices.clone(),
        activated: activated.clone(),
    };
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(Manpower)
        .module(fakes);
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    // The pages extend the app's layout; a vertical's test has no host, so it
    // brings the smallest one.
    let views = tempfile::tempdir().expect("a temporary views directory");
    let layout = views.path().join("layouts/app.html");
    std::fs::create_dir_all(layout.parent().unwrap()).unwrap();
    std::fs::write(
        &layout,
        "<!doctype html>\n<html><body><rx-outlet name=\"content\"/></body></html>\n",
    )
    .unwrap();
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    // The account manager runs the client accounts; the MD only sees them.
    permissions::define_role(
        app.db(),
        perm::ACCOUNT_MANAGER_ROLE,
        &[
            perm::CONTRACTS_VIEW,
            perm::CONTRACTS_MANAGE,
            perm::SITES_MANAGE,
            perm::REQUESTS_VIEW,
            perm::REQUESTS_CREATE,
            perm::REQUESTS_MANAGE,
            perm::DEPLOYMENTS_VIEW,
            perm::DEPLOYMENTS_MANAGE,
        ],
    )
    .await
    .unwrap();
    permissions::define_role(app.db(), perm::MD_ROLE, &[perm::CONTRACTS_VIEW])
        .await
        .unwrap();
    permissions::define_role(app.db(), perm::SUPERVISOR_ROLE, &[perm::DEPLOYMENTS_VIEW])
        .await
        .unwrap();
    Booted {
        app,
        parties,
        cards,
        portal,
        notices,
        activated,
        clock_days: std::sync::atomic::AtomicI64::new(0),
        made: std::sync::atomic::AtomicI64::new(0),
        _views: views,
    }
}
