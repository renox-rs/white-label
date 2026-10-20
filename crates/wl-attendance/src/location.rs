//! Work locations (wl-attendance §3): a place where attendance can be
//! recorded. Only what the checks need is here: the name, the scope (the
//! `Subject` of the project, site or branch), the outdoor flag and the
//! qualifications the context requires. Geofences and the required check-in
//! method come with the check-in issues.

use renox::db::{DateTime, Json};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::attendance::WORK_LOCATION_SUBJECT_TYPE;

/// A place where attendance is recorded.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_attendance_work_locations", index(scope_type, scope_id))]
pub struct WorkLocation {
    /// The primary key.
    pub id: i64,
    /// The name, e.g. `Site 1`.
    pub name: String,
    /// The type of the scope's `Subject`.
    pub scope_type: Option<String>,
    /// The id of the scope's `Subject`.
    pub scope_id: Option<i64>,
    /// Whether work is outdoors: the midday break applies.
    pub outdoor: bool,
    /// The qualification kinds the context requires (`HRM.RUL.14`).
    pub required_qualifications: Json<Vec<String>>,
    /// The role the context needs, for role-bound qualification kinds.
    pub qualification_role: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl WorkLocation {
    /// The location as a [`Subject`]: `attendance.work_location:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(WORK_LOCATION_SUBJECT_TYPE, self.id)
    }

    /// The scope (project, site, branch) the location is in, if any.
    pub fn scope(&self) -> Option<Subject> {
        match (&self.scope_type, self.scope_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// A location called `name`.
    pub fn named(name: &str) -> Self {
        WorkLocation {
            name: name.trim().to_owned(),
            ..WorkLocation::default()
        }
    }

    /// In `scope`.
    pub fn in_scope(mut self, scope: &Subject) -> Self {
        self.scope_type = Some(scope.subject_type.clone());
        self.scope_id = Some(scope.subject_id);
        self
    }

    /// Outdoors.
    pub fn outdoors(mut self) -> Self {
        self.outdoor = true;
        self
    }

    /// The context needs a qualification of `kind` (`HRM.RUL.14`).
    pub fn requiring(mut self, kind: &str) -> Self {
        let mut kinds = self.required_qualifications.to_vec();
        kinds.push(kind.trim().to_owned());
        self.required_qualifications = Json(kinds);
        self
    }

    /// The role the context needs, for role-bound kinds.
    pub fn for_role(mut self, role: &str) -> Self {
        self.qualification_role = Some(role.trim().to_lowercase());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_location_knows_its_scope_and_what_the_context_requires() {
        let site = Subject::new("construction.project", 4);
        let location = WorkLocation::named(" Site 1 ")
            .in_scope(&site)
            .outdoors()
            .requiring("sira_licence")
            .for_role("Guard");
        assert_eq!(location.name, "Site 1");
        assert_eq!(location.scope(), Some(site));
        assert!(location.outdoor);
        assert_eq!(location.required_qualifications.to_vec(), ["sira_licence"]);
        assert_eq!(location.qualification_role.as_deref(), Some("guard"));
        assert_eq!(WorkLocation::named("Depot").scope(), None);
    }
}
