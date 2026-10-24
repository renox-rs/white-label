//! BL-CON-19 (#298) in the real Construction host with the portal on: a
//! project whose final account is closed (`ProjectClosed`) turns the client's
//! portal grant read-only only when no other project of the client runs; the
//! grant stays `active` while one does (`POR.ACL.01`).

mod support;

use construction::client_portal::CLIENT_AUDIENCE;
use construction::{Project, ProjectClosed, ProjectStatus};
use host::roles::MEMBER;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::person;
use wl_kernel::documents::today;
use wl_kernel::{ClientConfig, PartyId};
use wl_portal::{AccessGrant, GrantStatus, NewGrant, PortalAccess};

/// A contracting client with the portal on.
const CLIENT: &str = r#"
[client]
name = "Gulf Build Contracting LLC"
app = "construction"

[modules]
enabled = ["parties", "hr", "attendance", "inventory", "portal"]

[[legal_entities]]
code = "GBC"
name_en = "Gulf Build Contracting LLC"
default = true
"#;

/// The client company of the projects.
const CLIENT_PARTY: PartyId = PartyId(31);

async fn project_in(app: &TestApp, code: &str, status: ProjectStatus) -> Project {
    let mut project = Project::new(code, "Marina Tower fit-out", CLIENT_PARTY);
    project.status = status;
    let mut tx = app.db().begin().await.unwrap();
    project
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    project
}

async fn status(app: &TestApp, grant: &AccessGrant) -> AccessGrant {
    AccessGrant::find_or_404(app.db(), grant.id).await.unwrap()
}

fn closed(project: &Project) -> ProjectClosed {
    ProjectClosed {
        project: project.subject(),
        client: CLIENT_PARTY,
        closed_on: today(),
        transition: "settle_final_account".to_owned(),
    }
}

#[renox::test]
async fn bl_con_19_the_grant_turns_read_only_only_when_the_last_project_closes() {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    let contact = person(&app, "Rana", MEMBER).await;
    let grant = PortalAccess::register(
        app.state(),
        contact.id,
        NewGrant::new(PartyId(77), CLIENT_PARTY, CLIENT_AUDIENCE, &[]),
    )
    .await
    .unwrap();
    assert_eq!(grant.status, GrantStatus::Active);

    let p = project_in(&app, "TWR-01", ProjectStatus::Closed).await;
    let q = project_in(&app, "TWR-02", ProjectStatus::InProgress).await;

    // P closed while Q runs: the grant stays active (Q is not touched).
    app.state().emit(closed(&p)).await.unwrap();
    assert_eq!(status(&app, &grant).await.status, GrantStatus::Active);
    assert_eq!(
        host::construction_portal::project_closed(app.state(), CLIENT_PARTY, today(), "test")
            .await
            .unwrap(),
        0
    );

    // Q closes too: the grant turns read-only from that day; wl-portal's
    // timer closes it 90 days later.
    renox::db::sql("UPDATE construction_projects SET status = ? WHERE id = ?")
        .bind("closed")
        .bind(q.id)
        .execute(app.db())
        .await
        .unwrap();
    app.state().emit(closed(&q)).await.unwrap();
    let now = status(&app, &grant).await;
    assert_eq!(now.status, GrantStatus::ReadOnly);
    assert_eq!(now.ended_on, Some(today()));
}
