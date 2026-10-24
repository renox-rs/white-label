//! `Subject`: a typed reference to any record ([ARCHITECTURE.md] §5, rule 1).
//!
//! Documents, tickets, invoices, notes and approval requests attach to
//! "something" through two columns, `subject_type` and `subject_id`, so a
//! module never needs a foreign key into another module's tables.
//!
//! [ARCHITECTURE.md]: https://github.com/renox-rs/white-label/blob/main/docs/ARCHITECTURE.md

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A record of some module: `property.lease` 42.
///
/// The type is `<module>.<entity>`, lower case with `_` inside a part.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Subject {
    /// `<module>.<entity>`, e.g. `property.lease`.
    pub subject_type: String,
    /// The record's id.
    pub subject_id: i64,
}

/// Why a text is not a subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidSubject(pub String);

impl fmt::Display for InvalidSubject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not a subject like property.lease:42", self.0)
    }
}

impl std::error::Error for InvalidSubject {}

impl Subject {
    /// A reference to record `id` of `subject_type`.
    ///
    /// # Panics
    ///
    /// When `subject_type` is not `<module>.<entity>`: types are constants
    /// in code, so a bad one is a programming error.
    pub fn new(subject_type: &str, id: i64) -> Self {
        assert!(
            is_type(subject_type),
            "`{subject_type}` is not a subject type like property.lease"
        );
        Subject {
            subject_type: subject_type.to_owned(),
            subject_id: id,
        }
    }

    /// The module part: `property` of `property.lease`.
    pub fn module(&self) -> &str {
        self.subject_type.split('.').next().unwrap_or_default()
    }

    /// The entity part: `lease` of `property.lease`.
    pub fn entity(&self) -> &str {
        self.subject_type.split('.').nth(1).unwrap_or_default()
    }

    /// The subject as a page shows it when the module's own label is not at
    /// hand: `accounting.engagement:7` as "Engagement #7".
    pub fn words(&self) -> String {
        words_of(&self.subject_type, self.subject_id)
    }
}

/// [`Subject::words`] of a type and id kept in two columns: the entity in
/// words, capitalised, and its number (`property.lease_renewal`, 3: "Lease
/// renewal #3").
pub fn words_of(subject_type: &str, id: i64) -> String {
    let entity = subject_type.rsplit('.').next().unwrap_or(subject_type);
    let words = entity.replace('_', " ");
    let mut chars = words.chars();
    let words: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    format!("{words} #{id}")
}

/// [`words_of`] as SQL, for a grid column over a type and an id kept in two
/// columns (`type_column`, `id_column` of the grid's table `{T}`): each type
/// of `types` reads as its entity and number ("Engagement #12"), so the
/// column shows, sorts, searches and exports as words and never as
/// `accounting.engagement #12`. A type missing from `types` (a row saved
/// while the page was built), or one with characters a type never has, shows
/// its number alone, `#12`. A bare type (`ticket`) reads as `Ticket #12`. Read `types` from the table first (`pluck`).
pub fn words_sql(type_column: &str, id_column: &str, types: &[String]) -> String {
    let id = format!("CAST({{T}}.\"{id_column}\" AS TEXT)");
    let cases: String = types
        .iter()
        // A type is lower-case words and dots (`property.lease`, or a
        // bare `ticket` some records keep): nothing that could end the SQL
        // string.
        .filter(|t| {
            t.starts_with(|c: char| c.is_ascii_lowercase())
                && t.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
        })
        .map(|t| {
            let words = words_of(t, 0);
            let prefix = words.strip_suffix('0').unwrap_or(&words);
            format!(" WHEN '{t}' THEN '{}' || {id}", prefix.replace('\'', "''"))
        })
        .collect();
    if cases.is_empty() {
        format!("'#' || {id}")
    } else {
        format!("CASE {{T}}.\"{type_column}\"{cases} ELSE '#' || {id} END")
    }
}

/// `property.lease:42`, the form used in occurrence keys and logs.
impl fmt::Display for Subject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.subject_type, self.subject_id)
    }
}

impl FromStr for Subject {
    type Err = InvalidSubject;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidSubject(text.to_owned());
        let (subject_type, id) = text.split_once(':').ok_or_else(invalid)?;
        let id: i64 = id.parse().map_err(|_| invalid())?;
        if !is_type(subject_type) || id <= 0 {
            return Err(invalid());
        }
        Ok(Subject::new(subject_type, id))
    }
}

fn is_type(text: &str) -> bool {
    let mut parts = text.split('.');
    let part = |p: Option<&str>| {
        p.is_some_and(|p| {
            p.starts_with(|c: char| c.is_ascii_lowercase())
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
    };
    part(parts.next()) && part(parts.next()) && parts.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subject_in_words_is_its_entity_and_number() {
        assert_eq!(
            Subject::new("accounting.engagement", 7).words(),
            "Engagement #7"
        );
        assert_eq!(words_of("property.lease_renewal", 3), "Lease renewal #3");
    }

    #[test]
    fn the_words_as_sql_name_each_known_type_and_skip_a_bad_one() {
        let sql = words_sql(
            "subject_type",
            "subject_id",
            &["accounting.engagement".to_owned(), "x'; drop".to_owned()],
        );
        assert_eq!(
            sql,
            "CASE {T}.\"subject_type\" WHEN 'accounting.engagement' THEN 'Engagement #' || \
             CAST({T}.\"subject_id\" AS TEXT) ELSE '#' || CAST({T}.\"subject_id\" AS TEXT) END"
        );
        assert_eq!(
            words_sql("subject_type", "subject_id", &[]),
            "'#' || CAST({T}.\"subject_id\" AS TEXT)"
        );
    }

    #[test]
    fn it_round_trips_through_text() {
        let lease = Subject::new("property.lease", 42);
        assert_eq!(lease.to_string(), "property.lease:42");
        assert_eq!("property.lease:42".parse::<Subject>().unwrap(), lease);
        assert_eq!(lease.module(), "property");
        assert_eq!(lease.entity(), "lease");
    }

    #[test]
    fn malformed_text_is_refused() {
        for bad in [
            "lease:1",
            "property.lease",
            "property.lease:x",
            "Property.lease:1",
            "a.b.c:1",
            "a.b:0",
        ] {
            assert!(bad.parse::<Subject>().is_err(), "{bad}");
        }
        assert!("manpower.work_permit:7".parse::<Subject>().is_ok());
    }

    #[test]
    #[should_panic(expected = "not a subject type")]
    fn a_bad_type_in_code_panics() {
        let _ = Subject::new("lease", 1);
    }
}
