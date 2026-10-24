//! Flagging an invoice dispute (wl-invoicing §5.10, #67 part 7): the
//! acceptance checks AC1, AC2, AC4, AC5, AC8, AC12 and AC13, and the ticket
//! side of AC6 and AC7 (the resolve itself is in `dispute_resolve.rs`).
//! Users are named by id (`DisputeBy::User`), since a test cannot build an
//! `AuthUser`; the ticket calls and the notices are recorded; reminders run
//! on explicit dates.

mod support;

use renox::auth::permissions;
use renox::prelude::*;
use wl_invoicing::dispute::{self, ACKNOWLEDGED, Dispute, DisputeVia, UNASSIGNED};
use wl_invoicing::dispute_resolve::{self, ResolveInput};
use wl_invoicing::permissions as perm;
use wl_invoicing::{CreditNote, Invoice, WlInvoicing, overdue, reminders};
use wl_kernel::approvals::{self, ApprovalPolicy};
use wl_kernel::invoicing::{
    DisputeBy, DisputeOutcome, DisputeRequest, InvoiceId, InvoiceStatus, VatCategory,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::{Assignee, TicketChannel, TicketId};

use support::{BLUE_DUNES, Booted, QUIET_SANDS, boot, boot_with};

/// Gross AED 3,000.00, 800.00, 400.00 and 800.00 (zero rated): AED 5,000.00.
const LINES: [(i64, VatCategory); 4] = [
    (300_000, VatCategory::ZeroRated),
    (80_000, VatCategory::ZeroRated),
    (40_000, VatCategory::ZeroRated),
    (80_000, VatCategory::ZeroRated),
];
const GROSS: i64 = 500_000;

fn is_422<T>(result: Result<T>) -> bool {
    matches!(
        result,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    )
}

fn by(user: &User) -> DisputeBy {
    DisputeBy::User { id: user.id }
}

fn request(invoice: &Invoice, by: DisputeBy) -> DisputeRequest {
    DisputeRequest::new(InvoiceId(invoice.id), "wrong hours", by)
}

async fn stored(t: &Booted, id: i64) -> Dispute {
    Dispute::find(t.db(), id).await.unwrap().unwrap()
}

/// Sets the `FIN.ACL.07` limit (fils): a credit above it needs an approval.
async fn set_limit(t: &Booted, fils: i64) {
    ApprovalPolicy::query()
        .where_(ApprovalPolicy::ACTION.eq("invoicing.dispute_credit"))
        .update(t.db(), &[("threshold", &fils)])
        .await
        .unwrap();
}

/// AC1: the dispute flags the invoice without moving its status, due date or
/// balance; a `billing_dispute` ticket on the invoice is assigned to the
/// owner and linked to the dispute; the party gets one acknowledgement and
/// `InvoiceDisputed` is emitted once, with the ticket.
#[renox::test]
async fn disputing_flags_the_invoice_opens_a_ticket_and_tells_the_party() {
    let t = boot().await;
    let a = t.user_a().await;
    let due = t.day(10);
    let invoice = t.issued(&LINES, Some(due), &a).await;
    assert_eq!(invoice.balance, GROSS);

    let id = dispute::raise(t.state(), request(&invoice, by(&a)).amount(120_000))
        .await
        .unwrap();

    let after = t.reload(&invoice).await;
    assert!(after.disputed);
    assert_eq!(after.dispute_id, Some(id));
    assert_eq!(after.status, InvoiceStatus::Issued);
    assert_eq!(after.due_date, Some(due));
    assert_eq!(after.balance, GROSS);

    let dispute = stored(&t, id).await;
    assert_eq!(dispute.reason, "wrong hours");
    assert_eq!(dispute.amount, 120_000);
    assert_eq!(dispute.raised_by_user_id, Some(a.id));
    assert_eq!(dispute.raised_via, DisputeVia::Staff);
    assert!(dispute.is_open());
    assert_eq!(dispute.ticket_id, Some(1));

    let opened = t.tickets.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].category_key, "billing_dispute");
    assert_eq!(opened[0].subject, invoice.subject());
    assert_eq!(opened[0].source, Some(dispute.subject()));
    assert_eq!(opened[0].assignee, Some(Assignee::user(a.id)));
    assert!(opened[0].description.contains("wrong hours"));
    assert!(opened[0].description.contains("AED 1,200.00"));

    let acknowledged = t.notices.of_type(ACKNOWLEDGED);
    assert_eq!(acknowledged.len(), 1);
    assert!(matches!(
        acknowledged[0].recipients[..],
        [NotifyRecipient::Party { id }] if id == BLUE_DUNES
    ));
    assert!(t.notices.of_type(UNASSIGNED).is_empty(), "the owner has it");

    let events = t.disputed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].invoice, invoice.invoice_id());
    assert_eq!(events[0].party, BLUE_DUNES);
    assert_eq!(events[0].dispute, id);
    assert_eq!(events[0].amount, 120_000);
    assert_eq!(events[0].ticket, Some(TicketId(1)));
    assert_eq!(events[0].raised_by, by(&a));
}

