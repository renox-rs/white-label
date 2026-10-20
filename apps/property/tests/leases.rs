//! PRP.LSE.01, PRP.LSE.02, PRP.ACL.04 and the happy path of the lease
//! lifecycle (01 Property §4.1, §7.1): draft, submit, activate, the charge
//! schedule, the invoice of an instalment, the cheque schedule, the Ejari
//! registration.

mod support;

use property::cheques::{ChequeEntry, of_version, register_cheques};
use property::invoicing;
use property::lease_machine::OFFERED;
use property::leases::{DraftEdit, NewLease, create_draft, revise_draft};
use property::{ChargeStatus, LeaseStatus, UnitStatus, UnitUse, VersionKind};
use renox::prelude::*;
use wl_kernel::cheques::ChequeStatus;
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};

use support::{AGENT, COMPANY, MANAGER, TENANT, aed, boot, d, day};

/// PRP.LSE.01: a draft lease is version 1 of a new lease with a rent plan
/// that adds up to the annual rent.
#[renox::test]
async fn prp_lse_01_a_draft_has_its_rent_plan() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let lease = t.lease(&unit, 10).await;

    assert_eq!(lease.status, LeaseStatus::Draft);
    assert_eq!((lease.version, lease.version_kind), (1, VersionKind::New));
    assert_eq!(lease.root_id, lease.id);
    assert_eq!(lease.number(), format!("L-{:06}", lease.id));
    assert_eq!(lease.tenant(), TENANT);
    assert_eq!(lease.building_id, building.id);
    assert_eq!(lease.vat_category, VatCategory::Exempt);
    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::Available);

    let charges = lease.charges(t.db()).await.unwrap();
    assert_eq!(charges.len(), 4);
    assert!(charges.iter().all(|c| c.status == ChargeStatus::Planned));
    assert!(charges.iter().all(|c| c.amount == aed(25_000)));
    assert_eq!(charges[0].due_date, day(10));
    assert_eq!(charges[0].description, "Rent 1/4");
    assert_eq!(property::charge::plan_total(&charges), lease.annual_rent);
}

/// A commercial unit's rent is standard-rated; bad terms are refused.
#[renox::test]
async fn prp_lse_01_the_vat_follows_the_use_and_bad_terms_are_refused() {
    let t = boot().await;
    let building = t.building().await;
    let shop = t.unit_of(&building, "S1", UnitUse::Commercial).await;
    let lease = t.lease(&shop, 0).await;
    assert_eq!(lease.vat_category, VatCategory::Standard);
    let charges = lease.charges(t.db()).await.unwrap();
    assert!(
        charges
            .iter()
            .all(|c| c.vat_category == VatCategory::Standard)
    );

    let unit = t.unit(&building, "1204").await;
    let ok = || NewLease::new(unit.id, TENANT, d(1, 1), d(12, 31), aed(100_000), 4);
    let mut backwards = ok();
    backwards.end_date = d(1, 1);
    let mut no_rent = ok();
    no_rent.annual_rent = 0;
    let mut too_many = ok();
    too_many.instalments = 13;
    let mut none = ok();
    none.instalments = 0;
    let mut stranger = ok();
    stranger.tenant = wl_kernel::PartyId(404);
    let mut no_unit = ok();
    no_unit.unit_id = 4040;
    for bad in [backwards, no_rent, too_many, none, stranger, no_unit] {
        let refused = create_draft(t.state(), None, bad).await;
        assert!(
            matches!(
                refused,
                Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
            ),
            "{refused:?}"
        );
    }
}

