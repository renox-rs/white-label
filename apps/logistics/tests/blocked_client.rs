//! BL-LOG-19 (#317): refuse new bookings and hold remittance payment for a
//! blocked client, 03 §3.3 `PTY.ACL.03`, §4.1 row *Client blocked*, §7.1 `pay`
//! and `repay`. The client is a real party in the real wl-parties, blocked for
//! non-payment; the block guard, the exception approval and the payments are
//! the real modules.

mod support;

use logistics::permissions as perm;
use logistics::{FreightJob, FreightStatus, RemittanceStatus, Shipment, ShipmentStatus};
use renox::auth::permissions;
use renox::prelude::*;
use wl_kernel::approvals::{ApprovalRequest, approve};
use wl_kernel::{BLOCK_EXCEPTION_ACTION, COMPANY_ADMIN_ROLE, PartyId};
use wl_payments::payment::Payment;

use support::{World, boot_with_parties};

const ACCOUNT: &str = "AE070331234567890123456";

/// The text every refused booking or freight job shows.
const NEUTRAL: &str = "This request cannot be completed online. Please contact us.";

/// A world with the client "Gulf Souq Trading LLC" as a real party, its COD
/// terms and its account.
async fn scene() -> (World, PartyId) {
    let w = boot_with_parties().await;
    let client = w.client_party("Gulf Souq Trading LLC").await;
    w.cod_terms(client, 0, Some(ACCOUNT)).await;
    (w, client)
}

/// The company admin: holds the admin role and the permission to approve a
/// block exception.
async fn company_admin(w: &World, name: &str, extra: &[&str]) -> User {
    let mut grants = vec![
        wl_parties::permissions::BLOCK_EXCEPTION,
        perm::REMITTANCES_VIEW,
    ];
    grants.extend_from_slice(extra);
    permissions::define_role(w.db(), COMPANY_ADMIN_ROLE, &grants)
        .await
        .unwrap();
    let user = w.user_with(name, &grants).await;
    user.assign_role(w.db(), COMPANY_ADMIN_ROLE).await.unwrap();
    user
}

fn csv() -> &'static str {
    "client_reference,recipient_name,recipient_phone,address,area,parcels,weight_grams,cod\n\
     R-1,Mariam Al Hosani,+971500000000,Villa 12,Al Barsha,1,1200,100\n\
     R-2,Omar Khalid,+971500000001,Flat 4,Deira,1,900,0\n"
}

/// The client's approved remittance of AED 500, the client blocked after the
/// draft was approved.
async fn approved_and_blocked() -> (World, PartyId, logistics::Remittance) {
    let (w, client) = scene().await;
    w.delivered(client, "ORD-1", "500").await;
    w.block_party(client, "non_payment").await;
    // The Monday draft is created although the client is blocked.
    w.rule().await;
    let draft = w.the_remittance_of(client).await;
    assert_eq!(draft.status, RemittanceStatus::Draft);
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    let approved = w.remittance(&draft).await;
    assert_eq!(approved.status, RemittanceStatus::Approved);
    (w, client, approved)
}

/// AC1: given client C blocked for *non-payment*, a portal booking and a CSV
/// upload are refused with the neutral text; the reason is not shown.
#[renox::test]
async fn a_blocked_client_cannot_book_and_the_text_is_neutral() {
    let (w, client) = scene().await;
    w.block_party(client, "non_payment").await;
    let id = client.0.to_string();

    // A client user in the portal.
    let booking = [
        ("client_party_id", id.as_str()),
        ("client_reference", "ORD-9"),
        ("recipient_name", "Mariam Al Hosani"),
        ("recipient_phone", "+971500000000"),
        ("address", "Villa 12, Al Barsha 2"),
        ("area", "Al Barsha"),
        ("parcels", "1"),
        ("weight_grams", "1200"),
        ("cod", "0"),
    ];
    let refused = w.post(&w.noor, "/shipments", &booking).await;
    refused.assert_status(422).assert_see(NEUTRAL);
    assert!(!refused.text().to_lowercase().contains("payment"));
    // The staff booking for that client is refused with the same text.
    w.post(&w.dispatcher, "/shipments", &booking)
        .await
        .assert_status(422)
        .assert_see(NEUTRAL);
    assert_eq!(Shipment::query().count(w.db()).await.unwrap(), 0);

    // The CSV upload: refused as a whole, and every row of a dry run.
    let upload = [("client_party_id", id.as_str()), ("csv", csv())];
    w.post(&w.noor, "/shipments/import", &upload)
        .await
        .assert_status(422)
        .assert_see(NEUTRAL);
    let dry = w
        .post(
            &w.noor,
            "/shipments/import",
            &[
                ("client_party_id", id.as_str()),
                ("csv", csv()),
                ("dry_run", "1"),
            ],
        )
        .await;
    dry.assert_ok();
    let text = dry.text();
    assert_eq!(text.matches(NEUTRAL).count(), 2, "{text}");
    assert!(!text.to_lowercase().contains("non_payment"));
    assert_eq!(Shipment::query().count(w.db()).await.unwrap(), 0);
}

