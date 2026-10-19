//! What the digest of a period covers (wl-kernel §5.13 step 6): the records
//! voided and the late entries made, grouped by the scope of the record, and
//! the managers of each scope.
//!
//! - **Voids** are the Void records whose time is in the period. The scope of
//!   a voided record is the one its `Voidable` entry gives, else the one
//!   [`scopes::scope_of`](crate::scopes::scope_of) finds.
//! - **Late entries** are the recorded ones whose entry time is in the
//!   period (an entry that waited for approval counts from the time it was
//!   typed in); their scope is the one stored on the entry.
//! - **Managers** of a scope are the users who hold `late_entries.approve`
//!   there, or everywhere ([`managers`]).

use std::collections::BTreeMap;

use renox::auth::permissions::{Scope, users_with_permission_in};
use renox::db::{DateTime, Transaction};
use renox::prelude::*;

use super::info::{info_of, rule_title};
use super::render::{FiredLine, Item, ItemKind, Section, record_name};
use crate::late_entries::{LateEntry, LateEntryStatus};
use crate::permissions::LATE_ENTRIES_APPROVE;
use crate::scopes;
use crate::subject::Subject;
use crate::voids::{self, VoidRecord};

/// The items of the period `from` to `until` (the end not included) by the
/// scope of their record, oldest first within a scope.
pub async fn gather(
    db: &Db,
    from: DateTime,
    until: DateTime,
) -> Result<Vec<(Scope, Vec<Item>)>> {
    let mut groups: BTreeMap<(String, String), (Scope, Vec<Item>)> = BTreeMap::new();
    let mut add = |scope: Scope, item: Item| {
        groups
            .entry((scope.kind().to_owned(), scope.id().to_owned()))
            .or_insert_with(|| (scope, Vec::new()))
            .1
            .push(item);
    };

    let void_rows = VoidRecord::query()
        .where_(VoidRecord::AT.gte(from))
        .where_(VoidRecord::AT.lt(until))
        .order_by(VoidRecord::AT)
        .order_by(VoidRecord::ID)
        .get(db)
        .await?;
    for void in void_rows {
        let subject = void.subject();
        let scope = void_scope(db, &subject).await?;
        let mut tx = db.begin().await?;
        let info = info_of(&mut tx, &subject).await?;
        drop(tx);
        add(
            scope,
            Item {
                kind: ItemKind::Void,
                record: subject,
                number: info.number.or(void.number.clone()),
                by: user_name(db, void.voided_by).await?,
                reason: void.reason.clone(),
                at: void.at,
                occurred_at: None,
                fired: Vec::new(),
                confidential: info.confidential,
            },
        );
    }

    let entries = LateEntry::query()
        .where_(LateEntry::STATUS.eq(LateEntryStatus::Recorded))
        .where_(LateEntry::ENTERED_AT.gte(from))
        .where_(LateEntry::ENTERED_AT.lt(until))
        .order_by(LateEntry::ENTERED_AT)
        .order_by(LateEntry::ID)
        .get(db)
        .await?;
    for entry in entries {
        let Some(record) = entry.record() else {
            continue;
        };
        let mut tx = db.begin().await?;
        let info = info_of(&mut tx, &record).await?;
        let fired = fired_lines(&mut tx, &entry).await?;
        drop(tx);
        add(
            entry.scope(),
            Item {
                kind: ItemKind::Late,
                number: info.number,
                by: user_name(db, entry.entered_by).await?,
                reason: entry.reason.clone(),
                at: entry.entered_at,
                occurred_at: Some(entry.occurred_at),
                fired,
                confidential: info.confidential,
                record,
            },
        );
    }
    Ok(groups.into_values().collect())
}

/// The runs a late entry lists for review, named for the digest.
async fn fired_lines(tx: &mut Transaction, entry: &LateEntry) -> Result<Vec<FiredLine>> {
    let mut lines = Vec::new();
    for run in &entry.fired_runs.0 {
        let info = info_of(tx, &run.subject).await?;
        lines.push(FiredLine {
            title: rule_title(&run.rule),
            record: record_name(&run.subject, info.number.as_deref()),
            at: run.at,
        });
    }
    Ok(lines)
}

/// The scope of a voided record: its `Voidable` entry's when it gives one,
/// else its machine's or resolver's ([`scopes::scope_of`]).
async fn void_scope(db: &Db, subject: &Subject) -> Result<Scope> {
    if let Some(voidable) = voids::get(&subject.subject_type)
        && voidable.has_scope()
    {
        let mut tx = db.begin().await?;
        let scope = voidable.scope_in(&mut tx, subject.subject_id).await?;
        drop(tx);
        return Ok(scope);
    }
    scopes::scope_of(db, subject).await
}

/// The name of user `id`, or `#id` when the user is gone.
async fn user_name(db: &Db, id: i64) -> Result<String> {
    Ok(User::find(db, id)
        .await?
        .map_or_else(|| format!("#{id}"), |user| user.name))
}

/// The managers of `scope`: the users who hold `late_entries.approve` there
/// or everywhere, by id.
pub async fn managers(db: &Db, scope: &Scope) -> Result<Vec<i64>> {
    Ok(users_with_permission_in(db, LATE_ENTRIES_APPROVE, scope)
        .await?
        .into_iter()
        .map(|user| user.id)
        .collect())
}

/// What each manager is told: the sections of every scope they manage, in
/// the order of the scopes' names. A manager of no scope in `groups` is not
/// in the map, so a manager with nothing in the period gets no digest.
pub async fn by_manager(
    db: &Db,
    groups: Vec<(Scope, Vec<Item>)>,
) -> Result<BTreeMap<i64, Vec<Section>>> {
    let mut told: BTreeMap<i64, Vec<Section>> = BTreeMap::new();
    for (scope, items) in groups {
        let name = scopes::label_of(db, &scope).await?;
        for user in managers(db, &scope).await? {
            told.entry(user).or_default().push(Section {
                scope: name.clone(),
                items: items.clone(),
            });
        }
    }
    for sections in told.values_mut() {
        sections.sort_by(|a, b| a.scope.cmp(&b.scope));
    }
    Ok(told)
}