/// PRP.ACL.04: a lease is edited only in draft; a new rent replans the
/// charges.
#[renox::test]
async fn prp_acl_04_only_a_draft_is_edited() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let lease = t.lease(&unit, 10).await;

    let edited = revise_draft(
        t.state(),
        lease.id,
        DraftEdit {
            annual_rent: Some(aed(120_000)),
            instalments: Some(2),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(edited.annual_rent, aed(120_000));
    let live: Vec<_> = edited
        .charges(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|c| c.status != ChargeStatus::Cancelled)
        .collect();
    assert_eq!(live.len(), 2);
    assert_eq!(property::charge::plan_total(&live), aed(120_000));

    t.submit(&agent, &lease).await.assert_ok();
    let refused = revise_draft(t.state(), lease.id, DraftEdit::default()).await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
}

/// Submit: the unit must be available, the plan must add up, and the tenant's
/// documents must be present (the document guard, by the kind of the tenant).
#[renox::test]
async fn prp_lse_01_submit_checks_the_unit_the_plan_and_the_tenants_documents() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;

    // The plan must add up to the annual rent.
    let lease = t.lease(&unit, 10).await;
    t.set(&lease, &[("annual_rent", &aed(90_000))]).await;
    let refused = t.submit(&agent, &lease).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("payment plan"),
        "{}",
        refused.text()
    );
    assert_eq!(t.status(&lease).await, LeaseStatus::Draft);
    t.set(&lease, &[("annual_rent", &aed(100_000))]).await;

    // A person's identity documents are asked for; a company's are not.
    t.docs.block_if(
        "emirates_id",
        "lease.submit",
        Some(("tenant_kind", &["person"])),
    );
    let refused = t.submit(&agent, &lease).await;
    refused.assert_status(422);
    assert!(refused.text().contains("emirates_id"), "{}", refused.text());
    t.docs.accept(&TENANT.subject(), "emirates_id");
    t.submit(&agent, &lease).await.assert_ok();
    let submitted = t.reload(&lease).await;
    assert_eq!(submitted.status, LeaseStatus::PendingSignature);
    assert_eq!(submitted.submitted_on, Some(support::day(0)));

    let company_unit = t.unit(&building, "1205").await;
    let new = NewLease::new(company_unit.id, COMPANY, d(1, 1), d(12, 31), aed(80_000), 2);
    let company_lease = create_draft(t.state(), None, new).await.unwrap();
    t.submit(&agent, &company_lease).await.assert_ok();
    assert_eq!(
        t.status(&company_lease).await,
        LeaseStatus::PendingSignature
    );
}

/// A unit that is not available cannot take a new lease.
#[renox::test]
async fn prp_lse_01_a_new_lease_needs_an_available_unit() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    t.docs.block("signed_tenancy_contract", "lease.activate");
    let first = t.pending(&agent, &unit, 0).await;
    t.docs.accept(&first.subject(), "signed_tenancy_contract");
    t.waive(&manager, &first).await;
    t.activate(&manager, &first).await.assert_ok();
    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::Occupied);

    let second = t.lease(&unit, 400).await;
    let refused = t.submit(&agent, &second).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("not available"),
        "{}",
        refused.text()
    );
}

/// Activate: the signed contract is needed, the deposit must be received or
/// waived, and only a user with `leases.activate` may.
#[renox::test]
async fn prp_lse_01_activate_needs_the_signed_contract_and_the_deposit() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    t.docs.block("signed_tenancy_contract", "lease.activate");
    let lease = t.pending(&agent, &unit, 0).await;

    // An agent drafts but does not activate.
    t.activate(&agent, &lease).await.assert_forbidden();
    // No signed contract.
    let refused = t.activate(&manager, &lease).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("signed_tenancy_contract"),
        "{}",
        refused.text()
    );
    t.docs.accept(&lease.subject(), "signed_tenancy_contract");
    // No deposit.
    let refused = t.activate(&manager, &lease).await;
    refused.assert_status(422);
    assert!(refused.text().contains("deposit"), "{}", refused.text());
    assert_eq!(t.status(&lease).await, LeaseStatus::PendingSignature);

    // The deposit is held for the lease.
    t.deposits.hold(&lease.agreement(), TENANT, aed(5_000));
    t.activate(&manager, &lease).await.assert_ok();
    let active = t.reload(&lease).await;
    assert_eq!(active.status, LeaseStatus::Active);
    assert_eq!(active.activated_on, Some(day(0)));
}

/// A property manager waives the deposit, and activation does not wait.
#[renox::test]
async fn prp_lse_01_a_waived_deposit_does_not_block_activation() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let lease = t.pending(&agent, &unit, 0).await;
    // The agent cannot waive.
    t.post(
        &agent,
        &format!("/property/leases/{}/waive-deposit", lease.id),
        &[],
    )
    .await
    .assert_forbidden();
    t.waive(&manager, &lease).await;
    assert!(t.reload(&lease).await.deposit_waived);
    t.activate(&manager, &lease).await.assert_ok();
    assert_eq!(t.status(&lease).await, LeaseStatus::Active);
}

