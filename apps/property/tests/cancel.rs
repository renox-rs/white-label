//! BL-PRP-01 (#251): cancel a lease after money or cheques were received
//! (`PRP.ACL.18`). With nothing received the cancel runs at once; with money
//! or cheques received it waits for a property manager, and the money
//! effects run only at the final approval, on the money received by then.

mod support;

use property::cancellation::{CancelWait, WaitOutcome, settle};
use property::{ChargeStatus, Lease, LeaseCharge, LeaseStatus};
use renox::prelude::*;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, approve, reject};
use wl_kernel::cheques::{ChequeBounced, ChequeId, ChequeStatus};
use wl_kernel::invoicing::{InvoiceStatus, invoicer};
use wl_kernel::payments::{PaymentId, PaymentMethod, PaymentReceived, RefundSource};
use wl_kernel::{PartyId, Subject};

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, TENANT, aed, boot};

/// An invoice of `charge`, issued, and `pay`ing it from the payment 1 when
/// asked; the charge is marked invoiced. The invoice's id.
async fn invoiced(t: &Booted, lease: &Lease, charge: &LeaseCharge, pay: bool) -> i64 {
    let draft = property::invoicing::draft_of(lease, charge);
    let id = invoicer(t.state())
        .draft(t.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    if pay {
        t.invoices.issue(id.0);
        t.invoices
            .pay(id.0, &Subject::new("payments.payment", 1), charge.amount);
    }
    LeaseCharge::query()
        .where_(LeaseCharge::ID.eq(charge.id))
        .update(
            t.db(),
            &[
                ("status", &ChargeStatus::Invoiced),
                ("invoice_id", &Some(id.0)),
            ],
        )
        .await
        .unwrap();
    id.0
}

/// A lease waiting for signature with a deposit of AED 5,000 held, the first
/// rent of AED 25,000 paid against an issued invoice, and three cheques of AED
/// 25,000 received.
struct Scene {
    t: Booted,
    agent: User,
    manager: User,
    lease: Lease,
    invoice: i64,
    cheques: Vec<ChequeId>,
}

async fn scene() -> Scene {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let lease = t.pending(&agent, &unit, 0).await;
    t.deposits.hold(&lease.agreement(), TENANT, aed(5_000));
    let charges = lease.charges(t.db()).await.unwrap();
    let invoice = invoiced(&t, &lease, &charges[0], true).await;
    let cheques = charges[1..]
        .iter()
        .enumerate()
        .map(|(i, c)| {
            t.cheques.receive(
                TENANT,
                &lease.agreement(),
                &format!("20{i}"),
                c.due_date,
                c.amount,
                &[c.subject()],
            )
        })
        .collect();
    Scene {
        t,
        agent,
        manager,
        lease,
        invoice,
        cheques,
    }
}

impl Scene {
    fn path(&self) -> String {
        format!("/property/leases/{}/cancel", self.lease.id)
    }

    /// The agent asks for the cancellation.
    async fn ask(&self, extra: &[(&str, &str)]) -> renox::testing::TestResponse {
        let mut form = vec![("reason", "prospect backed out")];
        form.extend_from_slice(extra);
        self.t.post(&self.agent, &self.path(), &form).await
    }

    /// The pending approval request about the lease.
    async fn request(&self) -> ApprovalRequest {
        ApprovalRequest::pending_about(self.t.db(), &self.lease.subject())
            .await
            .unwrap()
            .remove(0)
    }

    async fn lease_now(&self) -> Lease {
        self.t.reload(&self.lease).await
    }
}

/// AC1: with no payment and no cheque the leasing agent cancels at once; no
/// approval, no refund, no deposit settlement.
#[renox::test]
async fn bl_prp_01_a_lease_with_nothing_received_is_cancelled_at_once() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let lease = t.pending(&agent, &unit, 0).await;

    t.post(
        &agent,
        &format!("/property/leases/{}/cancel", lease.id),
        &[("reason", "prospect withdrew")],
    )
    .await
    .assert_ok();
    let cancelled = t.reload(&lease).await;
    assert_eq!(cancelled.status, LeaseStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("prospect withdrew")
    );
    assert_eq!(cancelled.cancelled_on, Some(support::day(0)));
    assert!(cancelled.money_settled_at.is_some());
    let approvals = ApprovalRequest::pending_about(t.db(), &lease.subject())
        .await
        .unwrap();
    assert!(approvals.is_empty());
    assert!(t.refunds.asked().is_empty());
    assert!(t.deposits.settled().is_empty());
    let events = t.cancelled.all();
    assert_eq!(events.len(), 1);
    assert!(!events[0].money_received);
    // The planned charges are cancelled with it.
    let charges = cancelled.charges(t.db()).await.unwrap();
    assert!(charges.iter().all(|c| c.status == ChargeStatus::Cancelled));
    // The unit was never taken.
    assert_eq!(
        t.unit_now(&unit).await.status,
        property::UnitStatus::Available
    );
}

