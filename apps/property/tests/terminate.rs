//! BL-PRP-11 (#261): `terminate` records the move-out, vacates the unit,
//! cancels the charges after the move-out date, credits the invoices of later
//! periods pro rata, returns the later cheques, starts the deposit settlement,
//! turns the tenant's portal read-only (`LeaseTerminated`) and, 7 days later,
//! chases the Ejari cancellation (`PRP.RUL.13`).

mod support;

use property::charge_type::{self, EARLY_TERMINATION_FEE};
use property::notices::EJARI_CANCELLATION;
use property::termination::{self, cancel_task_key};
use property::{
    ChargeStatus, CreditChoice, Lease, LeaseCharge, LeaseStatus, NewLease, UnitStatus,
};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::cheques::{ChequeId, ChequeStatus};
use wl_kernel::{Subject, VatCategory};

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot, d};

struct Scene {
    t: Booted,
    building: property::Building,
    unit: property::Unit,
    agent: User,
    manager: User,
}

async fn scene() -> Scene {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    Scene {
        t,
        building,
        unit,
        agent,
        manager,
    }
}

impl Scene {
    /// An `active` lease 2027-01-01 to 2027-12-31 at AED 9,000 a month, with
    /// a deposit of AED 5,000 held.
    async fn leased(&self) -> Lease {
        let new = NewLease::new(self.unit.id, TENANT, d(1, 1), d(12, 31), aed(108_000), 12);
        let lease = property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap();
        self.t.submit(&self.agent, &lease).await.assert_ok();
        self.t.activate(&self.manager, &lease).await.assert_ok();
        self.t.deposits.hold(&lease.agreement(), TENANT, aed(5_000));
        self.t.reload(&lease).await
    }

    /// The daily run drafts the invoices due by `day`; they are issued and
    /// paid.
    async fn invoice_and_pay(&self, day: NaiveDate) {
        property::invoicing::run(self.t.state(), day).await.unwrap();
        for invoice in self.t.invoices.all() {
            let id = invoice.summary.id.0;
            self.t.invoices.issue(id);
            self.t.invoices.pay(
                id,
                &Subject::new("payments.payment", id),
                invoice.summary.gross_total,
            );
        }
    }

    async fn charges(&self, lease: &Lease) -> Vec<LeaseCharge> {
        let mut all = lease.charges(self.t.db()).await.unwrap();
        all.sort_by_key(|c| (c.due_date, c.id));
        all
    }

    /// A cheque for each charge due on or after `from`.
    async fn cheques_from(&self, lease: &Lease, from: NaiveDate) -> Vec<(ChequeId, NaiveDate)> {
        let mut made = Vec::new();
        for charge in self.charges(lease).await.iter().filter(|c| c.due_date >= from) {
            let id = self.t.cheques.receive(
                TENANT,
                &lease.agreement(),
                &format!("C-{}", charge.due_date),
                charge.due_date,
                charge.amount,
                &[charge.subject()],
            );
            made.push((id, charge.due_date));
        }
        made
    }

    /// The tenant's notice (end of term, leaving 2027-07-20), the move-out
    /// inspection and the keys.
    async fn notice_and_inspection(&self, lease: &Lease, defects: &str) {
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/notice", lease.id),
                &[
                    ("date", "2027-06-15"),
                    ("party", "tenant"),
                    ("reason", "end_of_term"),
                    ("planned_move_out", "2027-07-20"),
                ],
            )
            .await
            .assert_ok();
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/move-out-inspection", lease.id),
                &[
                    ("done_on", "2027-07-19"),
                    ("keys_returned_on", "2027-07-20"),
                    ("defects", defects),
                ],
            )
            .await
            .assert_ok();
    }

    async fn terminate(
        &self,
        lease: &Lease,
        pairs: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/terminate", lease.id),
                pairs,
            )
            .await
    }
}

