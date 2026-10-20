//! BL-PRP-21 (#271): the owner disputes a line of a published statement. The
//! statement is flagged and a ticket opened; the flag is cleared by a revision,
//! or with an outcome: upheld, an adjustment line on the next statement, or the
//! adjustment lines of a cost charged to the wrong owner on the next statements
//! of both owners.

mod support;

use property::Lease;
use property::agreement::{AgreementStatus, ManagementAgreement};
use property::notices::OWNER_STATEMENT_DISPUTE_OUTCOME;
use property::statement::{self, LineKind, OwnerStatement, StatementLine, StatementStatus};
use property::statement_dispute::{
    DISPUTE_CATEGORY, DisputeOutcome, DisputeStatus, StatementDispute,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::PartyId;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::parties::PartyKind;

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, aed, boot, day};

/// The owner who complains.
const OWNER_X: PartyId = PartyId(31);
/// The owner the cost belongs to.
const OWNER_Y: PartyId = PartyId(32);

struct Scene {
    t: Booted,
    x: ManagementAgreement,
    y: ManagementAgreement,
    accountant: User,
    manager: User,
}

/// An active agreement for each of two owners, an accountant and a manager.
async fn scene() -> Scene {
    let t = boot().await;
    t.parties
        .add(OWNER_X.0, "Hessa Al Mansoori", PartyKind::Person);
    t.parties
        .add(OWNER_Y.0, "Rashid Al Falasi", PartyKind::Person);
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let site = t.user_in("Sami", MANAGER, building.id).await;
    let _lease: Lease = t.active_lease(&unit, &agent, &site).await;
    let x = agreement(&t, OWNER_X).await;
    let y = agreement(&t, OWNER_Y).await;
    property::agreement::add_unit(t.db(), &x, unit.id, day(-400))
        .await
        .unwrap();
    let accountant = t.user_with("Amal", ACCOUNTANT).await;
    let manager = t.user_with("Mona", MANAGER).await;
    Scene {
        t,
        x,
        y,
        accountant,
        manager,
    }
}

