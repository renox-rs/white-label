//! Owner drawings on payments (wl-invoicing §5.16, `FIN.ACL.21`, `FIN.ACL.23`;
//! wl-payments #119): what wl-payments asks of the drawings approval through
//! the kernel's `OwnerDrawings` and how it hears the decision, and how the
//! drawings payments reach budget vs actual. wl-payments is not part of this
//! app: its payments are a scripted `DrawingPaymentSource`.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::budget_report::{self, ReportQuery};
use wl_invoicing::{BudgetCategory, DrawingApproval, DrawingStatus, permissions as perm};
use wl_kernel::approvals;
use wl_kernel::drawings::{
    DRAWING_REQUEST_SUBJECT_TYPE, DrawingFuture, DrawingPayment, DrawingPaymentSource,
    DrawingRequest, DrawingStart, owner_drawings, register_drawing_payments,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{Direction, PaymentId};
use wl_kernel::{LegalEntity, PartyId, Subject};

use support::{Booted, boot};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, m, d).unwrap()
}

/// An accountant who asks for the drawing.
async fn accountant(t: &Booted, name: &str) -> User {
    t.helper(name, &[perm::VIEW, perm::SUPPLIER_BILLS_VIEW]).await
}

/// An owner: holds `drawings.approve`.
async fn owner(t: &Booted, name: &str) -> User {
    t.helper(name, &[perm::SUPPLIER_BILLS_VIEW, perm::DRAWINGS_APPROVE]).await
}

/// A drawing of `amount` fils on the payment request `id` in `entity`.
fn ask(entity: &LegalEntity, id: i64, amount: i64, by: &User) -> DrawingRequest {
    DrawingRequest {
        legal_entity_id: entity.id,
        category: "owner_drawings".into(),
        source: Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, id),
        amount,
        requested_by: by.id,
    }
}

/// The seeded category is a drawings category; its owner party is the one the
/// client sets; any other category is not one.
#[renox::test]
async fn the_contract_names_the_drawings_category_and_its_owner_party() {
    let t = boot().await;
    let drawings = owner_drawings(t.state());
    let category = drawings
        .category(t.db(), "owner_drawings")
        .await
        .unwrap()
        .expect("a drawings category");
    assert_eq!(category.name_en, "Owner drawings");
    assert_eq!(category.owner_party, None);
    assert!(drawings.category(t.db(), "marketing").await.unwrap().is_none());
    assert!(drawings.category(t.db(), "nothing").await.unwrap().is_none());

    let mut row = BudgetCategory::by_code(t.db(), "owner_drawings")
        .await
        .unwrap()
        .unwrap();
    row.owner_party_id = Some(7);
    row.save(t.db()).await.unwrap();
    let category = drawings.category(t.db(), "owner_drawings").await.unwrap().unwrap();
    assert_eq!(category.owner_party, Some(PartyId(7)));
}

/// Accountant A asks for a drawing on a payment: the request goes to the
/// holders of `drawings.approve`, A cannot approve it, owner B approves it and
/// `OwnerDrawingDecided` tells wl-payments; a rejection is announced too.
#[renox::test]
async fn a_payment_drawing_waits_for_the_owner_and_the_decision_is_announced() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let a = accountant(&t, "Amal").await;
    let b = owner(&t, "Bilal").await;
    let drawings = owner_drawings(t.state());

    let started = drawings
        .request(t.state(), ask(&e1, 5, 1_500_000, &a))
        .await
        .unwrap();
    assert_eq!(started, DrawingStart::Pending);
    let source = Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, 5);
    let approval = DrawingApproval::for_source(t.db(), &source).await.unwrap().remove(0);
    assert_eq!(approval.status, DrawingStatus::Requested);
    assert_eq!(approval.amount, 1_500_000);
    assert_eq!(approval.requested_by, Some(a.id));
    let request_id = approval.approval_request_id.expect("an approval request");
    assert_eq!(t.notices_of(&b, "approval-requested").await.len(), 1);
    assert!(t.drawing_decided.all().is_empty());

    // Asking again for the same amount changes nothing.
    drawings
        .request(t.state(), ask(&e1, 5, 1_500_000, &a))
        .await
        .unwrap();
    assert_eq!(DrawingApproval::for_source(t.db(), &source).await.unwrap().len(), 1);

    assert!(approvals::approve(t.state(), request_id, &a, "").await.is_err());
    approvals::approve(t.state(), request_id, &b, "")
        .await
        .unwrap()
        .expect("the approval runs");
    let decided = t.drawing_decided.all();
    assert_eq!(decided.len(), 1);
    assert!(decided[0].approved);
    assert_eq!(decided[0].approved_by, Some(b.id));
    assert_eq!(decided[0].source, source);
    assert_eq!(decided[0].legal_entity_id, e1.id);

    // Another request, rejected.
    drawings
        .request(t.state(), ask(&e1, 6, 200_000, &a))
        .await
        .unwrap();
    let second = DrawingApproval::for_source(t.db(), &Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, 6))
        .await
        .unwrap()
        .remove(0);
    approvals::reject(t.state(), second.approval_request_id.unwrap(), &b, "not now")
        .await
        .unwrap();
    let decided = t.drawing_decided.all();
    assert_eq!(decided.len(), 2);
    assert!(!decided[1].approved);
    assert_eq!(decided[1].reason.as_deref(), Some("not now"));
}

