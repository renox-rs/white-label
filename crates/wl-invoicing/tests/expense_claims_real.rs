//! Owner drawings on expense claims with the real wl-payments module (issue
//! #90 points 4 and 8; wl-invoicing §5.16, `FIN.ACL.21`, `FIN.ACL.23`;
//! wl-payments §4 *Expense claim*): a claim with a line in the *Owner
//! drawings* category waits for the drawings approval, which is wl-invoicing's
//! real approval; the decision reaches the claim through the kernel event. The
//! users act through the claim routes.

mod support;

use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::{
    BudgetCategory, DrawingApproval, DrawingStatus, WlInvoicing, permissions as perm,
};
use wl_kernel::approvals;
use wl_kernel::drawings::EXPENSE_CLAIM_SUBJECT_TYPE;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{ClientConfig, LegalEntity};
use wl_payments::expense_claim::{
    ClaimDrawing, ClaimStatus, ExpenseClaim, LineInput, replace_lines,
};
use wl_payments::{WlPayments, permissions as pay};

use support::{QUIET_SANDS, Recording, TestDirectory};

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "invoicing", "payments"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
trn = "100000000000003"
default = true
"#;

struct Real {
    app: TestApp,
    notices: Recording,
    _views: tempfile::TempDir,
}

/// wl-invoicing and wl-payments in one app, the owner party set on the
/// seeded drawings category, and the expense categories seeded.
async fn boot() -> Real {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let directory: std::sync::Arc<dyn wl_kernel::parties::PartyDirectory> =
        std::sync::Arc::new(TestDirectory::default());
    let notices = Recording::default();
    let notifier: std::sync::Arc<dyn wl_kernel::notify::Notifier> =
        std::sync::Arc::new(notices.clone());
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(WlInvoicing::new())
        .module(WlPayments::new())
        .provide(directory)
        .provide(notifier);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    wl_invoicing::budget_category::install(app.db())
        .await
        .expect("the budget categories are seeded");
    wl_payments::expense_category::install(app.db())
        .await
        .expect("the expense categories are seeded");
    let mut category = BudgetCategory::by_code(app.db(), "owner_drawings")
        .await
        .unwrap()
        .expect("the drawings category");
    category.owner_party_id = Some(QUIET_SANDS.0);
    category.save(app.db()).await.unwrap();
    Real {
        app,
        notices,
        _views: views,
    }
}

async fn user(t: &Real, name: &str, grants: &[&str]) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(t.app.db(), name, &email, "password123")
        .await
        .unwrap();
    let role = format!("role_{}", name.to_lowercase());
    permissions::define_role(t.app.db(), &role, grants)
        .await
        .unwrap();
    user.assign_role(t.app.db(), &role).await.unwrap();
    user
}

async fn entity(t: &Real, code: &str) -> LegalEntity {
    LegalEntity::query()
        .where_(LegalEntity::CODE.eq(code))
        .first(t.app.db())
        .await
        .unwrap()
        .unwrap()
}

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, m, d).unwrap()
}

/// `POST /expense-claims` as `by` in `entity`: AED 105.00 of fuel (VAT 5.00)
/// and a line of `drawing` dirhams in the category `drawing_category`.
async fn post_claim(
    t: &Real,
    by: &User,
    entity: &LegalEntity,
    drawing: &str,
    drawing_category: &str,
) -> ExpenseClaim {
    let id = entity.id.to_string();
    t.app.acting_as(by);
    t.app
        .post(
            "/expense-claims",
            &[
                ("legal_entity_id", id.as_str()),
                ("title", "March"),
                ("line_1_category", "fuel"),
                ("line_1_amount", "105.00"),
                ("line_1_vat", "5.00"),
                ("line_1_date", "2027-03-10"),
                ("line_2_category", drawing_category),
                ("line_2_amount", drawing),
                ("line_2_date", "2027-03-11"),
                ("line_2_description", "Villa decorator"),
            ],
        )
        .await
        .assert_status(303);
    ExpenseClaim::query()
        .order_by_desc(ExpenseClaim::ID)
        .first(t.app.db())
        .await
        .unwrap()
        .expect("a claim")
}

async fn act(
    t: &Real,
    by: &User,
    claim: &ExpenseClaim,
    action: &str,
    form: &[(&str, &str)],
) -> renox::testing::TestResponse {
    t.app.acting_as(by);
    t.app
        .post(&format!("/expense-claims/{}/{action}", claim.id), form)
        .await
}

async fn now(t: &Real, claim: &ExpenseClaim) -> ExpenseClaim {
    ExpenseClaim::find_or_404(t.app.db(), claim.id)
        .await
        .unwrap()
}

