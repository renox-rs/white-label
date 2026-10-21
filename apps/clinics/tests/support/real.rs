//! The clinics app with the **real** wl-invoicing: the payer bills, the
//! remittance, the approval of an overpayment or a recoupment and the
//! reversal of a payment are the module's own; only the party directory and
//! the staff tasks are stand-ins (the tasks are recorded).
//!
//! No scheduling and no booking: a claim is made from a visit row and its
//! charge lines, which is what the billing of a completed visit leaves, and
//! the claim then runs through the clinics routes and services.

#![allow(dead_code)]

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use renox::auth::{Auth, Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::AuthUser;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_invoicing::{PayerAgreement, VatRecipient, WlInvoicing, permissions as inv};
use wl_kernel::parties::{
    BillingIdentity, PartyDirectory, PartyFuture, PartyKind, PartyStatus, PartySummary,
    provide_directory,
};
use wl_kernel::payer_bills::{NewPayerBill, NewPayerBillLine, payer_bills};
use wl_kernel::notify::provide_notifier;
use wl_kernel::tickets::provide_task_creator;
use wl_kernel::{LegalEntity, PartyId};

use clinics::contracts::{self, NewContract};
use clinics::visit_machine::CHECK_IN;
use clinics::{ChargeKind, ChargeLine, Claim, PayerContract, Visit, VisitStatus, claims};

use super::{Messages, PATIENT, Tasks, day};

/// The payer of the claims.
pub const PAYER: PartyId = PartyId(900);

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const CLIENT: &str = r#"
[client]
name = "Palm Grove Clinic LLC"
app = "clinics"

[modules]
enabled = ["parties", "invoicing"]

[roles]
owner = "owner"

[[legal_entities]]
code = "PGC"
name_en = "Palm Grove Clinic LLC"
trn = "100000000000003"
default = true
"#;

/// Every party exists and can be billed.
struct AnyParty;

impl PartyDirectory for AnyParty {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            Ok(Some(PartySummary {
                id: party,
                code: format!("P-{:05}", party.0),
                kind: PartyKind::Person,
                display_name: format!("Party {}", party.0),
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
        Box::pin(async move { Ok(Some(format!("Party {}", party.0))) })
    }

    fn status<'a>(&'a self, _db: &'a Db, _party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async { Ok(Some(PartyStatus::Active)) })
    }

    fn billing_identity<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<BillingIdentity>> {
        Box::pin(async move {
            Ok(Some(BillingIdentity {
                party,
                legal_name_en: format!("Party {}", party.0),
                legal_name_ar: None,
                trn: None,
                billing_address: Some("Dubai".to_owned()),
                billing_email: None,
                einvoice_recipient_id: None,
            }))
        })
    }
}

/// Provides the stand-ins.
struct Harness {
    tasks: Tasks,
    messages: Messages,
    who: Arc<Mutex<Option<AuthUser>>>,
}

impl Module for Harness {
    fn name(&self) -> &'static str {
        "clinics-real-harness"
    }

    fn register(&self, app: &mut Registry) {
        provide_directory(app, AnyParty);
        provide_task_creator(app, self.tasks.clone());
        provide_notifier(app, self.messages.clone());
    }

    fn routes(&self) -> Routes {
        let who = self.who.clone();
        Routes::new().get("/t/whoami", move |user: AuthUser| {
            let who = who.clone();
            async move {
                *who.lock().unwrap() = Some(user);
                "ok"
            }
        })
    }
}

/// The app under test.
pub struct Real {
    /// The app.
    pub app: TestApp,
    /// The staff tasks opened (by the clinic and by wl-invoicing).
    pub tasks: Tasks,
    /// The notifications asked of the notifier.
    pub messages: Messages,
    /// The payer agreement the claims are billed under: 30 days to send,
    /// 30 to resubmit, twice at most.
    pub agreement: PayerAgreement,
    /// The payer contract with the same windows.
    pub contract: PayerContract,
    who: Arc<Mutex<Option<AuthUser>>>,
    _views: tempfile::TempDir,
}

/// Boots the clinics app with the real wl-invoicing, a payer agreement and
/// the contract under it.
pub async fn boot_real() -> Real {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let tasks = Tasks::default();
    let messages = Messages::default();
    let who = Arc::new(Mutex::new(None));
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(WlInvoicing::new())
        .module(clinics::Clinics)
        .module(Harness {
            tasks: tasks.clone(),
            messages: messages.clone(),
            who: who.clone(),
        });
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    let entity = LegalEntity::default_entity(app.db())
        .await
        .expect("the legal entity exists");
    let mut agreement = PayerAgreement {
        legal_entity_id: entity.id,
        payer_party_id: PAYER.0,
        name: "Daman Gold".to_owned(),
        active: true,
        vat_recipient: VatRecipient::Customer,
        first_submission_days: 30,
        resubmission_days: 30,
        resubmission_limit: 2,
        write_off_limit: 100_000,
        ..PayerAgreement::default()
    };
    agreement
        .insert(app.db())
        .await
        .expect("the agreement is recorded");
    let contract = contracts::create(
        app.state(),
        None,
        NewContract::new(PAYER, day(-300)).agreement(agreement.id),
    )
    .await
    .expect("the contract is recorded");
    Real {
        app,
        tasks,
        messages,
        agreement,
        contract,
        who,
        _views: views,
    }
}

/// The next booking id: each visit is for a booking of its own.
fn next_booking() -> i64 {
    static BOOKING: AtomicI64 = AtomicI64::new(1_000);
    BOOKING.fetch_add(1, Ordering::SeqCst)
}

impl Real {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// `user` as the signed-in user the services take: what a request of
    /// theirs carries now (their roles as they are at this call).
    pub async fn auth(&self, user: &User) -> AuthUser {
        self.app.acting_as(user);
        self.app.get("/t/whoami").await.assert_ok();
        self.who
            .lock()
            .unwrap()
            .take()
            .expect("the probe route saw the signed-in user")
    }

    /// A user holding exactly `grants` (through a role of their own). The
    /// same name gives the same user.
    pub async fn user(&self, name: &str, grants: &[&str]) -> AuthUser {
        let user = self.user_row(name, grants).await;
        self.auth(&user).await
    }

    /// The user row behind [`Real::user`].
    async fn user_row(&self, name: &str, grants: &[&str]) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        if let Some(found) = User::find_by_email(self.db(), &email).await.unwrap() {
            return found;
        }
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        let role = format!("role_{}", name.to_lowercase());
        permissions::define_role(self.db(), &role, grants)
            .await
            .unwrap();
        user.assign_role(self.db(), &role).await.unwrap();
        user
    }

    /// The billing coordinator: manages claims and records the payer's
    /// money; holds the role the claim tasks go to.
    pub async fn coordinator(&self) -> AuthUser {
        let grants = [
            clinics::permissions::VISITS_VIEW,
            clinics::permissions::CLAIMS_MANAGE,
            inv::PAYER_BILLS_VIEW,
            inv::PAYER_BILLS_ISSUE,
            inv::PAYER_BILLS_RECONCILE,
        ];
        let user = self.user_row("Cora", &grants).await;
        permissions::define_role(
            self.db(),
            clinics::permissions::BILLING_COORDINATOR_ROLE,
            &grants,
        )
        .await
        .unwrap();
        user.assign_role(self.db(), clinics::permissions::BILLING_COORDINATOR_ROLE)
            .await
            .unwrap();
        self.auth(&user).await
    }

    /// The second person who approves an overpayment or a recoupment.
    pub async fn approver(&self) -> AuthUser {
        self.user("Bea", &[inv::PAYER_BILLS_VIEW, inv::PAYER_BILLS_RECONCILE])
            .await
    }

    /// The clinic manager: holds the role the window tasks go to.
    pub async fn clinic_manager(&self) -> AuthUser {
        let grants = [clinics::permissions::VISITS_VIEW];
        let user = self.user_row("Mia", &grants).await;
        permissions::define_role(
            self.db(),
            clinics::permissions::CLINIC_MANAGER_ROLE,
            &grants,
        )
        .await
        .unwrap();
        user.assign_role(self.db(), clinics::permissions::CLINIC_MANAGER_ROLE)
            .await
            .unwrap();
        self.auth(&user).await
    }

    /// The owner / medical director: told when a compliance-critical rule
    /// fails or misses its run.
    pub async fn owner(&self) -> AuthUser {
        let grants = [
            clinics::permissions::VISITS_VIEW,
            clinics::permissions::MISSED_WINDOW_BILLING,
        ];
        let user = self.user_row("Olga", &grants).await;
        permissions::define_role(self.db(), clinics::permissions::OWNER_ROLE, &grants)
        .await
        .unwrap();
        user.assign_role(self.db(), clinics::permissions::OWNER_ROLE)
            .await
            .unwrap();
        self.auth(&user).await
    }

    /// A clinic manager called `name`: closes claims (`claims.writeoff`) and
    /// holds what the payer bill asks of the signed-in user. The same name
    /// gives the same user.
    pub async fn manager(&self, name: &str) -> AuthUser {
        self.user(
            name,
            &[
                clinics::permissions::CLAIMS_WRITEOFF,
                inv::PAYER_BILLS_VIEW,
                inv::PAYER_BILLS_WRITE_OFF,
            ],
        )
        .await
    }

    /// A draft claim for a completed visit on `visit_date`, with one payer
    /// line per `(code, claimed fils)`, made as billing makes it: the visit
    /// and its charge lines, the real draft payer bill, the claim around it.
    pub async fn claim(&self, visit_date: NaiveDate, lines: &[(&str, i64)]) -> Claim {
        self.claim_under(&self.contract, visit_date, lines).await
    }

    /// Like [`Real::claim`], under another payer contract.
    pub async fn claim_under(
        &self,
        contract: &PayerContract,
        visit_date: NaiveDate,
        lines: &[(&str, i64)],
    ) -> Claim {
        let mut visit = Visit {
            booking_id: next_booking(),
            patient_party_id: Some(PATIENT.0),
            service_code: "consultation".to_owned(),
            visit_date,
            checked_in_at: now(),
            payer_total: lines.iter().map(|(_, claimed)| claimed).sum(),
            status: VisitStatus::Completed,
            ..Visit::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        visit
            .insert_in_state(&mut tx, VisitStatus::Completed, CHECK_IN)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let mut request =
            NewPayerBill::new(self.agreement.id, PATIENT, visit.subject(), visit_date)
                .void_before_first_submission();
        for (code, claimed) in lines {
            ChargeLine {
                visit_id: visit.id,
                kind: ChargeKind::Service,
                code: (*code).to_owned(),
                units: 1,
                unit_price: *claimed,
                list_price: *claimed,
                payer_share: *claimed,
                ..ChargeLine::default()
            }
            .save(self.db())
            .await
            .unwrap();
            request = request.line(NewPayerBillLine::new(&format!("{code} x 1"), *claimed));
        }
        let bill = payer_bills(self.state())
            .create(self.state(), request, None)
            .await
            .expect("the payer bill is made");
        Visit::query()
            .where_(Visit::ID.eq(visit.id))
            .update(self.db(), &[("payer_bill_id", &Some(bill.0))])
            .await
            .unwrap();
        visit.payer_bill_id = Some(bill.0);
        claims::create_for_visit(self.state(), &visit, bill, Some(contract), None, None)
            .await
            .expect("the claim is made")
    }

    /// Claim `id`, as stored now.
    pub async fn claim_now(&self, id: i64) -> Claim {
        Claim::find_or_404(self.db(), id).await.unwrap()
    }

    /// The real payer bill of `claim`, as stored now.
    pub async fn bill_of(&self, claim: &Claim) -> wl_invoicing::PayerBill {
        wl_invoicing::PayerBill::find_or_404(self.db(), claim.payer_bill_id)
            .await
            .unwrap()
    }

    /// `POST path` as `by`.
    pub async fn post(
        &self,
        by: &User,
        path: &str,
        fields: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app.htmx().post(path, fields).await
    }

    /// `GET path` as `by`.
    pub async fn get(&self, by: &User, path: &str) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app.get(path).await
    }
}
