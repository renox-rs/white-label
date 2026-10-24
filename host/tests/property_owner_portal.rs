//! BL-PRP-29 (#279) in the real Property host: the owner disputes a line of a
//! published statement from the owner portal, which sets the dispute flag and
//! opens the ticket as staff do (BL-PRP-21); after the management agreement
//! ends the owner reads the statements for 90 days and cannot dispute, and on
//! day 91 the portal is closed.

mod support;

use host::roles::{ACCOUNTANT, MEMBER, PROPERTY_MANAGER};
use property::statement::{OwnerStatement, StatementLine};
use property::statement_dispute::StatementDispute;
use renox::chrono::{NaiveDate, NaiveDateTime};
use renox::prelude::*;
use renox::testing::TestApp;
use support::{agreement_of, booted, day, person, tenant};
use wl_kernel::{BusinessCalendar, PartyId};
use wl_portal::{AccessGrant, GrantStatus, NewGrant, PortalAccess};

/// Moves the clock to noon of `day` (Asia/Dubai, ahead of the real clock).
fn travel_to(app: &TestApp, day: &str) {
    let local = NaiveDateTime::parse_from_str(&format!("{day} 12:00"), "%Y-%m-%d %H:%M").unwrap();
    let ahead = (BusinessCalendar::default().from_local(local) - renox::db::now()).num_seconds();
    assert!(ahead > 0, "{day} is ahead of the clock");
    app.travel_back();
    app.travel(std::time::Duration::from_secs(ahead.unsigned_abs()));
}

struct Scene {
    app: TestApp,
    owner: PartyId,
    agreement: property::agreement::ManagementAgreement,
    statement: OwnerStatement,
    amc: StatementLine,
    hessa: User,
    grant: AccessGrant,
}