/// AC2: no reminder on any date while disputed, and the overdue run still
/// moves the invoice to `overdue` the day after its due date.
#[renox::test]
async fn a_disputed_invoice_gets_no_reminder_but_still_becomes_overdue() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&LINES, Some(t.day(10)), &a).await;
    dispute::raise(t.state(), request(&invoice, by(&a)).amount(120_000))
        .await
        .unwrap();

    for n in -10..=60 {
        assert_eq!(reminders::run(t.state(), t.day(n)).await.unwrap(), 0, "{n}");
    }
    assert!(t.notices.of_type(reminders::BEFORE_DUE).is_empty());
    assert!(t.notices.of_type(reminders::AFTER_DUE).is_empty());

    // Due yesterday: overdue, and still disputed.
    let late = t.issued(&LINES, Some(t.day(-1)), &a).await;
    dispute::raise(t.state(), request(&late, by(&a)).amount(120_000))
        .await
        .unwrap();
    overdue::run(t.state()).await.unwrap();
    let late = t.reload(&late).await;
    assert_eq!(late.status, InvoiceStatus::Overdue);
    assert!(late.disputed);
    assert_eq!(reminders::run(t.state(), t.day(30)).await.unwrap(), 0);
}

/// AC4: ticked lines default the amount to their gross sum and both travel
/// with the event; an amount with the lines is taken as given; a line that is
/// not on the invoice (it is on another one) is refused.
#[renox::test]
async fn ticked_lines_set_the_amount_and_travel_with_the_event() {
    let t = boot().await;
    let a = t.user_a().await;
    let first = t.issued(&LINES, None, &a).await;
    let id = dispute::raise(t.state(), request(&first, by(&a)).lines([2, 3]))
        .await
        .unwrap();
    let dispute = stored(&t, id).await;
    assert_eq!(dispute.amount, 120_000);
    assert_eq!(dispute.positions(), [2, 3]);
    let events = t.disputed.all();
    assert_eq!(events.len(), 1);
    let carried: Vec<(i32, i64)> = events[0]
        .lines
        .iter()
        .map(|line| (line.position, line.amount))
        .collect();
    assert_eq!(carried, [(2, 80_000), (3, 40_000)]);
    let description = &t.tickets.opened()[0].description;
    assert!(description.contains("Line 2") && description.contains("Line 3"));
    assert!(!description.contains("Line 1"));

    let second = t.issued(&LINES, None, &a).await;
    let id = dispute::raise(
        t.state(),
        request(&second, by(&a)).lines([2, 3]).amount(100_000),
    )
    .await
    .unwrap();
    assert_eq!(stored(&t, id).await.amount, 100_000);

    // The other invoice has five lines; its line 5 is not on this one.
    let five = [LINES[0], LINES[1], LINES[2], LINES[3], LINES[0]];
    let wide = t.issued(&five, None, &a).await;
    let third = t.issued(&LINES, None, &a).await;
    let refused = dispute::raise(t.state(), request(&third, by(&a)).lines([5])).await;
    assert!(is_422(refused));
    assert!(!t.reload(&third).await.disputed);
    dispute::raise(t.state(), request(&wide, by(&a)).lines([5]))
        .await
        .unwrap();
}

