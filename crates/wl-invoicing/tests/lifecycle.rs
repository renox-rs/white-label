//! Drafting, issuing, allocating and ageing an invoice (wl-invoicing §4,
//! `FIN.RUL.02`): the acceptance checks AC2, AC3 and AC5 of #64.

mod support;

use renox::chrono::Datelike;
use renox::prelude::*;
use wl_invoicing::{Allocation, InvoiceKind, invoicer};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, VatCategory};

use support::{BLUE_DUNES_EMAIL, boot};

/// The AED amounts of the plan: 1,000.00 plus 5% VAT.
const NET: i64 = 100_000;
const GROSS: i64 = 105_000;

fn number(entity: &str, n: u32) -> String {
    format!("INV-{entity}-{}-{n:04}", today().year())
}

/// AC2: lines of 1,000.00 `standard` and 500.00 `zero_rated` give a `draft`
/// with net 1,500.00, VAT 50.00, gross 1,550.00 and the totals per category.
#[renox::test]
async fn a_draft_has_totals_per_vat_category() {
    let t = boot().await;
    let draft = t
        .draft(
            &[
                (100_000, VatCategory::Standard),
                (50_000, VatCategory::ZeroRated),
            ],
            None,
        )
        .await;
    assert_eq!(draft.status, InvoiceStatus::Draft);
    assert_eq!(draft.kind, InvoiceKind::TaxInvoice);
    assert_eq!(draft.number, None);
    assert_eq!(draft.currency, "AED");
    assert_eq!(draft.net_total, 150_000);
    assert_eq!(draft.vat_total, 5_000);
    assert_eq!(draft.gross_total, 155_000);
    assert_eq!(draft.balance, 155_000);

    let totals = invoicer::totals_of(t.db(), draft.id).await.unwrap();
    assert_eq!(totals.categories.len(), 2);
    let standard = totals.category(VatCategory::Standard).unwrap();
    assert_eq!((standard.net, standard.vat), (100_000, 5_000));
    let zero_rated = totals.category(VatCategory::ZeroRated).unwrap();
    assert_eq!((zero_rated.net, zero_rated.vat), (50_000, 0));

    // The kernel contract reads it back.
    let summary = wl_kernel::invoicing::invoicer(t.state())
        .find(t.db(), InvoiceId(draft.id))
        .await
        .unwrap()
        .expect("the invoice is found");
    assert_eq!(summary.status, InvoiceStatus::Draft);
    assert_eq!(summary.gross_total, 155_000);
    assert_eq!(summary.balance, 155_000);
}

/// AC2: line VAT rounds half-up per line (AED 0.10 at 5% is AED 0.01).
#[renox::test]
async fn line_vat_rounds_half_up_per_line() {
    let t = boot().await;
    let draft = t
        .draft(
            &[(10, VatCategory::Standard), (10, VatCategory::Standard)],
            None,
        )
        .await;
    // 0.5 fils per line rounds up to 1 fil, and the total is their sum.
    assert_eq!(draft.net_total, 20);
    assert_eq!(draft.vat_total, 2);
    assert_eq!(draft.gross_total, 22);
}

/// AC3: numbers are gapless per entity and year, the snapshots are taken at
/// issue, `InvoiceIssued` is emitted and the due date follows the terms.
#[renox::test]
async fn issuing_numbers_the_invoice_and_snapshots_the_issuer_and_party() {
    let t = boot().await;
    let a = t.user_a().await;
    let first = t.issued(&[(NET, VatCategory::Standard)], None, &a).await;
    let second = t.issued(&[(NET, VatCategory::Standard)], None, &a).await;
    assert_eq!(first.number.as_deref(), Some(number("PGP", 1).as_str()));
    assert_eq!(second.number.as_deref(), Some(number("PGP", 2).as_str()));
    assert_eq!(first.status, InvoiceStatus::Issued);
    assert_eq!(first.owner_id, Some(a.id));
    assert_eq!(first.issue_date, Some(today()));
    assert_eq!(first.due_date, Some(t.day(30)));

    // The entity's TRN is edited afterwards; the invoice keeps the old one.
    let mut pgp = LegalEntity::default_entity(t.db()).await.unwrap();
    assert_eq!(pgp.trn.as_deref(), Some("100000000000003"));
    pgp.trn = Some("100000000000011".to_owned());
    pgp.save(t.db()).await.unwrap();
    let first = t.reload(&first).await;
    let issuer = first.issuer_snapshot.as_ref().expect("an issuer snapshot");
    assert_eq!(issuer.0.code, "PGP");
    assert_eq!(issuer.0.trn.as_deref(), Some("100000000000003"));

    let party = first.party_snapshot.as_ref().expect("a party snapshot");
    assert_eq!(party.0.legal_name_en, "Blue Dunes LLC");
    assert_eq!(party.0.billing_email.as_deref(), Some(BLUE_DUNES_EMAIL));

    let issued = t.issued.all();
    assert_eq!(issued.len(), 2);
    assert_eq!(issued[0].invoice.0, first.id);
    assert_eq!(issued[0].total, GROSS);
    assert_eq!(issued[0].due_date, t.day(30));
    assert_eq!(t.history(&first).await, ["issue"]);
}