/// A reason is needed, a user without `leases.cancel` is refused, and a
/// cancelled lease cannot be cancelled again.
#[renox::test]
async fn bl_prp_01_the_reason_the_permission_and_the_state_are_checked() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let accountant = t.user_in("Aisha", ACCOUNTANT, building.id).await;
    let nobody = t.user("Nora").await;
    let lease = t.pending(&agent, &unit, 0).await;
    let path = format!("/property/leases/{}/cancel", lease.id);

    t.post(&agent, &path, &[("reason", "  ")])
        .await
        .assert_status(422);
    t.post(&nobody, &path, &[("reason", "x")])
        .await
        .assert_forbidden();
    assert_eq!(t.status(&lease).await, LeaseStatus::PendingSignature);
    // The accountant may ask too (PRP.ACL.18).
    t.post(&accountant, &path, &[("reason", "prospect withdrew")])
        .await
        .assert_ok();
    assert_eq!(t.status(&lease).await, LeaseStatus::Cancelled);
    t.post(&agent, &path, &[("reason", "again")])
        .await
        .assert_status(422);
}

/// AC2: with money received the request waits for a property manager and the
/// lease stays `pending_signature`, with a pending-approval badge and no
/// `activate`.
#[renox::test]
async fn bl_prp_01_money_received_sends_the_cancellation_for_approval() {
    let s = scene().await;
    s.ask(&[]).await.assert_ok();
    assert_eq!(s.lease_now().await.status, LeaseStatus::PendingSignature);
    let request = s.request().await;
    assert_eq!(request.action, "property.lease_cancel");
    assert_eq!(request.requester_id, s.agent.id);
    assert_eq!(request.payload.0["reason"], "prospect backed out");
    assert!(request.payload.0["money_received"].as_i64().unwrap() > 0);
    // Nothing of the money effects ran.
    assert!(s.t.refunds.asked().is_empty());
    assert!(s.t.deposits.settled().is_empty());
    for id in &s.cheques {
        assert_eq!(s.t.cheques.get(*id).status, ChequeStatus::Received);
    }
    assert_eq!(
        s.t.invoices.get(s.invoice).unwrap().summary.status,
        InvoiceStatus::Paid
    );
    // The badge shows and `activate` is refused while the request waits.
    s.t.app.acting_as(&s.agent);
    s.t.app
        .get(&format!("/property/leases/{}", s.lease.id))
        .await
        .assert_ok()
        .assert_see("Waiting for approval");
    let refused = s.t.activate(&s.manager, &s.lease).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("waiting for approval"),
        "{}",
        refused.text()
    );
    // A second request is refused while one waits.
    s.ask(&[]).await.assert_status(422);
}