/// AC2: C's `picked_up` shipments are dispatched and delivered as normal; a
/// `created` shipment booked before the block can be picked up and shows
/// "client on hold" on the pickup list.
#[renox::test]
async fn shipments_booked_before_the_block_finish_and_the_dispatcher_sees_client_on_hold() {
    let (w, client) = scene().await;
    let picked = w.book_as(&w.dispatcher, client, "ORD-1").await;
    let picked = w.pick_up(&picked).await;
    let waiting = w.book_as(&w.dispatcher, client, "ORD-2").await;
    let cancelled = w.book_as(&w.dispatcher, client, "ORD-3").await;
    w.block_party(client, "non_payment").await;

    // The dispatcher sees who is on hold, on the shipments waiting for pickup.
    let list = w.get(&w.dispatcher, "/shipments").await;
    list.assert_ok();
    assert!(list.text().contains("Client on hold"));
    assert!(!list.text().to_lowercase().contains("non_payment"));

    // The picked up shipment goes to the hub, out and is delivered.
    let at_hub = w.at_hub(&picked).await;
    let (_out, delivery) = w.dispatch_from_hub(&at_hub).await;
    w.deliver_stop(&delivery, "0.00").await.assert_status(303);
    assert_eq!(
        w.reload(&picked).await.status,
        ShipmentStatus::Delivered,
        "a picked up shipment is delivered as normal"
    );
    // The one booked before the block is picked up as normal.
    let after = w.pick_up(&waiting).await;
    assert_eq!(after.status, ShipmentStatus::PickedUp);
    // And the client may still cancel one before pickup.
    w.post(&w.noor, &format!("/shipments/{}/cancel", cancelled.id), &[])
        .await
        .assert_status(303);
    assert_eq!(w.reload(&cancelled).await.status, ShipmentStatus::Cancelled);
}

/// AC3: C's Monday draft is created and can be approved; `pay` is refused
/// with "client on hold".
#[renox::test]
async fn the_draft_is_made_and_approved_but_pay_waits_for_the_unblock() {
    let (w, _client, approved) = approved_and_blocked().await;
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", approved.id),
        &[("payment_reference", "TRF-1")],
    )
    .await
    .assert_status(422)
    .assert_see("Client on hold");
    assert_eq!(
        w.remittance_status(&approved).await,
        RemittanceStatus::Approved
    );
    assert_eq!(Payment::query().count(w.db()).await.unwrap(), 0);
    // The accountant sees the refusal and the way to ask for an exception.
    w.get(&w.amal, &format!("/remittances/{}", approved.id))
        .await
        .assert_ok()
        .assert_see("Request exception");
}

/// Unblocking makes the waiting remittance payable with no other change.
#[renox::test]
async fn unblocking_makes_the_waiting_remittance_payable() {
    let (w, client, approved) = approved_and_blocked().await;
    let admin = company_admin(&w, "Aisha", &[wl_parties::permissions::BLOCK]).await;
    w.app.acting_as(&admin);
    let files: Vec<(&str, &str, &[u8])> = Vec::new();
    w.app
        .htmx()
        .post_multipart(
            &format!("/parties/{}/unblock", client.0),
            &[("note", "Settled")],
            &files,
        )
        .await
        .assert_ok();
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", approved.id),
        &[("payment_reference", "TRF-1")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.remittance_status(&approved).await, RemittanceStatus::Paid);
}

