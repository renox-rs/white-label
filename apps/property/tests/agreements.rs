//! BL-PRP-22 (#272): the lifecycle of a management agreement. `activate`
//! needs the owner, units, a fee and the signed agreement; `give_notice` makes
//! it `ending`, tells the managers and starts the handover pack; `end` is
//! refused without every receipt, with a deposit waiting for its settlement or
//! without an approved final statement, and then hands everything over.

mod support;

use property::agreement::{self, AgreementStatus, AgreementUnit, ManagementAgreement, NewAgreement};
use property::agreement_lifecycle::{self as lifecycle, HandoverItem, ItemKind, SIGNED_AGREEMENT_KIND};
use property::notices::{AGREEMENT_NOTICE, PAYEE_CHANGED};
use property::statement::{OwnerStatement, StatementStatus};
use property::{ChargeStatus, Lease, LeaseCharge, Unit};
use renox::chrono::Duration;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::cheques::{ChequeId, ChequeStatus};
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, invoicer};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::parties::PartyKind;
use wl_kernel::{PartyId, VatCategory};

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, TENANT, aed, boot, day};

/// The owner of the units.
const OWNER: PartyId = PartyId(31);

/// The company that manages the units after the handover.
const NEW_MANAGER: PartyId = PartyId(40);

struct Scene {
    t: Booted,
    agreement: ManagementAgreement,
    units: Vec<Unit>,
    leases: Vec<Lease>,
    cheques: Vec<ChequeId>,
    manager: User,
    accountant: User,
}