/// AC5: more than the balance, an invoice already disputed, a draft, a paid
/// or written-off invoice and a blank reason are refused, and nothing is
/// opened or sent; of two raises at once exactly one succeeds.
#[renox::test]
async fn refused_disputes_change_nothing_and_only_one_raise_wins() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&LINES, None, &a).await;

    let too_much = dispute::raise(t.state(), request(&invoice, by(&a)).amount(600_000)).await;
    assert!(is_422(too_much));
    let blank = dispute::raise(
        t.state(),
        DisputeRequest::new(InvoiceId(invoice.id), "   ", by(&a)).amount(1_000),
    )
    .await;
    assert!(is_422(blank));
    let draft = t.draft(&LINES, None).await;
    let drafted = dispute::raise(t.state(), request(&draft, by(&a)).amount(1_000)).await;
    assert!(is_422(drafted));
    assert!(!t.reload(&invoice).await.disputed);
    assert!(t.tickets.opened().is_empty());
    assert!(t.notices.all().is_empty());
    assert!(t.disputed.all().is_empty());

    let paid = t.issued(&LINES, None, &a).await;
    t.pay(&paid, 1, GROSS).await;
    assert_eq!(t.reload(&paid).await.status, InvoiceStatus::Paid);
    assert!(is_422(
        dispute::raise(t.state(), request(&paid, by(&a)).amount(1_000)).await
    ));
    let off = t.issued(&LINES, None, &a).await;
    Invoice::query()
        .where_(Invoice::ID.eq(off.id))
        .update(t.db(), &[("status", &"written_off")])
        .await
        .unwrap();
    assert!(is_422(
        dispute::raise(t.state(), request(&off, by(&a)).amount(1_000)).await
    ));

    // Two raises at once: exactly one wins.
    let (one, two) = renox::tokio::join!(
        dispute::raise(t.state(), request(&invoice, by(&a)).amount(1_000)),
        dispute::raise(t.state(), request(&invoice, by(&a)).amount(2_000)),
    );
    assert_eq!(one.is_ok() as u8 + two.is_ok() as u8, 1);
    assert!(is_422(if one.is_ok() { two } else { one }));
    let count = Dispute::query()
        .where_(Dispute::INVOICE_ID.eq(invoice.id))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(t.disputed.all().len(), 1);

    // And once flagged, a third is refused.
    assert!(is_422(
        dispute::raise(t.state(), request(&invoice, by(&a)).amount(1_000)).await
    ));
}

/// AC6 (ticket side): a credit within the limit resolves the ticket with the
/// note and tells the party the credit note number.
#[renox::test]
async fn a_credit_resolves_the_ticket_and_tells_the_party_the_note() {
    let t = boot().await;
    set_limit(&t, 1_000_000).await;
    let a = t.user_a().await;
    let cleo = t
        .user(
            "Cleo",
            &[perm::VIEW, perm::ISSUE, perm::DISPUTE, perm::CREDIT],
        )
        .await;
    let invoice = t.issued(&LINES, None, &a).await;
    dispute::raise(t.state(), request(&invoice, by(&a)).amount(120_000))
        .await
        .unwrap();
    assert!(t.tickets.resolved().is_empty());

    let done = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("Agreed on the call", 120_000),
        by(&cleo),
    )
    .await
    .unwrap();
    assert!(done.is_resolved());

    let after = t.reload(&invoice).await;
    assert!(!after.disputed);
    assert_eq!(after.balance, GROSS - 120_000);
    let note = CreditNote::find(t.db(), done.credit_note.unwrap())
        .await
        .unwrap()
        .unwrap();
    let number = note.number.clone().expect("the credit note is numbered");
    assert_eq!(
        t.tickets.resolved(),
        [(TicketId(1), "Agreed on the call".to_owned())]
    );
    let notices = t.notices.of_type("invoicing.dispute_resolved");
    assert_eq!(notices.len(), 1);
    assert!(notices[0].data.to_string().contains(&number));
    let events = t.resolved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, DisputeOutcome::Credited);
    assert_eq!(events[0].credited_amount, Some(120_000));
    assert_eq!(
        events[0].credit_note.as_ref().map(|c| c.subject_id),
        Some(note.id)
    );
}

