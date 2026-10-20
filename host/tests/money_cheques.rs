//! BL-PRP-26 (#276), the cheque settings of the Property client with the real
//! modules of the host (a fresh 01 deployment): a bounce adds no fee until
//! the client sets one (item 8), a third presentation is refused (9), the
//! tenant and the property manager are reminded at 3 days and the decision
//! task goes to the accountant and the property manager at 14 days (10), the
//! bounce tells the tenant and the property manager and leaves the accountant
//! a task (11), `refer_to_legal` is a civil case by default and flags the
//! lease (12), a cheque entered in error is returned only while it is not
//! deposited (13) and a deposit entered late keeps its deposit date (16).

mod support;

use host::roles::{ACCOUNTANT, PROPERTY_MANAGER, RECEIVABLES};
use property::UnitUse;
use property::legal_case::{CaseStatus, CaseType, LegalCase};
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_kernel::PartyId;
use wl_kernel::cheques::{ChequeId, ChequeStatus, RegisterCheque, ReturnCheques, cheques};
use wl_kernel::documents::today;
use wl_kernel::invoicing::VatCategory;
use wl_payments::cheque::ChequeReturn;
use wl_payments::followup::{FollowupKind, FollowupStatus};
use wl_payments::notices::{
    CHEQUE_BOUNCED, CHEQUE_BOUNCED_STAFF, CHEQUE_REMINDER, CHEQUE_REMINDER_STAFF,
};
use wl_payments::{Cheque, FollowupTask, cheque_chase};

/// The people and the lease of one tenant.
struct Scene {
    app: TestApp,
    accountant: User,
    manager: User,
    tenant: PartyId,
    lease: property::Lease,
    rent: wl_invoicing::Invoice,
}

async fn scene() -> Scene {
    let app = booted().await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let unit = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    let lease = draft_lease(&app, &unit, tenant, -30).await;
    let rent = invoiced_charge(
        &app,
        &clerk,
        &lease,
        2_000_000,
        VatCategory::Exempt,
        day(0),
        1,
    )
    .await;
    Scene {
        app,
        accountant,
        manager,
        tenant,
        lease,
        rent,
    }
}

impl Scene {
    /// A received cheque of the tenant dated `date_in` days from today,
    /// paying the rent invoice, received with the lease.
    async fn cheque(&self, number: &str, date_in: i64) -> Cheque {
        let entity = entity(&self.app, "PGP").await;
        let request = RegisterCheque::new(
            self.tenant,
            entity.id,
            "ENBD",
            number,
            day(date_in),
            2_000_000,
        )
        .link(self.rent.subject())
        .agreement(self.lease.subject());
        let summary = cheques(self.app.state())
            .register(self.app.state(), request, Some(self.accountant.id))
            .await
            .expect("the cheque is registered");
        Cheque::find_or_404(self.app.db(), summary.id.0)
            .await
            .unwrap()
    }

    async fn post(&self, cheque: &Cheque, action: &str, pairs: &[(&str, &str)]) -> u16 {
        self.app.acting_as(&self.accountant);
        self.app
            .htmx()
            .post(&format!("/cheques/{}/{action}", cheque.id), pairs)
            .await
            .status
            .as_u16()
    }

    async fn reload(&self, cheque: &Cheque) -> Cheque {
        Cheque::find_or_404(self.app.db(), cheque.id).await.unwrap()
    }

    /// The cheque deposited and returned for insufficient funds with the
    /// certificate, `honoured` dirhams paid (`""` for nothing).
    async fn bounced(&self, number: &str, honoured: &str) -> Cheque {
        let cheque = self.cheque(number, 0).await;
        assert_eq!(self.post(&cheque, "deposit", &[]).await, 200);
        let deposited = self.reload(&cheque).await;
        assert_eq!(
            self.post(
                &deposited,
                "bounce",
                &[
                    ("reason", "insufficient_funds"),
                    ("certificate", "CERT-77"),
                    ("honoured", honoured),
                ],
            )
            .await,
            200
        );
        let bounced = self.reload(&cheque).await;
        assert_eq!(bounced.status, ChequeStatus::Bounced);
        bounced
    }
}

