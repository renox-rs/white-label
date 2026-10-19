//! Who is told about an expiring document (wl-documents §5.2 step 3, #48 A8).
//!
//! The responsible staff are the holders of the kind's responsible role. The
//! scope they are looked for in follows this order:
//!
//! 1. the subject's scope (its machine, through [`wl_kernel::scopes`]): every
//!    holder there, and in the global scope, gets every item;
//! 2. else the scopes of the affected records: each holder gets only the
//!    items in the scopes where they hold the role (a global holder gets
//!    them all); a record without a scope is seen by global holders only;
//! 3. else the scopes the kind's [`ReminderSchedule`] returns;
//! 4. else every holder of the role in any scope, plus global holders.
//!
//! Users are de-duplicated and ordered by id. A kind without a responsible
//! role falls back to the company administrators.

use std::collections::{BTreeMap, BTreeSet};

use renox::auth::permissions::Scope;
use renox::db::{now, sql};
use renox::prelude::*;
use wl_kernel::documents::{AffectedGroup, reminder_schedule};
use wl_kernel::{COMPANY_ADMIN_ROLE, Subject, scopes};

/// One user to tell and the affected items they may see.
#[derive(Debug, Clone, PartialEq)]
pub struct Recipient {
    /// The user's id.
    pub user_id: i64,
    /// The items this user may see, grouped by guarded transition in the
    /// order given. Empty for a reminder that lists no item.
    pub groups: Vec<AffectedGroup>,
}

/// The role whose holders are told about documents of a kind with
/// `responsible_role`: that role, else the company administrators.
pub fn role_for(responsible_role: Option<&str>) -> &str {
    responsible_role.unwrap_or(COMPANY_ADMIN_ROLE)
}

/// The role to remind about `document` of `kind`: for an SOP version the
/// SOP's owner role (#52 A11), else [`role_for`] the kind's.
///
/// # Errors
///
/// The database's errors.
pub async fn role_of(
    db: &Db,
    kind: &crate::DocumentKind,
    document: &crate::Document,
) -> Result<String> {
    if kind.key == crate::sops::KIND_SOP
        && document.subject_type == crate::sops::SUBJECT_TYPE
        && let Some(sop) = crate::Sop::find(db, document.subject_id).await?
    {
        return Ok(sop.owner_role);
    }
    Ok(role_for(kind.responsible_role.as_deref()).to_owned())
}

/// The recipients for the document of kind `kind` on `subject`, with the
/// `items` (the new affected records; empty for a plain reminder) each one
/// may see.
pub async fn resolve(
    state: &AppState,
    role: &str,
    kind: &str,
    subject: &Subject,
    items: &[AffectedGroup],
) -> Result<Vec<Recipient>> {
    let db = &state.db;
    // 1. The subject's scope.
    let scope = scopes::scope_of(db, subject).await?;
    if !scope.is_global() {
        return Ok(everyone(holders_in(db, role, &scope).await?, items));
    }
    // 2. The scopes of the affected records.
    if items
        .iter()
        .flat_map(|group| &group.records)
        .any(|record| record.scope.as_ref().is_some_and(|s| !s.is_global()))
    {
        return by_record_scope(db, role, items).await;
    }
    // 3. The kind's reminder schedule.
    if let Some(schedule) = reminder_schedule(kind)
        && let Some(named) = schedule.scopes(state, subject).await?
    {
        let mut users = BTreeSet::new();
        for scope in &named {
            users.extend(holders_in(db, role, scope).await?);
        }
        return Ok(everyone(users, items));
    }
    // 4. Every scope.
    Ok(everyone(holders_anywhere(db, role).await?, items))
}

pub(super) fn everyone(users: BTreeSet<i64>, items: &[AffectedGroup]) -> Vec<Recipient> {
    users
        .into_iter()
        .map(|user_id| Recipient {
            user_id,
            groups: items.to_vec(),
        })
        .collect()
}