/// AC3: the requester cannot approve (SoD), and a manager who requested
/// cannot approve their own request.
#[renox::test]
async fn bl_prp_01_the_requester_cannot_approve() {
    let s = scene().await;
    s.ask(&[]).await.assert_ok();
    let request = s.request().await;
    let refused = approve(s.t.state(), request.id, &s.agent, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");

    // A property manager asking for the cancellation is not its approver.
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let lease = t.pending(&manager, &unit, 0).await;
    t.deposits.hold(&lease.agreement(), TENANT, aed(5_000));
    t.post(
        &manager,
        &format!("/property/leases/{}/cancel", lease.id),
        &[("reason", "backed out")],
    )
    .await
    .assert_ok();
    let own = ApprovalRequest::pending_about(t.db(), &lease.subject())
        .await
        .unwrap()
        .remove(0);
    let refused = approve(t.state(), own.id, &manager, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(t.status(&lease).await, LeaseStatus::PendingSignature);
}

/// AC4: at the manager's approval the lease is cancelled, the cheques go back
/// on one receipt, the rent invoice is unallocated and credited, its payment
/// is asked back from the credit balance, and the deposit's settlement starts.
#[renox::test]
async fn bl_prp_01_the_money_effects_run_at_the_final_approval() {
    let s = scene().await;
    s.ask(&[]).await.assert_ok();
    let request = s.request().await;
    let decided = approve(s.t.state(), request.id, &s.manager, "ok")
        .await
        .unwrap();
    assert!(decided.is_some());

    let cancelled = s.lease_now().await;
    assert_eq!(cancelled.status, LeaseStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("prospect backed out")
    );
    assert!(cancelled.money_settled_at.is_some());

    for id in &s.cheques {
        assert_eq!(s.t.cheques.get(*id).status, ChequeStatus::Returned);
    }
    assert_eq!(
        s.t.cheques.returns().len(),
        1,
        "one receipt for all of them"
    );
    assert_eq!(s.t.cheques.returns()[0].cheques.len(), 3);

    let invoice = s.t.invoices.get(s.invoice).unwrap();
    assert_eq!(invoice.summary.status, InvoiceStatus::Credited);
    assert!(invoice.allocations.is_empty());

    let refunds = s.t.refunds.asked();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, aed(25_000));
    assert_eq!(refunds[0].party, TENANT);
    assert_eq!(refunds[0].source, RefundSource::PartyCredit(TENANT));

    let settled = s.t.deposits.settled();
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].0.source, s.lease.agreement());
    assert_eq!(settled[0].0.forfeit, 0);
    assert_eq!(settled[0].1.held, aed(5_000));

    let charges = cancelled.charges(s.t.db()).await.unwrap();
    assert!(charges.iter().all(|c| c.status == ChargeStatus::Cancelled));
    let events = s.t.cancelled.all();
    assert_eq!(events.len(), 1);
    assert!(events[0].money_received);
    assert!(s.t.tasks.open().is_empty(), "{:?}", s.t.tasks.open());
    // A repeat does nothing more.
    let again = settle(s.t.state(), s.lease.id, None).await.unwrap();
    assert!(!again.ran);
    assert_eq!(s.t.refunds.asked().len(), 1);
}

/// AC5: a forfeit of AED 2,000 with its clause is published as a deduction of
/// the settlement; without a clause, or above the deposit, it is refused.
#[renox::test]
async fn bl_prp_01_a_forfeit_needs_its_clause_and_stays_within_the_deposit() {
    let s = scene().await;
    s.ask(&[("forfeit", "2,000.00")]).await.assert_status(422);
    s.ask(&[("forfeit", "6,000"), ("clause", "Reservation form §4")])
        .await
        .assert_status(422);
    assert!(
        ApprovalRequest::pending_about(s.t.db(), &s.lease.subject())
            .await
            .unwrap()
            .is_empty()
    );

    s.ask(&[("forfeit", "2,000.00"), ("clause", "Reservation form §4")])
        .await
        .assert_ok();
    let request = s.request().await;
    assert_eq!(request.payload.0["forfeit"], aed(2_000));
    approve(s.t.state(), request.id, &s.manager, "")
        .await
        .unwrap();

    let settled = s.t.deposits.settled();
    assert_eq!(settled[0].0.forfeit, aed(2_000));
    assert_eq!(settled[0].0.clause.as_deref(), Some("Reservation form §4"));
    let lease = s.lease_now().await;
    assert_eq!(lease.cancel_forfeit, aed(2_000));
    assert_eq!(
        lease.cancel_forfeit_clause.as_deref(),
        Some("Reservation form §4")
    );
}

