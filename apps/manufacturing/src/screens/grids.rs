//! What the list screens share now that they are `renox::grid`s (#523, #556):
//! the baseline every list carries, the export guard and the badge tones.
//!
//! The rows a person may see are worked out the way the pages always did
//! (the plant and line scopes of each record, `may_see`), and the grid pages
//! `id IN (those)`: search, sort, filters and pages act on top of the scope,
//! never instead of it.

use renox::grid::{Grid, GridRequest};
use renox::prelude::*;
use serde::Serialize;

/// The baseline of a list: 25 rows a page, the person's search, filters,
/// sort and page size remembered, the advanced filter and cards on a phone.
pub(crate) fn baseline(grid: Grid) -> Grid {
    grid.per_page(25)
        .remember()
        .advanced_filter()
        .cards_on_mobile()
}

/// `grid` with its export menu when `can_export`.
pub(crate) fn exporting(grid: Grid, can_export: bool) -> Grid {
    if can_export { grid.exports() } else { grid }
}

/// The file of the export the request asks for (`?export=csv`), if any. A
/// person who may not export gets 403: [`Grid::export`] answers any request
/// that names a format, whether the menu is shown or not. Custom columns
/// (names asked of other modules, actions) are never exported.
///
/// # Errors
///
/// 403 for a person who may not export; the database's errors.
pub(crate) async fn export_file<M: Model + Serialize>(
    grid: &Grid,
    can_export: bool,
    query: renox::db::Query<M>,
    request: &GridRequest,
) -> Result<Option<Response>> {
    if request.param("export").is_none() {
        return Ok(None);
    }
    if !can_export {
        return Err(Error::Forbidden);
    }
    grid.export(query, request).await
}

/// The tone of a grid badge for the kit's badge kind of a screen (`error`
/// reads `danger` in a grid).
pub(crate) fn tone(kind: &'static str) -> &'static str {
    if kind == "error" { "danger" } else { kind }
}