/// An agreement for the owner with three units, each with an active lease, a
/// deposit of AED 5,000 held and three cheques of AED 25,000 received; a
/// property manager (M) and an accountant (A).
async fn scene(status: AgreementStatus) -> Scene {
    let t = boot().await;
    t.parties.add(OWNER.0, "Hessa Al Mansoori", PartyKind::Person);
    t.parties.add(NEW_MANAGER.0, "Palm Managers LLC", PartyKind::Company);
    let building = t.building().await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let site_manager = t.user_in("Sami", MANAGER, building.id).await;
    let mut units = Vec::new();
    let mut leases = Vec::new();
    let mut cheques = Vec::new();
    for number in ["1001", "1002", "1003"] {
        let unit = t.unit(&building, number).await;
        let lease = t.active_lease(&unit, &agent, &site_manager).await;
        t.deposits.hold(&lease.agreement(), TENANT, aed(5_000));
        for n in 1..=3 {
            cheques.push(t.cheques.receive(
                TENANT,
                &lease.agreement(),
                &format!("{number}{n}"),
                day(30 * n),
                aed(25_000),
                &[],
            ));
        }
        units.push(unit);
        leases.push(lease);
    }
    let mut agreement = ManagementAgreement {
        owner_party_id: OWNER.0,
        fee_bp: 500,
        payout_frequency: "monthly".to_owned(),
        notice_days: 90,
        start_date: day(-400),
        status,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    agreement
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    for unit in &units {
        agreement::add_unit(t.db(), &agreement, unit.id, day(-400))
            .await
            .unwrap();
    }
    let accountant = t.user_with("Amal", ACCOUNTANT).await;
    let manager = t.user_with("Mona", MANAGER).await;
    Scene {
        t,
        agreement,
        units,
        leases,
        cheques,
        manager,
        accountant,
    }
}

impl Scene {
    fn path(&self, tail: &str) -> String {
        format!("/property/agreements/{}/{tail}", self.agreement.id)
    }

    async fn agreement_now(&self) -> ManagementAgreement {
        ManagementAgreement::find_or_404(self.t.db(), self.agreement.id)
            .await
            .unwrap()
    }

    /// The handover date of the tests: 60 days ago, inside the leases' terms.
    fn handover(&self) -> String {
        day(-60).to_string()
    }

    /// The manager gives notice on behalf of the owner.
    async fn give_notice(&self, holder: &str) -> renox::testing::TestResponse {
        let notice_on = day(-100).to_string();
        let handover = self.handover();
        self.t
            .post(
                &self.manager,
                &self.path("give-notice"),
                &[
                    ("notice_on", notice_on.as_str()),
                    ("party", "owner"),
                    ("handover", handover.as_str()),
                    ("holder", holder),
                ],
            )
            .await
    }

    /// The lines of the pack.
    async fn items(&self) -> Vec<HandoverItem> {
        HandoverItem::of_agreement(self.t.db(), self.agreement.id)
            .await
            .unwrap()
    }

    /// The manager records a receipt for every line but those in `skip`.
    async fn receipts(&self, skip: &[i64]) {
        for item in self.items().await {
            if skip.contains(&item.id) {
                continue;
            }
            self.t
                .post(
                    &self.manager,
                    &self.path("receipts"),
                    &[
                        ("item", item.id.to_string().as_str()),
                        ("reference", format!("RCPT-{}", item.id).as_str()),
                    ],
                )
                .await
                .assert_ok();
        }
    }

    /// The accountant prepares the final statement; the draft is returned.
    async fn final_draft(&self) -> OwnerStatement {
        self.t
            .post(&self.accountant, &self.path("final-statement"), &[])
            .await
            .assert_ok();
        OwnerStatement::of_agreement(self.t.db(), self.agreement.id)
            .await
            .unwrap()
            .pop()
            .expect("the final statement")
    }

    /// `user` runs the transition `name` on the statement.
    async fn statement_run(&self, user: &User, statement: &OwnerStatement, name: &str) {
        self.t
            .post_plain(
                user,
                &format!("/property/statements/{}/transitions/{name}", statement.id),
                &[],
            )
            .await
            .assert_status(303);
    }

    /// Notice given, every receipt recorded and the final statement approved.
    async fn ready_to_end(&self) -> OwnerStatement {
        self.give_notice("").await.assert_ok();
        self.receipts(&[]).await;
        let draft = self.final_draft().await;
        self.statement_run(&self.manager, &draft, "approve").await;
        draft
    }

    async fn end(&self) -> renox::testing::TestResponse {
        self.t.post(&self.manager, &self.path("end"), &[]).await
    }

    /// An issued rent invoice of `amount` fils for the first lease, left unpaid.
    async fn open_invoice(&self, amount: i64) -> i64 {
        let lease = &self.leases[0];
        let draft = InvoiceDraft::new(TENANT, lease.subject())
            .due_date(day(-90))
            .line(DraftLine::new("Rent", amount, VatCategory::Exempt));
        let id = invoicer(self.t.state())
            .draft(self.t.db(), draft)
            .await
            .unwrap()
            .expect("invoicing is on")
            .0;
        self.t.invoices.issue(id);
        LeaseCharge::create(
            self.t.db(),
            LeaseCharge {
                lease_id: lease.id,
                position: 50,
                charge_type: "rent".to_owned(),
                description: "Rent".to_owned(),
                due_date: day(-90),
                amount,
                vat_category: VatCategory::Exempt,
                status: ChargeStatus::Invoiced,
                invoice_id: Some(id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        id
    }
}

/// AC1: given an `active` agreement for owner O with 3 units, `give_notice`
/// (party owner, a handover date) makes it `ending`, notifies the property
/// manager and the accountant and starts the pack.
#[renox::test]
async fn bl_prp_22_give_notice_makes_it_ending_and_starts_the_pack() {
    let s = scene(AgreementStatus::Active).await;
    let invoice = s.open_invoice(aed(8_000)).await;
    s.t.set(
        &s.leases[0],
        &[("ejari_number", &Some("EJ-100-1".to_owned()))],
    )
    .await;

    // The accountant does not manage agreements.
    let notice_on = day(-100).to_string();
    let handover = s.handover();
    let form = [
        ("notice_on", notice_on.as_str()),
        ("party", "owner"),
        ("handover", handover.as_str()),
        ("holder", ""),
    ];
    s.t.post(&s.accountant, &s.path("give-notice"), &form)
        .await
        .assert_forbidden();
    assert_eq!(s.agreement_now().await.status, AgreementStatus::Active);

    s.give_notice("").await.assert_ok();
    let ending = s.agreement_now().await;
    assert_eq!(ending.status, AgreementStatus::Ending);
    assert_eq!(ending.notice_party.as_deref(), Some("owner"));
    assert_eq!(ending.notice_on, Some(day(-100)));
    assert_eq!(ending.handover_date, Some(day(-60)));
    assert_eq!(ending.notice_by, Some(s.manager.id));

    let told = s.t.notices.of_type(AGREEMENT_NOTICE);
    assert_eq!(told.len(), 1);
    for user in [s.manager.id, s.accountant.id] {
        assert!(told[0]
            .recipients
            .iter()
            .any(|r| matches!(r, NotifyRecipient::User { id } if *id == user)));
    }

    // The pack: 3 deposits (AED 15,000), 9 cheques, the open invoice and the
    // Ejari number.
    let pack = lifecycle::pack(s.t.state(), &ending).await.unwrap();
    assert_eq!(pack.deposits.len(), 3);
    assert_eq!(pack.deposit_total(), aed(15_000));
    assert_eq!(pack.cheques.len(), 9);
    assert_eq!(pack.invoices.len(), 1);
    assert_eq!(pack.invoices[0].balance, aed(8_000));
    assert_eq!(pack.ejari.len(), 1);
    assert_eq!(pack.ejari[0].number, "EJ-100-1");
    assert_eq!(pack.missing_receipts(), 12);
    let _ = invoice;

    // Starting the pack again changes nothing.
    assert_eq!(lifecycle::start_pack(s.t.state(), &ending).await.unwrap(), 0);

    // A second notice is refused: the agreement is no longer active.
    s.give_notice("").await.assert_status(422);
}

/// A notice needs its handover date after the notice date, and a new holder
/// that is a party.
#[renox::test]
async fn bl_prp_22_a_notice_needs_a_handover_after_it_and_a_known_holder() {
    let s = scene(AgreementStatus::Active).await;
    let notice_on = day(-10).to_string();
    let early = day(-20).to_string();
    s.t.post(
        &s.manager,
        &s.path("give-notice"),
        &[
            ("notice_on", notice_on.as_str()),
            ("party", "owner"),
            ("handover", early.as_str()),
            ("holder", ""),
        ],
    )
    .await
    .assert_status(422);
    s.give_notice("999").await.assert_status(422);
    assert_eq!(s.agreement_now().await.status, AgreementStatus::Active);
}

/// AC3: `end` without a receipt for one cheque is refused.
#[renox::test]
async fn bl_prp_22_end_without_a_receipt_for_one_cheque_is_refused() {
    let s = scene(AgreementStatus::Active).await;
    s.give_notice("").await.assert_ok();
    let items = s.items().await;
    let last_cheque = items
        .iter()
        .filter(|item| item.kind == ItemKind::Cheque)
        .next_back()
        .expect("a cheque line")
        .id;
    s.receipts(&[last_cheque]).await;
    let draft = s.final_draft().await;
    s.statement_run(&s.manager, &draft, "approve").await;

    s.end().await.assert_status(422);
    assert_eq!(s.agreement_now().await.status, AgreementStatus::Ending);
    assert!(s.t.cheques.returns().is_empty());
    assert!(s.t.deposits.transferred().is_empty());

    // With the last receipt it ends.
    s.t.post(
        &s.manager,
        &s.path("receipts"),
        &[("item", last_cheque.to_string().as_str()), ("reference", "RCPT-LAST")],
    )
    .await
    .assert_ok();
    s.end().await.assert_ok();
    assert_eq!(s.agreement_now().await.status, AgreementStatus::Ended);
}

/// AC4 and AC5: `end` with all receipts and an approved final statement
/// cancels the charges after the handover date, returns the 9 cheques as
/// *handed over*, transfers the 3 deposits, tells each tenant the new payee
/// and ends the agreement; the final statement is not yet published.
#[renox::test]
async fn bl_prp_22_end_hands_everything_over() {
    let s = scene(AgreementStatus::Active).await;
    s.open_invoice(aed(8_000)).await;
    s.give_notice(&NEW_MANAGER.0.to_string()).await.assert_ok();
    s.receipts(&[]).await;
    let draft = s.final_draft().await;
    assert_eq!(draft.status, StatementStatus::Draft);
    assert_eq!(draft.prepared_by, Some(s.accountant.id));
    assert_eq!(draft.period_end, day(-60));

    // The final statement is still a draft: refused.
    s.end().await.assert_status(422);
    // The preparer does not approve it.
    s.t.post_plain(
        &s.accountant,
        &format!("/property/statements/{}/transitions/approve", draft.id),
        &[],
    )
    .await
    .assert_forbidden();
    s.statement_run(&s.manager, &draft, "approve").await;

    s.end().await.assert_ok();
    let ended = s.agreement_now().await;
    assert_eq!(ended.status, AgreementStatus::Ended);
    assert_eq!(ended.ended_on, Some(support::day(0)));

    // The charges after the handover date are cancelled; the others stay.
    let handover = day(-60);
    for lease in &s.leases {
        let charges = LeaseCharge::of_lease(s.t.db(), lease.id).await.unwrap();
        assert!(charges.iter().any(|c| c.due_date > handover));
        for charge in charges {
            if charge.invoice_id.is_some() {
                assert_eq!(charge.status, ChargeStatus::Invoiced);
            } else if charge.due_date > handover {
                assert_eq!(charge.status, ChargeStatus::Cancelled, "{}", charge.due_date);
            } else {
                assert_eq!(charge.status, ChargeStatus::Scheduled, "{}", charge.due_date);
            }
        }
        let marked = Lease::find_or_404(s.t.db(), lease.id).await.unwrap();
        assert_eq!(marked.handed_over_on, Some(handover));
        assert_eq!(marked.status, property::LeaseStatus::Active);
    }

    // The cheques went back as handed over, all 9.
    let returns = s.t.cheques.returns();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].cheques.len(), 9);
    assert_eq!(returns[0].reason, "handed over");
    for id in &s.cheques {
        assert_eq!(s.t.cheques.get(*id).status, ChequeStatus::Returned);
    }

    // The 3 deposits were handed to the new manager against the receipts.
    let moved = s.t.deposits.transferred();
    assert_eq!(moved.len(), 3);
    for (request, by) in &moved {
        assert_eq!(request.new_holder, NEW_MANAGER);
        assert!(request.receipt.starts_with("RCPT-"));
        assert_eq!(*by, s.manager.id);
    }
    for lease in &s.leases {
        assert!(!s.t.deposits.is_held(&lease.agreement()));
    }
    assert!(s.items().await.iter().all(|item| item.done));

    // Each tenant is told the new payee.
    let told = s.t.notices.of_type(PAYEE_CHANGED);
    assert_eq!(told.len(), 3);
    assert!(told[0].data.to_string().contains("Palm Managers LLC"));
    assert!(told[0]
        .recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::Party { id } if *id == TENANT)));

    // The units are out the day before the handover; issued invoices stay
    // with the owner and are not credited.
    let rows: Vec<AgreementUnit> = ended.units(s.t.db()).await.unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.date_out == Some(handover - Duration::days(1))));
    assert!(s.t.invoices.credits().is_empty());
    assert!(s.t.tasks.with_key("property.handover_problems").is_empty());

    // The reminders on the invoices of a handed-over lease stop.
    let held = wl_kernel::chase_holds::hold(s.t.db(), &s.leases[0].subject())
        .await
        .unwrap();
    assert_eq!(held, wl_kernel::chase_holds::ChaseHold::Held);

    // `ended` is final: no second end.
    s.end().await.assert_status(422);
}