/// AC3: a draft's own due date is kept.
#[renox::test]
async fn a_due_date_on_the_draft_is_kept() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t
        .issued(&[(NET, VatCategory::Standard)], Some(t.day(10)), &a)
        .await;
    assert_eq!(invoice.due_date, Some(t.day(10)));
}

/// AC3: an issue that is refused uses no number, so the next one has none
/// missing; a refusal writes nothing.
#[renox::test]
async fn a_refused_issue_leaves_no_gap_in_the_numbers() {
    let t = boot().await;
    let a = t.user_a().await;
    let empty = t.draft(&[], None).await;
    t.post_issue(&empty, &a).await.assert_status(422);
    let empty = t.reload(&empty).await;
    assert_eq!(empty.status, InvoiceStatus::Draft);
    assert_eq!(empty.number, None);
    assert!(empty.issuer_snapshot.is_none() && empty.party_snapshot.is_none());
    assert!(t.issued.all().is_empty());

    let first = t.issued(&[(NET, VatCategory::Standard)], None, &a).await;
    assert_eq!(first.number.as_deref(), Some(number("PGP", 1).as_str()));
}

/// AC3: a `standard` line from an entity without a TRN is refused (422); a
/// line without VAT is not.
#[renox::test]
async fn a_standard_line_needs_a_vat_registered_entity() {
    let t = boot().await;
    let a = t.user_a().await;
    let free_zone = LegalEntity::query()
        .where_(LegalEntity::CODE.eq("PGFZ"))
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert!(!free_zone.vat_registered);
    let of = |price, category| {
        wl_kernel::invoicing::InvoiceDraft::new(
            support::BLUE_DUNES,
            wl_kernel::Subject::new("property.lease", 9),
        )
        .legal_entity(free_zone.id)
        .line(wl_kernel::invoicing::DraftLine::new(
            "Rent", price, category,
        ))
    };
    let standard = t.draft_of(of(NET, VatCategory::Standard)).await;
    t.post_issue(&standard, &a).await.assert_status(422);
    assert_eq!(t.reload(&standard).await.status, InvoiceStatus::Draft);

    let exempt = t.draft_of(of(NET, VatCategory::Exempt)).await;
    let exempt = t.issue(&exempt, &a).await;
    assert_eq!(exempt.number.as_deref(), Some(number("PGFZ", 1).as_str()));
}

/// AC3: without `invoices.issue` an issue is refused (403); only a draft can
/// be issued.
#[renox::test]
async fn issuing_needs_the_permission_and_a_draft() {
    let t = boot().await;
    let a = t.user_a().await;
    let viewer = t.viewer().await;
    let draft = t.draft(&[(NET, VatCategory::Standard)], None).await;
    t.post_issue(&draft, &viewer).await.assert_forbidden();
    let issued = t.issue(&draft, &a).await;
    t.post_issue(&issued, &a).await.assert_status(422);
    assert_eq!(t.history(&issued).await, ["issue"]);
}