async fn agreement(t: &Booted, owner: PartyId) -> ManagementAgreement {
    let mut agreement = ManagementAgreement {
        owner_party_id: owner.0,
        fee_bp: 500,
        payout_frequency: "monthly".to_owned(),
        notice_days: 90,
        start_date: day(-400),
        status: AgreementStatus::Active,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    agreement
        .insert_in_state(&mut tx, AgreementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    agreement
}

impl Scene {
    /// `user` runs the transition `name` on the statement `id`.
    async fn run(
        &self,
        user: &User,
        id: i64,
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
                &format!("/property/statements/{id}/transitions/{name}"),
                &pairs,
            )
            .await
    }

    /// A published statement of owner X for a month in the past with the
    /// expense "AMC chiller Q1" of AED 4,200, and the id of that line.
    async fn published(&self) -> (OwnerStatement, i64) {
        let draft = statement::generate(self.t.state(), self.x.id, day(-60), day(-31))
            .await
            .unwrap()
            .expect("a draft");
        self.t
            .post(
                &self.accountant,
                &format!("/property/statements/{}/lines", draft.id),
                &[
                    ("kind", "expense"),
                    ("description", "AMC chiller Q1"),
                    ("amount", "4,200"),
                ],
            )
            .await
            .assert_ok();
        self.run(&self.manager, draft.id, "approve", "")
            .await
            .assert_status(303);
        self.run(&self.accountant, draft.id, "publish", "")
            .await
            .assert_status(303);
        let published = OwnerStatement::find_or_404(self.t.db(), draft.id)
            .await
            .unwrap();
        let line = published
            .lines(self.t.db())
            .await
            .unwrap()
            .into_iter()
            .find(|l| l.kind == LineKind::Expense)
            .expect("the expense");
        (published, line.id)
    }

    /// The accountant records the dispute of `line` (blank for the whole).
    async fn dispute(
        &self,
        statement: i64,
        line: &str,
        comment: &str,
    ) -> renox::testing::TestResponse {
        self.t
            .post(
                &self.accountant,
                &format!("/property/statements/{statement}/disputes"),
                &[("line", line), ("comment", comment)],
            )
            .await
    }

    /// The accountant clears the dispute `dispute`.
    async fn clear(
        &self,
        statement: i64,
        dispute: i64,
        extra: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        let dispute = dispute.to_string();
        let mut form = vec![("dispute", dispute.as_str())];
        form.extend_from_slice(extra);
        self.t
            .post(
                &self.accountant,
                &format!("/property/statements/{statement}/disputes/clear"),
                &form,
            )
            .await
    }

    async fn statement(&self, id: i64) -> OwnerStatement {
        OwnerStatement::find_or_404(self.t.db(), id).await.unwrap()
    }

    async fn disputes(&self, id: i64) -> Vec<StatementDispute> {
        StatementDispute::of_statement(self.t.db(), id)
            .await
            .unwrap()
    }

    /// The lines of `agreement` that wait for the next draft.
    async fn waiting(&self, agreement: i64) -> Vec<StatementLine> {
        StatementLine::query()
            .where_(StatementLine::AGREEMENT_ID.eq(agreement))
            .where_(StatementLine::STATEMENT_ID.is_null())
            .order_by(StatementLine::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }
}

/// AC1: flagging the line "AMC chiller Q1 AED 4,200" of a published statement
/// sets the flag and opens a ticket; only a published or settled statement is
/// disputed.
#[renox::test]
async fn bl_prp_21_flagging_a_line_sets_the_flag_and_opens_a_ticket() {
    let s = scene().await;
    let (published, line) = s.published().await;
    assert!(!published.disputed);

    s.dispute(
        published.id,
        &line.to_string(),
        "The owner did not agree to the chiller",
    )
    .await
    .assert_ok();
    let flagged = s.statement(published.id).await;
    assert!(flagged.disputed);
    assert_eq!(flagged.status, StatementStatus::Published);
    let disputes = s.disputes(published.id).await;
    assert_eq!(disputes.len(), 1);
    assert_eq!(disputes[0].line_id, Some(line));
    assert!(
        disputes[0].description.contains("AMC chiller Q1"),
        "{}",
        disputes[0].description
    );
    assert!(
        disputes[0].description.contains("AED 4,200.00"),
        "{}",
        disputes[0].description
    );
    assert_eq!(disputes[0].status, DisputeStatus::Open);

    let tickets = s.t.tasks.with_key("property.statement_dispute:");
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].category, DISPUTE_CATEGORY);
    assert_eq!(tickets[0].subject, published.subject());
    assert!(
        tickets[0].title.contains(&published.number()),
        "{}",
        tickets[0].title
    );
    assert!(tickets[0].open);
    assert_eq!(
        disputes[0].ticket_number,
        Some(format!("TKT-{}", tickets[0].id))
    );

    // The page shows the flag.
    s.t.app.acting_as(&s.manager);
    s.t.app
        .get(&format!("/property/statements/{}", published.id))
        .await
        .assert_ok()
        .assert_see("Disputed")
        .assert_see("AMC chiller Q1");
}

/// A draft is not disputed; the comment is required; the line must be the
/// statement's; only the accountant records it.
#[renox::test]
async fn bl_prp_21_what_can_be_disputed() {
    let s = scene().await;
    let draft = statement::generate(s.t.state(), s.x.id, day(-30), day(-1))
        .await
        .unwrap()
        .expect("a draft");
    s.dispute(draft.id, "", "too much").await.assert_status(422);
    assert!(!s.statement(draft.id).await.disputed);

    let (published, line) = s.published().await;
    s.dispute(published.id, "", "  ").await.assert_status(422);
    // A line of another statement.
    s.dispute(draft.id, "", "x").await.assert_status(422);
    let other = statement::generate(s.t.state(), s.y.id, day(-60), day(-31))
        .await
        .unwrap()
        .expect("a draft");
    s.t.post(
        &s.accountant,
        &format!("/property/statements/{}/lines", other.id),
        &[
            ("kind", "expense"),
            ("description", "Lift"),
            ("amount", "10"),
        ],
    )
    .await
    .assert_ok();
    let foreign = other.lines(s.t.db()).await.unwrap()[0].id.to_string();
    s.dispute(published.id, &foreign, "wrong statement")
        .await
        .assert_status(404);
    // The manager does not record disputes.
    s.t.post(
        &s.manager,
        &format!("/property/statements/{}/disputes", published.id),
        &[("line", &line.to_string()), ("comment", "x")],
    )
    .await
    .assert_forbidden();
    assert!(s.disputes(published.id).await.is_empty());
}

