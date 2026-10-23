//! BL-AUT-08 (#404) against the REAL shared modules of the host (wl-invoicing,
//! wl-notify, wl-documents, wl-scheduling, wl-parties): a withdrawn claim tells
//! the insurer in a real message, the work done is invoiced to the customer on
//! the real books and never to the insurer (no payer bill), the private
//! estimate unsigned after seven days is cancelled by the hourly rule, and the
//! fee of a declined job card is invoiced to the customer. The app's own tests
//! (`apps/auto/tests/aut_08_withdraw.rs`) show the same points on stand-ins.

mod support;

use auto::claims::{self, Withdrawal};
use auto::declines;
use auto::job_cards;
use auto::{ClaimStatus, InsuranceClaim, JobCard, JobCardStatus};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::autofx::{approved_job, boot, sent_of, shop};
use wl_kernel::PartyId;
use wl_kernel::invoicing::{InvoiceStatus, invoicer};
use wl_kernel::payer_bills::payer_bills;

/// Withdrawn after approval with *stop*: the real message to the insurer, the
/// real invoice of AED 1,200 + VAT to the customer, no payer bill.
#[renox::test]
async fn aut_08_real_a_withdrawn_claim_bills_the_work_done_to_the_customer_only() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let job = approved_job(&w, &s, &[("Panel repair", 6_000, 20_000)], &[0]).await;

    let cancelled = claims::withdraw(
        w.state(),
        &s.carla,
        job.claim.id,
        Withdrawal::new("Withdrawn by the customer")
            .document("withdrawal-letter-80")
            .stop(),
    )
    .await
    .unwrap();
    assert_eq!(cancelled.status, ClaimStatus::Cancelled);

    let letters = sent_of(&w, "auto.claim_withdrawal_letter").await;
    assert_eq!(letters.len(), 1, "the insurer is told in writing");
    assert_eq!(letters[0].address.as_deref(), Some("claims@orient.example"));
    assert!(letters[0].rendered_body.contains("OR-2026-118"));

    let card = JobCard::find_or_404(w.db(), job.card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::Cancelled);
    let billed = invoicer(w.state())
        .find(
            w.db(),
            wl_kernel::invoicing::InvoiceId(card.invoice_id.expect("the invoice")),
        )
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(billed.party, s.owner);
    assert_eq!(billed.gross_total, 126_000, "AED 1,200 and 5% VAT");
    assert_ne!(billed.status, InvoiceStatus::Draft);
    assert!(
        payer_bills(w.state())
            .of_source(w.db(), &cancelled.subject())
            .await
            .unwrap()
            .is_empty(),
        "the insurer is not billed after the withdrawal"
    );
}

/// *Continue privately*, unsigned after seven days: the hourly rule cancels the
/// job card and the real invoice for the work done goes to the customer.
#[renox::test]
async fn aut_08_real_an_unsigned_private_estimate_is_cancelled_after_seven_days() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let job = approved_job(&w, &s, &[("Panel repair", 6_000, 20_000)], &[0]).await;
    claims::withdraw(
        w.state(),
        &s.carla,
        job.claim.id,
        Withdrawal::new("Withdrawn by the customer")
            .document("withdrawal-letter-80")
            .continue_privately(),
    )
    .await
    .unwrap();
    let card = JobCard::find_or_404(w.db(), job.card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::AwaitingApproval);
    assert!(card.private_estimate);
    assert!(card.invoice_id.is_none());
    let pending = job_cards::pending_round(w.db(), card.id).await.unwrap();
    assert_eq!(pending.expect("a private estimate").net_total, 120_000);

    let entered = renox::db::now() - TimeDelta::days(8);
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(w.db(), &[("awaiting_since", &Some(entered))])
        .await
        .unwrap();
    let done = declines::expire_estimates(w.state(), renox::db::now())
        .await
        .unwrap();
    assert_eq!(done.cancelled, [card.id]);
    let card = JobCard::find_or_404(w.db(), card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::Cancelled);
    let billed = invoicer(w.state())
        .find(
            w.db(),
            wl_kernel::invoicing::InvoiceId(card.invoice_id.expect("the invoice")),
        )
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(billed.party, PartyId(s.owner.0));
    assert_eq!(billed.gross_total, 126_000);
    let claim = InsuranceClaim::find_or_404(w.db(), job.claim.id)
        .await
        .unwrap();
    assert!(claim.payer_bill_id.is_none());
}

/// The e-mail channel, where the insurer's mailbox refuses everything for good.
struct RefusingEmail;

impl wl_notify::ChannelProvider for RefusingEmail {
    fn name(&self) -> &'static str {
        "refusing-email"
    }

    fn channel(&self) -> wl_kernel::notify::NotifyChannel {
        wl_kernel::notify::NotifyChannel::Email
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        outgoing: &'a wl_notify::Outgoing,
    ) -> wl_notify::channels::ProviderFuture<'a, std::result::Result<String, wl_notify::SendError>>
    {
        Box::pin(async move {
            if outgoing.address == "claims@orient.example" {
                Err(wl_notify::SendError::permanent("550", "mailbox unknown"))
            } else {
                Ok(format!("mail:{}", outgoing.message.0))
            }
        })
    }
}

/// The withdrawal letter cannot be delivered (the insurer's mailbox refuses it
/// for good): the real notifier gives up and, the letter being statutory, the
/// insurance coordinator gets a real task about the claim (#404, #414 point 11).
#[renox::test]
async fn aut_08_real_a_withdrawal_letter_that_gives_up_opens_a_task_for_the_coordinator() {
    let w = support::autofx::boot_tuned(
        host::Tuning::default().notify(|n| n.provider(RefusingEmail)),
        |app| app,
    )
    .await;
    let s = shop(&w, None).await;
    let job = approved_job(&w, &s, &[("Panel repair", 6_000, 20_000)], &[0]).await;

    claims::withdraw(
        w.state(),
        &s.carla,
        job.claim.id,
        Withdrawal::new("Withdrawn by the customer")
            .document("withdrawal-letter-80")
            .stop(),
    )
    .await
    .unwrap();
    w.app.run_all_jobs().await;

    let tasks: Vec<_> = wl_ticketing::Ticket::query()
        .get(w.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|t| {
            t.source_key
                .as_deref()
                .is_some_and(|k| k.starts_with("notify.statutory:"))
        })
        .collect();
    assert_eq!(tasks.len(), 1, "one task for the coordinator: {tasks:?}");
    assert!(
        tasks[0].title.contains("another means"),
        "{}",
        tasks[0].title
    );
    assert_eq!(
        wl_notify::StatutoryNotice::query()
            .count(w.db())
            .await
            .unwrap(),
        1,
        "the notice is kept until its proof is recorded"
    );
    assert_eq!(tasks[0].assignee_user_id, Some(s.carla.id), "{tasks:?}");
}
