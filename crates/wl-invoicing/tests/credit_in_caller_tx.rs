//! BL-KRN-11 (#459): the money contracts join the caller's transaction. The
//! `Invoicer` variants (`credit_partly_in`, `credit_in_full_in`,
//! `discard_draft_in`) run inside a transaction the caller opened: nothing
//! is committed by them, the credit note number comes from the series in
//! that transaction, and the events wait on the caller's `AfterCommit`.

mod support;

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_invoicing::{CreditNote, Invoice};
use wl_kernel::after_commit::AfterCommit;
use wl_kernel::invoicing::{
    CreditLine, CreditNoteStatus, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};

use support::{Booted, boot};

/// One zero-rated line, so the credit's gross is its net.
const LINES: [(i64, VatCategory); 1] = [(500_000, VatCategory::ZeroRated)];

/// AED 3,193.55.
const CREDIT: i64 = 319_355;

/// The credit notes issued, as the transitions emitted (`fake_events`
/// records them; `CreditNoteIssued` is built from these by the module's
/// listener).
fn issued_events(t: &Booted) -> usize {
    t.app
        .emitted::<Transitioned<CreditNote>>()
        .into_iter()
        .filter(|event| event.to == "issued")
        .count()
}

async fn notes(t: &Booted) -> Vec<CreditNote> {
    CreditNote::query()
        .order_by(CreditNote::ID)
        .get(t.db())
        .await
        .unwrap()
}

async fn credit_in_tx(t: &Booted, invoice: &Invoice, commit: bool) -> Option<CreditNote> {
    let state = t.state();
    let mut tx = state.db.begin().await.unwrap();
    let mut after = AfterCommit::new();
    let id = invoicer(state)
        .credit_partly_in(
            state,
            &mut tx,
            &mut after,
            InvoiceId(invoice.id),
            vec![CreditLine::new(1, CREDIT)],
            "Rent after the move-out date".into(),
            None,
        )
        .await
        .expect("the credit note is issued in the transaction");
    assert!(!after.is_empty(), "the events wait for the commit");
    if !commit {
        // The caller's next step fails: everything is rolled back.
        drop(tx);
        drop(after);
        return None;
    }
    tx.commit().await.unwrap();
    after.run(state).await.unwrap();
    Some(CreditNote::find_or_404(t.db(), id.0).await.unwrap())
}

/// Done when 1: a caller transaction that issues a credit note of AED
/// 3,193.55 through the variant and then fails leaves no credit note, its
/// number unused and no `CreditNoteIssued`.
#[renox::test]
async fn a_rolled_back_caller_leaves_no_credit_note_number_or_event() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    t.app.fake_events();

    assert!(credit_in_tx(&t, &invoice, false).await.is_none());
    assert!(notes(&t).await.is_empty());
    assert_eq!(issued_events(&t), 0);
    let after = t.reload(&invoice).await;
    assert_eq!(after.status, InvoiceStatus::Issued);
    assert_eq!(after.balance, invoice.gross_total);

    // The number was given back: the next credit note takes the first one.
    let note = credit_in_tx(&t, &invoice, true).await.expect("committed");
    assert!(
        note.number.as_deref().is_some_and(|n| n.ends_with("-0001")),
        "{:?}",
        note.number
    );
}

/// Done when 2: the same run committed gives the credit note, the number
/// and the event once.
#[renox::test]
async fn a_committed_caller_gets_the_credit_note_its_number_and_one_event() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    t.app.fake_events();

    let note = credit_in_tx(&t, &invoice, true).await.expect("committed");
    assert_eq!(note.gross_total, CREDIT);
    assert_eq!(note.status, CreditNoteStatus::Applied);
    assert!(note.number.as_deref().is_some_and(|n| n.ends_with("-0001")));
    assert_eq!(notes(&t).await.len(), 1);
    assert_eq!(issued_events(&t), 1);
    let after = t.reload(&invoice).await;
    assert_eq!(after.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(after.balance, invoice.gross_total - CREDIT);
}

/// `credit_in_full_in` rolled back leaves the invoice `issued`; committed it
/// is `credited`.
#[renox::test]
async fn crediting_in_full_follows_the_callers_commit() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    let state = t.state();

    let mut tx = state.db.begin().await.unwrap();
    let mut after = AfterCommit::new();
    invoicer(state)
        .credit_in_full_in(
            state,
            &mut tx,
            &mut after,
            InvoiceId(invoice.id),
            "Lease cancelled".into(),
            None,
        )
        .await
        .unwrap();
    drop(tx);
    drop(after);
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Issued);
    assert!(notes(&t).await.is_empty());

    let mut tx = state.db.begin().await.unwrap();
    let mut after = AfterCommit::new();
    invoicer(state)
        .credit_in_full_in(
            state,
            &mut tx,
            &mut after,
            InvoiceId(invoice.id),
            "Lease cancelled".into(),
            None,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    after.run(state).await.unwrap();
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Credited);
}

/// `discard_draft_in` deletes the draft only when the caller commits.
#[renox::test]
async fn a_draft_is_discarded_only_when_the_caller_commits() {
    let t = boot().await;
    let draft = t.draft(&LINES, None).await;
    let state = t.state();

    let mut tx = state.db.begin().await.unwrap();
    assert!(
        invoicer(state)
            .discard_draft_in(&mut tx, InvoiceId(draft.id))
            .await
            .unwrap()
    );
    drop(tx);
    assert!(Invoice::find(t.db(), draft.id).await.unwrap().is_some());

    let mut tx = state.db.begin().await.unwrap();
    assert!(
        invoicer(state)
            .discard_draft_in(&mut tx, InvoiceId(draft.id))
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    assert!(Invoice::find(t.db(), draft.id).await.unwrap().is_none());
}