/// AC2: clearing with the outcome *adjustment line* (-4,200) puts the line on
/// the next draft statement, releases the flag, resolves the ticket and tells
/// the owner; the published statement is not edited.
#[renox::test]
async fn bl_prp_21_an_adjustment_line_goes_on_the_next_draft() {
    let s = scene().await;
    let (published, line) = s.published().await;
    s.dispute(published.id, &line.to_string(), "The chiller was not ours")
        .await
        .assert_ok();
    let dispute = s.disputes(published.id).await[0].id;

    // An adjustment needs an amount.
    s.clear(published.id, dispute, &[("outcome", "adjustment")])
        .await
        .assert_status(422);
    assert!(s.statement(published.id).await.disputed);

    s.clear(
        published.id,
        dispute,
        &[
            ("outcome", "adjustment"),
            ("amount", "4,200"),
            ("direction", "debit"),
        ],
    )
    .await
    .assert_ok();

    let cleared = s.statement(published.id).await;
    assert!(!cleared.disputed);
    assert_eq!(cleared.expenses, published.expenses);
    let done = &s.disputes(published.id).await[0];
    assert_eq!(done.status, DisputeStatus::Cleared);
    assert_eq!(done.outcome, Some(DisputeOutcome::Adjusted));
    assert_eq!(done.adjustment, Some(-aed(4_200)));
    assert!(!s.t.tasks.with_key("property.statement_dispute:")[0].open);

    let waiting = s.waiting(s.x.id).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].kind, LineKind::Adjustment);
    assert_eq!(waiting[0].amount, -aed(4_200));
    assert_eq!(waiting[0].linked_to, Some(line));
    assert!(
        waiting[0].description.contains(&published.number()),
        "{}",
        waiting[0].description
    );

    let told = s.t.notices.of_type(OWNER_STATEMENT_DISPUTE_OUTCOME);
    assert_eq!(told.len(), 1);
    assert!(
        told[0]
            .recipients
            .iter()
            .any(|r| matches!(r, NotifyRecipient::Party { id } if *id == OWNER_X))
    );

    // The next draft shows it.
    let next = statement::generate(s.t.state(), s.x.id, day(-30), day(10))
        .await
        .unwrap()
        .expect("a draft");
    assert_eq!(next.adjustments, -aed(4_200));
    assert!(s.waiting(s.x.id).await.is_empty());
    // A cleared dispute is not cleared twice.
    s.clear(published.id, dispute, &[("outcome", "upheld")])
        .await
        .assert_status(422);
}

/// AC3: clearing by `revise` supersedes the statement and creates revision 2;
/// the dispute is cleared as revised, the ticket resolved and the owner told.
#[renox::test]
async fn bl_prp_21_a_revision_clears_the_flag() {
    let s = scene().await;
    let (published, line) = s.published().await;
    s.dispute(published.id, &line.to_string(), "Wrong amount")
        .await
        .assert_ok();

    s.run(&s.accountant, published.id, "revise", "AMC bill was for Q2")
        .await
        .assert_status(303);

    let old = s.statement(published.id).await;
    assert_eq!(old.status, StatementStatus::Superseded);
    assert!(!old.disputed);
    let done = &s.disputes(published.id).await[0];
    assert_eq!(done.status, DisputeStatus::Cleared);
    assert_eq!(done.outcome, Some(DisputeOutcome::Revised));
    assert!(!s.t.tasks.with_key("property.statement_dispute:")[0].open);

    let all = OwnerStatement::of_agreement(s.t.db(), s.x.id)
        .await
        .unwrap();
    let revision = all.iter().find(|r| r.revision == 2).expect("revision 2");
    assert_eq!(revision.status, StatementStatus::Draft);
    assert!(!revision.disputed);
    assert_eq!(
        s.t.notices.of_type(OWNER_STATEMENT_DISPUTE_OUTCOME).len(),
        1
    );
}