async fn drawing_of(t: &Real, claim: &ExpenseClaim) -> Vec<DrawingApproval> {
    DrawingApproval::for_source(t.app.db(), &claim.subject())
        .await
        .unwrap()
}

/// Given claimant A submits claim EC-12 with a line of AED 2,000 in *Owner
/// drawings*, the claim waits for owner B: approver C cannot approve it, A
/// cannot approve it, and after B approves the drawing C approves the claim.
#[renox::test]
async fn a_claim_with_a_drawing_waits_for_the_owner() {
    let t = boot().await;
    let pgp = entity(&t, "PGP").await;
    let a = user(&t, "Amal", &[pay::EXPENSES_VIEW, pay::EXPENSES_SUBMIT]).await;
    let b = user(&t, "Bilal", &[perm::DRAWINGS_APPROVE]).await;
    let c = user(&t, "Chadi", &[pay::EXPENSES_VIEW, pay::EXPENSES_APPROVE]).await;

    let claim = post_claim(&t, &a, &pgp, "2000.00", "owner_drawings").await;
    assert_eq!(claim.status, ClaimStatus::Draft);
    assert_eq!((claim.total, claim.vat_total), (210_500, 500));
    act(&t, &a, &claim, "submit", &[]).await.assert_status(303);

    // The drawing: the whole line, as the VAT of a drawing is blocked.
    let submitted = now(&t, &claim).await;
    assert_eq!(submitted.status, ClaimStatus::Submitted);
    assert_eq!(submitted.drawing_amount, 200_000);
    assert_eq!(submitted.drawing_status, Some(ClaimDrawing::Requested));
    assert!(!submitted.drawing_self_approved);
    let approval = drawing_of(&t, &claim).await.remove(0);
    assert_eq!(approval.status, DrawingStatus::Requested);
    assert_eq!(approval.amount, 200_000);
    assert_eq!(approval.requested_by, Some(a.id));
    assert_eq!(approval.source_type, EXPENSE_CLAIM_SUBJECT_TYPE);
    let request = approval.approval_request_id.expect("an approval request");

    // Until the owner approves the drawing nobody approves the claim; the
    // claimant is refused the approval of the drawing itself.
    act(&t, &c, &claim, "approve", &[]).await.assert_status(422);
    act(&t, &a, &claim, "approve", &[]).await.assert_status(403);
    assert!(
        approvals::approve(t.app.state(), request, &a, "")
            .await
            .is_err()
    );
    assert_eq!(now(&t, &claim).await.status, ClaimStatus::Submitted);

    approvals::approve(t.app.state(), request, &b, "")
        .await
        .unwrap()
        .expect("the approval runs");
    let after = now(&t, &claim).await;
    assert_eq!(after.drawing_status, Some(ClaimDrawing::Approved));
    assert_eq!(after.drawing_approved_by, Some(b.id));
    assert!(!after.drawing_self_approved);
    // The claimant never approves their own claim; another approver does.
    act(&t, &a, &claim, "approve", &[]).await.assert_status(403);
    act(&t, &c, &claim, "approve", &[]).await.assert_status(303);
    let done = now(&t, &claim).await;
    assert_eq!(done.status, ClaimStatus::Approved);
    assert_eq!(done.approved_by, Some(c.id));
}