/// A category that is not owner drawings is refused and nothing is made.
#[renox::test]
async fn a_category_that_is_not_a_drawings_category_is_refused() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let a = accountant(&t, "Amal").await;
    let mut request = ask(&e1, 9, 100, &a);
    request.category = "marketing".into();
    let refused = owner_drawings(t.state()).request(t.state(), request).await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    assert!(
        DrawingApproval::for_source(t.db(), &Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, 9))
            .await
            .unwrap()
            .is_empty()
    );
}

/// The entity's only holder of `drawings.approve` asks for a drawing: it is
/// approved at once, flagged, and the accountant is told (`FIN.ACL.23`).
#[renox::test]
async fn the_sole_owner_is_approved_at_once_and_the_accountant_is_told() {
    let t = boot().await;
    let e4 = t.new_entity("E4").await;
    let aya = t.accountant("Aya").await;
    let omar = t
        .scoped_user("Omar", &[perm::DRAWINGS_APPROVE], e4.id)
        .await;
    let started = owner_drawings(t.state())
        .request(t.state(), ask(&e4, 11, 500_000, &omar))
        .await
        .unwrap();
    assert_eq!(started, DrawingStart::Approved { self_approved: true });
    let approval = DrawingApproval::for_source(t.db(), &Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, 11))
        .await
        .unwrap()
        .remove(0);
    assert_eq!(approval.status, DrawingStatus::Approved);
    assert!(approval.self_approved);
    let told: Vec<_> = t
        .notices
        .of_type("invoicing.drawing_payment_self_approved")
        .iter()
        .flat_map(|r| r.recipients.clone())
        .collect();
    assert_eq!(told.len(), 1);
    assert!(matches!(told[0], NotifyRecipient::User { id } if id == aya.id));
    // Nothing is announced: the request answered at once.
    assert!(t.drawing_decided.all().is_empty());
}

/// The payments wl-payments recorded with purpose *owner drawings*.
struct Payments;

impl DrawingPaymentSource for Payments {
    fn payments<'a>(
        &'a self,
        _db: &'a Db,
        entities: &'a [i64],
        from: NaiveDate,
        to: NaiveDate,
    ) -> DrawingFuture<'a, Vec<DrawingPayment>> {
        Box::pin(async move {
            let all = [
                (1, Direction::Out, 1_500_000, day(3, 10), false),
                (2, Direction::In, 500_000, day(3, 20), false),
                (3, Direction::Out, 90_000, day(4, 2), true),
            ];
            Ok(all
                .into_iter()
                .filter(|(_, _, _, date, _)| *date >= from && *date <= to)
                .filter(|_| entities.contains(&1))
                .map(|(id, direction, amount, date, self_approved)| DrawingPayment {
                    payment: PaymentId(id),
                    legal_entity_id: 1,
                    category: "owner_drawings".into(),
                    direction,
                    amount,
                    date,
                    party: PartyId(7),
                    self_approved,
                })
                .collect())
        })
    }
}

/// A recorded drawing of AED 15,000 dated in March and a repayment of AED
/// 5,000 later in March are the owner drawings line of March: AED 10,000, never
/// operating cost; the sole owner's drawing of April is flagged.
#[renox::test]
async fn the_drawings_payments_are_on_the_drawings_line_of_budget_vs_actual() {
    register_drawing_payments("test.payments", Payments);
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let report = |month: u32| {
        let query = ReportQuery {
            entities: vec![e1.id],
            month: day(month, 1),
            ytd: false,
            drawings_lines: true,
            uncategorised: "Uncategorised".into(),
        };
        let db = t.db().clone();
        let config = t.config();
        async move { budget_report::build(&db, &config, &query).await.unwrap() }
    };
    let march = report(3).await;
    assert_eq!(march.drawings_total().1, 1_000_000);
    assert_eq!(march.expense_total().1, 0);
    assert_eq!(march.drawings_items.len(), 2);
    assert_eq!(march.drawings_items[0].amount, 1_500_000);
    assert_eq!(march.drawings_items[1].amount, -500_000);
    assert_eq!(
        march.drawings_items[0].source,
        Subject::new("payments.payment", 1)
    );
    let april = report(4).await;
    assert_eq!(april.drawings_total().1, 90_000);
    assert!(april.drawings_items[0].self_approved);
}