/// AC7 (ticket side): while the approval waits, the ticket stays open; the
/// approval by a second user resolves it. A rejection leaves it open.
#[renox::test]
async fn the_ticket_waits_for_the_approval() {
    let t = boot().await;
    set_limit(&t, 50_000).await;
    let a = t.user_a().await;
    let cleo = t
        .user(
            "Cleo",
            &[perm::VIEW, perm::ISSUE, perm::DISPUTE, perm::CREDIT],
        )
        .await;
    let erin = t.user("Erin", &[perm::VIEW, perm::CREDIT]).await;
    let invoice = t.issued(&LINES, None, &a).await;
    dispute::raise(t.state(), request(&invoice, by(&a)).amount(120_000))
        .await
        .unwrap();

    let first = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("Agreed", 120_000),
        by(&cleo),
    )
    .await
    .unwrap();
    let request_id = first.pending_request.expect("an approval is pending");
    assert!(t.reload(&invoice).await.disputed);
    assert!(t.tickets.resolved().is_empty());
    assert!(t.resolved.all().is_empty());

    approvals::reject(t.state(), request_id, &erin, "too much")
        .await
        .unwrap();
    assert!(t.reload(&invoice).await.disputed);
    assert!(t.tickets.resolved().is_empty());

    let second = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("Agreed again", 120_000),
        by(&cleo),
    )
    .await
    .unwrap();
    let request_id = second.pending_request.expect("a new approval is pending");
    approvals::approve(t.state(), request_id, &erin, "ok")
        .await
        .unwrap();
    assert!(!t.reload(&invoice).await.disputed);
    assert_eq!(
        t.tickets.resolved(),
        [(TicketId(1), "Agreed again".to_owned())]
    );
    assert_eq!(t.resolved.all().len(), 1);
}

/// AC8: `upheld` clears the flag and leaves the due date; no reminder goes
/// out during the grace period; the latest step due goes out once, the day
/// it ends, and nothing after. (The upheld day is the real today, so the
/// invoice fell due 19 days before it: steps 3/1/7/14 as in the example.)
#[renox::test]
async fn upheld_pauses_reminders_through_the_grace_period_then_sends_one() {
    let t = boot().await;
    let a = t.user_a().await;
    let due = t.day(-19);
    let invoice = t.issued(&LINES, Some(due), &a).await;
    dispute::raise(t.state(), request(&invoice, by(&a)).amount(120_000))
        .await
        .unwrap();
    dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::upheld("Hours verified"),
        by(&a),
    )
    .await
    .unwrap();
    let after = t.reload(&invoice).await;
    assert!(!after.disputed);
    assert_eq!(after.due_date, Some(due));
    assert_eq!(after.dispute_grace_until, Some(t.day(7)));

    for n in 0..7 {
        assert_eq!(reminders::run(t.state(), t.day(n)).await.unwrap(), 0, "{n}");
    }
    assert!(t.notices.of_type(reminders::AFTER_DUE).is_empty());
    assert_eq!(reminders::run(t.state(), t.day(7)).await.unwrap(), 1);
    let sent = t.notices.of_type(reminders::AFTER_DUE);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].data["days"], 14);
    for n in [7, 8, 20, 40] {
        assert_eq!(reminders::run(t.state(), t.day(n)).await.unwrap(), 0, "{n}");
    }
    assert_eq!(t.notices.of_type(reminders::AFTER_DUE).len(), 1);
}

/// AC12: a party's dispute is refused by default; with portal disputes on it
/// works for the invoice's own party and not for another.
#[renox::test]
async fn a_party_disputes_only_when_the_portal_allows_it_and_only_its_own() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&LINES, None, &a).await;
    let own = DisputeBy::Party { id: BLUE_DUNES };
    let refused = dispute::raise(t.state(), request(&invoice, own).amount(1_000)).await;
    assert!(matches!(refused, Err(Error::Forbidden)));
    assert!(!t.reload(&invoice).await.disputed);

    let t = boot_with(WlInvoicing::new().portal_disputes(true)).await;
    let a = t.user_a().await;
    let invoice = t.issued(&LINES, None, &a).await;
    let other = DisputeBy::Party { id: QUIET_SANDS };
    let refused = dispute::raise(t.state(), request(&invoice, other).amount(1_000)).await;
    assert!(matches!(refused, Err(Error::Forbidden)));
    let id = dispute::raise(t.state(), request(&invoice, own).amount(1_000))
        .await
        .unwrap();
    let dispute = stored(&t, id).await;
    assert_eq!(dispute.raised_via, DisputeVia::Portal);
    assert_eq!(dispute.raised_by_party_id, Some(BLUE_DUNES.0));
    assert_eq!(t.tickets.opened()[0].channel, TicketChannel::Portal);
    assert_eq!(t.disputed.all()[0].raised_by, own);
}