/// AC1: monthly invoices of AED 9,000 issued up to July 2027 and paid, the
/// charges from August scheduled and five cheques received for August to
/// December. With the move-out on 2027-07-20 the charges from August are
/// cancelled, the July invoice gets a credit note of AED 3,193.55 (11 of 31
/// days), the five cheques are returned with a receipt, the deposit settlement
/// starts, the unit is vacated and the portal event is emitted.
#[renox::test]
async fn bl_prp_11_terminating_settles_the_charges_the_invoices_and_the_cheques() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice_and_pay(d(7, 1)).await;
    let cheques = s.cheques_from(&lease, d(8, 1)).await;
    assert_eq!(cheques.len(), 5);
    s.notice_and_inspection(&lease, "no").await;
    assert_eq!(s.t.unit_now(&s.unit).await.status, UnitStatus::NoticeGiven);

    s.terminate(&lease, &[("move_out", "2027-07-20"), ("credit", "refund")])
        .await
        .assert_ok();

    let after = s.t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::Terminated);
    assert_eq!(after.actual_move_out, Some(d(7, 20)));
    assert_eq!(after.credit_choice, CreditChoice::Refund);
    assert!(after.terminated_on.is_some());

    // The charges: January to July invoiced, August to December cancelled.
    let charges = s.charges(&after).await;
    assert_eq!(charges.len(), 12);
    for charge in &charges {
        let expected = if charge.due_date > d(7, 20) {
            ChargeStatus::Cancelled
        } else {
            ChargeStatus::Invoiced
        };
        assert_eq!(charge.status, expected, "{}", charge.due_date);
    }

    // The July invoice: 11 of 31 days, AED 3,193.55; nothing else credited.
    let july = charges.iter().find(|c| c.due_date == d(7, 1)).unwrap();
    let invoice_id = july.invoice_id.expect("the July invoice");
    let credits = s.t.invoices.credits();
    assert_eq!(credits.len(), 1);
    assert_eq!(credits[0].invoice, invoice_id);
    assert_eq!(credits[0].lines.len(), 1);
    assert_eq!(credits[0].lines[0].net, 319_355);
    assert!(credits[0].reason.contains("moved out"), "{}", credits[0].reason);
    assert_eq!(s.t.invoices.credited(invoice_id), 319_355);
    assert_eq!(after.termination_credit, 319_355);

    // The five cheques are returned against one receipt.
    for (id, _) in &cheques {
        assert_eq!(s.t.cheques.get(*id).status, ChequeStatus::Returned);
    }
    let returns = s.t.cheques.returns();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].cheques.len(), 5);

    // The deposit settlement started; the unit is vacated.
    assert_eq!(s.t.deposits.started(), vec![lease.agreement()]);
    assert_eq!(s.t.unit_now(&s.unit).await.status, UnitStatus::Available);

    // The tenant's portal turns read-only on the event.
    let events = s.t.terminated.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].move_out, d(7, 20));
    assert_eq!(events[0].credited, 319_355);
    assert_eq!(events[0].tenant, TENANT);

    // The page shows the credit.
    s.t.app.acting_as(&s.manager);
    s.t.app
        .get(&format!("/property/leases/{}", lease.id))
        .await
        .assert_ok()
        .assert_see("AED 3,193.55");

    // The money effects run once.
    let again = termination::settle(s.t.state(), lease.id, None).await.unwrap();
    assert!(!again.ran);
    assert_eq!(s.t.invoices.credits().len(), 1);
}

/// AC2: with `rent_due_until` 2027-09-30 the charges and cheques of August and
/// September stay; only October to December are cancelled and returned; July
/// is not credited.
#[renox::test]
async fn bl_prp_11_rent_due_until_keeps_the_periods_the_contract_keeps_due() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice_and_pay(d(7, 1)).await;
    let cheques = s.cheques_from(&lease, d(8, 1)).await;
    s.notice_and_inspection(&lease, "no").await;

    s.terminate(
        &lease,
        &[
            ("move_out", "2027-07-20"),
            ("rent_due_until", "2027-09-30"),
            ("credit", "keep"),
        ],
    )
    .await
    .assert_ok();

    let after = s.t.reload(&lease).await;
    assert_eq!(after.rent_due_until, Some(d(9, 30)));
    assert_eq!(after.credit_choice, CreditChoice::Keep);
    for charge in s.charges(&after).await {
        let expected = if charge.due_date > d(9, 30) {
            ChargeStatus::Cancelled
        } else if charge.due_date >= d(8, 1) {
            ChargeStatus::Scheduled
        } else {
            ChargeStatus::Invoiced
        };
        assert_eq!(charge.status, expected, "{}", charge.due_date);
    }
    assert!(s.t.invoices.credits().is_empty(), "July is not credited");
    for (id, date) in &cheques {
        let expected = if *date > d(9, 30) {
            ChequeStatus::Returned
        } else {
            ChequeStatus::Received
        };
        assert_eq!(s.t.cheques.get(*id).status, expected, "{date}");
    }
    assert_eq!(s.t.cheques.returns()[0].cheques.len(), 3);
}

