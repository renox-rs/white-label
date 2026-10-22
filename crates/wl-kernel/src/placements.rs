//! Where a worker is placed (the employee page, #511). A vertical that places
//! workers at client sites (09 Manpower: deployments) provides
//! [`Placements`]; `wl-hr` lists them on the employee page without depending
//! on the vertical. Without one [`placements`] gives [`NoPlacements`] and the
//! page has no *Deployments* tab.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::workforce::EmployeeId;

/// One placement of a worker, as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    /// The page of the placement.
    pub url: String,
    /// The client's name.
    pub client: String,
    /// The site's name.
    pub site: String,
    /// The role or trade in words.
    pub role: String,
    /// The first day.
    pub from: NaiveDate,
    /// The last day; `None` while open-ended.
    pub to: Option<NaiveDate>,
    /// The status in words, in the page's language.
    pub status: String,
    /// Whether the worker is on site now (`active`, or suspended).
    pub on_site: bool,
}

/// The future a [`Placements`] method returns.
pub type PlacementsFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The placements of a worker. The vertical provides it.
pub trait Placements: Send + Sync + 'static {
    /// The placements of `employee` that `user` may see, newest first, with
    /// the words in `locale`. `None` when `user` may see none of them (the
    /// page then has no tab); `Some(empty)` for a worker on the bench. The
    /// default is `None`.
    fn of_employee<'a>(
        &'a self,
        _state: &'a AppState,
        _user: &'a AuthUser,
        _employee: EmployeeId,
        _locale: &'a str,
    ) -> PlacementsFuture<'a, Option<Vec<Placement>>> {
        Box::pin(async { Ok(None) })
    }
}

/// The placements when no vertical provides them: none are shown.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPlacements;

impl Placements for NoPlacements {}

/// Makes `placements` the app's [`Placements`]; the vertical calls it in
/// `register`.
pub fn provide_placements(app: &mut Registry, placements: impl Placements) {
    let placements: Arc<dyn Placements> = Arc::new(placements);
    app.provide(placements);
}

/// The app's [`Placements`], or [`NoPlacements`] when none is provided.
pub fn placements(state: &AppState) -> Arc<dyn Placements> {
    match state.provided::<Arc<dyn Placements>>() {
        Some(found) => Arc::clone(&found),
        None => Arc::new(NoPlacements),
    }
}
