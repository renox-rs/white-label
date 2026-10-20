//! BL-PRP-19 (#269): the owner statement. `PRP.RUL.05` generates the draft of
//! the previous month for every agreement that reports; the accountant checks
//! it, a property manager who did not prepare it approves or returns it, the
//! accountant publishes it to the owner, and a published statement with no
//! payout is revised into a new draft revision.

mod support;

use property::agreement::{AgreementStatus, ManagementAgreement};
use property::notices::{
    OWNER_STATEMENT, OWNER_STATEMENT_DRAFT, OWNER_STATEMENT_RETURNED, OWNER_STATEMENT_REVISED,
};
use property::statement::{self, LineKind, OwnerStatement, StatementLine, StatementStatus};
use property::{ChargeStatus, Lease, LeaseCharge};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::parties::PartyKind;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, invoicer};
use wl_kernel::{PartyId, Subject, VatCategory};

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, TENANT, aed, boot};

/// The owner of the units.
const OWNER: PartyId = PartyId(31);

fn ymd(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

struct Scene {
    t: Booted,
    agreement: ManagementAgreement,
    lease: Lease,
    accountant: User,
    manager: User,
}

/// An active agreement with the owner at a fee of 5%, covering one unit
/// whose lease is active, an accountant and a property manager.
async fn scene() -> Scene {
    let t = boot().await;
    t.parties.add(OWNER.0, "Hessa Al Mansoori", PartyKind::Person);
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let site_manager = t.user_in("Sami", MANAGER, building.id).await;
    let lease = t.active_lease(&unit, &agent, &site_manager).await;
    let mut agreement = ManagementAgreement {
        owner_party_id: OWNER.0,
        fee_bp: 500,
        payout_frequency: "monthly".to_owned(),
        notice_days: 90,
        start_date: ymd(2027, 1, 1),
        status: AgreementStatus::Active,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    agreement
        .insert_in_state(&mut tx, AgreementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    property::agreement::add_unit(t.db(), &agreement, unit.id, ymd(2027, 1, 1))
        .await
        .unwrap();
    let accountant = t.user_with("Amal", ACCOUNTANT).await;
    let manager = t.user_with("Mona", MANAGER).await;
    Scene {
        t,
        agreement,
        lease,
        accountant,
        manager,
    }
}

impl Scene {
    /// An issued rent invoice of `amount` fils due on `due`, with `paid` fils
    /// paid, as a charge of the lease. Its id.
    async fn invoice(&self, amount: i64, paid: i64, due: NaiveDate, position: i32) -> i64 {
        let draft = InvoiceDraft::new(TENANT, self.lease.subject())
            .due_date(due)
            .line(DraftLine::new("Rent", amount, VatCategory::Exempt));
        let id = invoicer(self.t.state())
            .draft(self.t.db(), draft)
            .await
            .unwrap()
            .expect("invoicing is on")
            .0;
        self.t.invoices.issue(id);
        if paid > 0 {
            self.t.invoices.pay(id, &Subject::new("payments.payment", 5), paid);
        }
        LeaseCharge::create(
            self.t.db(),
            LeaseCharge {
                lease_id: self.lease.id,
                position,
                charge_type: "rent".to_owned(),
                description: format!("Rent {position}"),
                due_date: due,
                amount,
                vat_category: VatCategory::Exempt,
                status: ChargeStatus::Invoiced,
                invoice_id: Some(id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        id
    }

    /// A draft statement for February 2027 with one expense line, generated
    /// by the rule.
    async fn february(&self) -> OwnerStatement {
        self.invoice(aed(12_000), aed(12_000), ymd(2027, 2, 10), 10).await;
        statement::generate(self.t.state(), self.agreement.id, ymd(2027, 2, 1), ymd(2027, 2, 28))
            .await
            .unwrap()
            .expect("a draft")
    }

    async fn reload(&self, statement: &OwnerStatement) -> OwnerStatement {
        OwnerStatement::find_or_404(self.t.db(), statement.id).await.unwrap()
    }

    /// `user` runs the transition `name` on `statement`; the reason is sent
    /// when it is not blank.
    async fn run(
        &self,
        user: &User,
        statement: &OwnerStatement,
        name: &str,
        reason: &str,
    ) -> renox::testing::TestResponse {
        let pairs: Vec<(&str, &str)> = if reason.is_empty() {
            Vec::new()
        } else {
            vec![("reason", reason)]
        };
        self.t
            .post_plain(
                user,
                &format!("/property/statements/{}/transitions/{name}", statement.id),
                &pairs,
            )
            .await
    }

    /// The accountant adds an expense of `amount` dirhams.
    async fn expense(&self, user: &User, statement: &OwnerStatement, amount: &str) -> renox::testing::TestResponse {
        self.t
            .post(
                user,
                &format!("/property/statements/{}/lines", statement.id),
                &[("kind", "expense"), ("description", "AMC chiller"), ("amount", amount)],
            )
            .await
    }
}

/// `PRP.RUL.05`: on day 5 the draft for the previous month is generated for
/// the agreement: the rent collected on the invoices of its units' leases, the
/// fee, and the balance brought forward from the latest earlier statement; the
/// accountants are told; a second run changes nothing.
#[renox::test]
async fn bl_prp_19_the_rule_generates_the_draft_of_the_previous_month() {
    let s = scene().await;
    let accountant = s.accountant.id;
    s.invoice(aed(12_000), aed(12_000), ymd(2027, 2, 10), 10).await;
    let half = s.invoice(aed(6_000), aed(3_000), ymd(2027, 2, 20), 11).await;
    // An unpaid invoice adds nothing.
    s.invoice(aed(5_000), 0, ymd(2027, 2, 25), 12).await;

    let early = statement::run(s.t.state(), ymd(2027, 3, 4)).await.unwrap();
    assert_eq!(early.generated, 0);
    let ran = statement::run(s.t.state(), ymd(2027, 3, 5)).await.unwrap();
    assert_eq!((ran.generated, ran.skipped, ran.failed), (1, 0, 0));

    let february = OwnerStatement::of_agreement(s.t.db(), s.agreement.id).await.unwrap();
    assert_eq!(february.len(), 1);
    let february = &february[0];
    assert_eq!(february.status, StatementStatus::Draft);
    assert_eq!((february.period_start, february.period_end), (ymd(2027, 2, 1), ymd(2027, 2, 28)));
    assert_eq!(february.owner_party_id, OWNER.0);
    assert_eq!(february.revision, 1);
    assert!(february.prepared_by.is_none());
    assert_eq!(february.brought_forward, 0);
    assert_eq!(february.collected, aed(15_000));
    assert_eq!(february.fee, aed(750));
    assert_eq!(february.balance(), aed(14_250));
    let lines = february.lines(s.t.db()).await.unwrap();
    assert_eq!(lines.iter().filter(|l| l.kind == LineKind::Collected).count(), 2);
    assert_eq!(lines.iter().filter(|l| l.kind == LineKind::Fee).count(), 1);

    // The accountants are told.
    let told = s.t.notices.of_type(OWNER_STATEMENT_DRAFT);
    assert_eq!(told.len(), 1);
    assert!(told[0]
        .recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == accountant)));

    // The same day again changes nothing.
    let again = statement::run(s.t.state(), ymd(2027, 3, 5)).await.unwrap();
    assert_eq!((again.generated, again.skipped), (0, 1));

    // Next month brings forward the balance of the latest statement that
    // counts, and only the rent paid since on the half-paid invoice.
    s.t.invoices.pay(half, &Subject::new("payments.payment", 6), aed(3_000));
    let april = statement::run(s.t.state(), ymd(2027, 4, 5)).await.unwrap();
    assert_eq!(april.generated, 1);
    let march = OwnerStatement::of_agreement(s.t.db(), s.agreement.id).await.unwrap();
    assert_eq!(march.len(), 2);
    let march = &march[0];
    assert_eq!(march.period_start, ymd(2027, 3, 1));
    assert_eq!(march.brought_forward, aed(14_250));
    assert_eq!(march.collected, aed(3_000));
    assert_eq!(march.fee, aed(150));
}

/// An agreement that does not report (a draft) gets no statement.
#[renox::test]
async fn bl_prp_19_a_draft_agreement_gets_no_statement() {
    let s = scene().await;
    let draft = property::agreement::create_draft(
        s.t.state(),
        None,
        property::agreement::NewAgreement::new(OWNER, ymd(2027, 1, 1), 500),
    )
    .await
    .unwrap();
    assert_eq!(draft.status, AgreementStatus::Draft);
    let none = statement::generate(s.t.state(), draft.id, ymd(2027, 2, 1), ymd(2027, 2, 28))
        .await
        .unwrap();
    assert!(none.is_none());
}

/// AC1: a draft edited by a user who also holds the approval permission cannot
/// be approved by that user; another property manager approves it. A statement
/// the rule generated and nobody edited can be approved by any property
/// manager.
#[renox::test]
async fn bl_prp_19_the_preparer_cannot_approve_but_another_manager_can() {
    let s = scene().await;
    let draft = s.february().await;

    // Pia holds both roles; she edits the draft, so she prepared it.
    let pia = s.t.user_with("Pia", ACCOUNTANT).await;
    pia.assign_role(s.t.db(), MANAGER).await.unwrap();
    s.expense(&pia, &draft, "1,200").await.assert_ok();
    let edited = s.reload(&draft).await;
    assert_eq!(edited.prepared_by, Some(pia.id));
    assert_eq!(edited.expenses, aed(1_200));

    s.run(&pia, &edited, "approve", "").await.assert_status(422);
    assert_eq!(s.reload(&draft).await.status, StatementStatus::Draft);
    // The accountant has no approval permission.
    s.run(&s.accountant, &edited, "approve", "").await.assert_forbidden();

    s.run(&s.manager, &edited, "approve", "").await.assert_status(303);
    let approved = s.reload(&draft).await;
    assert_eq!(approved.status, StatementStatus::Approved);
    assert_eq!(approved.approved_by, Some(s.manager.id));
    assert!(approved.approved_on.is_some());

    // Nobody edited the second one: Pia may approve it.
    let untouched = statement::generate(s.t.state(), s.agreement.id, ymd(2027, 3, 1), ymd(2027, 3, 31))
        .await
        .unwrap()
        .expect("a draft");
    assert!(untouched.prepared_by.is_none());
    s.run(&pia, &untouched, "approve", "").await.assert_status(303);
    assert_eq!(s.reload(&untouched).await.status, StatementStatus::Approved);
}

/// AC2: the property manager returns an approved statement with a reason; it
/// is a draft again and the preparer is told. An approved statement is not
/// edited.
#[renox::test]
async fn bl_prp_19_a_returned_statement_is_a_draft_and_the_preparer_is_told() {
    let s = scene().await;
    let draft = s.february().await;
    s.expense(&s.accountant, &draft, "800").await.assert_ok();
    let edited = s.reload(&draft).await;
    s.run(&s.manager, &edited, "approve", "").await.assert_status(303);

    // Edits after approve are refused.
    let refused = s.expense(&s.accountant, &draft, "50").await;
    refused.assert_status(422);
    assert!(refused.text().contains("return the statement first"), "{}", refused.text());

    // A reason is required.
    let approved = s.reload(&draft).await;
    s.run(&s.manager, &approved, "return", "").await.assert_status(422);
    s.run(&s.manager, &approved, "return", "missing AMC bill")
        .await
        .assert_status(303);
    let returned = s.reload(&draft).await;
    assert_eq!(returned.status, StatementStatus::Draft);
    assert_eq!(returned.return_reason.as_deref(), Some("missing AMC bill"));
    assert!(returned.approved_by.is_none());

    let told = s.t.notices.of_type(OWNER_STATEMENT_RETURNED);
    assert_eq!(told.len(), 1);
    assert!(told[0]
        .recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == s.accountant.id)));
    assert!(told[0].data.to_string().contains("missing AMC bill"));

    // The draft can be edited again.
    s.expense(&s.accountant, &draft, "50").await.assert_ok();
}

/// AC3: publishing sends the statement to the owner's e-mail; the manager who
/// approved it does not publish it (that is the accountant's).
#[renox::test]
async fn bl_prp_19_publishing_sends_the_statement_to_the_owner() {
    let s = scene().await;
    let draft = s.february().await;
    s.run(&s.manager, &draft, "approve", "").await.assert_status(303);
    let approved = s.reload(&draft).await;

    s.run(&s.manager, &approved, "publish", "").await.assert_forbidden();
    s.run(&s.accountant, &approved, "publish", "").await.assert_status(303);

    let published = s.reload(&draft).await;
    assert_eq!(published.status, StatementStatus::Published);
    assert_eq!(published.published_by, Some(s.accountant.id));
    assert!(published.published_on.is_some());

    let sent = s.t.notices.of_type(OWNER_STATEMENT);
    assert_eq!(sent.len(), 1);
    assert!(sent[0]
        .recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::Party { id } if *id == OWNER)));
    let data = sent[0].data.to_string();
    assert!(data.contains("AED 11,400.00"), "{data}");
    assert!(data.contains("AED 600.00"), "{data}");
}

