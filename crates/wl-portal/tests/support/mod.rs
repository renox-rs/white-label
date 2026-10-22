//! Booting wl-portal in a `TestApp` with auth: a block guard that blocks the
//! parties the test names (and would give their reasons to anyone who asks),
//! a recording notifier, and test routes that run the portal guard the way a
//! portal page of another module does.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use renox::auth::{Auth, Permissions, permissions};
use renox::axum::extract::Form;
use renox::prelude::*;
use renox::testing::TestApp;
use serde::Deserialize;
use wl_kernel::PartyId;
use wl_kernel::block_guard::{
    ActionKind, BlockFuture, BlockGuard, BlockReasonLine, BlockRefusal, GuardOutcome,
    provide_block_guard,
};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRequest, provide_notifier};
use wl_kernel::workforce::{Workforce, WorkforceFuture, provide_workforce};
use wl_portal::{AccessGrant, NewGrant, PortalAccess, PortalAction, PortalGuard, WlPortal};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// Sara M., a tenant who acts for herself.
pub const SARA_PARTY: i64 = 70;
/// Acme Holdings, a company with two member companies.
pub const ACME_ONE: i64 = 81;
/// The second member company.
pub const ACME_TWO: i64 = 82;
/// A child, the dependant of a guardian.
pub const CHILD: i64 = 91;
/// The guardian's own party.
pub const GUARDIAN: i64 = 90;

/// The parties that are blocked now. The guard refuses everything but money
/// in for them, and, unlike the portal, would show the reason to any viewer:
/// the portal must not use it.
#[derive(Clone, Default)]
pub struct Blocks(Arc<Mutex<BTreeSet<i64>>>);

impl Blocks {
    /// Blocks `party`.
    pub fn block(&self, party: i64) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(party);
    }

    /// Unblocks `party`.
    pub fn unblock(&self, party: i64) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&party);
    }

    fn is_blocked(&self, party: i64) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&party)
    }
}

impl BlockGuard for Blocks {
    fn check_in<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        _viewer: Option<&'a User>,
        party: PartyId,
        kind: ActionKind,
    ) -> BlockFuture<'a, GuardOutcome> {
        Box::pin(async move {
            if !kind.is_guarded() || !self.is_blocked(party.0) {
                return Ok(GuardOutcome::Allowed);
            }
            Ok(GuardOutcome::Blocked(BlockRefusal::detailed(
                party,
                kind,
                vec![BlockReasonLine {
                    label: "Sanctions".to_owned(),
                    link: format!("/parties/{}/blocks/1", party.0),
                }],
                "Compliance",
            )))
        })
    }
}

/// The users whose employee was reported as absconded: the workforce switches
/// their portal access off, as `wl-hr` does from the approved report.
#[derive(Clone, Default)]
pub struct Absent(Arc<Mutex<BTreeSet<i64>>>, Arc<Mutex<BTreeSet<i64>>>);

impl Absent {
    /// The portal access of `user` is switched off from now on.
    pub fn disable(&self, user: i64) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(user);
    }

    /// The portal of `user` is read-only from now on (the employee exited).
    pub fn make_read_only(&self, user: i64) {
        self.1
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(user);
    }

    /// The portal of `user` is writable again.
    pub fn make_writable(&self, user: i64) {
        self.1
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&user);
    }
}

impl Workforce for Absent {
    fn access_disabled<'a>(&'a self, _db: &'a Db, user_id: i64) -> WorkforceFuture<'a, bool> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(&user_id))
        })
    }

    fn read_only<'a>(&'a self, _db: &'a Db, user_id: i64) -> WorkforceFuture<'a, bool> {
        Box::pin(async move {
            Ok(self
                .1
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(&user_id))
        })
    }
}

/// What the test routes did: the actions that went through.
#[derive(Clone, Default)]
pub struct Done(Arc<Mutex<Vec<String>>>);

impl Done {
    /// The keys of the actions that went through, oldest first.
    pub fn all(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Records what the notifier is asked for.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

impl Recording {
    /// Every request so far.
    pub fn all(&self) -> Vec<NotifyRequest> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request);
            Ok(vec![MessageId(1)])
        })
    }
}

/// The form of a test action: the party the user acts for and the page
/// language.
#[derive(Deserialize)]
struct ActionForm {
    party: i64,
    #[serde(default)]
    lang: String,
}

/// A portal page of another module: runs the guard for `action`, then "does"
/// it (records it).
async fn act(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
    Form(form): Form<ActionForm>,
) -> Result<String> {
    let action = PortalAction::from_key(&key).ok_or(Error::NotFound)?;
    let lang = state.lang(if form.lang.is_empty() {
        "en"
    } else {
        &form.lang
    });
    PortalGuard::require(&state, &lang, &user, PartyId(form.party), action).await?;
    if let Some(done) = state.provided::<Done>() {
        done.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{key}:{}", form.party));
    }
    Ok(format!("done {key}"))
}

/// The form of a staff action on a grant.
#[derive(Deserialize, Default)]
struct GrantForm {
    #[serde(default)]
    reason: String,
    #[serde(default)]
    person: i64,
    #[serde(default)]
    party: i64,
    #[serde(default)]
    roles: String,
}

