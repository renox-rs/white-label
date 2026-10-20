//! What a CDD `exit` does to the client's engagements (04 §4.6 step 5, §7.1
//! *CDD file* `exit`, `reject`; BL-ACC-09).
//!
//! The MLRO exits a client, or rejects the file of an existing client at a
//! review (a client with an `active` or `suspended` engagement). After the
//! commit [`after`] gives the partner **one** task, due in five working days
//! ([`DUE_WORKING_DAYS`]), "End engagement — relationship ended", listing
//! every `active` or `suspended` engagement of the client and, under each,
//! the open returns the manager must cancel with *engagement ended* before
//! the end can run (the End checklist, BL-ACC-06). The task never says why
//! the client is exited: the reason stays on the CDD file, for the MLRO and
//! deputy only (`ACC.ACL.14`).
//!
//! Nothing else happens by itself: the engagements run on until the partner
//! ends them, no message goes to the client (the MLRO's neutral template
//! does, when they decide), and no AML case is opened or closed. No new work
//! is accepted: a new engagement for the client is refused with *On hold*
//! ([`refuse_new_work`]).

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::calendar::calendar;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{TicketRef, task_creator};

use crate::cdd::CddFile;
use crate::db::refused;
use crate::engagement::{Engagement, EngagementStatus};
use crate::permissions::PARTNER_ROLE;
use crate::returns;
use crate::texts::english;

/// The working days the partner has to end the engagements.
pub const DUE_WORKING_DAYS: u32 = 5;

/// The source key of the partner's task for the CDD file `file`.
pub fn partner_key(file: &CddFile) -> String {
    format!("accounting.cdd_exit:{}:{}", file.client_party_id, file.id)
}

/// The engagements of `client` an exit asks the partner to end: `active` and
/// `suspended`.
///
/// # Errors
///
/// The database's errors.
pub async fn to_end(db: &Db, client: PartyId) -> Result<Vec<Engagement>> {
    Ok(Engagement::of_client(db, client)
        .await?
        .into_iter()
        .filter(|e| {
            matches!(
                e.status,
                EngagementStatus::Active | EngagementStatus::Suspended
            )
        })
        .collect())
}

/// After the CDD file `file` was exited, or rejected at the review of an
/// existing client: the partner's task to end the engagements. Answers the
/// task, `None` when the client has no engagement to end or ticketing is
/// off. Idempotent: a task still open under the same key is returned.
///
/// # Errors
///
/// The database's and the task service's errors.
pub async fn after(state: &AppState, file: &CddFile) -> Result<Option<TicketRef>> {
    let engagements = to_end(&state.db, file.client()).await?;
    if engagements.is_empty() {
        return Ok(None);
    }
    let key = partner_key(file);
    if let Some(open) = task_creator(state).find_open(state, &key).await? {
        return Ok(Some(open));
    }
    let mut lines = Vec::new();
    for engagement in &engagements {
        lines.push(english(
            "accounting.exits.engagement_line",
            &[
                ("number", &engagement.number()),
                ("title", &engagement.title),
                (
                    "status",
                    &english(
                        &format!(
                            "accounting.engagements.status.{}",
                            engagement.status.as_str()
                        ),
                        &[],
                    ),
                ),
                (
                    "link",
                    &format!("/accounting/engagements/{}", engagement.id),
                ),
            ],
        ));
        for ret in returns::open_of(&state.db, engagement.id).await? {
            lines.push(english(
                "accounting.exits.return_line",
                &[("number", &ret.number()), ("period", &ret.period_end)],
            ));
        }
    }
    let title = english("accounting.exits.partner_title", &[]);
    let description = english(
        "accounting.exits.partner_description",
        &[("lines", &lines.join("\n"))],
    );
    let due = calendar(state).add_working_days(renox::db::now(), DUE_WORKING_DAYS);
    let task = RoleTask::new(file.subject(), PARTNER_ROLE, &title, &key)
        .description(&description)
        .due_at(due);
    role_tasks::open(state, task).await
}

/// Engagement `id` ended: when it was the last `active` or `suspended`
/// engagement of a client whose relationship ended through CDD, the
/// partner's task is completed.
///
/// # Errors
///
/// The database's and the task service's errors.
pub async fn on_ended(state: &AppState, id: i64) -> Result {
    let engagement = Engagement::find_or_404(&state.db, id).await?;
    let client = engagement.client();
    let Some(file) = CddFile::current(&state.db, client).await? else {
        return Ok(());
    };
    if !file.ended_relationship() || !to_end(&state.db, client).await?.is_empty() {
        return Ok(());
    }
    let creator = task_creator(state);
    if let Some(open) = creator.find_open(state, &partner_key(&file)).await? {
        let note = english("accounting.exits.task_closed", &[]);
        creator.complete(state, open.id, &note).await?;
    }
    Ok(())
}

/// A new engagement for `client` is refused with *On hold* once its CDD
/// relationship ended: the current file is `exited`, or `rejected` (at a
/// review of an existing client, or at onboarding). The text is neutral: it
/// does not say why.
///
/// # Errors
///
/// A 422 for such a client; the database's errors.
pub async fn refuse_new_work(db: &Db, client: PartyId) -> Result {
    let ended = CddFile::current(db, client)
        .await?
        .is_some_and(|file| file.ended_relationship());
    if ended {
        return Err(refused(english("accounting.exits.on_hold", &[])));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_names_the_client_and_the_file() {
        let file = CddFile {
            id: 4,
            client_party_id: 9,
            ..CddFile::default()
        };
        assert_eq!(partner_key(&file), "accounting.cdd_exit:9:4");
    }

    #[test]
    fn the_task_says_nothing_of_the_reason() {
        let text = format!(
            "{} {}",
            english("accounting.exits.partner_title", &[]),
            english("accounting.exits.partner_description", &[("lines", &"")])
        )
        .to_lowercase();
        for word in ["aml", "sanction", "screening", "cdd", "exit"] {
            assert!(!text.contains(word), "`{word}` in {text}");
        }
        assert_eq!(
            english("accounting.exits.partner_title", &[]),
            "End engagement — relationship ended"
        );
    }
}
