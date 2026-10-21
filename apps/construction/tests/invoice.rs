//! BL-CON-06 (#285): invoicing a certified IPA with accepted and contested
//! deductions. The tax invoice covers the certified work; accepted back- and
//! contra-charges become a supplier bill from the client for the accountant to
//! approve and set off; accepted liquidated damages are a credit note against
//! the invoice; the unpaid part for contested lines is disputed; the retention
//! of the period is held with no tax document. The invoices, the bills and the
//! set-off live in wl-invoicing and wl-payments, which the tests replace with
//! fakes that record what the vertical asks for (the real bill is tested in
//! wl-invoicing, the set-off and its reversal in wl-payments).

mod support;

use construction::retention::withhold_in;
use construction::{
    DeductionKind, DeductionStatus, Ipa, IpaStatus, Project, ProjectStatus, Retention,
    RetentionPart, RetentionStatus,
};
use renox::prelude::*;
use wl_kernel::invoicing::{CreditLine, DraftLine, InvoiceStatus, VatCategory};

use support::{Booted, CLIENT_PARTY, boot, date};

/// AED in fils.
const fn aed(amount: i64) -> i64 {
    amount * 100
}

/// IPA 6 of a running project that withholds 10% retention, certified for
/// AED 180,000 of work: BC-12 AED 5,000 and LD-1 AED 30,000 accepted, CC-3
/// AED 12,000 contested.
async fn certified(t: &Booted, manager: &User) -> (Project, Ipa) {
    let project = t.project(ProjectStatus::InProgress, Some(manager)).await;
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(t.db(), &[("retention_bps", &1_000_i64)])
        .await
        .unwrap();
    let ipa = t
        .ipa(
            &project,
            6,
            IpaStatus::Certified,
            aed(200_000),
            Some(aed(180_000)),
        )
        .await;
    t.deduction(
        &ipa,
        DeductionKind::BackCharge,
        "BC-12",
        aed(5_000),
        DeductionStatus::Accepted,
    )
    .await;
    t.deduction(
        &ipa,
        DeductionKind::LiquidatedDamages,
        "LD-1",
        aed(30_000),
        DeductionStatus::Accepted,
    )
    .await;
    t.deduction(
        &ipa,
        DeductionKind::ContraCharge,
        "CC-3",
        aed(12_000),
        DeductionStatus::Contested,
    )
    .await;
    (t.reload(&project).await, ipa)
}

