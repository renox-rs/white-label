//! BL-PAY-30 (#469) with the real modules of the Property host: cheques that
//! were never deposited go back against one handover receipt per return
//! call, generated as a numbered bilingual PDF (wl-documents, BL-DOC-11),
//! stored on the handover and shown on each cheque, and e-mailed to the
//! tenant with the PDF attached (BL-NTF-10).
//!
//! The cheques go back through the kernel's `Cheques::return_undeposited`,
//! the call lease `terminate` makes for the cheques dated after the move-out.
//! The handover to a new manager (agreement `end`, a receiver other than the
//! tenant) waits for the kernel's `ReturnCheques` to name the receiver.

mod support;

use host::roles::ACCOUNTANT;
use property::UnitUse;
use renox::chrono::Datelike;
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_kernel::PartyId;
use wl_kernel::cheques::{ChequeId, RegisterCheque, ReturnCheques, cheques};
use wl_kernel::documents::today;
use wl_payments::cheque_handover::{ChequeHandover, HANDED_OVER};

struct Scene {
    app: TestApp,
    accountant: User,
    tenant: PartyId,
    lease: property::Lease,
}

async fn scene() -> Scene {
    let app = booted().await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let unit = unit_of(&app, "TWR-C", "702", UnitUse::Residential).await;
    let lease = draft_lease(&app, &unit, tenant, -60).await;
    Scene {
        app,
        accountant,
        tenant,
        lease,
    }
}

impl Scene {
    /// A received cheque of the tenant for the lease, dated `months` months on.
    async fn cheque(&self, number: &str, months: i64) -> ChequeId {
        let entity = entity(&self.app, "PGP").await;
        let request = RegisterCheque::new(
            self.tenant,
            entity.id,
            "ENBD",
            number,
            day(30 * months),
            1_000_000,
        )
        .agreement(self.lease.subject());
        cheques(self.app.state())
            .register(self.app.state(), request, Some(self.accountant.id))
            .await
            .expect("the cheque is registered")
            .id
    }

    async fn hand_back(&self, ids: Vec<ChequeId>) -> ChequeHandover {
        let done = cheques(self.app.state())
            .return_undeposited(
                self.app.state(),
                ReturnCheques::new(ids, "Lease terminated"),
                Some(self.accountant.id),
            )
            .await
            .unwrap()
            .expect("cheques went back");
        ChequeHandover::by_receipt(self.app.db(), &done.receipt)
            .await
            .unwrap()
            .expect("the handover of the call")
    }
}

#[renox::test]
async fn five_cheques_go_back_against_one_numbered_receipt_sent_to_the_tenant() {
    let s = scene().await;
    let mut ids = Vec::new();
    for (i, number) in ["7001", "7002", "7003", "7004", "7005"].iter().enumerate() {
        ids.push(s.cheque(number, i as i64 + 1).await);
    }
    let handover = s.hand_back(ids.clone()).await;
    assert_eq!(handover.cheques.0.len(), 5, "one receipt lists the five cheques");
    assert_eq!(handover.total, 5_000_000);
    assert_eq!(handover.receiver_party_id, s.tenant.0);
    let number = handover.number.clone().expect("the receipt is numbered");
    assert_eq!(number, format!("CHR-{}-0001", today().year()));
    let document = handover.document_id.expect("the receipt is stored");
    let stored = wl_documents::Document::find_or_404(s.app.db(), document)
        .await
        .unwrap();
    assert_eq!(stored.subject_type, wl_payments::cheque_handover::SUBJECT_TYPE);
    assert_eq!(stored.subject_id, handover.id);

    // Each cheque links to the receipt.
    s.app.acting_as(&s.accountant);
    for id in &ids {
        s.app
            .get(&format!("/cheques/{}", id.0))
            .await
            .assert_ok()
            .assert_see(&format!("Cheque handover receipt {number}"));
    }

    // The tenant gets it by e-mail with the PDF attached.
    let sent = messages_to_party(&s.app, HANDED_OVER, s.tenant).await;
    assert_eq!(sent.len(), 1);
    let attached = wl_kernel::document_store::attachments_in(&sent[0].data.0);
    assert_eq!(attached.len(), 1, "the receipt is attached");
    assert_eq!(attached[0].document.0, document);
    assert!(attached[0].file_name.contains(&number));
}

#[renox::test]
async fn a_second_return_the_next_day_gets_the_next_number() {
    let s = scene().await;
    let first = s.cheque("7101", 1).await;
    let second = s.cheque("7102", 2).await;
    let one = s.hand_back(vec![first]).await;
    s.app.travel(std::time::Duration::from_secs(24 * 3600));
    let two = s
        .app
        .at_travelled_time(async {
            cheques(s.app.state())
                .return_undeposited(
                    s.app.state(),
                    ReturnCheques::new(vec![second], "Lease terminated"),
                    Some(s.accountant.id),
                )
                .await
        })
        .await
        .unwrap()
        .expect("the cheque went back");
    let two = ChequeHandover::by_receipt(s.app.db(), &two.receipt)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(one.id, two.id, "one receipt per return call");
    let n = |h: &ChequeHandover| {
        h.number
            .as_deref()
            .and_then(|n| n.rsplit('-').next())
            .and_then(|n| n.parse::<i64>().ok())
            .expect("a number")
    };
    assert_eq!(n(&two), n(&one) + 1);
    s.app.travel_back();
}

#[renox::test]
async fn a_receipt_is_never_regenerated_when_a_cheque_changes_later() {
    let s = scene().await;
    let id = s.cheque("7201", 1).await;
    let handover = s.hand_back(vec![id]).await;
    let document = handover.document_id;
    // Issuing again (as a later event would) keeps the receipt.
    let again = wl_payments::cheque_handover::issue(s.app.state(), &handover)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.document_id, document);
    assert_eq!(again.number, handover.number);
}