/// AC5: allocating 420.00 and then 630.00 to an invoice of 1,050.00.
#[renox::test]
async fn allocations_move_the_invoice_to_partially_paid_and_paid() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&[(NET, VatCategory::Standard)], None, &a).await;
    assert_eq!(invoice.balance, GROSS);

    t.pay(&invoice, 1, 42_000).await;
    let partly = t.reload(&invoice).await;
    assert_eq!(partly.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(partly.balance, 63_000);
    assert_eq!(partly.allocated_total, 42_000);
    let rows = Allocation::query()
        .where_(Allocation::INVOICE_ID.eq(invoice.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].amount, 42_000);
    assert_eq!(rows[0].source().to_string(), "payments.payment:1");
    assert!(t.paid.all().is_empty());

    t.pay(&invoice, 2, 63_000).await;
    let paid = t.reload(&invoice).await;
    assert_eq!(paid.status, InvoiceStatus::Paid);
    assert_eq!(paid.balance, 0);
    assert_eq!(paid.allocated_total, GROSS);
    assert_eq!(t.paid.all().len(), 1);
    assert_eq!(t.paid.all()[0].invoice.0, invoice.id);
    assert_eq!(
        t.history(&invoice).await,
        ["issue", "allocate", "allocate_in_full"]
    );
}

/// AC5: an amount above the balance, or 0 or less, is refused and writes
/// nothing; so is an allocation to a draft.
#[renox::test]
async fn an_allocation_above_the_balance_or_not_positive_is_refused() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&[(NET, VatCategory::Standard)], None, &a).await;
    let contract = wl_kernel::invoicing::invoicer(t.state());
    for amount in [GROSS + 1, 0, -100] {
        let refused = contract
            .allocate(
                t.state(),
                InvoiceId(invoice.id),
                wl_kernel::Subject::new("payments.payment", 1),
                amount,
            )
            .await;
        assert!(refused.is_err(), "{amount} fils");
    }
    let same = t.reload(&invoice).await;
    assert_eq!(same.status, InvoiceStatus::Issued);
    assert_eq!(same.balance, GROSS);
    assert_eq!(Allocation::query().count(t.db()).await.unwrap(), 0);

    let draft = t.draft(&[(NET, VatCategory::Standard)], None).await;
    let refused = contract
        .allocate(
            t.state(),
            InvoiceId(draft.id),
            wl_kernel::Subject::new("payments.payment", 2),
            100,
        )
        .await;
    assert!(refused.is_err());
}

/// AC5 (`FIN.RUL.02`): after the due date the overdue run moves an issued
/// invoice with a balance to `overdue` and emits `InvoiceOverdue`; a paid
/// one, and one not yet due, are left alone.
#[renox::test]
async fn the_overdue_run_moves_only_invoices_with_a_balance_past_due() {
    let t = boot().await;
    let a = t.user_a().await;
    let late = t
        .issued(&[(NET, VatCategory::Standard)], Some(t.day(-5)), &a)
        .await;
    let settled = t
        .issued(&[(NET, VatCategory::Standard)], Some(t.day(-5)), &a)
        .await;
    t.pay(&settled, 1, GROSS).await;
    let not_yet = t
        .issued(&[(NET, VatCategory::Standard)], Some(t.day(5)), &a)
        .await;
    let partly = t
        .issued(&[(NET, VatCategory::Standard)], Some(t.day(-1)), &a)
        .await;
    t.pay(&partly, 2, 42_000).await;

    let moved = wl_invoicing::overdue::run(t.state()).await.unwrap();
    assert_eq!(moved, 2);
    assert_eq!(t.reload(&late).await.status, InvoiceStatus::Overdue);
    assert_eq!(t.reload(&partly).await.status, InvoiceStatus::Overdue);
    assert_eq!(t.reload(&settled).await.status, InvoiceStatus::Paid);
    assert_eq!(t.reload(&not_yet).await.status, InvoiceStatus::Issued);

    let events = t.overdue.all();
    assert_eq!(events.len(), 2);
    let first = events.iter().find(|e| e.invoice.0 == late.id).unwrap();
    assert_eq!(first.balance, GROSS);
    assert_eq!(first.days_overdue, 5);
    let second = events.iter().find(|e| e.invoice.0 == partly.id).unwrap();
    assert_eq!(second.balance, 63_000);

    // A second run finds nothing more.
    assert_eq!(wl_invoicing::overdue::run(t.state()).await.unwrap(), 0);
    assert_eq!(t.overdue.all().len(), 2);

    // An overdue invoice can still be paid.
    t.pay(&late, 3, GROSS).await;
    assert_eq!(t.reload(&late).await.status, InvoiceStatus::Paid);
}