/// AC13: an owner who no longer holds `invoices.issue` does not get the
/// ticket: it opens without a user, for ticketing to route to the
/// receivables role (BL-TKT-12), and every holder is told. With
/// ticketing off the dispute is flagged without a ticket and resolving works.
#[renox::test]
async fn without_an_owner_the_ticket_is_unassigned_and_without_ticketing_there_is_none() {
    let t = boot().await;
    let olga = t
        .user("Olga", &[perm::VIEW, perm::ISSUE, perm::DISPUTE])
        .await;
    let b = t.user_b().await;
    let invoice = t.issued(&LINES, None, &olga).await;
    assert_eq!(invoice.owner_id, Some(olga.id));
    // Olga loses the right to issue (and keeps the right to dispute).
    permissions::define_role(t.db(), "role_olga", &[perm::VIEW, perm::DISPUTE])
        .await
        .unwrap();

    let id = dispute::raise(t.state(), request(&invoice, by(&olga)).amount(1_000))
        .await
        .unwrap();
    assert_eq!(stored(&t, id).await.ticket_id, Some(1));
    let opened = t.tickets.opened();
    assert_eq!(
        opened[0].assignee, None,
        "routed to the role, not to a user"
    );
    let unassigned = t.notices.of_type(UNASSIGNED);
    assert_eq!(unassigned.len(), 1);
    let to: Vec<i64> = unassigned[0]
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect();
    assert!(to.contains(&b.id));
    assert!(!to.contains(&olga.id));
    assert_eq!(t.notices.of_type(ACKNOWLEDGED).len(), 1);

    // Ticketing off.
    t.tickets.off(true);
    let plain = t.issued(&LINES, None, &b).await;
    let id = dispute::raise(t.state(), request(&plain, by(&b)).amount(1_000))
        .await
        .unwrap();
    let dispute = stored(&t, id).await;
    assert_eq!(dispute.ticket_id, None);
    assert!(t.reload(&plain).await.disputed);
    assert_eq!(t.tickets.opened().len(), 1, "no second ticket");
    assert_eq!(t.disputed.all()[1].ticket, None);
    dispute_resolve::resolve(
        t.state(),
        plain.id,
        ResolveInput::upheld("Fine as it was"),
        by(&b),
    )
    .await
    .unwrap();
    let after = t.reload(&plain).await;
    assert!(!after.disputed);
    assert_eq!(stored(&t, id).await.outcome, Some(DisputeOutcome::Upheld));
}

/// #156 point 5, BL-TKT-12: an invoice the system issued (no owner) opens
/// the dispute ticket without a user, for ticketing to route to the
/// receivables role (the holders of `invoices.issue` in the invoice's
/// scope), and every holder is told.
#[renox::test]
async fn a_system_issued_invoice_is_assigned_to_the_receivables_role() {
    let t = boot().await;
    let amal = t.user_a().await;
    let bilal = t.user_b().await;
    let invoice = t.issued(&LINES, None, &amal).await;
    Invoice::query()
        .where_(Invoice::ID.eq(invoice.id))
        .update(t.db(), &[("owner_id", &None::<i64>)])
        .await
        .unwrap();
    assert_eq!(t.reload(&invoice).await.owner_id, None);

    dispute::raise(t.state(), request(&invoice, by(&amal)).amount(1_000))
        .await
        .unwrap();
    assert_eq!(
        t.tickets.opened()[0].assignee,
        None,
        "never the lowest user id as a stand-in for the role"
    );
    let told: Vec<i64> = t.notices.of_type(UNASSIGNED)[0]
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect();
    assert!(told.contains(&amal.id) && told.contains(&bilal.id));
}

/// #156 point 5: an owner who has been removed (the user is gone) is
/// replaced by the receivables role; one who still holds the permission
/// keeps the ticket.
#[renox::test]
async fn a_removed_owner_hands_the_ticket_to_the_receivables_role() {
    let t = boot().await;
    let mut nadia = t.clerk("Nadia").await;
    let b = t.user_b().await;
    let invoice = t.issued(&LINES, None, &nadia).await;
    assert_eq!(invoice.owner_id, Some(nadia.id));
    nadia.delete(t.db()).await.unwrap();

    dispute::raise(t.state(), request(&invoice, by(&b)).amount(1_000))
        .await
        .unwrap();
    assert_eq!(
        t.tickets.opened()[0].assignee,
        None,
        "the receivables role, routed by ticketing"
    );
    assert_eq!(t.notices.of_type(UNASSIGNED).len(), 1);
}