/// AC4: revising a published statement with no payout makes it superseded,
/// creates revision 2 as a draft with the same lines and balance brought
/// forward, and tells the owner a corrected statement follows. The revision
/// can be published and revised again.
#[renox::test]
async fn bl_prp_19_a_revision_replaces_a_published_statement() {
    let s = scene().await;
    let draft = s.february().await;
    s.run(&s.manager, &draft, "approve", "").await.assert_status(303);
    s.run(&s.accountant, &draft, "publish", "").await.assert_status(303);
    let published = s.reload(&draft).await;
    let before = published.lines(s.t.db()).await.unwrap().len();

    // A reason is required.
    s.run(&s.accountant, &published, "revise", "").await.assert_status(422);
    s.run(&s.accountant, &published, "revise", "AMC bill missing")
        .await
        .assert_status(303);

    let old = s.reload(&draft).await;
    assert_eq!(old.status, StatementStatus::Superseded);
    assert_eq!(old.revise_reason.as_deref(), Some("AMC bill missing"));
    let all = OwnerStatement::of_agreement(s.t.db(), s.agreement.id).await.unwrap();
    assert_eq!(all.len(), 2);
    let revision = all.iter().find(|r| r.revision == 2).expect("revision 2");
    assert_eq!(revision.status, StatementStatus::Draft);
    assert_eq!(revision.previous_id, Some(old.id));
    assert_eq!(revision.period_start, old.period_start);
    assert_eq!(revision.brought_forward, old.brought_forward);
    assert_eq!(revision.balance(), old.balance());
    assert_eq!(revision.prepared_by, Some(s.accountant.id));
    assert_eq!(revision.lines(s.t.db()).await.unwrap().len(), before);

    let told = s.t.notices.of_type(OWNER_STATEMENT_REVISED);
    assert_eq!(told.len(), 1);
    assert!(told[0]
        .recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::Party { id } if *id == OWNER)));

    // The old one is not revised twice; the accountant corrects the revision
    // (the manager approves, the accountant publishes and may revise it).
    s.run(&s.accountant, &old, "revise", "again").await.assert_status(422);
    s.expense(&s.accountant, revision, "1,000").await.assert_ok();
    let revision = s.reload(revision).await;
    assert_eq!(revision.expenses, aed(1_000));
    s.run(&s.manager, &revision, "approve", "").await.assert_status(303);
    s.run(&s.accountant, &revision, "publish", "").await.assert_status(303);
    s.run(&s.accountant, &revision, "revise", "once more")
        .await
        .assert_status(303);
    assert_eq!(s.reload(&revision).await.status, StatementStatus::Superseded);

    // The next statement brings forward the revision that counts.
    let latest = OwnerStatement::of_agreement(s.t.db(), s.agreement.id).await.unwrap();
    assert_eq!(latest.len(), 3);
}

