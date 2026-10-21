//! BL-CON-07 (#286): making a retention part due and invoicing it
//! (`make_due`, `invoice`, `settle`, `CON.RUL.07`). Retention withheld on
//! certified IPAs is not invoiced with them: each release part is `held`,
//! then `due`, then invoiced on its own tax invoice at 5% VAT, then `settled`
//! when that invoice is paid, credited or written off. The invoices live in
//! wl-invoicing, which the tests replace with a fake that records what the
//! vertical asks for.

mod support;

use construction::vat_notice::{self, RetentionNotice};
use construction::{
    GuaranteeKind, GuaranteeStatus, IpaStatus, Project, ProjectStatus, RetentionPart,
    RetentionStatus,
};
use renox::prelude::*;
use wl_kernel::invoicing::{
    CreditNoteId, InvoiceCredited, InvoiceId, InvoicePaid, InvoiceStatus, InvoiceWrittenOff,
};

use support::{Booted, CLIENT_PARTY, boot, date, users_of};

/// AED in fils.
const fn aed(amount: i64) -> i64 {
    amount * 100
}

fn complete_path(project: &Project) -> String {
    format!("/construction/projects/{}/complete", project.id)
}

fn invoice_path(part: i64) -> String {
    format!("/construction/retentions/{part}/invoice")
}

/// A project in `status` whose two parts hold `first` and `final_part`.
async fn with_parts(
    t: &Booted,
    manager: &User,
    status: ProjectStatus,
    held: (RetentionStatus, i64),
    rest: (RetentionStatus, i64),
) -> Project {
    let project = t.project(status, Some(manager)).await;
    t.retention(&project, RetentionPart::First, held.1, held.0)
        .await;
    t.retention(&project, RetentionPart::Final, rest.1, rest.0)
        .await;
    project
}

/// Sets columns of the project.
async fn set_completed(t: &Booted, project: &Project, completed: (i32, u32, u32)) {
    let on = Some(date(completed.0, completed.1, completed.2));
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(t.db(), &[("completed_on", &on)])
        .await
        .unwrap();
}

/// A completed project whose defects liability period ended on 2028-06-14,
/// with its first part invoiced and its final part held.
async fn ended_project(t: &Booted, manager: &User, name: &str) -> Project {
    let project = with_parts(
        t,
        manager,
        ProjectStatus::PracticalCompletion,
        (RetentionStatus::Invoiced, aed(50_000)),
        (RetentionStatus::Held, aed(50_000)),
    )
    .await;
    let ended = Some(date(2028, 6, 14));
    let name = name.to_owned();
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(
            t.db(),
            &[("defects_liability_ends_on", &ended), ("name", &name)],
        )
        .await
        .unwrap();
    project
}

/// Done when 1: IPAs that withheld AED 100,000 at 50:50, project `complete`
/// with the handover certificate dated 2027-06-15 makes the first part (AED
/// 50,000) `due` and notifies the accountant.
#[renox::test]
async fn completing_a_project_makes_the_first_part_due_and_tells_the_accountant() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = with_parts(
        &t,
        &pm,
        ProjectStatus::InProgress,
        (RetentionStatus::Held, aed(50_000)),
        (RetentionStatus::Held, aed(50_000)),
    )
    .await;

    t.on(date(2027, 6, 15), async {
        t.post(
            &pm,
            &complete_path(&project),
            &[
                ("certificate", "HANDOVER-1.pdf"),
                ("completed_on", "2027-06-15"),
            ],
        )
        .await
        .assert_ok();
    })
    .await;

    let done = t.reload(&project).await;
    assert_eq!(done.status, ProjectStatus::PracticalCompletion);
    assert_eq!(done.handover_certificate.as_deref(), Some("HANDOVER-1.pdf"));
    assert_eq!(done.completed_on, Some(date(2027, 6, 15)));
    assert_eq!(done.defects_liability_ends_on, Some(date(2028, 6, 14)));
    let first = t.part_of(&project, RetentionPart::First).await;
    assert_eq!(first.status, RetentionStatus::Due);
    assert_eq!(first.amount, aed(50_000));
    assert!(first.due_since.is_some());
    assert_eq!(
        t.part_of(&project, RetentionPart::Final).await.status,
        RetentionStatus::Held
    );
    let told = t.notices.notices_titled("first retention");
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), [accountant.id]);
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("AED 50,000.00")
    );
}