/// A draft invoice of a period wholly after the move-out is discarded and its
/// charge cancelled.
#[renox::test]
async fn bl_prp_11_a_draft_invoice_of_a_later_period_is_discarded() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice_and_pay(d(7, 1)).await;
    // August is drafted but not issued.
    property::invoicing::run(s.t.state(), d(8, 1)).await.unwrap();
    let august = s
        .charges(&lease)
        .await
        .into_iter()
        .find(|c| c.due_date == d(8, 1))
        .unwrap();
    let draft_id = august.invoice_id.expect("the August draft");
    assert!(s.t.invoices.get(draft_id).is_some());
    s.notice_and_inspection(&lease, "no").await;

    s.terminate(&lease, &[("move_out", "2027-07-20")]).await.assert_ok();

    assert!(s.t.invoices.get(draft_id).is_none(), "the draft is discarded");
    let after = s
        .charges(&lease)
        .await
        .into_iter()
        .find(|c| c.id == august.id)
        .unwrap();
    assert_eq!(after.status, ChargeStatus::Cancelled);
    // Only July was credited.
    assert_eq!(s.t.invoices.credits().len(), 1);
}

/// A one-off charge stays: the early termination penalty due after the
/// move-out date is not cancelled.
#[renox::test]
async fn bl_prp_11_a_one_off_penalty_is_not_cancelled() {
    let s = scene().await;
    let lease = s.leased().await;
    charge_type::configure(
        s.t.db(),
        EARLY_TERMINATION_FEE,
        "Early termination fee",
        VatCategory::Exempt,
    )
    .await
    .unwrap();
    // The tenant leaves earlier than planned.
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/notice", lease.id),
        &[
            ("date", "2027-06-15"),
            ("party", "tenant"),
            ("reason", "early_termination"),
            ("planned_move_out", "2027-08-31"),
            ("penalty", "20000"),
        ],
    )
    .await
    .assert_ok();
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/move-out-inspection", lease.id),
        &[("done_on", "2027-07-19"), ("keys_returned_on", "2027-07-20")],
    )
    .await
    .assert_ok();

    s.terminate(&lease, &[("move_out", "2027-07-20")]).await.assert_ok();

    let charges = s.charges(&lease).await;
    let penalty = charges
        .iter()
        .find(|c| c.charge_type == EARLY_TERMINATION_FEE)
        .unwrap();
    assert_eq!(penalty.status, ChargeStatus::Scheduled);
    assert_eq!(penalty.due_date, d(8, 31), "the actual date does not move it");
    let cancelled = charges
        .iter()
        .filter(|c| c.status == ChargeStatus::Cancelled)
        .count();
    assert_eq!(cancelled, 5, "the rent of August to December");
}

/// Defects found by the inspection send the unit to maintenance.
#[renox::test]
async fn bl_prp_11_defects_send_the_unit_to_maintenance() {
    let s = scene().await;
    let lease = s.leased().await;
    s.notice_and_inspection(&lease, "yes").await;
    s.terminate(&lease, &[("move_out", "2027-07-20")]).await.assert_ok();
    assert_eq!(
        s.t.unit_now(&s.unit).await.status,
        UnitStatus::UnderMaintenance
    );
}

/// `terminate` needs the inspection completed and all the keys back, and the
/// move-out date.
#[renox::test]
async fn bl_prp_11_terminating_needs_the_inspection_and_the_keys() {
    let s = scene().await;
    let lease = s.leased().await;
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/notice", lease.id),
        &[
            ("date", "2027-06-15"),
            ("party", "tenant"),
            ("reason", "end_of_term"),
            ("planned_move_out", "2027-07-20"),
        ],
    )
    .await
    .assert_ok();
    let refused = s.terminate(&lease, &[("move_out", "2027-07-20")]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("inspection"), "{}", refused.text());

    // The inspection without the keys.
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/move-out-inspection", lease.id),
        &[("done_on", "2027-07-19")],
    )
    .await
    .assert_ok();
    let refused = s.terminate(&lease, &[("move_out", "2027-07-20")]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("keys"), "{}", refused.text());
    assert_eq!(s.t.status(&lease).await, LeaseStatus::NoticeGiven);
    assert!(s.t.cheques.returns().is_empty());
    assert!(s.t.deposits.started().is_empty());

    // The inspection also closes its ticket.
    let tickets = s.t.tasks.with_key(&property::notice::move_out_key(&lease));
    assert!(!tickets[0].open);

    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/move-out-inspection", lease.id),
        &[("keys_returned_on", "2027-07-20")],
    )
    .await
    .assert_ok();
    s.terminate(&lease, &[]).await.assert_status(422);
    s.terminate(&lease, &[("move_out", "2027-07-20")]).await.assert_ok();
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Terminated);
}