/// AC5: with the final statement `approved` but not yet published, `end`
/// succeeds; the statement is then published and paid out to the owner with
/// the agreement already `ended`.
#[renox::test]
async fn bl_prp_22_the_final_statement_is_published_and_paid_out_after_the_end() {
    let s = scene(AgreementStatus::Active).await;
    s.give_notice("").await.assert_ok();
    s.receipts(&[]).await;
    let draft = s.final_draft().await;
    s.t.post(
        &s.accountant,
        &format!("/property/statements/{}/lines", draft.id),
        &[("kind", "collected"), ("description", "Rent to the handover"), ("amount", "42,000")],
    )
    .await
    .assert_ok();
    s.statement_run(&s.manager, &draft, "approve").await;
    let approved = OwnerStatement::find_or_404(s.t.db(), draft.id).await.unwrap();
    assert_eq!(approved.status, StatementStatus::Approved);

    s.end().await.assert_ok();
    assert_eq!(s.agreement_now().await.status, AgreementStatus::Ended);

    s.statement_run(&s.accountant, &approved, "publish").await;
    let published = OwnerStatement::find_or_404(s.t.db(), draft.id).await.unwrap();
    assert_eq!(published.status, StatementStatus::Published);
    assert_eq!(s.agreement_now().await.status, AgreementStatus::Ended);

    // The payout is asked for, and a second user approves it.
    s.t.post(
        &s.accountant,
        &format!("/property/statements/{}/pay-out", published.id),
        &[],
    )
    .await
    .assert_ok();
    assert_eq!(s.agreement_now().await.status, AgreementStatus::Ended);
}

