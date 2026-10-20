//! The cheque register (wl-payments §4 *Cheque*, §5.3; base of BL-PAY-09 to
//! BL-PAY-14): the post-dated cheques of a lease are registered through the
//! kernel contract, put on the deposit list a day before their date,
//! deposited from their date (a field cheque only after a cash handover,
//! `FIN.ACL.20`) and cleared into a payment allocated to the invoice they
//! pay.

mod support;

use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::cheques::{
    ChequeSchedule, ChequeStatus, RegisterCheque, ScheduledCheque, cheques as cheque_contract,
};
use wl_kernel::documents::today;
use wl_kernel::payments::{Direction, PaymentMethod, PaymentSource};
use wl_kernel::{PartyId, Subject};
use wl_payments::cheque_deposit::flag_due_run;
use wl_payments::{Cheque, Payment, WlPayments};

use support::{BLUE_DUNES, QUIET_SANDS, boot, boot_with};
use wl_payments::gateway::FakeGateway;

fn lease() -> Subject {
    Subject::new("property.lease", 9)
}

async fn schedule(t: &support::Booted, numbers: &[&str]) -> ChequeSchedule {
    let entity = t.entity("PGP").await;
    let mut schedule = ChequeSchedule::new(lease(), BLUE_DUNES, entity.id, "ENBD");
    for (i, number) in numbers.iter().enumerate() {
        let date = today() + TimeDelta::days(30 * (i as i64 + 1));
        schedule = schedule.cheque(
            ScheduledCheque::new(number, date, 2_000_000)
                .link(Subject::new("invoicing.charge", i as i64 + 1)),
        );
    }
    schedule
}

/// A lease's post-dated cheques are registered in one go, each linked to the
/// charge it pays, and read back per agreement in date order.
#[renox::test]
async fn the_post_dated_cheques_of_a_lease_are_registered_together() {
    let t = boot().await;
    let schedule = schedule(&t, &["1001", "1002", "1003"]).await;
    let contract = cheque_contract(t.state());
    let registered = contract
        .register_schedule(t.state(), schedule, None)
        .await
        .unwrap();
    assert_eq!(registered.len(), 3);
    assert!(
        registered
            .iter()
            .all(|c| c.status == ChequeStatus::Received)
    );
    assert!(registered.iter().all(|c| c.agreement == Some(lease())));
    assert_eq!(
        registered[1].linked,
        vec![Subject::new("invoicing.charge", 2)]
    );
    assert_eq!(registered[0].open_amount, 2_000_000);

    let read = contract.of_agreement(t.db(), &lease()).await.unwrap();
    let numbers: Vec<&str> = read.iter().map(|c| c.number.as_str()).collect();
    assert_eq!(numbers, ["1001", "1002", "1003"]);
    let found = contract
        .find(t.db(), read[2].id)
        .await
        .unwrap()
        .expect("found");
    assert_eq!(found.amount, 2_000_000);
    assert!(contract.find(t.db(), 999.into()).await.unwrap().is_none());
}

/// One refused cheque refuses the whole schedule: nothing is registered.
#[renox::test]
async fn a_schedule_with_a_refused_cheque_registers_none() {
    let t = boot().await;
    let entity = t.entity("PGP").await;
    let contract = cheque_contract(t.state());
    let mut bad = schedule(&t, &["2001", "2002"]).await;
    bad.cheques[1].amount = 0;
    assert!(
        contract
            .register_schedule(t.state(), bad, None)
            .await
            .is_err()
    );
    assert!(
        contract
            .of_agreement(t.db(), &lease())
            .await
            .unwrap()
            .is_empty()
    );

    let mut twice = schedule(&t, &["2001", "2001"]).await;
    twice.legal_entity_id = entity.id;
    assert!(
        contract
            .register_schedule(t.state(), twice, None)
            .await
            .is_err()
    );
    assert!(
        contract
            .of_agreement(t.db(), &lease())
            .await
            .unwrap()
            .is_empty()
    );

    let empty = ChequeSchedule::new(lease(), BLUE_DUNES, entity.id, "ENBD");
    assert!(
        contract
            .register_schedule(t.state(), empty, None)
            .await
            .is_err()
    );
}