/// `complete` needs the handover certificate and its date, and a running
/// project; nothing is made due otherwise.
#[renox::test]
async fn completing_needs_the_certificate_its_date_and_a_running_project() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qadir").await;
    let project = with_parts(
        &t,
        &pm,
        ProjectStatus::InProgress,
        (RetentionStatus::Held, aed(50_000)),
        (RetentionStatus::Held, aed(50_000)),
    )
    .await;
    let path = complete_path(&project);

    let no_certificate = t
        .post(
            &pm,
            &path,
            &[("certificate", " "), ("completed_on", "2027-06-15")],
        )
        .await;
    no_certificate.assert_status(422);
    assert!(no_certificate.text().contains("handover certificate"));
    t.post(&pm, &path, &[("certificate", "H.pdf")])
        .await
        .assert_status(422);
    t.post(
        &qs,
        &path,
        &[("certificate", "H.pdf"), ("completed_on", "2027-06-15")],
    )
    .await
    .assert_status(403);

    assert_eq!(t.reload(&project).await.status, ProjectStatus::InProgress);
    assert_eq!(
        t.part_of(&project, RetentionPart::First).await.status,
        RetentionStatus::Held
    );
    let setup = t.project(ProjectStatus::Setup, Some(&pm)).await;
    t.post(
        &pm,
        &complete_path(&setup),
        &[("certificate", "H.pdf"), ("completed_on", "2027-06-15")],
    )
    .await
    .assert_status(422);
}

/// Done when 2: `invoice` on the first part issues AED 50,000 + VAT 2,500 and
/// the part is `invoiced`; on the final part, still `held`, it is not offered.
#[renox::test]
async fn invoicing_the_first_part_issues_the_retention_invoice() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = with_parts(
        &t,
        &pm,
        ProjectStatus::PracticalCompletion,
        (RetentionStatus::Due, aed(50_000)),
        (RetentionStatus::Held, aed(50_000)),
    )
    .await;
    set_completed(&t, &project, (2027, 6, 15)).await;
    let first = t.part_of(&project, RetentionPart::First).await;
    let last = t.part_of(&project, RetentionPart::Final).await;

    t.on(date(2027, 6, 16), async {
        // The final part is still held: it is not offered.
        let refused = t.post(&accountant, &invoice_path(last.id), &[]).await;
        refused.assert_status(422);
        assert!(refused.text().contains("held"), "{}", refused.text());
        assert!(t.invoices.drafts().is_empty());
        // A user who may not issue invoices cannot invoice the first part.
        t.post(&pm, &invoice_path(first.id), &[])
            .await
            .assert_status(403);
        t.post(&accountant, &invoice_path(first.id), &[])
            .await
            .assert_ok();
    })
    .await;

    let invoiced = t.reload_part(first.id).await;
    assert_eq!(invoiced.status, RetentionStatus::Invoiced);
    let drafts = t.invoices.drafts();
    assert_eq!(drafts.len(), 1);
    let (invoice, draft) = &drafts[0];
    assert_eq!(invoiced.invoice_id, Some(*invoice));
    assert_eq!(draft.party, CLIENT_PARTY);
    assert_eq!(draft.source, first.subject());
    assert_eq!(draft.scope, Some(project.subject()));
    assert_eq!(draft.date_of_supply, Some(date(2027, 6, 15)));
    assert_eq!(draft.lines.len(), 1);
    assert_eq!(draft.lines[0].net(), aed(50_000));
    assert_eq!(draft.lines[0].vat(), aed(2_500));
    assert_eq!(t.invoices.issued(), [*invoice]);
    assert_eq!(t.invoices.get(*invoice).gross_total, aed(52_500));
    assert_eq!(
        t.reload_part(last.id).await.status,
        RetentionStatus::Held,
        "the final part waits for the end of the defects liability period"
    );

    // An invoiced part is not invoiced twice.
    t.on(date(2027, 6, 17), async {
        t.post(&accountant, &invoice_path(first.id), &[])
            .await
            .assert_status(422);
    })
    .await;
    assert_eq!(t.invoices.drafts().len(), 1);
}