/// What `activate` does: the unit is occupied, the charges are scheduled,
/// `LeaseActivated` is emitted and the Ejari task is opened for the property
/// manager.
#[renox::test]
async fn prp_lse_01_activate_occupies_the_unit_schedules_the_charges_and_opens_the_ejari_task() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let lease = t.pending(&agent, &unit, 0).await;
    t.waive(&manager, &lease).await;
    t.activate(&manager, &lease).await.assert_ok();

    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::Occupied);
    let charges = t.reload(&lease).await.charges(t.db()).await.unwrap();
    assert!(charges.iter().all(|c| c.status == ChargeStatus::Scheduled));

    let events = t.activated.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].lease, lease.id);
    assert_eq!(events[0].tenant, TENANT);
    assert_eq!(events[0].annual_rent, aed(100_000));

    let tasks = t.tasks.with_key("property.ejari:");
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].open);
    assert!(
        tasks[0].title.contains(&lease.number()),
        "{}",
        tasks[0].title
    );
    assert_eq!(tasks[0].subject, lease.subject());
    assert_eq!(
        tasks[0].assignee,
        Some(wl_kernel::tickets::Assignee::user(manager.id))
    );
}

/// PRP.LSE.02: the Ejari number is typed in and closes the task; a blank
/// number, a lease that is not in force and a user without the permission are
/// refused.
#[renox::test]
async fn prp_lse_02_the_ejari_registration_is_recorded() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let lease = t.pending(&agent, &unit, 0).await;

    // Not in force yet.
    let path = format!("/property/leases/{}/ejari", lease.id);
    let form = [("number", "EJ-9981"), ("registered_on", "2027-01-05")];
    t.post(&manager, &path, &form).await.assert_status(422);

    t.waive(&manager, &lease).await;
    t.activate(&manager, &lease).await.assert_ok();
    t.post(&agent, &path, &form).await.assert_forbidden();
    t.post(
        &manager,
        &path,
        &[("number", " "), ("registered_on", "2027-01-05")],
    )
    .await
    .assert_status(422);
    t.post(&manager, &path, &form).await.assert_ok();

    let recorded = t.reload(&lease).await;
    assert_eq!(recorded.ejari_number.as_deref(), Some("EJ-9981"));
    assert_eq!(recorded.ejari_registered_on, Some(d(1, 5)));
    assert_eq!(recorded.ejari_expires_on, Some(recorded.end_date));
    assert!(t.tasks.open().is_empty(), "{:?}", t.tasks.open());

    // The certificate is guarded by the document kinds that block the step.
    t.docs.block("ejari_certificate", "lease.record_ejari");
    let refused = t
        .post(
            &manager,
            &path,
            &[("number", "EJ-2"), ("registered_on", "2027-01-06")],
        )
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("ejari_certificate"),
        "{}",
        refused.text()
    );
}

/// FIN.BIL.01: the daily run drafts one invoice per scheduled charge due,
/// once, with the right party, source, scope, VAT and dates; a lease being
/// prepared is not billed.
#[renox::test]
async fn fin_bil_01_each_instalment_becomes_one_invoice_on_its_due_date() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let lease = t.pending(&agent, &unit, 0).await;

    // Not active: nothing is billed.
    let report = invoicing::run(t.state(), day(0)).await.unwrap();
    assert_eq!(report.invoiced, 0);

    t.waive(&manager, &lease).await;
    t.activate(&manager, &lease).await.assert_ok();
    let report = invoicing::run(t.state(), day(0)).await.unwrap();
    assert_eq!(report.invoiced, 1);
    let again = invoicing::run(t.state(), day(0)).await.unwrap();
    assert_eq!(again.invoiced, 0);

    let invoices = t.invoices.all();
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert_eq!(invoice.draft.party, TENANT);
    assert_eq!(invoice.draft.source, lease.subject());
    assert_eq!(
        invoice.draft.scope.as_ref().unwrap().to_string(),
        format!("property.building:{}", building.id)
    );
    assert_eq!(invoice.draft.due_date, Some(day(0)));
    assert_eq!(invoice.draft.date_of_supply, Some(day(0)));
    assert_eq!(invoice.draft.lines[0].unit_price, aed(25_000));
    assert_eq!(invoice.draft.lines[0].vat_category, VatCategory::Exempt);
    assert_eq!(invoice.summary.status, InvoiceStatus::Draft);

    let charges = t.reload(&lease).await.charges(t.db()).await.unwrap();
    assert_eq!(charges[0].status, ChargeStatus::Invoiced);
    assert_eq!(charges[0].invoice_id, Some(invoice.summary.id.0));
    assert_eq!(charges[1].status, ChargeStatus::Scheduled);

    // A later day bills the next instalment (due in three months).
    let later = invoicing::run(t.state(), day(100)).await.unwrap();
    assert_eq!(later.invoiced, 1);
    assert_eq!(t.invoices.all().len(), 2);
}