/// AC6: with the final statement still `draft` (or none), `end` is refused.
#[renox::test]
async fn bl_prp_22_without_an_approved_final_statement_end_is_refused() {
    let s = scene(AgreementStatus::Active).await;
    s.give_notice("").await.assert_ok();
    s.receipts(&[]).await;
    s.end().await.assert_status(422);
    let draft = s.final_draft().await;
    s.end().await.assert_status(422);
    // Returned after approval, it is a draft again.
    s.statement_run(&s.manager, &draft, "approve").await;
    s.t.post_plain(
        &s.manager,
        &format!("/property/statements/{}/transitions/return", draft.id),
        &[("reason", "a cost is missing")],
    )
    .await
    .assert_status(303);
    s.end().await.assert_status(422);
    assert_eq!(s.agreement_now().await.status, AgreementStatus::Ending);
}

/// A deposit in `settlement_pending` cannot be transferred: `end` is refused
/// until it is refunded or forfeited.
#[renox::test]
async fn bl_prp_22_a_deposit_in_settlement_refuses_the_end() {
    let s = scene(AgreementStatus::Active).await;
    let draft = s.ready_to_end().await;
    s.t.deposits
        .settle_with(&s.leases[1].agreement(), TENANT, aed(5_000), -20, &[]);
    s.end().await.assert_status(422);
    assert_eq!(s.agreement_now().await.status, AgreementStatus::Ending);
    assert!(s.t.deposits.transferred().is_empty());
    let _ = draft;
}