/// Done when 3: when the retention invoice is paid, credited or written off,
/// the part is `settled`; an invoice still open settles nothing, and a
/// settled part stays terminal.
#[renox::test]
async fn a_part_is_settled_when_its_invoice_is_paid_credited_or_written_off() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    for ending in [
        InvoiceStatus::Paid,
        InvoiceStatus::Credited,
        InvoiceStatus::WrittenOff,
    ] {
        let project = with_parts(
            &t,
            &pm,
            ProjectStatus::PracticalCompletion,
            (RetentionStatus::Due, aed(50_000)),
            (RetentionStatus::Held, aed(50_000)),
        )
        .await;
        let first = t.part_of(&project, RetentionPart::First).await;
        t.post(&accountant, &invoice_path(first.id), &[])
            .await
            .assert_ok();
        let invoice = t.reload_part(first.id).await.invoice_id.unwrap();

        // The invoice is partly paid: nothing settles yet.
        t.invoices.set_status(invoice, InvoiceStatus::PartiallyPaid);
        t.state()
            .emit(InvoicePaid {
                invoice: InvoiceId(invoice),
                party: CLIENT_PARTY,
                source: first.subject(),
            })
            .await
            .unwrap();
        assert_eq!(
            t.reload_part(first.id).await.status,
            RetentionStatus::Invoiced,
            "{ending:?}"
        );

        t.invoices.set_status(invoice, ending);
        match ending {
            InvoiceStatus::Paid => {
                t.state()
                    .emit(InvoicePaid {
                        invoice: InvoiceId(invoice),
                        party: CLIENT_PARTY,
                        source: first.subject(),
                    })
                    .await
                    .unwrap();
            }
            InvoiceStatus::Credited => {
                t.state()
                    .emit(InvoiceCredited {
                        invoice: InvoiceId(invoice),
                        credit_note: CreditNoteId(1),
                        amount: aed(52_500),
                        reason: "Retention waived".into(),
                        released_origins: Vec::new(),
                    })
                    .await
                    .unwrap();
            }
            _ => {
                t.state()
                    .emit(InvoiceWrittenOff {
                        invoice: InvoiceId(invoice),
                        party: CLIENT_PARTY,
                        source: first.subject(),
                        amount: aed(52_500),
                        reason_code: "insolvent".into(),
                        approver: None,
                    })
                    .await
                    .unwrap();
            }
        }
        assert_eq!(
            t.reload_part(first.id).await.status,
            RetentionStatus::Settled,
            "{ending:?}"
        );

        // The invoice is reopened later: the part stays settled.
        t.invoices.set_status(invoice, InvoiceStatus::Overdue);
        t.state()
            .emit(InvoicePaid {
                invoice: InvoiceId(invoice),
                party: CLIENT_PARTY,
                source: first.subject(),
            })
            .await
            .unwrap();
        assert_eq!(
            t.reload_part(first.id).await.status,
            RetentionStatus::Settled
        );
    }
}

/// Done when 4: retention 10%, cap AED 60,000 at 50:50 and AED 55,000 already
/// withheld, an IPA certifying AED 100,000 of work withholds AED 5,000 (not
/// 10,000): each part gets AED 2,500 and holds AED 30,000; the accountant is
/// told the cap is reached once; the next IPA withholds nothing.
#[renox::test]
async fn the_ipa_that_reaches_the_cap_withholds_the_rest_and_tells_the_accountant() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = with_parts(
        &t,
        &pm,
        ProjectStatus::InProgress,
        (RetentionStatus::Held, aed(27_500)),
        (RetentionStatus::Held, aed(27_500)),
    )
    .await;
    let cap = Some(aed(60_000));
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(
            t.db(),
            &[("retention_bps", &1_000_i64), ("retention_cap", &cap)],
        )
        .await
        .unwrap();
    let seventh = t
        .ipa(
            &project,
            7,
            IpaStatus::Certified,
            aed(110_000),
            Some(aed(100_000)),
        )
        .await;
    let eighth = t
        .ipa(
            &project,
            8,
            IpaStatus::Certified,
            aed(60_000),
            Some(aed(100_000)),
        )
        .await;

    t.on(date(2027, 3, 15), async {
        t.post(
            &accountant,
            &format!("/construction/ipas/{}/invoice", seventh.id),
            &[],
        )
        .await
        .assert_ok();
    })
    .await;

    for which in [RetentionPart::First, RetentionPart::Final] {
        let part = t.part_of(&project, which).await;
        assert_eq!(part.amount, aed(30_000), "{which:?}");
    }
    let told = t.notices.notices_titled("retention cap");
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), [accountant.id]);
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("AED 60,000.00")
    );

    // The next IPA withholds nothing and the accountant is not told again.
    t.on(date(2027, 4, 15), async {
        t.post(
            &accountant,
            &format!("/construction/ipas/{}/invoice", eighth.id),
            &[],
        )
        .await
        .assert_ok();
    })
    .await;
    for which in [RetentionPart::First, RetentionPart::Final] {
        assert_eq!(t.part_of(&project, which).await.amount, aed(30_000));
    }
    assert_eq!(t.notices.notices_titled("retention cap").len(), 1);
}

