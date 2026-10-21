//! Everyone who signed up, and their roles, for admins (`users.manage`).
//! Role changes are written to the activity log.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridRequest};
use renox::nav::NavLink;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::roles::{ADMIN, ROLES, USERS};

pub struct Users;

impl Module for Users {
    fn name(&self) -> &'static str {
        "users"
    }

    /// The sidebar link, for those who may manage users.
    fn register(&self, app: &mut Registry) {
        app.nav(
            "app",
            NavLink::route("users.index", "nav.users")
                .icon("users")
                .group("nav.admin")
                .order(10)
                .permission(USERS),
        );
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/users", index)
            .name("users.index")
            .put("/users/{id}/roles", update_roles)
            .name("users.roles")
            .post("/users", invite_user)
            .name("users.invite")
            .require_permission(USERS)
            .require_verified()
    }
}

pub fn grid() -> Grid {
    Grid::new("users")
        .title("Users")
        .column(
            Column::text("name", "Name")
                .mobile()
                .searchable()
                .description("email"),
        )
        .column(
            Column::text("email", "Email")
                .hidden()
                .searchable()
                .copyable(),
        )
        .column(Column::custom("roles", "Roles").mobile())
        .column(Column::datetime("email_verified_at", "Verified"))
        .column(Column::date("created_at", "Joined"))
        .column(Column::custom("actions", "Change"))
        .sort_by("-created_at")
        .cards_on_mobile()
        .empty_state(
            "Nobody has signed up yet",
            Some("People show up here once they register."),
        )
}

/// Every user's roles, by user id.
async fn roles_by_user(db: &Db) -> Result<BTreeMap<i64, Vec<String>>> {
    let pairs: Vec<(i64, String)> = renox::db::sql(
        "SELECT role_user.user_id, roles.name FROM role_user \
         JOIN roles ON roles.id = role_user.role_id ORDER BY roles.name",
    )
    .fetch_as(db)
    .await?;
    let mut map: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for (user, role) in pairs {
        map.entry(user).or_default().push(role);
    }
    Ok(map)
}

async fn index(State(db): State<Db>, request: GridRequest) -> Result<View> {
    let roles = roles_by_user(&db).await?;
    let page = grid().page(User::query(), &request).await?;
    // The roles sheets sit after the grid (a grid is a form, and forms
    // don't nest), one per person on the page.
    let members: Vec<_> = page
        .items()
        .iter()
        .map(|user| {
            json!({
                "id": user.id,
                "name": user.name,
                "roles": roles.get(&user.id).cloned().unwrap_or_default(),
            })
        })
        .collect();
    let users =
        page.extend(|user| json!({ "roles": roles.get(&user.id).cloned().unwrap_or_default() }));
    let role_names: Vec<&str> = ROLES.iter().map(|(name, _)| *name).collect();
    // The figures over the grid.
    let people = User::query().count(&db).await?;
    let verified = User::query()
        .where_not_null("email_verified_at")
        .count(&db)
        .await?;
    let admins = roles
        .values()
        .filter(|names| names.iter().any(|name| name == ADMIN))
        .count();
    let totals = json!({ "people": people, "admins": admins, "verified": verified });
    Ok(view(
        "users/index.html",
        context! { users, members, role_names, totals },
    ))
}

/// Ticked boxes: `roles=admin&roles=member` (none ticked: none sent).
#[derive(Deserialize, Serialize, Validate)]
pub struct RolesForm {
    #[serde(default)]
    pub roles: Vec<String>,
}

async fn update_roles(
    State(db): State<Db>,
    admin: AuthUser,
    Path(id): Path<i64>,
    Valid(form): Valid<RolesForm>,
) -> Result<(Toast, HxRefresh)> {
    let user = User::find_or_404(&db, id).await?;
    let chosen: Vec<&str> = form.roles.iter().map(String::as_str).collect();
    if let Some(unknown) = chosen
        .iter()
        .find(|r| !ROLES.iter().any(|(name, _)| name == *r))
    {
        let mut errors = Errors::new();
        errors.add("roles", format!("There is no role called {unknown}."));
        return Err(ValidationError::new(errors).into());
    }
    // Nobody locks themselves out of this page.
    if user.id == admin.id && !chosen.contains(&ADMIN) {
        let mut errors = Errors::new();
        errors.add("roles", "You can't take away your own admin role.");
        return Err(ValidationError::new(errors).into());
    }
    let before = user.roles(&db).await?;
    user.sync_roles(&db, &chosen).await?;
    renox::audit::record(
        &db,
        renox::audit::Entry::new("user.roles_changed")
            .user(admin.id)
            .subject("users", user.id)
            .data(json!({ "from": before, "to": chosen })),
    )
    .await?;
    Ok((
        Toast::success(format!("{}'s roles saved.", user.name)),
        HxRefresh,
    ))
}