/// Staff invite: `portal_users.invite`.
async fn invite(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<GrantForm>,
) -> Result<String> {
    let roles: Vec<&str> = form.roles.split(',').filter(|r| !r.is_empty()).collect();
    let grant = PortalAccess::invite(
        &state,
        &user,
        NewGrant::new(PartyId(form.person), PartyId(form.party), "tenant", &roles),
    )
    .await?;
    Ok(grant.id.to_string())
}

/// Staff act on a grant: `revoke`, `suspend`, `reinstate`, `end` or `close`.
async fn grant_action(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, action)): Path<(i64, String)>,
    Form(form): Form<GrantForm>,
) -> Result<String> {
    let reason = Some(form.reason.as_str()).filter(|r| !r.is_empty());
    let grant = match action.as_str() {
        "revoke" => PortalAccess::revoke(&state, &user, id).await?,
        "suspend" => PortalAccess::suspend(&state, &user, id, &form.reason).await?,
        "reinstate" => PortalAccess::reinstate(&state, &user, id).await?,
        "end" => PortalAccess::end(&state, Some(&user), id, reason).await?,
        "close" => PortalAccess::close(&state, Some(&user), id, reason).await?,
        _ => return Err(Error::NotFound),
    };
    Ok(grant.status.as_str().to_owned())
}

/// Provides the fakes and the test routes.
struct Harness {
    /// Whether the fake block guard is provided (else the app has the real
    /// one of wl-parties).
    fake_guard: bool,
    blocks: Blocks,
    done: Done,
    notices: Recording,
    absent: Absent,
}

impl Module for Harness {
    fn name(&self) -> &'static str {
        "portal-test-harness"
    }

    fn register(&self, app: &mut Registry) {
        if self.fake_guard {
            provide_block_guard(app, self.blocks.clone());
        }
        provide_notifier(app, self.notices.clone());
        provide_workforce(app, self.absent.clone());
        app.provide(self.done.clone());
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/test/portal/{key}", act)
            .post("/test/grants", invite)
            .post("/test/grants/{id}/{action}", grant_action)
            .require_auth()
    }
}

/// The app under test.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// The blocked parties.
    pub blocks: Blocks,
    /// The actions that went through.
    pub done: Done,
    /// The users whose portal access is switched off.
    pub absent: Absent,
    /// What the notifier was asked for.
    pub notices: Recording,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
}

impl Booted {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// A user holding exactly `grants` (through a role of their own).
    pub async fn user(&self, name: &str, grants: &[&str]) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        let role = format!("role_{}", name.to_lowercase());
        permissions::define_role(self.db(), &role, grants)
            .await
            .unwrap();
        user.assign_role(self.db(), &role).await.unwrap();
        user
    }

    /// A portal user with an `active` grant for `party` (self-registered).
    pub async fn portal_user(&self, name: &str, person: i64, party: i64) -> (User, AccessGrant) {
        let user = self.user(name, &[]).await;
        let grant = PortalAccess::register(
            self.state(),
            user.id,
            NewGrant::new(
                PartyId(person),
                PartyId(party),
                "tenant",
                &["primary_contact"],
            ),
        )
        .await
        .unwrap();
        (user, grant)
    }

    /// `user` posts `form` to `path`.
    pub async fn post_as(
        &self,
        user: &User,
        path: &str,
        form: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app.post(path, form).await
    }

    /// `user` runs the test action `key` for `party` in `lang`.
    pub async fn act(
        &self,
        user: &User,
        key: &str,
        party: i64,
        lang: &str,
    ) -> renox::testing::TestResponse {
        let party = party.to_string();
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/test/portal/{key}"),
                &[("party", party.as_str()), ("lang", lang)],
            )
            .await
    }
}

/// `WlPortal` booted with auth, the fakes and the test routes.
pub async fn boot() -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let (blocks, done, notices, absent) = (
        Blocks::default(),
        Done::default(),
        Recording::default(),
        Absent::default(),
    );
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(WlPortal)
        .module(Harness {
            fake_guard: true,
            blocks: blocks.clone(),
            done: done.clone(),
            notices: notices.clone(),
            absent: absent.clone(),
        });
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    Booted {
        app,
        blocks,
        done,
        absent,
        notices,
        _views: views,
    }
}

/// A client with parties on.
const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
default = true
"#;

/// `WlPortal` booted with the real `wl-parties` (its block guard, its demo
/// parties seeded) and the test routes. `Booted::blocks` is not used.
pub async fn boot_real() -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let (blocks, done, notices, absent) = (
        Blocks::default(),
        Done::default(),
        Recording::default(),
        Absent::default(),
    );
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(wl_parties::WlParties::new())
        .module(WlPortal)
        .module(Harness {
            fake_guard: false,
            blocks: blocks.clone(),
            done: done.clone(),
            notices: notices.clone(),
            absent: absent.clone(),
        });
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    wl_parties::seed::run(app.state())
        .await
        .expect("the demo parties seed");
    Booted {
        app,
        blocks,
        done,
        absent,
        notices,
        _views: views,
    }
}
