//! The idempotent installer of the default onboarding template (wl-hr §7,
//! issue #216): the *all staff* induction. [`install`] stores it where no
//! all-staff template exists yet and keeps what the client changed.

use renox::prelude::*;

use crate::template::{self, OnboardingTemplate};

/// What [`install`] created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Templates created.
    pub templates: usize,
}

impl Installed {
    /// The number of rows of every kind created.
    pub fn total(&self) -> usize {
        self.templates
    }
}

/// Stores the *all staff* induction template when the database has no
/// all-staff template.
pub async fn install(db: &Db) -> Result<Installed> {
    let existing = OnboardingTemplate::query()
        .where_(OnboardingTemplate::ALL_STAFF.eq(true))
        .count(db)
        .await?;
    if existing > 0 {
        return Ok(Installed::default());
    }
    match template::create(db, template::induction()).await? {
        Ok(_) => Ok(Installed { templates: 1 }),
        Err(refused) => Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            refused.message(),
        )),
    }
}