/// The form of "Add a user": a name, an address and the roles to give.
#[derive(Deserialize, Serialize, Validate)]
pub struct InviteForm {
    #[validate(required)]
    pub name: String,
    #[validate(required, email)]
    pub email: String,
    #[serde(default)]
    pub roles: Vec<String>,
}

async fn invite_user(
    State(state): State<AppState>,
    admin: AuthUser,
    Valid(form): Valid<InviteForm>,
) -> Result<(Toast, HxRefresh)> {
    let roles: Vec<&str> = form.roles.iter().map(String::as_str).collect();
    let invitation = invite(&state, &form.name, &form.email, &roles).await?;
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new("user.invited")
            .user(admin.id)
            .subject("users", invitation.user.id)
            .data(json!({ "roles": roles, "mailed": invitation.mailed })),
    )
    .await?;
    let message = if invitation.mailed {
        format!(
            "{} is added; a link to choose a password was sent.",
            invitation.user.name
        )
    } else {
        format!(
            "{} is added, but the mail could not be sent. Send them to the sign-in page and \"Forgot your password?\".",
            invitation.user.name
        )
    };
    Ok((Toast::success(message), HxRefresh))
}

/// A user made by an admin.
pub struct Invitation {
    pub user: User,
    /// Where the person chooses their password (valid for an hour, like a
    /// reset link; "Forgot your password?" makes a new one).
    pub link: String,
    /// The mail with the link was sent. A mail that fails does not undo the
    /// user: the person can ask for a link on the sign-in page.
    pub mailed: bool,
}

/// Adds a verified user with a password nobody knows and the `roles` (at
/// least `member`), and mails them the link to choose their own. There is no
/// public sign-up; this is how people get in.
pub async fn invite(
    state: &AppState,
    name: &str,
    email: &str,
    roles: &[&str],
) -> Result<Invitation> {
    let db = &state.db;
    let email = renox::auth::normalize_email(email);
    if name.trim().is_empty() || !email.contains('@') {
        let mut errors = Errors::new();
        errors.add("email", "Give a name and an email address.");
        return Err(ValidationError::new(errors).into());
    }
    if User::find_by_email(db, &email).await?.is_some() {
        let mut errors = Errors::new();
        errors.add("email", "There is a user with this email address already.");
        return Err(ValidationError::new(errors).into());
    }
    super::roles::define(db).await?;
    let mut user = User::register(db, name, &email, &renox::random_token()).await?;
    user.email_verified_at = Some(renox::db::now());
    user.save(db).await?;
    let mut given: Vec<&str> = roles.to_vec();
    if given.is_empty() {
        given.push(super::roles::MEMBER);
    }
    user.sync_roles(db, &given).await?;

    let token = renox::random_token();
    renox::db::sql(
        "INSERT INTO password_reset_tokens (email, token, created_at) VALUES (?, ?, ?) \
         ON CONFLICT (email) DO UPDATE SET token = excluded.token, created_at = excluded.created_at",
    )
    .bind(&user.email)
    .bind(digest(&token))
    .bind(renox::db::now())
    .execute(db)
    .await?;
    let query = user.email.replace('+', "%2B");
    let link = format!(
        "{}?email={query}",
        state.absolute_url("password.reset", &[&token])?
    );
    let mail = state.mail_view(
        &user.email,
        format!("You are invited to {}", state.config.name),
        "mail/invite",
        context! { link, name => user.name, brand => state.config.name },
    );
    let mailed = match mail {
        Ok(mail) => match state.mailer.send(mail).await {
            Ok(()) => true,
            Err(err) => {
                eprintln!(
                    "invitation: the mail to user {} was not sent: {err}",
                    user.id
                );
                false
            }
        },
        Err(err) => {
            eprintln!(
                "invitation: the mail to user {} was not made: {err}",
                user.id
            );
            false
        }
    };
    Ok(Invitation { user, link, mailed })
}

fn digest(token: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