/// AC6: when the manager rejects, the lease stays `pending_signature`,
/// nothing is credited or returned, and the request ends rejected.
#[renox::test]
async fn bl_prp_01_a_rejected_cancellation_changes_nothing() {
    let s = scene().await;
    s.ask(&[]).await.assert_ok();
    let request = s.request().await;
    reject(s.t.state(), request.id, &s.manager, "The prospect signed")
        .await
        .unwrap();

    assert_eq!(s.lease_now().await.status, LeaseStatus::PendingSignature);
    for id in &s.cheques {
        assert_eq!(s.t.cheques.get(*id).status, ChequeStatus::Received);
    }
    assert_eq!(
        s.t.invoices.get(s.invoice).unwrap().summary.status,
        InvoiceStatus::Paid
    );
    assert!(s.t.refunds.asked().is_empty());
    assert!(s.t.deposits.settled().is_empty());
    let ended = ApprovalRequest::find(s.t.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ended.status, ApprovalStatus::Rejected);
    // The requester is told, with the manager's comment.
    let told: Vec<String> = s
        .agent
        .notifications(s.t.db(), 50)
        .await
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == wl_kernel::approvals::notify::DECIDED_KIND)
        .map(|n| n.data.to_string())
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].contains("The prospect signed"), "{}", told[0]);
    let manager_told = s
        .manager
        .notifications(s.t.db(), 50)
        .await
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == wl_kernel::approvals::notify::DECIDED_KIND)
        .count();
    assert_eq!(
        manager_told, 0,
        "the decider is not told of his own decision"
    );
    // The lease can be asked to cancel again, and activation is not blocked.
    s.ask(&[]).await.assert_ok();
}