/// Owner O with a published statement whose lines are the rent collected and
/// the expense *Lift AMC Q2*; Hessa acts for O in the owner portal.
async fn scene() -> Scene {
    let app = booted().await;
    let owner = tenant(&app, "Omar Al Futtaim").await;
    let agreement = agreement_of(&app, owner).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let draft = property::statement::generate(app.state(), agreement.id, day(-60), day(-31))
        .await
        .unwrap()
        .expect("a draft");
    app.acting_as(&accountant);
    for (kind, description, amount) in [
        ("collected", "Rent", "20000"),
        ("expense", "Lift AMC Q2", "1550"),
    ] {
        app.htmx()
            .post(
                &format!("/property/statements/{}/lines", draft.id),
                &[
                    ("kind", kind),
                    ("description", description),
                    ("amount", amount),
                ],
            )
            .await
            .assert_ok();
    }
    app.acting_as(&manager);
    app.post(
        &format!("/property/statements/{}/transitions/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    app.acting_as(&accountant);
    app.post(
        &format!("/property/statements/{}/transitions/publish", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    let statement = OwnerStatement::find_or_404(app.db(), draft.id)
        .await
        .unwrap();
    let amc = statement
        .lines(app.db())
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.description == "Lift AMC Q2")
        .expect("the AMC line");
    let hessa = person(&app, "Hessa", MEMBER).await;
    let grant = PortalAccess::register(
        app.state(),
        hessa.id,
        NewGrant::new(owner, owner, "owner", &["owner"]),
    )
    .await
    .unwrap();
    Scene {
        app,
        owner,
        agreement,
        statement,
        amc,
        hessa,
        grant,
    }
}

impl Scene {
    fn dispute_path(&self) -> String {
        format!(
            "/portal/{}/owner-statements/{}/dispute",
            self.owner.0, self.statement.id
        )
    }

    async fn dispute(&self) -> renox::testing::TestResponse {
        self.app.acting_as(&self.hessa);
        let line = self.amc.id.to_string();
        self.app
            .htmx()
            .post(
                &self.dispute_path(),
                &[("line", &line), ("comment", "The lift was out of order")],
            )
            .await
    }

    async fn disputes(&self) -> Vec<StatementDispute> {
        StatementDispute::of_statement(self.app.db(), self.statement.id)
            .await
            .unwrap()
    }
}

/// Done when 1: owner O disputes the line "Lift AMC Q2" in the portal: the
/// dispute flag is set and the ticket is opened (as BL-PRP-21).
#[renox::test]
async fn prp29_the_owner_disputes_a_statement_line_from_the_portal() {
    let s = scene().await;
    s.app.acting_as(&s.hessa);
    let page = s
        .app
        .get(&format!("/portal/{}/owner_statements", s.owner.0))
        .await;
    page.assert_ok();
    assert!(
        page.text().contains(&s.statement.number()),
        "{}",
        page.text()
    );

    s.dispute().await.assert_ok();

    let statement = OwnerStatement::find_or_404(s.app.db(), s.statement.id)
        .await
        .unwrap();
    assert!(statement.disputed);
    let disputes = s.disputes().await;
    assert_eq!(disputes.len(), 1);
    assert_eq!(disputes[0].line_id, Some(s.amc.id));
    assert_eq!(disputes[0].opened_by, Some(s.hessa.id));
    assert!(disputes[0].ticket_id.is_some(), "the ticket is opened");
    assert!(disputes[0].ticket_number.is_some());
}

/// Must not: an owner cannot dispute a statement of another owner, nor reach
/// another owner's portal.
#[renox::test]
async fn prp29_an_owner_cannot_dispute_another_owners_statement() {
    let s = scene().await;
    let other = tenant(&s.app, "Another Owner").await;
    let zaid = person(&s.app, "Zaid", MEMBER).await;
    PortalAccess::register(
        s.app.state(),
        zaid.id,
        NewGrant::new(other, other, "owner", &["owner"]),
    )
    .await
    .unwrap();
    s.app.acting_as(&zaid);
    let line = s.amc.id.to_string();
    let own_party = s
        .app
        .htmx()
        .post(
            &format!(
                "/portal/{}/owner-statements/{}/dispute",
                other.0, s.statement.id
            ),
            &[("line", &line), ("comment", "Not mine")],
        )
        .await;
    assert_eq!(own_party.status, StatusCode::NOT_FOUND);
    let foreign = s
        .app
        .htmx()
        .post(
            &s.dispute_path(),
            &[("line", &line), ("comment", "Not mine")],
        )
        .await;
    assert_eq!(foreign.status, StatusCode::FORBIDDEN);
    assert!(s.disputes().await.is_empty());
}

/// Done when 2: after the agreement ended on 2028-06-30, O reads the
/// statements until 2028-09-28 and cannot dispute; on 2028-09-29 the portal is
/// closed for O.
#[renox::test]
async fn prp29_after_the_agreement_ends_the_owner_reads_for_90_days_then_the_portal_closes() {
    let s = scene().await;
    renox::db::sql("UPDATE property_management_agreements SET status = 'ended' WHERE id = ?")
        .bind(s.agreement.id)
        .execute(s.app.db())
        .await
        .unwrap();
    let ended_on = NaiveDate::from_ymd_opt(2028, 6, 30).unwrap();
    travel_to(&s.app, "2028-06-30");
    s.app
        .at_travelled_time(s.app.state().emit(property::AgreementEnded {
            agreement: s.agreement.id,
            owner: s.owner,
            ended_on,
        }))
        .await
        .unwrap();
    let grant = AccessGrant::find_or_404(s.app.db(), s.grant.id)
        .await
        .unwrap();
    assert_eq!(grant.status, GrantStatus::ReadOnly);
    assert_eq!(grant.ended_on, Some(ended_on));

    travel_to(&s.app, "2028-09-28");
    s.app.run_scheduled(wl_portal::CLOSE_TASK).await.unwrap();
    s.app.acting_as(&s.hessa);
    let page = s
        .app
        .get(&format!("/portal/{}/owner_statements", s.owner.0))
        .await;
    page.assert_ok();
    assert!(page.text().contains(&s.statement.number()));
    let refused = s.dispute().await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert!(s.disputes().await.is_empty());

    travel_to(&s.app, "2028-09-29");
    s.app.run_scheduled(wl_portal::CLOSE_TASK).await.unwrap();
    let grant = AccessGrant::find_or_404(s.app.db(), s.grant.id)
        .await
        .unwrap();
    assert_eq!(grant.status, GrantStatus::Closed);
    s.app.acting_as(&s.hessa);
    let closed = s
        .app
        .get(&format!("/portal/{}/owner_statements", s.owner.0))
        .await;
    assert_eq!(closed.status, StatusCode::FORBIDDEN);
    s.app.travel_back();
}