/// A cheque number is registered once per party and bank, and an unknown
/// party is refused.
#[renox::test]
async fn a_cheque_is_registered_once_and_for_a_known_party() {
    let t = boot().await;
    let entity = t.entity("PGP").await;
    let contract = cheque_contract(t.state());
    let request = RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "3001", today(), 100_000);
    contract
        .register(t.state(), request.clone(), None)
        .await
        .unwrap();
    assert!(
        contract
            .register(t.state(), request.clone(), None)
            .await
            .is_err()
    );
    // Another party or bank may use the same number.
    let other = RegisterCheque::new(QUIET_SANDS, entity.id, "ENBD", "3001", today(), 100_000);
    assert!(contract.register(t.state(), other, None).await.is_ok());
    let ghost = RegisterCheque::new(PartyId(99), entity.id, "ENBD", "3002", today(), 100_000);
    assert!(contract.register(t.state(), ghost, None).await.is_err());
    let free = RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "3003", today(), 0);
    assert!(contract.register(t.state(), free, None).await.is_err());
}

/// `FIN.RUL.03`: a received cheque goes on the deposit list the day before
/// its date; an issued cheque does not.
#[renox::test]
async fn a_cheque_goes_on_the_deposit_list_one_day_before_its_date() {
    let t = boot().await;
    let entity = t.entity("PGP").await;
    let soon = today() + TimeDelta::days(1);
    let later = today() + TimeDelta::days(2);
    let on_time = t
        .register_cheque(RegisterCheque::new(
            BLUE_DUNES, entity.id, "ENBD", "4001", soon, 100_000,
        ))
        .await;
    let early = t
        .register_cheque(RegisterCheque::new(
            BLUE_DUNES, entity.id, "ENBD", "4002", later, 100_000,
        ))
        .await;
    let issued = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "4003", soon, 100_000).issued(),
        )
        .await;
    assert_eq!(flag_due_run(t.state(), today()).await.unwrap(), 1);
    assert_eq!(
        t.cheque(on_time.id).await.status,
        ChequeStatus::DueForDeposit
    );
    assert_eq!(t.cheque(early.id).await.status, ChequeStatus::Received);
    assert_eq!(t.cheque(issued.id).await.status, ChequeStatus::Received);
    // A second run changes nothing.
    assert_eq!(flag_due_run(t.state(), today()).await.unwrap(), 0);
    // The day before the next one it is listed too.
    assert_eq!(
        flag_due_run(t.state(), today() + TimeDelta::days(1))
            .await
            .unwrap(),
        1
    );
    assert_eq!(t.cheque(early.id).await.status, ChequeStatus::DueForDeposit);
}

/// A cheque is deposited from its date, by a user with `cheques.deposit`.
#[renox::test]
async fn a_cheque_is_deposited_from_its_date() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let entity = t.entity("PGP").await;
    let future = today() + TimeDelta::days(10);
    let post_dated = t
        .register_cheque(RegisterCheque::new(
            BLUE_DUNES, entity.id, "ENBD", "5001", future, 100_000,
        ))
        .await;
    t.cheque_action(&staff, &post_dated, "deposit", &[])
        .await
        .assert_status(422);
    assert_eq!(t.cheque(post_dated.id).await.status, ChequeStatus::Received);

    let due = t.received_cheque(BLUE_DUNES, "5002", 100_000, None).await;
    let deposited = t.deposit_cheque(&staff, &due).await;
    assert_eq!(deposited.status, ChequeStatus::Deposited);
    assert_eq!(deposited.presentations, 1);
    assert_eq!(deposited.deposit_date, Some(today()));
    // Not twice.
    t.cheque_action(&staff, &deposited, "deposit", &[])
        .await
        .assert_status(422);

    let viewer = t
        .user("Vera", &[wl_payments::permissions::CHEQUES_VIEW])
        .await;
    let other = t.received_cheque(BLUE_DUNES, "5003", 100_000, None).await;
    t.cheque_action(&viewer, &other, "deposit", &[])
        .await
        .assert_forbidden();
}