/// AC7: `activate` without a signed agreement is refused; with it, and with
/// the owner, a unit and a fee, it is accepted.
#[renox::test]
async fn bl_prp_22_activate_needs_the_signed_agreement() {
    let t = boot().await;
    t.parties.add(OWNER.0, "Hessa Al Mansoori", PartyKind::Person);
    let building = t.building().await;
    let unit = t.unit(&building, "2001").await;
    let manager = t.user_with("Mona", MANAGER).await;
    let accountant = t.user_with("Amal", ACCOUNTANT).await;
    let draft = agreement::create_draft(t.state(), None, NewAgreement::new(OWNER, day(0), 500))
        .await
        .unwrap();
    assert_eq!(draft.status, AgreementStatus::Draft);
    let path = format!("/property/agreements/{}/activate", draft.id);
    t.docs.block(SIGNED_AGREEMENT_KIND, "agreement.activate");

    // No unit yet.
    t.post(&manager, &path, &[]).await.assert_status(422);
    agreement::add_unit(t.db(), &draft, unit.id, day(0)).await.unwrap();
    // The signed agreement is missing.
    t.post(&manager, &path, &[]).await.assert_status(422);
    t.docs.accept(&draft.subject(), SIGNED_AGREEMENT_KIND);
    // The accountant does not manage agreements.
    t.post(&accountant, &path, &[]).await.assert_forbidden();
    t.post(&manager, &path, &[]).await.assert_ok();
    let active = ManagementAgreement::find_or_404(t.db(), draft.id).await.unwrap();
    assert_eq!(active.status, AgreementStatus::Active);
}

/// An agreement without a fee is not activated.
#[renox::test]
async fn bl_prp_22_activate_needs_a_fee() {
    let t = boot().await;
    t.parties.add(OWNER.0, "Hessa Al Mansoori", PartyKind::Person);
    let building = t.building().await;
    let unit = t.unit(&building, "2002").await;
    let manager = t.user_with("Mona", MANAGER).await;
    let free = agreement::create_draft(t.state(), None, NewAgreement::new(OWNER, day(0), 0))
        .await
        .unwrap();
    agreement::add_unit(t.db(), &free, unit.id, day(0)).await.unwrap();
    t.post(&manager, &format!("/property/agreements/{}/activate", free.id), &[])
        .await
        .assert_status(422);
    assert_eq!(
        ManagementAgreement::find_or_404(t.db(), free.id).await.unwrap().status,
        AgreementStatus::Draft
    );
}

/// The agreement pages: the list and an ending agreement with its pack.
#[renox::test]
async fn bl_prp_22_the_pages_show_the_agreement_and_its_pack() {
    let s = scene(AgreementStatus::Active).await;
    s.give_notice("").await.assert_ok();
    s.t.app.acting_as(&s.manager);
    s.t.app.get("/property/agreements").await.assert_ok().assert_see("MA-");
    s.t.app
        .get(&format!("/property/agreements/{}", s.agreement.id))
        .await
        .assert_ok()
        .assert_see("Hessa Al Mansoori");
    s.t.app.acting_as(&s.accountant);
    s.t.app.get("/property/agreements").await.assert_forbidden();
}
