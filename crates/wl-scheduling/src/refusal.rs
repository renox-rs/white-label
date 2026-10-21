//! Why a booking service did not do what was asked.

use renox::db::DbError;
use renox::prelude::*;
use renox::state_machine::TransitionError;

use crate::slots::{FreeSlot, Unfit};
use crate::texts::{english, t_with};

/// Why a booking service refused.
#[derive(Debug)]
#[non_exhaustive]
pub enum Refusal {
    /// Every seat of the slot is taken (`SCH.BKG.01`): the nearest free slots,
    /// or the waiting list.
    Taken {
        /// Up to three free slots of the same service type.
        nearest: Vec<FreeSlot>,
        /// Whether the booker may join the waiting list.
        waiting_list: bool,
    },
    /// A resource is closed or blocked then.
    Unavailable(Unfit),
    /// A resource may not deliver the service then (`SCH.ACL.04`); the text
    /// names why, for staff only.
    Ineligible(String),
    /// The request is wrong: a text in English that is safe to show.
    Invalid(String),
    /// The machine refused the transition.
    Transition(TransitionError),
    /// Anything else (the database, an effect).
    Failed(Error),
}

impl Refusal {
    /// A refusal with the English text of `key`.
    pub(crate) fn invalid(key: &str) -> Self {
        Refusal::Invalid(english(key, &[]))
    }

    /// The text to show in `lang`.
    pub fn message(&self, lang: &Lang) -> String {
        match self {
            Refusal::Taken {
                nearest,
                waiting_list,
            } => {
                let mut text = t_with(lang, "wl_scheduling.problem.taken", &[]);
                if !nearest.is_empty() {
                    let list = nearest
                        .iter()
                        .map(|slot| wl_kernel::documents::dubai_text(slot.start))
                        .collect::<Vec<_>>()
                        .join(", ");
                    text.push(' ');
                    text.push_str(&t_with(
                        lang,
                        "wl_scheduling.problem.nearest",
                        &[("slots", &list)],
                    ));
                }
                if *waiting_list {
                    text.push(' ');
                    text.push_str(&t_with(lang, "wl_scheduling.problem.waiting_list", &[]));
                }
                text
            }
            Refusal::Unavailable(Unfit::Closed(name)) => {
                t_with(lang, "wl_scheduling.problem.closed", &[("name", name)])
            }
            Refusal::Unavailable(Unfit::Blocked(name)) => {
                t_with(lang, "wl_scheduling.problem.blocked", &[("name", name)])
            }
            Refusal::Unavailable(Unfit::Taken) => t_with(lang, "wl_scheduling.problem.taken", &[]),
            Refusal::Ineligible(text) | Refusal::Invalid(text) => text.clone(),
            Refusal::Transition(error) => error.to_string(),
            Refusal::Failed(error) => error.to_string(),
        }
    }

    /// The error a page answers with: 409 for a slot just taken, 422 for a
    /// refusal, the transition's own status for the machine's.
    pub fn into_error(self, lang: &Lang) -> Error {
        match self {
            Refusal::Taken { .. } => Error::Status(StatusCode::CONFLICT, self.message(lang)),
            Refusal::Unavailable(_) | Refusal::Ineligible(_) | Refusal::Invalid(_) => {
                Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message(lang))
            }
            Refusal::Transition(error) => error.into_error(),
            Refusal::Failed(error) => error,
        }
    }

    /// Whether the slot was just taken.
    pub fn is_taken(&self) -> bool {
        matches!(self, Refusal::Taken { .. })
    }
}

impl From<TransitionError> for Refusal {
    fn from(error: TransitionError) -> Self {
        Refusal::Transition(error)
    }
}

impl From<Error> for Refusal {
    fn from(error: Error) -> Self {
        Refusal::Failed(error)
    }
}

impl From<DbError> for Refusal {
    fn from(error: DbError) -> Self {
        Refusal::Failed(error.into())
    }
}
