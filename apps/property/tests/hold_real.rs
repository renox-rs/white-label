//! BL-PRP-15 (#265) with the real wl-invoicing and wl-payments: a tenant on
//! hold (`PTY.ACL.03`) goes on paying the invoices of the active lease (money
//! in is never refused), while the refund of the deposit of its lease waits as
//! an exception request for the exception role and no refund is made.

mod support;

use property::{ChargeStatus, LeaseStatus, NewLease};
use renox::auth::permissions::define_role;
use renox::chrono::Duration as Days;
use renox::prelude::*;
use wl_kernel::BLOCK_EXCEPTION_ACTION;
use wl_kernel::approvals::{ApprovalRequest, approve};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::payments::{AllocationTarget, PaymentMethod, RecordPayment, payment_recorder};
use wl_payments::deposit::DepositStatus;
use wl_payments::{Deposit, Refund};

use support::{ACCOUNTANT, AGENT, MANAGER, Real, TENANT, aed, boot_with, d};

/// AC3: the lease of a blocked tenant is `active`; its invoice (issued before
/// the block) is paid by the tenant, the real payments accept and allocate the
/// money, the invoice is `paid` and the lease stays `active`.
#[renox::test]
async fn bl_prp_15_the_blocked_tenant_still_pays_the_invoices_of_the_active_lease() {
    let t = boot_with(Real::MONEY).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let new = NewLease::new(unit.id, TENANT, d(3, 1), d(12, 31), aed(120_000), 12);
    let lease = property::leases::create_draft(t.state(), None, new)
        .await
        .unwrap();
    t.waive(&manager, &lease).await;
    t.submit(&agent, &lease).await.assert_ok();
    t.activate(&manager, &lease).await.assert_ok();
    property::invoicing::run(t.state(), d(3, 1)).await.unwrap();
    define_role(
        t.db(),
        "invoice_clerk",
        &[
            wl_invoicing::permissions::VIEW,
            wl_invoicing::permissions::ISSUE,
        ],
    )
    .await
    .unwrap();
    let clerk = t.user_with("Clara", "invoice_clerk").await;
    let march = t
        .reload(&lease)
        .await
        .charges(t.db())
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.due_date == d(3, 1))
        .expect("the March charge");
    let invoice = march.invoice_id.expect("its invoice");
    let issued = t
        .post(&clerk, &format!("/invoices/{invoice}/issue"), &[])
        .await;
    assert!(issued.status.is_redirection(), "{}", issued.status);

    // The tenant is put on hold.
    t.blocks.block(TENANT);

    let entity = wl_kernel::LegalEntity::query()
        .where_(wl_kernel::LegalEntity::CODE.eq("PGP"))
        .first(t.db())
        .await
        .unwrap()
        .expect("the default entity");
    let mut request = RecordPayment::new(TENANT, entity.id, PaymentMethod::Transfer, march.amount);
    request.reference = Some("TRF-MARCH".to_owned());
    let payment = payment_recorder(t.state())
        .record(t.state(), request, None)
        .await
        .expect("a payment from a blocked tenant is received");
    payment_recorder(t.state())
        .allocate(
            t.state(),
            payment.id,
            AllocationTarget::Invoice(InvoiceId(invoice)),
            march.amount,
        )
        .await
        .expect("and allocated to the invoice of its lease");

    let paid = invoicer(t.state())
        .find(t.db(), InvoiceId(invoice))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(paid.status, InvoiceStatus::Paid);
    assert_eq!(paid.balance, 0);
    let after = t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::Active, "the lease is not ended");
    let charge = after
        .charges(t.db())
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.id == march.id)
        .unwrap();
    assert_eq!(charge.status, ChargeStatus::Invoiced);
    // Billing goes on too.
    let report = property::invoicing::run(t.state(), d(4, 1)).await.unwrap();
    assert!(report.invoiced >= 1, "{report:?}");
}

/// AC4: the deposit of AED 5,000 of a cancelled lease is refunded after the
/// window; for a blocked tenant the refund does not run: an exception request
/// waits for the exception role, the deposit stays `settlement_pending` and no refund
/// exists. When the exception role approves, AED 5,000 is requested back.
#[renox::test]
async fn bl_prp_15_the_deposit_refund_of_a_blocked_tenant_waits_for_the_exception_role() {
    let t = boot_with(Real::PAYMENTS).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let accountant = t.user_with("Aisha", ACCOUNTANT).await;
    let lease = t.pending(&agent, &unit, 0).await;
    t.hold_real_deposit(&lease, aed(5_000)).await;
    t.post(
        &agent,
        &format!("/property/leases/{}/cancel", lease.id),
        &[("reason", "prospect backed out")],
    )
    .await
    .assert_ok();
    let request = ApprovalRequest::pending_about(t.db(), &lease.subject())
        .await
        .unwrap()
        .remove(0);
    approve(t.state(), request.id, &manager, "ok")
        .await
        .unwrap()
        .expect("the final approval");
    assert_eq!(t.status(&lease).await, LeaseStatus::Cancelled);
    let deposit = Deposit::query()
        .where_(Deposit::SOURCE_ID.eq(lease.agreement().subject_id))
        .first(t.db())
        .await
        .unwrap()
        .expect("the deposit");
    assert_eq!(deposit.status, DepositStatus::SettlementPending);

    // The tenant is put on hold; the window passes.
    t.blocks.block(TENANT);
    let asked = t
        .at(
            today() + Days::days(15),
            t.post(
                &accountant,
                &format!("/deposits/{}/refund", deposit.id),
                &[("method", "transfer")],
            ),
        )
        .await;
    asked.assert_ok();

    let waiting = Deposit::find_or_404(t.db(), deposit.id).await.unwrap();
    assert_eq!(waiting.status, DepositStatus::SettlementPending);
    assert_eq!(waiting.refund_id, None);
    assert_eq!(Refund::query().count(t.db()).await.unwrap(), 0);
    let requests = ApprovalRequest::pending_about(t.db(), &deposit.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, BLOCK_EXCEPTION_ACTION);
    assert_eq!(requests[0].transition, "refund");

    // The requester cannot approve it; the exception role does, and the refund follows.
    let refused = approve(t.state(), requests[0].id, &accountant, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    define_role(t.db(), "finance_manager", &[BLOCK_EXCEPTION_ACTION])
        .await
        .unwrap();
    let officer = t.user_with("Fiona", "finance_manager").await;
    let decided = t
        .at(
            today() + Days::days(15),
            approve(t.state(), requests[0].id, &officer, "approved"),
        )
        .await
        .unwrap();
    assert!(decided.is_some());
    let refunded = Deposit::find_or_404(t.db(), deposit.id).await.unwrap();
    assert_eq!(refunded.status, DepositStatus::Refunded);
    let refunds = Refund::query().get(t.db()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, aed(5_000));
}