/// Items 8, 10 and 11: a bounce with AED 6,000 honoured. No fee is charged
/// (none is set), the tenant and the property manager are told, the
/// accountant has a follow-up task; the reminder goes out at 3 days to the
/// tenant and the property manager and the decision tasks at 14 days to the
/// accountant and the property manager, not before.
#[renox::test]
async fn a_bounce_adds_no_fee_tells_the_tenant_and_the_manager_and_is_chased() {
    let s = scene().await;
    let cheque = s.bounced("1042", "6000.00").await;
    assert_eq!(cheque.open_amount, 1_400_000);

    // Item 8: no fee until the client sets one.
    assert_eq!(cheque.bounce_fee_total, 0);
    let returns = ChequeReturn::of_cheque(s.app.db(), cheque.id)
        .await
        .unwrap();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].fee_amount, 0);
    assert_eq!(returns[0].fee_subject_id, None, "no fee charge was created");

    // Item 11: the tenant and the property manager are told, the accountant
    // has the follow-up task.
    assert_eq!(
        messages_to_party(&s.app, CHEQUE_BOUNCED, s.tenant)
            .await
            .len(),
        1
    );
    assert_eq!(
        messages_to(&s.app, CHEQUE_BOUNCED_STAFF, &s.manager)
            .await
            .len(),
        1
    );
    let follow_up = FollowupTask::open_of_cheque(s.app.db(), cheque.id)
        .await
        .unwrap();
    assert_eq!(follow_up.len(), 1);
    assert_eq!(follow_up[0].kind, FollowupKind::FollowUpCheque);
    assert_eq!(follow_up[0].role, "accountant");

    // Item 10: nothing on day 2.
    let report = cheque_chase::run(s.app.state(), day(2)).await.unwrap();
    assert_eq!((report.reminded, report.decisions), (0, 0));
    assert!(
        messages_to_party(&s.app, CHEQUE_REMINDER, s.tenant)
            .await
            .is_empty()
    );
    // Day 3: the tenant (open amount) and the property manager are reminded.
    let report = cheque_chase::run(s.app.state(), day(3)).await.unwrap();
    assert_eq!((report.reminded, report.decisions), (1, 0));
    let reminded = messages_to_party(&s.app, CHEQUE_REMINDER, s.tenant).await;
    assert_eq!(reminded.len(), 1);
    assert!(reminded[0].rendered_body.contains("AED 14,000.00"));
    assert!(
        !reminded[0].rendered_body.contains("bounce fee"),
        "the reminder shows only the open amount without a fee: {}",
        reminded[0].rendered_body
    );
    assert_eq!(
        messages_to(&s.app, CHEQUE_REMINDER_STAFF, &s.manager)
            .await
            .len(),
        1
    );
    // Day 14: the decision task for the accountant and the property manager.
    let report = cheque_chase::run(s.app.state(), day(14)).await.unwrap();
    assert_eq!((report.reminded, report.decisions), (0, 1));
    let tasks: Vec<FollowupTask> = FollowupTask::of_cheque(s.app.db(), cheque.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.kind == FollowupKind::DecideCheque)
        .collect();
    let mut roles: Vec<&str> = tasks.iter().map(|t| t.role.as_str()).collect();
    roles.sort_unstable();
    assert_eq!(roles, ["accountant", "property_manager"]);
    assert!(tasks.iter().all(|t| t.status == FollowupStatus::Open));
}