/// AC3: with an Ejari number and no cancellation date the property manager is
/// notified 7 days after the lease was terminated, once; with the cancellation
/// recorded before, nothing is sent.
#[renox::test]
async fn bl_prp_11_the_ejari_cancellation_is_chased_after_seven_days() {
    let s = scene().await;
    let lease = s.leased().await;
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/ejari", lease.id),
        &[("number", "EJ-4471"), ("registered_on", "2027-01-05")],
    )
    .await
    .assert_ok();
    s.notice_and_inspection(&lease, "no").await;
    s.terminate(&lease, &[("move_out", "2027-07-20")]).await.assert_ok();
    // The lease became `terminated` on 2027-07-20.
    s.t.set(&lease, &[("terminated_on", &Some(d(7, 20)))]).await;

    assert_eq!(termination::chase_ejari(s.t.state(), d(7, 26)).await.unwrap(), 0);
    assert!(s.t.notices.of_type(EJARI_CANCELLATION).is_empty());

    assert_eq!(termination::chase_ejari(s.t.state(), d(7, 27)).await.unwrap(), 1);
    let told = s.t.notices.of_type(EJARI_CANCELLATION);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["ejari_number"], "EJ-4471");
    let key = cancel_task_key(&lease);
    let tasks = s.t.tasks.with_key(&key);
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].open);
    // Once only.
    assert_eq!(termination::chase_ejari(s.t.state(), d(7, 28)).await.unwrap(), 0);
    assert_eq!(s.t.notices.of_type(EJARI_CANCELLATION).len(), 1);

    // Recording the cancellation closes the task.
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/ejari-cancellation", lease.id),
        &[("cancelled_on", "2027-07-28")],
    )
    .await
    .assert_ok();
    assert_eq!(
        s.t.reload(&lease).await.ejari_cancelled_on,
        Some(d(7, 28))
    );
    assert!(!s.t.tasks.with_key(&key)[0].open);
}

/// With the cancellation date recorded on 2027-07-25, nothing is sent on
/// 2027-07-27; a lease without a registration number is not chased either.
#[renox::test]
async fn bl_prp_11_a_cancelled_or_unregistered_ejari_is_not_chased() {
    let s = scene().await;
    let lease = s.leased().await;
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/ejari", lease.id),
        &[("number", "EJ-4471"), ("registered_on", "2027-01-05")],
    )
    .await
    .assert_ok();
    s.notice_and_inspection(&lease, "no").await;
    s.terminate(&lease, &[("move_out", "2027-07-20")]).await.assert_ok();
    s.t.set(&lease, &[("terminated_on", &Some(d(7, 20)))]).await;
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/ejari-cancellation", lease.id),
        &[("cancelled_on", "2027-07-25")],
    )
    .await
    .assert_ok();
    assert_eq!(termination::chase_ejari(s.t.state(), d(7, 27)).await.unwrap(), 0);
    assert!(s.t.notices.of_type(EJARI_CANCELLATION).is_empty());
    assert!(s.t.tasks.with_key(&cancel_task_key(&lease)).is_empty());

    // No registration number: nothing to cancel.
    let other_unit = s.t.unit(&s.building, "1300").await;
    let new = NewLease::new(other_unit.id, TENANT, d(1, 1), d(12, 31), aed(108_000), 12);
    let other = property::leases::create_draft(s.t.state(), None, new).await.unwrap();
    s.t.submit(&s.agent, &other).await.assert_ok();
    s.t.activate(&s.manager, &other).await.assert_ok();
    s.t.set(
        &other,
        &[
            ("status", &LeaseStatus::Terminated),
            ("terminated_on", &Some(d(7, 20))),
        ],
    )
    .await;
    assert_eq!(termination::chase_ejari(s.t.state(), d(7, 27)).await.unwrap(), 0);
}

/// Only a property manager of the building terminates a lease.
#[renox::test]
async fn bl_prp_11_only_a_manager_terminates() {
    let s = scene().await;
    let lease = s.leased().await;
    s.notice_and_inspection(&lease, "no").await;
    s.t.post(
        &s.agent,
        &format!("/property/leases/{}/terminate", lease.id),
        &[("move_out", "2027-07-20")],
    )
    .await
    .assert_forbidden();
    assert_eq!(s.t.status(&lease).await, LeaseStatus::NoticeGiven);
}