/// FIN.CHQ.01: the cheques are registered against the lease, each linked to
/// the charge it pays; a bad charge or no cheque is refused.
#[renox::test]
async fn fin_chq_01_the_cheque_schedule_is_registered_against_the_lease() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let lease = t.lease(&unit, 0).await;
    let charges = lease.charges(t.db()).await.unwrap();

    let entries: Vec<ChequeEntry> = charges
        .iter()
        .enumerate()
        .map(|(i, c)| ChequeEntry::new(&format!("10{i}"), c.due_date, c.amount, Some(c.id)))
        .collect();
    let summaries = register_cheques(t.state(), None, &lease, "ENBD", entries)
        .await
        .unwrap();
    assert_eq!(summaries.len(), 4);
    assert!(
        summaries
            .iter()
            .all(|c| c.agreement == Some(lease.agreement()))
    );
    assert_eq!(summaries[0].linked, vec![charges[0].subject()]);
    assert!(summaries.iter().all(|c| c.status == ChequeStatus::Received));
    assert_eq!(of_version(t.state(), &lease).await.unwrap().len(), 4);

    let none = register_cheques(t.state(), None, &lease, "ENBD", Vec::new()).await;
    assert!(none.is_err());
    let stray = vec![ChequeEntry::new("200", day(1), aed(1_000), Some(9_999))];
    let refused = register_cheques(t.state(), None, &lease, "ENBD", stray).await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
}

/// The form on the lease page records the cheques, one per line.
#[renox::test]
async fn fin_chq_01_the_cheque_form_links_the_lines_to_the_instalments() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let lease = t.lease(&unit, 0).await;
    let lines = format!(
        "101 | {} | 25,000.00 | 1\n102 | {} | 25000 | 2",
        day(0),
        day(90)
    );
    t.post(
        &agent,
        &format!("/property/leases/{}/cheques", lease.id),
        &[("bank", "ENBD"), ("cheques", &lines)],
    )
    .await
    .assert_ok();
    let cheques = of_version(t.state(), &lease).await.unwrap();
    assert_eq!(cheques.len(), 2);
    assert_eq!(cheques[1].amount, aed(25_000));
    let charges = lease.charges(t.db()).await.unwrap();
    assert_eq!(cheques[1].linked, vec![charges[1].subject()]);
    t.post(
        &agent,
        &format!("/property/leases/{}/cheques", lease.id),
        &[("bank", "ENBD"), ("cheques", "bad line")],
    )
    .await
    .assert_status(422);
}

/// The lease pages: the list and a lease with its plan, versions and history.
#[renox::test]
async fn prp_lse_01_the_lease_pages_show_the_lease() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let lease = t.pending(&agent, &unit, 0).await;
    t.app.acting_as(&agent);
    t.app
        .get("/property/leases")
        .await
        .assert_ok()
        .assert_view("property/leases.html")
        .assert_see(&lease.number())
        .assert_see("Tariq Hassan");
    t.app
        .get(&format!("/property/leases/{}", lease.id))
        .await
        .assert_ok()
        .assert_view("property/lease.html")
        .assert_see("Rent 1/4")
        .assert_see("Pending signature")
        .assert_see("AED 100,000.00");
    // Another building's user does not see it.
    let other = t.building_coded("TWR-B").await;
    let outsider = t.user_in("Omar", AGENT, other.id).await;
    t.app.acting_as(&outsider);
    t.app
        .get(&format!("/property/leases/{}", lease.id))
        .await
        .assert_forbidden();
    assert!(OFFERED.contains(&"submit"));
}