/// AC4: with an exception approved by the company admin, `pay` succeeds once;
/// the accountant who requested the exception cannot approve it.
#[renox::test]
async fn an_exception_approved_by_the_company_admin_lets_one_payment_go_ahead() {
    let (w, client, approved) = approved_and_blocked().await;
    let pay = format!("/remittances/{}/pay", approved.id);

    // Amal asks for the exception; nothing is paid.
    w.post(&w.amal, &pay, &[("block_exception", "1")])
        .await
        .assert_status(303);
    let requests = ApprovalRequest::pending_about(w.db(), &approved.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, BLOCK_EXCEPTION_ACTION);
    assert_eq!(requests[0].requester_id, w.amal.id);
    // A second request while one is pending is refused.
    w.post(&w.amal, &pay, &[("block_exception", "1")])
        .await
        .assert_status(422);
    // Paying still waits.
    w.post(&w.amal, &pay, &[("payment_reference", "TRF-1")])
        .await
        .assert_status(422);
    assert_eq!(Payment::query().count(w.db()).await.unwrap(), 0);

    // The requester cannot approve it, nor can another accountant: it is the
    // company admin's.
    let refused = approve(w.state(), requests[0].id, &w.amal, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    let refused = approve(w.state(), requests[0].id, &w.bashar, "").await;
    assert!(refused.is_err());
    assert!(!w.remittance(&approved).await.block_exception_granted);

    let admin = company_admin(&w, "Aisha", &[]).await;
    approve(
        w.state(),
        requests[0].id,
        &admin,
        "Paid arrears on 4 November",
    )
    .await
    .unwrap();
    let granted = w.remittance(&approved).await;
    assert!(granted.block_exception_granted);
    assert_eq!(granted.status, RemittanceStatus::Approved);

    // `pay` succeeds, once.
    w.post(&w.amal, &pay, &[("payment_reference", "TRF-1")])
        .await
        .assert_status(303);
    let paid = w.remittance(&approved).await;
    assert_eq!(paid.status, RemittanceStatus::Paid);
    assert!(!paid.block_exception_granted, "the exception is used up");
    assert_eq!(Payment::query().count(w.db()).await.unwrap(), 1);

    // The next remittance of the still blocked client waits again.
    w.delivered(client, "ORD-2", "200").await;
    w.rule().await;
    let next = w
        .remittances_of(client)
        .await
        .into_iter()
        .find(|r| r.status == RemittanceStatus::Draft)
        .expect("the next draft");
    w.post(&w.bashar, &format!("/remittances/{}/approve", next.id), &[])
        .await
        .assert_status(303);
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", next.id),
        &[("payment_reference", "TRF-2")],
    )
    .await
    .assert_status(422)
    .assert_see("Client on hold");
}

/// The requester cannot approve its own exception even when it is the company
/// admin.
#[renox::test]
async fn the_requester_cannot_approve_its_own_exception_even_as_company_admin() {
    let (w, _client, approved) = approved_and_blocked().await;
    let admin = company_admin(
        &w,
        "Aisha",
        &[perm::REMITTANCES_PAY, perm::REMITTANCES_VIEW],
    )
    .await;
    w.post(
        &admin,
        &format!("/remittances/{}/pay", approved.id),
        &[("block_exception", "1")],
    )
    .await
    .assert_status(303);
    let request = ApprovalRequest::pending_about(w.db(), &approved.subject())
        .await
        .unwrap()
        .remove(0);
    let own = approve(w.state(), request.id, &admin, "").await;
    assert!(own.is_err(), "{own:?}");
    assert!(!w.remittance(&approved).await.block_exception_granted);
}