/// A drawing owner B rejects leaves the claim `submitted` with the rejection
/// shown; it is approved only after approver C returns it, claimant A recodes
/// the line to *Other* and submits it again (no drawing, no request).
#[renox::test]
async fn a_rejected_drawing_holds_the_claim_until_the_line_is_recoded() {
    let t = boot().await;
    let pgp = entity(&t, "PGP").await;
    let a = user(&t, "Amal", &[pay::EXPENSES_VIEW, pay::EXPENSES_SUBMIT]).await;
    let b = user(&t, "Bilal", &[perm::DRAWINGS_APPROVE]).await;
    let c = user(&t, "Chadi", &[pay::EXPENSES_VIEW, pay::EXPENSES_APPROVE]).await;

    let claim = post_claim(&t, &a, &pgp, "2000.00", "owner_drawings").await;
    act(&t, &a, &claim, "submit", &[]).await.assert_status(303);
    let request = drawing_of(&t, &claim).await[0]
        .approval_request_id
        .expect("an approval request");
    approvals::reject(t.app.state(), request, &b, "personal use")
        .await
        .unwrap();
    let rejected = now(&t, &claim).await;
    assert_eq!(rejected.status, ClaimStatus::Submitted);
    assert_eq!(rejected.drawing_status, Some(ClaimDrawing::Rejected));
    assert_eq!(rejected.drawing_reason.as_deref(), Some("personal use"));
    act(&t, &c, &claim, "approve", &[]).await.assert_status(422);
    // The page shows the rejection to the approver.
    t.app.acting_as(&c);
    t.app
        .get("/expense-claims")
        .await
        .assert_ok()
        .assert_see("personal use");

    // A return needs a reason; the claimant then recodes the line.
    act(&t, &c, &claim, "return", &[("reason", " ")])
        .await
        .assert_status(422);
    act(
        &t,
        &c,
        &claim,
        "return",
        &[("reason", "Not a business cost")],
    )
    .await
    .assert_status(303);
    let back = now(&t, &claim).await;
    assert_eq!(back.status, ClaimStatus::Draft);
    assert_eq!(back.drawing_status, None);
    assert_eq!(back.decision_reason.as_deref(), Some("Not a business cost"));
    replace_lines(
        t.app.db(),
        claim.id,
        &[
            LineInput {
                expense_date: day(3, 10),
                merchant: String::new(),
                category: "fuel".into(),
                description: String::new(),
                amount: 10_500,
                vat_amount: 500,
            },
            LineInput {
                expense_date: day(3, 11),
                merchant: String::new(),
                category: "other".into(),
                description: "Villa decorator".into(),
                amount: 200_000,
                vat_amount: 0,
            },
        ],
    )
    .await
    .unwrap();
    act(&t, &a, &claim, "submit", &[]).await.assert_status(303);
    let again = now(&t, &claim).await;
    assert_eq!((again.drawing_amount, again.drawing_status), (0, None));
    act(&t, &c, &claim, "approve", &[]).await.assert_status(303);
    assert_eq!(now(&t, &claim).await.status, ClaimStatus::Approved);

    // A rejection needs a reason too.
    let other = post_claim(&t, &a, &pgp, "10.00", "other").await;
    act(&t, &a, &other, "submit", &[]).await.assert_status(303);
    act(&t, &c, &other, "reject", &[("reason", "")])
        .await
        .assert_status(422);
    act(&t, &c, &other, "reject", &[("reason", "No receipt")])
        .await
        .assert_status(303);
    assert_eq!(now(&t, &other).await.status, ClaimStatus::Rejected);
}

/// Entity E4's only holder of `drawings.approve` is owner O: O's expense line
/// in *Owner drawings* is approved without a second approver, flagged
/// self-approved, and the accountant role is told; the claim itself is still
/// approved by another user.
#[renox::test]
async fn the_sole_owners_expense_line_is_approved_at_once_and_flagged() {
    let t = boot().await;
    let mut e4 = LegalEntity {
        code: "E4".to_owned(),
        name_en: "E4 LLC".to_owned(),
        trn: Some("100000000000045".to_owned()),
        vat_registered: true,
        status: "active".to_owned(),
        ..Default::default()
    };
    e4.insert(t.app.db()).await.unwrap();
    let o = user(&t, "Omar", &[pay::EXPENSES_VIEW, pay::EXPENSES_SUBMIT]).await;
    permissions::define_role(t.app.db(), "e4_owner", &[perm::DRAWINGS_APPROVE])
        .await
        .unwrap();
    o.assign_role_in(
        t.app.db(),
        "e4_owner",
        &renox::auth::permissions::Scope::of_id::<LegalEntity>(e4.id),
    )
    .await
    .unwrap();
    permissions::define_role(
        t.app.db(),
        "accountant",
        &[pay::EXPENSES_VIEW, pay::EXPENSES_APPROVE],
    )
    .await
    .unwrap();
    let aya = User::register(t.app.db(), "Aya", "aya@example.com", "password123")
        .await
        .unwrap();
    aya.assign_role(t.app.db(), "accountant").await.unwrap();

    let claim = post_claim(&t, &o, &e4, "2000.00", "owner_drawings").await;
    act(&t, &o, &claim, "submit", &[]).await.assert_status(303);
    let submitted = now(&t, &claim).await;
    assert_eq!(submitted.drawing_status, Some(ClaimDrawing::Approved));
    assert!(submitted.drawing_self_approved);
    assert_eq!(submitted.drawing_approved_by, Some(o.id));
    let approval = drawing_of(&t, &claim).await.remove(0);
    assert_eq!(approval.status, DrawingStatus::Approved);
    assert!(approval.self_approved);

    // The accountant is told, once.
    let told: Vec<NotifyRecipient> = t
        .notices
        .of_type("invoicing.drawing_payment_self_approved")
        .iter()
        .flat_map(|r| r.recipients.clone())
        .collect();
    assert_eq!(told.len(), 1);
    assert!(matches!(told[0], NotifyRecipient::User { id } if id == aya.id));

    // The owner does not approve their own claim; the accountant does.
    act(&t, &o, &claim, "approve", &[]).await.assert_status(403);
    act(&t, &aya, &claim, "approve", &[])
        .await
        .assert_status(303);
    assert_eq!(now(&t, &claim).await.status, ClaimStatus::Approved);
}
