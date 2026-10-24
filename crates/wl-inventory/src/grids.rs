//! What the register screens share to be `renox::grid` grids: the part of a
//! query the user may see by scope, and the badge colours a status maps to.

use renox::db::Query;
use renox::prelude::{AuthUser, Db, Error, IntoResponse, Model, Response, Result};
use wl_kernel::scopes::{Reach, reach};

/// Keeps the rows of `query` whose scope (`type_col`, `id_col`) the user
/// reaches with any of `permissions`: all of them for a global holder, the
/// rows of the user's scopes for a scoped one, none otherwise. This is the
/// filter the screens used to apply row by row after loading everything; in
/// the query, a page of the grid holds only what the user may see, and the
/// count and the paging agree with it.
pub(crate) async fn within_reach<M: Model>(
    db: &Db,
    user: &AuthUser,
    permissions: &[&str],
    query: Query<M>,
    type_col: &str,
    id_col: &str,
) -> Result<Query<M>> {
    let mut scopes = Vec::new();
    for permission in permissions {
        match reach(db, user, permission).await {
            Ok(found) if found.is_all() => return Ok(query),
            Ok(found) => scopes.extend(found.scopes().iter().cloned()),
            Err(Error::Forbidden) => {}
            Err(other) => return Err(other),
        }
    }
    Ok(narrow(query, &Reach::only(scopes), type_col, id_col))
}

/// `query` limited to the scopes of `reach`.
fn narrow<M: Model>(query: Query<M>, reach: &Reach, type_col: &str, id_col: &str) -> Query<M> {
    let mut pairs: Vec<(String, i64)> = Vec::new();
    for scope in reach.scopes() {
        if let Ok(id) = scope.id().parse::<i64>() {
            let pair = (scope.kind().to_owned(), id);
            if !pairs.contains(&pair) {
                pairs.push(pair);
            }
        }
    }
    if pairs.is_empty() {
        return query.none();
    }
    query.where_any(|mut any| {
        for (kind, id) in pairs {
            any = any.where_all(|all| all.where_eq(type_col, kind).where_eq(id_col, id));
        }
        any
    })
}

/// `value` as a JSON answer.
pub(crate) fn json<T: serde::Serialize>(value: &T) -> Result<Response> {
    let body = renox::serde_json::to_string(value)
        .map_err(|error| Error::Internal(renox::anyhow::Error::new(error)))?;
    Ok(([("content-type", "application/json")], body).into_response())
}