/// Done when 5: with the final part still `held`, the accountant is notified
/// on 2028-05-16, thirty days before a year after the completion certificate;
/// not before, once, and the accountant may dismiss it with a reason, which
/// changes nothing else.
#[renox::test]
async fn the_accountant_is_told_thirty_days_before_vat_on_the_retention_becomes_due() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = with_parts(
        &t,
        &pm,
        ProjectStatus::PracticalCompletion,
        (RetentionStatus::Invoiced, aed(50_000)),
        (RetentionStatus::Held, aed(50_000)),
    )
    .await;
    set_completed(&t, &project, (2027, 6, 15)).await;

    t.on(date(2028, 5, 15), async {
        assert!(vat_notice::run(t.state()).await.unwrap().is_empty());
    })
    .await;
    assert!(t.notices.notices_titled("VAT on the retention").is_empty());

    t.on(date(2028, 5, 16), async {
        let made = vat_notice::run(t.state()).await.unwrap();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].limit_on, date(2028, 6, 15));
        assert_eq!(made[0].notified_on, date(2028, 5, 16));
        // A run on the same day again does nothing new.
        assert!(vat_notice::run(t.state()).await.unwrap().is_empty());
    })
    .await;
    let told = t.notices.notices_titled("VAT on the retention");
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), [accountant.id]);
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("2028-06-15")
    );

    // The accountant dismisses the notice with a reason; nothing else changes.
    let path = format!(
        "/construction/projects/{}/retention-notice/dismiss",
        project.id
    );
    t.on(date(2028, 5, 17), async {
        t.post(&accountant, &path, &[("reason", " ")])
            .await
            .assert_status(422);
        t.post(&pm, &path, &[("reason", "Client agreed to extend")])
            .await
            .assert_status(403);
        t.post(&accountant, &path, &[("reason", "Client agreed to extend")])
            .await
            .assert_ok();
    })
    .await;
    let notice: RetentionNotice = vat_notice::notice_of(t.db(), project.id)
        .await
        .unwrap()
        .expect("the notice");
    assert!(notice.is_dismissed());
    assert_eq!(notice.dismissed_by, Some(accountant.id));
    assert_eq!(
        notice.dismissed_reason.as_deref(),
        Some("Client agreed to extend")
    );
    assert_eq!(
        t.part_of(&project, RetentionPart::Final).await.status,
        RetentionStatus::Held
    );
}

/// The rule looks only at projects with a part `held` or `due`: when every
/// part is invoiced it stops, and it is made up the next day it runs.
#[renox::test]
async fn the_vat_rule_stops_with_the_invoice_and_makes_up_a_missed_day() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let _accountant = t.accountant("Aisha").await;
    let done = with_parts(
        &t,
        &pm,
        ProjectStatus::PracticalCompletion,
        (RetentionStatus::Invoiced, aed(50_000)),
        (RetentionStatus::Settled, aed(50_000)),
    )
    .await;
    set_completed(&t, &done, (2027, 6, 15)).await;
    let late = with_parts(
        &t,
        &pm,
        ProjectStatus::PracticalCompletion,
        (RetentionStatus::Due, aed(10_000)),
        (RetentionStatus::Held, aed(10_000)),
    )
    .await;
    set_completed(&t, &late, (2027, 6, 15)).await;

    t.on(date(2028, 5, 20), async {
        let made = vat_notice::run(t.state()).await.unwrap();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].project_id, late.id);
    })
    .await;
}