/// Splits `items` per holder of `role` in each record's own scope.
async fn by_record_scope(db: &Db, role: &str, items: &[AffectedGroup]) -> Result<Vec<Recipient>> {
    let mut cache: BTreeMap<(String, String), BTreeSet<i64>> = BTreeMap::new();
    let mut seen: BTreeMap<i64, Vec<AffectedGroup>> = BTreeMap::new();
    for group in items {
        for record in &group.records {
            let scope = record.scope.clone().unwrap_or_default();
            let key = (scope.kind().to_owned(), scope.id().to_owned());
            if !cache.contains_key(&key) {
                cache.insert(key.clone(), holders_in(db, role, &scope).await?);
            }
            for user in &cache[&key] {
                let groups = seen.entry(*user).or_default();
                match groups.iter_mut().find(|g| g.guarded == group.guarded) {
                    Some(existing) => existing.records.push(record.clone()),
                    None => groups.push(AffectedGroup {
                        guarded: group.guarded.clone(),
                        records: vec![record.clone()],
                    }),
                }
            }
        }
    }
    Ok(seen
        .into_iter()
        .map(|(user_id, groups)| Recipient { user_id, groups })
        .collect())
}

/// The users holding `role` in `scope` or globally, now.
pub(crate) async fn holders_in(db: &Db, role: &str, scope: &Scope) -> Result<BTreeSet<i64>> {
    Ok(
        renox::auth::permissions::users_with_role_in(db, role, scope)
            .await?
            .into_iter()
            .map(|user| user.id)
            .collect(),
    )
}