async fn parts(t: &Booted, project: &Project) -> Vec<Retention> {
    Retention::query()
        .where_(Retention::PROJECT_ID.eq(project.id))
        .order_by(Retention::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// Done when 1 to 6: the tax invoice is AED 180,000 + VAT 9,000, the supplier
/// bill from the client is AED 5,000 + 250 VAT with a task for the
/// accountant, the credit note is AED 30,000 + 1,500 VAT, the invoice is
/// disputed for AED 12,600, and the retention of the period is held with no
/// retention invoice.
#[renox::test]
async fn invoicing_issues_the_invoice_the_credit_note_the_dispute_and_the_bill() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let (project, ipa) = certified(&t, &pm).await;

    t.on(date(2027, 3, 15), async {
        t.post(
            &accountant,
            &format!("/construction/ipas/{}/invoice", ipa.id),
            &[],
        )
        .await
        .assert_ok();
    })
    .await;

    let done = t.reload_ipa(ipa.id).await;
    assert_eq!(done.status, IpaStatus::Invoiced);

    // The tax invoice covers the certified work, never reduced by deductions.
    let drafts = t.invoices.drafts();
    assert_eq!(drafts.len(), 1, "no retention invoice, no second invoice");
    let (invoice, draft) = &drafts[0];
    assert_eq!(done.invoice_id, Some(*invoice));
    assert_eq!(draft.party, CLIENT_PARTY);
    assert_eq!(draft.source, ipa.subject());
    assert_eq!(draft.scope, Some(project.subject()));
    assert_eq!(draft.lines.len(), 1);
    assert_eq!(draft.lines[0].net(), aed(180_000));
    assert_eq!(draft.lines[0].vat(), aed(9_000));
    assert_eq!(t.invoices.issued(), [*invoice]);
    assert_eq!(t.invoices.get(*invoice).status, InvoiceStatus::Issued);
    assert_eq!(t.invoices.get(*invoice).gross_total, aed(189_000));

    // The accepted liquidated damages are a credit note: AED 30,000 + 1,500.
    let credits = t.invoices.credits();
    assert_eq!(credits.len(), 1);
    assert_eq!(credits[0].invoice.0, *invoice);
    assert_eq!(credits[0].lines, [CreditLine::new(1, aed(30_000))]);
    assert!(credits[0].reason.contains("LD-1"));
    assert!(!credits[0].reason.contains("BC-12"));
    assert_eq!(done.credit_note_id, Some(1));
    assert_eq!(
        DraftLine::new("credit", aed(30_000), VatCategory::Standard).vat(),
        aed(1_500),
        "VAT follows the invoice line"
    );

    // The contested part is disputed: AED 12,000 + 600 VAT; reminders pause.
    let disputes = t.invoices.disputes();
    assert_eq!(disputes.len(), 1);
    assert_eq!(disputes[0].amount, Some(aed(12_600)));
    assert_eq!(disputes[0].reason, "contested certificate deduction");
    assert_eq!(disputes[0].source, Some(ipa.subject()));
    assert_eq!(done.dispute_id, Some(1));
    let summary = t.invoices.get(*invoice);
    assert!(summary.disputed);
    assert!(summary.chasing_paused(date(2027, 3, 16)));

    // The accepted back-charge is a supplier bill from the client: AED 5,000
    // + 250 VAT, and the accountant has a task to approve and set it off.
    let bills = t.bills.requests();
    assert_eq!(bills.len(), 1);
    assert_eq!(bills[0].supplier, CLIENT_PARTY);
    assert_eq!(bills[0].source, ipa.subject());
    assert_eq!(bills[0].lines.len(), 1);
    assert_eq!(bills[0].lines[0].net(), aed(5_000));
    assert_eq!(bills[0].lines[0].vat(), aed(250));
    assert!(bills[0].lines[0].description.contains("BC-12"));
    assert_eq!(done.supplier_bill_id, Some(1));
    let set_off = t.tasks.with_key("construction.ipa_set_off:");
    assert_eq!(set_off.len(), 1);
    assert!(set_off[0].ticket.title.contains("set it off"));

    // The retention of the period is held: 10% of AED 180,000, split 50:50.
    let held = parts(&t, &project).await;
    assert_eq!(held.len(), 2);
    assert_eq!(held[0].part, RetentionPart::First);
    assert_eq!(held[0].amount, aed(9_000));
    assert_eq!(held[1].part, RetentionPart::Final);
    assert_eq!(held[1].amount, aed(9_000));
    assert!(held.iter().all(|part| part.status == RetentionStatus::Held));
}

/// Nothing is invoiced for another user, an IPA that is not certified, or a
/// line that is still undecided; nothing is made then.
#[renox::test]
async fn only_the_accountant_invoices_a_certified_ipa_with_every_line_decided() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let (project, ipa) = certified(&t, &pm).await;
    let path = format!("/construction/ipas/{}/invoice", ipa.id);

    t.on(date(2027, 3, 15), async {
        t.post(&pm, &path, &[]).await.assert_status(403);
        let submitted = t
            .ipa(&project, 7, IpaStatus::Submitted, aed(100_000), None)
            .await;
        t.post(
            &accountant,
            &format!("/construction/ipas/{}/invoice", submitted.id),
            &[],
        )
        .await
        .assert_status(422);
        t.deduction(
            &ipa,
            DeductionKind::BackCharge,
            "BC-99",
            aed(100),
            DeductionStatus::Undecided,
        )
        .await;
        let refused = t.post(&accountant, &path, &[]).await;
        refused.assert_status(422);
        assert!(
            refused.text().contains("Accept or contest"),
            "{}",
            refused.text()
        );
    })
    .await;
    assert!(t.invoices.drafts().is_empty());
    assert!(t.bills.requests().is_empty());
    assert_eq!(t.reload_ipa(ipa.id).await.status, IpaStatus::Certified);
    assert!(parts(&t, &project).await.is_empty());
}