/// Done when 6: a retention bond `active` linked to the final part lets
/// `retentions.manage` make it `due` and invoice it before the defects
/// liability period ends. A bond below the held amount is a warning, not a
/// refusal; without an active linked bond it is refused.
#[renox::test]
async fn an_active_retention_bond_lets_the_accountant_make_the_final_part_due() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = with_parts(
        &t,
        &pm,
        ProjectStatus::PracticalCompletion,
        (RetentionStatus::Held, aed(50_000)),
        (RetentionStatus::Held, aed(50_000)),
    )
    .await;
    let first = t.part_of(&project, RetentionPart::First).await;
    let last = t.part_of(&project, RetentionPart::Final).await;
    let expires = Some(date(2028, 12, 31));
    let draft = t
        .guarantee(
            &project,
            GuaranteeKind::RetentionBond,
            aed(50_000),
            GuaranteeStatus::Draft,
            expires,
        )
        .await;
    let active = t
        .guarantee(
            &project,
            GuaranteeKind::RetentionBond,
            aed(50_000),
            GuaranteeStatus::Active,
            expires,
        )
        .await;
    let lower = t
        .guarantee(
            &project,
            GuaranteeKind::RetentionBond,
            aed(40_000),
            GuaranteeStatus::Active,
            expires,
        )
        .await;
    let performance = t
        .guarantee(
            &project,
            GuaranteeKind::Performance,
            aed(50_000),
            GuaranteeStatus::Active,
            expires,
        )
        .await;
    let link = |part: i64| format!("/construction/retentions/{part}/bond");
    let make_due = |part: i64| format!("/construction/retentions/{part}/make-due");
    let id = |guarantee: i64| guarantee.to_string();

    t.on(date(2027, 9, 1), async {
        // Without a linked bond a user cannot make the part due.
        let refused = t.post(&accountant, &make_due(last.id), &[]).await;
        refused.assert_status(422);
        assert!(refused.text().contains("No retention bond"));
        // A project manager may not manage the retention.
        t.post(&pm, &link(last.id), &[("guarantee_id", &id(active.id))])
            .await
            .assert_status(403);
        // A performance bond is not a retention bond.
        t.post(
            &accountant,
            &link(last.id),
            &[("guarantee_id", &id(performance.id))],
        )
        .await
        .assert_status(422);
        // A bond that is still a draft links but does not replace the part.
        t.post(
            &accountant,
            &link(last.id),
            &[("guarantee_id", &id(draft.id))],
        )
        .await
        .assert_ok();
        let refused = t.post(&accountant, &make_due(last.id), &[]).await;
        refused.assert_status(422);
        assert!(refused.text().contains("not active"));
        assert_eq!(t.reload_part(last.id).await.status, RetentionStatus::Held);

        // The active bond replaces the final part: due, then invoiced.
        t.post(
            &accountant,
            &link(last.id),
            &[("guarantee_id", &id(active.id))],
        )
        .await
        .assert_ok();
        assert_eq!(t.reload_part(last.id).await.bond_id, Some(active.id));
        t.post(&accountant, &make_due(last.id), &[])
            .await
            .assert_ok();
        t.post(&accountant, &invoice_path(last.id), &[])
            .await
            .assert_ok();

        // A bond below the held amount links and replaces with a warning.
        t.post(
            &accountant,
            &link(first.id),
            &[("guarantee_id", &id(lower.id))],
        )
        .await
        .assert_ok();
        t.post(&accountant, &make_due(first.id), &[])
            .await
            .assert_ok();
    })
    .await;

    let last = t.reload_part(last.id).await;
    assert_eq!(last.status, RetentionStatus::Invoiced);
    assert!(last.due_since.is_some());
    let drafts = t.invoices.drafts();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].1.lines[0].net(), aed(50_000));
    assert_eq!(drafts[0].1.lines[0].vat(), aed(2_500));
    assert_eq!(
        t.reload_part(first.id).await.status,
        RetentionStatus::Due,
        "a bond below the amount is a warning, not a refusal"
    );
    // The accountant was told once for each part that became due.
    assert_eq!(t.notices.notices_titled("retention of").len(), 2);
    // The project is still in its defects liability period.
    assert_eq!(
        t.reload(&project).await.status,
        ProjectStatus::PracticalCompletion
    );
}