/// FIN.ACL.20: a cheque collected in the field is deposited only after a
/// cash handover confirms it; the rule can be turned off.
#[renox::test]
async fn a_field_cheque_waits_for_its_handover() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let driver = t.user("Dara", &[]).await;
    let entity = t.entity("PGP").await;
    let field = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "6001", today(), 100_000)
                .collected_by(driver.id, true),
        )
        .await;
    assert!(field.field_collected);
    let refused = t.cheque_action(&staff, &field, "deposit", &[]).await;
    refused.assert_status(422);
    assert_eq!(t.cheque(field.id).await.status, ChequeStatus::Received);

    // A cheque entered at the office by the same person is not a field cheque.
    let office = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "6002", today(), 100_000)
                .collected_by(driver.id, false),
        )
        .await;
    assert_eq!(
        t.deposit_cheque(&staff, &office).await.status,
        ChequeStatus::Deposited
    );

    // The rule off.
    let loose = boot_with(
        WlPayments::new().field_cheque_handover(false),
        FakeGateway::new(),
    )
    .await;
    let staff = loose.cheque_staff("Amal").await;
    let entity = loose.entity("PGP").await;
    let field = loose
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "6003", today(), 100_000)
                .collected_by(staff.id, true),
        )
        .await;
    assert_eq!(
        loose.deposit_cheque(&staff, &field).await.status,
        ChequeStatus::Deposited
    );
}

/// `clear` records a payment for the open amount (cheque method, money in)
/// and allocates it to the invoice the cheque pays.
#[renox::test]
async fn clearing_a_received_cheque_records_and_allocates_its_payment() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let cheque = t
        .received_cheque(BLUE_DUNES, "7001", 2_000_000, Some(rent))
        .await;
    // Not before it is deposited.
    t.cheque_action(&staff, &cheque, "clear", &[])
        .await
        .assert_status(422);
    let cheque = t.deposit_cheque(&staff, &cheque).await;
    t.cheque_action(&staff, &cheque, "clear", &[])
        .await
        .assert_ok();

    let cleared = t.cheque(cheque.id).await;
    assert_eq!(cleared.status, ChequeStatus::Cleared);
    assert_eq!(cleared.open_amount, 0);
    let payments = Payment::query()
        .where_(Payment::SOURCE.eq(PaymentSource::Cheque))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    let payment = &payments[0];
    assert_eq!(payment.amount, 2_000_000);
    assert_eq!(payment.method, PaymentMethod::Cheque);
    assert_eq!(payment.direction, Direction::In);
    assert_eq!(payment.detail("number").as_deref(), Some("7001"));
    assert_eq!(payment.detail("bank").as_deref(), Some("ENBD"));
    let calls = t.invoices.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].invoice, calls[0].amount), (rent, 2_000_000));
    assert_eq!(calls[0].source, payment.subject());
    assert_eq!(t.received.all().len(), 1, "PaymentReceived");
    // A first presentation that clears is not a resolved bounce.
    assert!(t.cheque_resolved.all().is_empty());
}

/// An issued cheque that clears records an outgoing payment.
#[renox::test]
async fn clearing_an_issued_cheque_records_an_outgoing_payment() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let entity = t.entity("PGP").await;
    let cheque = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "8001", today(), 1_050_000)
                .issued()
                .link(Subject::new("invoicing.supplier_bill", 4)),
        )
        .await;
    let cheque = t.deposit_cheque(&staff, &cheque).await;
    t.cheque_action(&staff, &cheque, "clear", &[])
        .await
        .assert_ok();
    let payments = Payment::query()
        .where_(Payment::SOURCE.eq(PaymentSource::Cheque))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].direction, Direction::Out);
    assert_eq!(payments[0].amount, 1_050_000);
    assert!(
        t.received.all().is_empty(),
        "money out is not a payment received"
    );
    // Without invoicing's supplier bills the payment stays unallocated.
    assert!(t.allocations(&payments[0]).await.is_empty());
    let found: Cheque = t.cheque(cheque.id).await;
    assert_eq!(found.status, ChequeStatus::Cleared);
}