/// A run that stopped after the draft resumes: the drafted invoice is issued,
/// nothing is drafted twice, and an invoiced IPA is not invoiced again.
#[renox::test]
async fn a_run_that_stopped_halfway_resumes_without_making_a_document_twice() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let (_project, ipa) = certified(&t, &pm).await;
    let drafted = t.invoices.put(InvoiceStatus::Draft);
    Ipa::query()
        .where_(Ipa::ID.eq(ipa.id))
        .update(t.db(), &[("invoice_id", &Some(drafted))])
        .await
        .unwrap();
    let path = format!("/construction/ipas/{}/invoice", ipa.id);

    t.on(date(2027, 3, 15), async {
        t.post(&accountant, &path, &[]).await.assert_ok();
        t.post(&accountant, &path, &[]).await.assert_status(422);
    })
    .await;
    assert!(t.invoices.drafts().is_empty());
    assert_eq!(t.invoices.issued(), [drafted]);
    assert_eq!(t.invoices.credits().len(), 1);
    assert_eq!(t.invoices.disputes().len(), 1);
    assert_eq!(t.bills.requests().len(), 1);
    assert_eq!(t.reload_ipa(ipa.id).await.status, IpaStatus::Invoiced);
}

/// An IPA with no deduction is invoiced with the invoice alone; and a
/// back-charge only makes a bill, never a credit note.
#[renox::test]
async fn an_ipa_makes_only_the_documents_its_lines_call_for() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let plain = t
        .ipa(
            &project,
            1,
            IpaStatus::Certified,
            aed(100_000),
            Some(aed(90_000)),
        )
        .await;
    t.on(date(2027, 3, 15), async {
        t.post(
            &accountant,
            &format!("/construction/ipas/{}/invoice", plain.id),
            &[],
        )
        .await
        .assert_ok();
    })
    .await;
    assert_eq!(t.invoices.drafts().len(), 1);
    assert!(t.invoices.credits().is_empty());
    assert!(t.invoices.disputes().is_empty());
    assert!(t.bills.requests().is_empty());
    assert!(t.tasks.with_key("construction.ipa_set_off:").is_empty());
    // No retention % on the project: nothing is withheld.
    assert!(parts(&t, &project).await.is_empty());
    let invoiced = t.reload_ipa(plain.id).await;
    assert_eq!(invoiced.status, IpaStatus::Invoiced);
    assert_eq!(invoiced.credit_note_id, None);
    assert_eq!(invoiced.dispute_id, None);
    assert_eq!(invoiced.supplier_bill_id, None);
}

/// The retention stops at the cap: the IPA that reaches it withholds the rest,
/// later IPAs nothing; the parts hold the cap split 50:50.
#[renox::test]
async fn retention_stops_at_the_cap() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let mut project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    project.retention_bps = 1_000;
    project.retention_cap = Some(aed(10_000));

    let mut tx = t.db().begin().await.unwrap();
    assert_eq!(
        withhold_in(&mut tx, &project, aed(180_000)).await.unwrap(),
        aed(10_000),
        "10% of AED 180,000 is cut to the cap"
    );
    assert_eq!(
        withhold_in(&mut tx, &project, aed(50_000)).await.unwrap(),
        0
    );
    tx.commit().await.unwrap();
    let held = parts(&t, &project).await;
    assert_eq!(held.len(), 2);
    assert_eq!(held[0].amount, aed(5_000));
    assert_eq!(held[1].amount, aed(5_000));

    // Without a cap the amounts add up on the parts that exist.
    project.retention_cap = None;
    let mut tx = t.db().begin().await.unwrap();
    assert_eq!(
        withhold_in(&mut tx, &project, aed(10_000)).await.unwrap(),
        aed(1_000)
    );
    tx.commit().await.unwrap();
    let held = parts(&t, &project).await;
    assert_eq!(held.len(), 2);
    assert_eq!(held[0].amount, aed(5_000) + aed(500));
    assert_eq!(held[1].amount, aed(5_000) + aed(500));
}