/// AC5: revising after a payout was recorded is refused.
#[renox::test]
async fn bl_prp_19_a_statement_with_a_payout_is_not_revised() {
    let s = scene().await;
    let draft = s.february().await;
    s.run(&s.manager, &draft, "approve", "").await.assert_status(303);
    s.run(&s.accountant, &draft, "publish", "").await.assert_status(303);
    // The payout is recorded (BL-PRP-20 does this when it settles).
    OwnerStatement::query()
        .where_(OwnerStatement::ID.eq(draft.id))
        .update(s.t.db(), &[("payout_amount", &Some(aed(11_400)))])
        .await
        .unwrap();
    let published = s.reload(&draft).await;
    let refused = s.run(&s.accountant, &published, "revise", "late correction").await;
    refused.assert_status(422);
    assert!(refused.text().contains("payout"), "{}", refused.text());
    assert_eq!(s.reload(&draft).await.status, StatementStatus::Published);
}

/// Lines are edited on a draft only, by the accountant, and removed again.
#[renox::test]
async fn bl_prp_19_the_accountant_adds_and_removes_lines_of_a_draft() {
    let s = scene().await;
    let draft = s.february().await;
    let before = s.reload(&draft).await;

    // The property manager does not edit.
    s.expense(&s.manager, &draft, "100").await.assert_forbidden();
    s.expense(&s.accountant, &draft, "100").await.assert_ok();
    let credit = s
        .t
        .post(
            &s.accountant,
            &format!("/property/statements/{}/lines", draft.id),
            &[
                ("kind", "adjustment"),
                ("description", "Correction of January"),
                ("amount", "40"),
                ("direction", "debit"),
            ],
        )
        .await;
    credit.assert_ok();
    let edited = s.reload(&draft).await;
    assert_eq!(edited.expenses, before.expenses + aed(100));
    assert_eq!(edited.adjustments, -aed(40));
    assert_eq!(edited.balance(), before.balance() - aed(140));

    let lines: Vec<StatementLine> = edited.lines(s.t.db()).await.unwrap();
    let expense = lines.iter().find(|l| l.kind == LineKind::Expense).unwrap();
    assert_eq!(expense.amount, -aed(100));
    let line = expense.id.to_string();
    s.t.post(
        &s.accountant,
        &format!("/property/statements/{}/lines/remove", draft.id),
        &[("line", &line)],
    )
    .await
    .assert_ok();
    assert_eq!(s.reload(&draft).await.expenses, 0);

    // A blank description or a zero amount is refused.
    s.t.post(
        &s.accountant,
        &format!("/property/statements/{}/lines", draft.id),
        &[("kind", "expense"), ("description", " "), ("amount", "5")],
    )
    .await
    .assert_status(422);
}

/// The pages: the list shows the statements, a statement shows its lines and
/// offers the transitions the user may run.
#[renox::test]
async fn bl_prp_19_the_pages_show_the_statement_and_its_actions() {
    let s = scene().await;
    let draft = s.february().await;

    s.t.app.acting_as(&s.manager);
    s.t.app
        .get("/property/statements")
        .await
        .assert_ok()
        .assert_view("property/statements.html")
        .assert_see(&draft.number())
        .assert_see("Hessa Al Mansoori");
    s.t.app
        .get(&format!("/property/statements/{}", draft.id))
        .await
        .assert_ok()
        .assert_view("property/statement.html")
        .assert_see("AED 11,400.00")
        .assert_see("Approve");

    // A user with neither permission sees nothing.
    let outsider = s.t.user("Omar").await;
    s.t.app.acting_as(&outsider);
    s.t.app.get("/property/statements").await.assert_forbidden();
}