/// `repay` of a blocked client waits as `pay` does, and an exception serves
/// it too.
#[renox::test]
async fn a_returned_remittance_of_a_blocked_client_is_not_repaid_without_an_exception() {
    let (w, client) = scene().await;
    w.delivered(client, "ORD-1", "500").await;
    w.rule().await;
    let draft = w.the_remittance_of(client).await;
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", draft.id),
        &[("payment_reference", "TRF-1")],
    )
    .await
    .assert_status(303);
    w.post(
        &w.amal,
        &format!("/remittances/{}/return-payment", draft.id),
        &[("advice", "BANK-ADV-1")],
    )
    .await
    .assert_status(303);
    w.post(
        &w.amal,
        &format!("/remittances/{}/confirm-account", draft.id),
        &[("note", "Client e-mail")],
    )
    .await
    .assert_status(303);
    w.block_party(client, "non_payment").await;

    let repay = format!("/remittances/{}/repay", draft.id);
    w.post(&w.amal, &repay, &[("payment_reference", "TRF-2")])
        .await
        .assert_status(422)
        .assert_see("Client on hold");
    w.post(&w.amal, &repay, &[("block_exception", "1")])
        .await
        .assert_status(303);
    let request = ApprovalRequest::pending_about(w.db(), &draft.subject())
        .await
        .unwrap()
        .remove(0);
    let admin = company_admin(&w, "Aisha", &[]).await;
    approve(w.state(), request.id, &admin, "ok").await.unwrap();
    w.post(&w.amal, &repay, &[("payment_reference", "TRF-2")])
        .await
        .assert_status(303);
    assert_eq!(w.remittance_status(&draft).await, RemittanceStatus::Paid);
}

/// An exception is asked only for a blocked client and a remittance that
/// waits to be paid; it needs `remittances.pay`.
#[renox::test]
async fn an_exception_is_asked_only_for_a_blocked_client_by_the_accountant() {
    let (w, client) = scene().await;
    w.delivered(client, "ORD-1", "500").await;
    w.rule().await;
    let draft = w.the_remittance_of(client).await;
    let pay = format!("/remittances/{}/pay", draft.id);
    // A draft waits for no payment.
    w.post(&w.amal, &pay, &[("block_exception", "1")])
        .await
        .assert_status(422);
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    // The client is not blocked: no exception is needed.
    w.post(&w.amal, &pay, &[("block_exception", "1")])
        .await
        .assert_status(422);
    w.block_party(client, "non_payment").await;
    w.post(&w.dispatcher, &pay, &[("block_exception", "1")])
        .await
        .assert_forbidden();
    w.post(&w.amal, &pay, &[("block_exception", "1")])
        .await
        .assert_status(303);
}

/// AC5: opening a new freight job for C is refused; C's open freight jobs
/// continue.
#[renox::test]
async fn a_new_freight_job_is_refused_and_an_open_one_continues() {
    let (w, client) = scene().await;
    let id = client.0.to_string();
    w.post(
        &w.dispatcher,
        "/freight",
        &[
            ("client_party_id", id.as_str()),
            ("description", "2 pallets to Muscat"),
        ],
    )
    .await
    .assert_status(303);
    let open = FreightJob::query()
        .first(w.db())
        .await
        .unwrap()
        .expect("the job is open");
    assert_eq!(open.status, FreightStatus::Booked);

    w.block_party(client, "non_payment").await;
    w.post(
        &w.dispatcher,
        "/freight",
        &[
            ("client_party_id", id.as_str()),
            ("description", "A second job"),
        ],
    )
    .await
    .assert_status(422)
    .assert_see(NEUTRAL);
    assert_eq!(FreightJob::query().count(w.db()).await.unwrap(), 1);
    // Only a holder of `freight.manage` opens one.
    w.post(
        &w.hub,
        "/freight",
        &[("client_party_id", id.as_str()), ("description", "x")],
    )
    .await
    .assert_forbidden();

    // The open job continues.
    w.post(
        &w.dispatcher,
        &format!("/freight/{}/note", open.id),
        &[("note", "Customs cleared")],
    )
    .await
    .assert_status(303);
    let after = FreightJob::find(w.db(), open.id).await.unwrap().unwrap();
    assert_eq!(after.status, FreightStatus::Booked);
    assert_eq!(after.notes, "Customs cleared");
}