/// AC7: a cheque deposited by the time of the approval is listed in a task;
/// when it clears, its amount joins the refunds; when it bounces, nothing is
/// refunded.
#[renox::test]
async fn bl_prp_01_a_deposited_cheque_waits_for_clear_or_bounce() {
    let s = scene().await;
    s.t.cheques
        .set_status(s.cheques[0], ChequeStatus::Deposited);
    s.t.cheques
        .set_status(s.cheques[1], ChequeStatus::Deposited);
    s.ask(&[]).await.assert_ok();
    let request = s.request().await;
    approve(s.t.state(), request.id, &s.manager, "")
        .await
        .unwrap();

    // The undeposited one went back; the two deposited ones wait.
    assert_eq!(s.t.cheques.get(s.cheques[2]).status, ChequeStatus::Returned);
    assert_eq!(
        s.t.cheques.get(s.cheques[0]).status,
        ChequeStatus::Deposited
    );
    let waits = CancelWait::waiting_of_lease(s.t.db(), s.lease.id)
        .await
        .unwrap();
    assert_eq!(waits.len(), 2);
    let tasks = s.t.tasks.with_key("property.cancel_cheques:");
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0].description.contains("200"),
        "{}",
        tasks[0].description
    );
    assert!(
        tasks[0].description.contains("201"),
        "{}",
        tasks[0].description
    );
    assert_eq!(s.t.refunds.asked().len(), 1, "only the paid rent so far");

    // The first clears: its amount joins the refunds.
    s.t.cheques.set_status(s.cheques[0], ChequeStatus::Cleared);
    s.t.state()
        .emit(PaymentReceived {
            payment: PaymentId(9),
            party: TENANT,
            amount: aed(25_000),
            method: PaymentMethod::Cheque,
            allocations: Vec::new(),
        })
        .await
        .unwrap();
    let refunds = s.t.refunds.asked();
    assert_eq!(refunds.len(), 2);
    assert_eq!(refunds[1].amount, aed(25_000));
    // The second bounces: nothing is refunded for it.
    s.t.cheques.set_status(s.cheques[1], ChequeStatus::Bounced);
    s.t.state()
        .emit(ChequeBounced {
            cheque: s.cheques[1],
            party: PartyId(TENANT.0),
            credit_group: None,
            amount: aed(25_000),
            honoured_amount: 0,
            open_amount: aed(25_000),
            currency: "AED".into(),
            reason: "insufficient funds".into(),
            presentation: 1,
            linked: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(s.t.refunds.asked().len(), 2);
    let outcomes: Vec<_> = CancelWait::query()
        .order_by(CancelWait::ID)
        .get(s.t.db())
        .await
        .unwrap()
        .into_iter()
        .map(|w| w.outcome)
        .collect();
    assert_eq!(outcomes, [WaitOutcome::Refunded, WaitOutcome::Bounced]);
}

/// AC8: a cheque received after the request, still undeposited at approval, is
/// returned with the others on the same receipt; a payment received after
/// the request joins the refund.
#[renox::test]
async fn bl_prp_01_the_money_is_counted_at_approval_not_at_request() {
    let s = scene().await;
    s.ask(&[]).await.assert_ok();
    let request = s.request().await;
    // A fourth cheque, and the second rent paid, while the request waits.
    let charges = s.lease.charges(s.t.db()).await.unwrap();
    let late = s.t.cheques.receive(
        TENANT,
        &s.lease.agreement(),
        "299",
        charges[3].due_date,
        charges[3].amount,
        &[],
    );
    let second = invoiced(&s.t, &s.lease, &charges[1], true).await;
    approve(s.t.state(), request.id, &s.manager, "")
        .await
        .unwrap();

    assert_eq!(s.t.cheques.get(late).status, ChequeStatus::Returned);
    assert_eq!(s.t.cheques.returns().len(), 1, "one receipt");
    assert_eq!(s.t.cheques.returns()[0].cheques.len(), 4);
    assert_eq!(
        s.t.invoices.get(second).unwrap().summary.status,
        InvoiceStatus::Credited
    );
    let refunds = s.t.refunds.asked();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, aed(50_000));
}

/// A draft invoice of a charge is discarded, not credited.
#[renox::test]
async fn bl_prp_01_a_draft_invoice_is_discarded() {
    let s = scene().await;
    let charges = s.lease.charges(s.t.db()).await.unwrap();
    let draft = invoiced(&s.t, &s.lease, &charges[1], false).await;
    s.ask(&[]).await.assert_ok();
    let request = s.request().await;
    approve(s.t.state(), request.id, &s.manager, "")
        .await
        .unwrap();
    assert!(s.t.invoices.get(draft).is_none());
    assert_eq!(
        s.t.invoices.get(s.invoice).unwrap().summary.status,
        InvoiceStatus::Credited
    );
}

/// A renewal version cancelled with money received never touches the deposit
/// of the lease it renews.
#[renox::test]
async fn bl_prp_01_cancelling_a_renewal_leaves_the_deposit_alone() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let v1 = t.active_lease(&unit, &agent, &manager).await;
    t.system(&v1, property::lease_machine::FLAG_RENEWAL).await;
    t.deposits.hold(&v1.agreement(), TENANT, aed(5_000));
    let v2 = property::leases::create_version(
        t.state(),
        None,
        v1.id,
        property::NewVersion::renewal(
            support::day(65),
            support::day(429),
            aed(110_000),
            aed(5_000),
            4,
        ),
    )
    .await
    .unwrap();
    // The deposit alone is not money received by the renewal.
    t.post(
        &agent,
        &format!("/property/leases/{}/cancel", v2.id),
        &[("reason", "tenant is leaving")],
    )
    .await
    .assert_ok();
    assert_eq!(t.status(&v2).await, LeaseStatus::Cancelled);
    assert!(t.deposits.settled().is_empty());
    assert_eq!(t.status(&v1).await, LeaseStatus::RenewalDue);
}