/// The users holding `role` in any scope, now (assignment dates respected).
async fn holders_anywhere(db: &Db, role: &str) -> Result<BTreeSet<i64>> {
    let at = now();
    let ids: Vec<i64> = sql("SELECT DISTINCT ru.user_id FROM role_user ru \
         JOIN roles r ON r.id = ru.role_id WHERE r.name = ? \
         AND (ru.starts_at IS NULL OR ru.starts_at <= ?) \
         AND (ru.ends_at IS NULL OR ru.ends_at > ?) \
         ORDER BY ru.user_id")
    .bind(role)
    .bind(at)
    .bind(at)
    .scalars(db)
    .await?;
    Ok(ids.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use renox::auth::permissions::define_role;
    use renox::testing::TestApp;
    use wl_kernel::documents::{
        DocumentFuture, PlannedRecord, ReminderSchedule, register_reminder_schedule,
    };

    use super::*;
    use crate::WlDocuments;

    async fn boot() -> TestApp {
        let app = TestApp::new(
            App::new()
                .module(renox::auth::Auth::new())
                .module(renox::auth::Permissions)
                .module(WlDocuments),
        )
        .await;
        define_role(app.db(), "coordinator", &[]).await.unwrap();
        app
    }

    async fn staff(app: &TestApp, name: &str, scope: Option<&Scope>) -> i64 {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(app.db(), name, &email, "password123")
            .await
            .unwrap();
        match scope {
            Some(scope) => user.assign_role_in(app.db(), "coordinator", scope).await,
            None => user.assign_role(app.db(), "coordinator").await,
        }
        .unwrap();
        user.id
    }

    fn building(id: i64) -> Scope {
        Scope::new("buildings", id)
    }

    fn visit(id: i64, scope: Option<Scope>) -> PlannedRecord {
        PlannedRecord {
            record: Subject::new("test.visit", id),
            date: "2026-12-20".parse().unwrap(),
            label: format!("Visit {id}"),
            scope,
        }
    }

    fn group(records: Vec<PlannedRecord>) -> AffectedGroup {
        AffectedGroup {
            guarded: "ticket.resolve".into(),
            records,
        }
    }

    fn ids(recipients: &[Recipient]) -> Vec<i64> {
        recipients.iter().map(|r| r.user_id).collect()
    }

    #[test]
    fn a_kind_without_a_role_goes_to_the_administrators() {
        assert_eq!(role_for(Some("pro")), "pro");
        assert_eq!(role_for(None), COMPANY_ADMIN_ROLE);
    }

    #[renox::test]
    async fn the_subjects_scope_wins_and_everyone_there_gets_every_item() {
        let app = boot().await;
        let a = staff(&app, "Ann", Some(&building(1))).await;
        let _b = staff(&app, "Bob", Some(&building(2))).await;
        let g = staff(&app, "Gus", None).await;
        scopes::register("res_test.subject", |_db, _id| {
            Box::pin(async { Ok(Scope::new("buildings", 1)) })
        });
        let items = [group(vec![visit(1, Some(building(2)))])];
        let got = resolve(
            app.state(),
            "coordinator",
            "res_kind_a",
            &Subject::new("res_test.subject", 1),
            &items,
        )
        .await
        .unwrap();
        assert_eq!(ids(&got), vec![a, g]);
        assert!(got.iter().all(|r| r.groups == items));
    }

    #[renox::test]
    async fn record_scopes_split_the_items_per_holder() {
        let app = boot().await;
        let a = staff(&app, "Ann", Some(&building(1))).await;
        let b = staff(&app, "Bob", Some(&building(2))).await;
        let g = staff(&app, "Gus", None).await;
        let v1 = visit(1, Some(building(1)));
        let v2 = visit(2, Some(building(2)));
        let v3 = visit(3, Some(building(1)));
        let got = resolve(
            app.state(),
            "coordinator",
            "res_kind_b",
            &Subject::new("res_test.none", 1),
            &[group(vec![v1.clone(), v2.clone(), v3.clone()])],
        )
        .await
        .unwrap();
        assert_eq!(ids(&got), vec![a, b, g]);
        assert_eq!(got[0].groups, vec![group(vec![v1.clone(), v3.clone()])]);
        assert_eq!(got[1].groups, vec![group(vec![v2.clone()])]);
        assert_eq!(got[2].groups, vec![group(vec![v1, v2, v3])]);
    }

    struct Plant1;

    impl ReminderSchedule for Plant1 {
        fn scopes<'a>(
            &'a self,
            _state: &'a AppState,
            _subject: &'a Subject,
        ) -> DocumentFuture<'a, Option<Vec<Scope>>> {
            Box::pin(async { Ok(Some(vec![Scope::new("plants", 1)])) })
        }
    }

    struct NoAnswer;

    impl ReminderSchedule for NoAnswer {
        fn scopes<'a>(
            &'a self,
            _state: &'a AppState,
            _subject: &'a Subject,
        ) -> DocumentFuture<'a, Option<Vec<Scope>>> {
            Box::pin(async { Ok(None) })
        }
    }

    #[renox::test]
    async fn the_reminder_schedule_names_the_scopes_else_every_scope() {
        let app = boot().await;
        let p1 = staff(&app, "Pia", Some(&Scope::new("plants", 1))).await;
        let p2 = staff(&app, "Pat", Some(&Scope::new("plants", 2))).await;
        let subject = Subject::new("res_test.none", 2);
        register_reminder_schedule("res_kind_c", Plant1);
        let got = resolve(app.state(), "coordinator", "res_kind_c", &subject, &[])
            .await
            .unwrap();
        assert_eq!(ids(&got), vec![p1]);
        assert!(got[0].groups.is_empty());
        register_reminder_schedule("res_kind_d", NoAnswer);
        for kind in ["res_kind_d", "res_kind_unregistered"] {
            let got = resolve(app.state(), "coordinator", kind, &subject, &[])
                .await
                .unwrap();
            assert_eq!(ids(&got), vec![p1, p2]);
        }
    }

    #[renox::test]
    async fn scope_less_records_fall_through_to_every_scope() {
        let app = boot().await;
        let a = staff(&app, "Ann", Some(&building(1))).await;
        let g = staff(&app, "Gus", None).await;
        let items = [group(vec![visit(1, None)])];
        let got = resolve(
            app.state(),
            "coordinator",
            "res_kind_e",
            &Subject::new("res_test.none", 3),
            &items,
        )
        .await
        .unwrap();
        assert_eq!(ids(&got), vec![a, g]);
    }
}