/// Done when 7: on a project terminated with effective date 2027-05-31, both
/// parts are `due` and can be invoiced from that date; `CON.RUL.07` notifies
/// the accountant on 2028-05-01.
#[renox::test]
async fn a_terminated_project_has_both_parts_due_and_the_vat_notice_counts_from_its_end() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let md = t.md("Bader").await;
    let accountant = t.accountant("Aisha").await;
    let project = with_parts(
        &t,
        &pm,
        ProjectStatus::InProgress,
        (RetentionStatus::Held, aed(50_000)),
        (RetentionStatus::Held, aed(50_000)),
    )
    .await;
    let first = t.part_of(&project, RetentionPart::First).await;
    let last = t.part_of(&project, RetentionPart::Final).await;

    t.on(date(2027, 5, 20), async {
        t.ask_to_terminate(&pm, &project, "agreed", "2027-05-31")
            .await
            .assert_ok();
        t.approve(&md, &project).await;
        // Due, but the effective date is not reached.
        let early = t.post(&accountant, &invoice_path(first.id), &[]).await;
        early.assert_status(422);
        assert!(early.text().contains("2027-05-31"), "{}", early.text());
    })
    .await;
    for id in [first.id, last.id] {
        assert_eq!(t.reload_part(id).await.status, RetentionStatus::Due);
    }
    assert_eq!(t.notices.notices_titled("retention of").len(), 2);

    t.on(date(2027, 5, 31), async {
        t.post(&accountant, &invoice_path(first.id), &[])
            .await
            .assert_ok();
    })
    .await;
    assert_eq!(
        t.reload_part(first.id).await.status,
        RetentionStatus::Invoiced
    );

    // The year runs from the effective date: the final part is still due.
    t.on(date(2028, 4, 30), async {
        assert!(vat_notice::run(t.state()).await.unwrap().is_empty());
    })
    .await;
    t.on(date(2028, 5, 1), async {
        let made = vat_notice::run(t.state()).await.unwrap();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].limit_on, date(2028, 5, 31));
    })
    .await;
}

/// A terminated project whose retention does not become due on termination
/// keeps its parts `held`.
#[renox::test]
async fn retention_stays_held_when_it_does_not_become_due_on_termination() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let md = t.md("Bader").await;
    let project = with_parts(
        &t,
        &pm,
        ProjectStatus::InProgress,
        (RetentionStatus::Held, aed(50_000)),
        (RetentionStatus::Held, aed(50_000)),
    )
    .await;
    let off = false;
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(t.db(), &[("retention_due_on_termination", &off)])
        .await
        .unwrap();

    t.ask_to_terminate(&pm, &project, "agreed", "2027-05-31")
        .await
        .assert_ok();
    t.approve(&md, &project).await;

    for which in [RetentionPart::First, RetentionPart::Final] {
        assert_eq!(
            t.part_of(&project, which).await.status,
            RetentionStatus::Held
        );
    }
    assert!(t.notices.notices_titled("retention of").is_empty());
}

/// The final part becomes due when the project is closed after its defects
/// liability period, or on the daily run after the end date; `close` is
/// refused while the period runs.
#[renox::test]
async fn the_final_part_becomes_due_at_the_end_of_the_defects_liability_period() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let by_close = ended_project(&t, &pm, "Closed by the manager").await;
    let by_run = ended_project(&t, &pm, "Made due by the daily run").await;
    let close = |project: &Project| format!("/construction/projects/{}/close", project.id);

    t.on(date(2028, 6, 14), async {
        // The period ends today: it has not passed.
        let refused = t.post(&pm, &close(&by_close), &[]).await;
        refused.assert_status(422);
        assert!(refused.text().contains("2028-06-14"), "{}", refused.text());
        assert!(
            construction::release::make_final_due_at_defects_end(t.state(), date(2028, 6, 14))
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await;
    assert_eq!(
        t.part_of(&by_close, RetentionPart::Final).await.status,
        RetentionStatus::Held
    );

    t.on(date(2028, 6, 15), async {
        t.post(&accountant, &close(&by_close), &[])
            .await
            .assert_status(403);
        t.post(&pm, &close(&by_close), &[]).await.assert_ok();
        let made =
            construction::release::make_final_due_at_defects_end(t.state(), date(2028, 6, 15))
                .await
                .unwrap();
        assert_eq!(made, [by_run.id]);
    })
    .await;

    let closed = t.reload(&by_close).await;
    assert_eq!(closed.status, ProjectStatus::Closed);
    assert!(closed.closed_at.is_some());
    assert_eq!(
        t.part_of(&by_close, RetentionPart::Final).await.status,
        RetentionStatus::Due
    );
    assert_eq!(
        t.reload(&by_run).await.status,
        ProjectStatus::PracticalCompletion
    );
    assert_eq!(
        t.part_of(&by_run, RetentionPart::Final).await.status,
        RetentionStatus::Due
    );
    assert_eq!(
        t.notices.notices_titled("final retention").len(),
        2,
        "the accountant is told once for each"
    );
}