/// Item 9: a cheque is presented twice at most; the third presentation is
/// refused.
#[renox::test]
async fn a_third_presentation_is_refused() {
    let s = scene().await;
    let first = s.bounced("2001", "").await;
    assert_eq!(first.presentations, 1);

    // The second presentation is allowed and bounces again.
    let again = day(2).to_string();
    assert_eq!(
        s.post(
            &first,
            "represent",
            &[("date", again.as_str()), ("funds_confirmed", "on")]
        )
        .await,
        200
    );
    let second = s.reload(&first).await;
    assert_eq!(
        (second.status, second.presentations),
        (ChequeStatus::Deposited, 2)
    );
    assert_eq!(
        s.post(
            &second,
            "bounce",
            &[
                ("reason", "account_closed"),
                ("certificate", "CERT-2"),
                ("honoured", "")
            ]
        )
        .await,
        200
    );
    let bounced = s.reload(&first).await;
    assert_eq!(bounced.status, ChequeStatus::Bounced);
    assert_eq!(
        bounced.bounce_fee_total, 0,
        "still no fee at the second bounce"
    );

    // The third is refused.
    let third = day(5).to_string();
    assert_eq!(
        s.post(
            &bounced,
            "represent",
            &[("date", third.as_str()), ("funds_confirmed", "on")]
        )
        .await,
        422
    );
    let after = s.reload(&first).await;
    assert_eq!(
        (after.status, after.presentations),
        (ChequeStatus::Bounced, 2)
    );
}

/// Item 12: `refer_to_legal` is a civil case unless another is chosen, and
/// sets the legal case flag of the lease the cheque was received with.
#[renox::test]
async fn refer_to_legal_is_a_civil_case_and_flags_the_lease() {
    let s = scene().await;
    let cheque = s.bounced("3001", "").await;
    assert_eq!(s.post(&cheque, "refer", &[]).await, 200);
    let referred = s.reload(&cheque).await;
    assert_eq!(referred.status, ChequeStatus::LegalCase);
    assert_eq!(
        referred.case_type.map(|case| case.as_str()),
        Some("civil_execution"),
        "civil case by default"
    );
    let flags = LegalCase::query()
        .where_(LegalCase::CHEQUE_ID.eq(cheque.id))
        .get(s.app.db())
        .await
        .unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].status, CaseStatus::Open);
    assert_eq!(flags[0].case_type, CaseType::Arrears);
    assert_eq!(flags[0].lease_id, s.lease.id);
}

/// Item 13: a cheque entered in error is returned (voided) while it is
/// received; a deposited one is not touched.
#[renox::test]
async fn a_received_cheque_is_voided_and_a_deposited_one_is_not() {
    let s = scene().await;
    let received = s.cheque("4001", 0).await;
    let deposited = s.cheque("4002", 0).await;
    assert_eq!(s.post(&deposited, "deposit", &[]).await, 200);

    let done = cheques(s.app.state())
        .return_undeposited(
            s.app.state(),
            ReturnCheques::new(
                vec![ChequeId(received.id), ChequeId(deposited.id)],
                "Entered in error",
            ),
            Some(s.accountant.id),
        )
        .await
        .unwrap()
        .expect("one cheque went back");
    assert_eq!(done.returned, [ChequeId(received.id)]);
    assert_eq!(done.skipped, [ChequeId(deposited.id)]);
    assert_eq!(s.reload(&received).await.status, ChequeStatus::Returned);
    assert_eq!(s.reload(&deposited).await.status, ChequeStatus::Deposited);
}

/// Item 16: a deposit entered late is accepted with the day it was made as
/// its deposit date (the business time), not the day it was entered; a
/// deposit dated in the future is refused.
#[renox::test]
async fn a_deposit_entered_late_keeps_the_day_it_was_made() {
    let s = scene().await;
    let cheque = s.cheque("5001", -10).await;

    let future = day(1).to_string();
    assert_eq!(
        s.post(&cheque, "deposit", &[("deposit_date", future.as_str())])
            .await,
        422
    );
    assert_eq!(s.reload(&cheque).await.status, ChequeStatus::Received);

    // Entered today for a deposit made a week ago.
    let made = day(-7);
    assert_eq!(
        s.post(
            &cheque,
            "deposit",
            &[("deposit_date", made.to_string().as_str())]
        )
        .await,
        200
    );
    let deposited = s.reload(&cheque).await;
    assert_eq!(deposited.status, ChequeStatus::Deposited);
    assert_eq!(deposited.deposit_date, Some(made));
    assert_ne!(deposited.deposit_date, Some(today()));

    // A deposit before the cheque's date is still refused.
    let early = s.cheque("5002", -3).await;
    assert_eq!(
        s.post(
            &early,
            "deposit",
            &[("deposit_date", day(-7).to_string().as_str())]
        )
        .await,
        422
    );
}