/// AC4: a cost of AED 1,800 allocated to the wrong owner produces -1,800 on
/// owner X's next statement and +1,800 on owner Y's, linked.
#[renox::test]
async fn bl_prp_21_a_cost_of_the_wrong_owner_is_corrected_on_both_statements() {
    let s = scene().await;
    let (published, line) = s.published().await;
    s.dispute(published.id, &line.to_string(), "The cost is owner Y's")
        .await
        .assert_ok();
    let dispute = s.disputes(published.id).await[0].id;
    let y = s.y.id.to_string();

    // The other owner must be another agreement.
    let own = s.x.id.to_string();
    s.clear(
        published.id,
        dispute,
        &[
            ("outcome", "reallocation"),
            ("amount", "1,800"),
            ("direction", "debit"),
            ("other_agreement", &own),
        ],
    )
    .await
    .assert_status(422);
    s.clear(
        published.id,
        dispute,
        &[
            ("outcome", "reallocation"),
            ("amount", "1,800"),
            ("direction", "debit"),
            ("other_agreement", "9999"),
        ],
    )
    .await
    .assert_status(422);
    assert!(s.waiting(s.x.id).await.is_empty() && s.waiting(s.y.id).await.is_empty());

    s.clear(
        published.id,
        dispute,
        &[
            ("outcome", "reallocation"),
            ("amount", "1,800"),
            ("direction", "debit"),
            ("other_agreement", &y),
        ],
    )
    .await
    .assert_ok();

    let mine = s.waiting(s.x.id).await;
    let theirs = s.waiting(s.y.id).await;
    assert_eq!((mine.len(), theirs.len()), (1, 1));
    assert_eq!(mine[0].amount, -aed(1_800));
    assert_eq!(theirs[0].amount, aed(1_800));
    assert_eq!(mine[0].linked_to, Some(theirs[0].id));
    assert_eq!(theirs[0].linked_to, Some(mine[0].id));
    assert_eq!(
        s.disputes(published.id).await[0].outcome,
        Some(DisputeOutcome::Reallocated)
    );
    assert!(!s.statement(published.id).await.disputed);

    // Each owner's next statement carries its line.
    let next_x = statement::generate(s.t.state(), s.x.id, day(-30), day(10))
        .await
        .unwrap()
        .unwrap();
    let next_y = statement::generate(s.t.state(), s.y.id, day(-30), day(10))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next_x.adjustments, -aed(1_800));
    assert_eq!(next_y.adjustments, aed(1_800));
}

/// The flag stays while any dispute is open; a settled statement is disputed
/// after its payout and cleared as upheld, never revised.
#[renox::test]
async fn bl_prp_21_the_flag_stays_while_a_dispute_is_open_and_a_settled_statement_is_upheld() {
    let s = scene().await;
    let (published, line) = s.published().await;
    s.dispute(published.id, &line.to_string(), "first")
        .await
        .assert_ok();
    s.dispute(published.id, "", "second").await.assert_ok();
    let disputes = s.disputes(published.id).await;
    assert_eq!(disputes.len(), 2);

    s.clear(published.id, disputes[0].id, &[("outcome", "upheld")])
        .await
        .assert_ok();
    assert!(s.statement(published.id).await.disputed);
    s.clear(
        published.id,
        disputes[1].id,
        &[("outcome", "upheld"), ("note", "checked")],
    )
    .await
    .assert_ok();
    assert!(!s.statement(published.id).await.disputed);
    assert_eq!(
        s.disputes(published.id).await[1].note.as_deref(),
        Some("checked")
    );

    // A settled statement (after its payout).
    let mut settled = OwnerStatement {
        agreement_id: s.x.id,
        owner_party_id: OWNER_X.0,
        period_start: day(-120),
        period_end: day(-91),
        revision: 1,
        status: StatementStatus::Settled,
        payout_amount: Some(aed(1_000)),
        ..Default::default()
    };
    let mut tx = s.t.db().begin().await.unwrap();
    settled
        .insert_in_state(&mut tx, StatementStatus::Settled, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    s.dispute(settled.id, "", "after the payout")
        .await
        .assert_ok();
    assert!(s.statement(settled.id).await.disputed);
    // It cannot be revised.
    s.run(&s.accountant, settled.id, "revise", "late")
        .await
        .assert_status(422);
    let dispute = s.disputes(settled.id).await[0].id;
    s.clear(settled.id, dispute, &[("outcome", "upheld")])
        .await
        .assert_ok();
    assert!(!s.statement(settled.id).await.disputed);
}
