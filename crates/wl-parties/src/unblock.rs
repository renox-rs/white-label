//! Unblocking a party (wl-parties §4 `unblock`): `blocked` → `active`.
//!
//! The transition needs `parties.block` and a reason. Its effect, in the
//! transition's transaction, closes every open Block record of the party
//! (unblocked by the actor, now, with the note and the basis) and clears the
//! party's `hold_reason`. The checks that need the database (separation of
//! duties against every "blocked by" user, `parties.block_reason.view` for a
//! restricted record, the sanctions basis and files) belong to the service
//! that runs the transition, as a guard has no database. In the same
//! transaction the party's open review tasks (#36) get the note "Party
//! unblocked" and go back to their decider role, still open.

use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::{NEUTRAL_BLOCK_REASON, PartyId, PartyStatus, PartyUnblocked};

use crate::block_record::BlockRecord;
use crate::evidence::{UnblockBasis, UnblockEvidence};
use crate::party::Party;
use crate::permissions;
use crate::review_tasks::return_to_deciders;
use crate::texts;

/// The transition.
pub const UNBLOCK: &str = "unblock";

/// What an unblock run carries to its guard and effect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnblockInput {
    /// Why the block is lifted; stored on each closed record.
    pub note: String,
    /// The statement behind lifting a `sanctions` block.
    pub basis: Option<UnblockBasis>,
}

/// The `unblock` transition, for the machine.
pub(crate) fn transition() -> Transition<Party> {
    Transition::new(UNBLOCK)
        .label("Unblock")
        .tone("success")
        .from([PartyStatus::Blocked])
        .to(PartyStatus::Active)
        .permission(permissions::BLOCK)
        .reason_required()
        .guard("note_given", |_party: &Party, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if !input.note.trim().is_empty() => Ok(()),
                    _ => Err(texts::english("wl_parties.errors.note_required")),
                }
            })
        })
        .effect(|party: &Party, ctx, tx| {
            let party_id = party.id;
            Box::pin(async move {
                let input = input(ctx).unwrap_or_default();
                let user = ctx.actor().map(|user| user.id);
                for mut record in BlockRecord::open_for(&mut *tx, party_id).await? {
                    record.close(user, &input.note, input.basis);
                    record.save(&mut *tx).await?;
                }
                // Not the status column, so outside the machine's guard.
                renox::db::sql(format!(
                    "UPDATE {} SET hold_reason = NULL, updated_at = ? WHERE id = ?",
                    Party::TABLE
                ))
                .bind(renox::db::now())
                .bind(party_id)
                .execute(&mut *tx)
                .await?;
                Ok(())
            })
        })
}

/// The run's input, if it is an [`UnblockInput`].
fn input(ctx: &TransitionCtx) -> Option<UnblockInput> {
    ctx.input_as::<UnblockInput>().ok()
}

/// The code of the block that needs evidence to be lifted.
const SANCTIONS: &str = "sanctions";
/// The largest evidence file (20 MB).
const MAX_FILE_BYTES: usize = 20 * 1024 * 1024;
/// The file types evidence may have.
const FILE_TYPES: [&str; 7] = ["pdf", "jpg", "jpeg", "png", "heic", "docx", "xlsx"];

/// What a user gives to lift a block.
#[derive(Clone, Default)]
pub struct UnblockRequest {
    /// Why the block is lifted; required.
    pub note: String,
    /// The statement behind lifting a `sanctions` block.
    pub basis: Option<UnblockBasis>,
    /// The evidence files; at least one for a `sanctions` block.
    pub files: Vec<Upload>,
}

/// Why a request is refused, as the text key of its message. `None` when
/// the request may go on. Two refusals share one neutral message, so a user
/// without `parties.block_reason.view` cannot tell a restricted hold from a
/// separation-of-duties conflict (no tipping off).
fn refusal(
    records: &[BlockRecord],
    actor: i64,
    can_view_restricted: bool,
    request: &UnblockRequest,
) -> Option<&'static str> {
    if request.note.trim().is_empty() {
        return Some("errors.note_required");
    }
    let by_actor = records
        .iter()
        .any(|record| record.blocked_by == Some(actor));
    let hidden = !can_view_restricted && records.iter().any(|record| record.restricted);
    if by_actor || hidden {
        return Some("errors.unblock_other_user");
    }
    if records.iter().any(|record| record.reason_code == SANCTIONS) {
        if request.basis.is_none() {
            return Some("errors.unblock_basis_required");
        }
        if request.files.is_empty() {
            return Some("errors.unblock_evidence_required");
        }
    }
    request.files.iter().find_map(file_refusal)
}

/// Why `file` cannot be kept as evidence: empty, too large or of a type
/// that is not allowed.
fn file_refusal(file: &Upload) -> Option<&'static str> {
    if file.size() == 0 {
        Some("errors.unblock_file_empty")
    } else if file.size() > MAX_FILE_BYTES {
        Some("errors.unblock_file_large")
    } else if !file
        .extension()
        .is_some_and(|extension| FILE_TYPES.contains(&extension.as_str()))
    {
        Some("errors.unblock_file_type")
    } else {
        None
    }
}

/// The records the evidence files belong to: the `sanctions` ones, else the
/// oldest open one.
fn evidence_targets(records: &[BlockRecord]) -> Vec<&BlockRecord> {
    let sanctions: Vec<&BlockRecord> = records
        .iter()
        .filter(|record| record.reason_code == SANCTIONS)
        .collect();
    if sanctions.is_empty() {
        records.first().into_iter().collect()
    } else {
        sanctions
    }
}

/// The reason of the history row: the note, or the neutral `on_hold` when
/// any record is restricted (the note then lives only on the records).
fn history_reason(records: &[BlockRecord], note: &str) -> String {
    if records.iter().any(|record| record.restricted) {
        NEUTRAL_BLOCK_REASON.to_owned()
    } else {
        note.trim().to_owned()
    }
}

/// The display reason `PartyUnblocked` carries: `on_hold` when any closed
/// record was restricted, else the party's hold reason before the unblock.
fn event_reason(records: &[BlockRecord], hold_reason: Option<&str>) -> String {
    match hold_reason.map(str::trim) {
        Some(code) if !code.is_empty() && records.iter().all(|record| !record.restricted) => {
            code.to_owned()
        }
        _ => NEUTRAL_BLOCK_REASON.to_owned(),
    }
}

/// A refusal as the error the caller shows (422).
fn refused(key: &str) -> TransitionError {
    TransitionError::Guard(texts::english(&format!("wl_parties.{key}")))
}

/// `user` lifts the block of party `party` (`unblock`). Needs
/// `parties.block`. In one transaction it locks the party, loads the open
/// Block records, checks the note, separation of duties (the user blocked
/// none of them), `parties.block_reason.view` for a restricted one, and for
/// `sanctions` the basis and a file, then runs the transition (which closes
/// the records and clears the hold) and stores the evidence privately.
/// After the commit it emits `PartyUnblocked` with the display reason only.
///
/// # Errors
///
/// `Forbidden` without the permission, `NotFrom` when the party is not
/// `blocked`, `Guard` (422) for a blank note, a conflict of duties or a
/// missing permission (one neutral message), a missing basis or file, or a
/// file that is empty, too large or of a type that is not allowed. Nothing
/// is written then.
pub async fn unblock(
    state: &AppState,
    user: &AuthUser,
    party: i64,
    request: UnblockRequest,
) -> std::result::Result<(), TransitionError> {
    if !user.has_permission(permissions::BLOCK) {
        return Err(TransitionError::Forbidden);
    }
    // The write lock on SQLite, the row lock on PostgreSQL.
    let mut tx = state.db.begin_immediate().await?;
    let party = Party::query()
        .where_eq("id", party)
        .lock_for_update()
        .first(&mut tx)
        .await?
        .ok_or(renox::Error::NotFound)?;
    let records = BlockRecord::open_for(&mut tx, party.id).await?;
    let can_view = user.has_permission(permissions::BLOCK_REASON_VIEW);
    if let Some(key) = refusal(&records, user.id, can_view, &request) {
        return Err(refused(key));
    }
    let input = UnblockInput {
        note: request.note.trim().to_owned(),
        basis: request.basis,
    };
    let outcome = party
        .transition(UNBLOCK)
        .by(user)
        .reason(history_reason(&records, &request.note))
        .input(&input)
        .run_in(&mut tx)
        .await?;
    if outcome.is_pending() {
        tx.commit().await?;
        return Ok(());
    }
    // Open review tasks go back to their deciders, in the same transaction.
    return_to_deciders(&mut tx, party.id).await?;
    for record in evidence_targets(&records) {
        let dir = format!("parties/{}/unblock/{}", party.id, record.id);
        for file in &request.files {
            let key = file.store(&state.storage, &dir).await?;
            let mime = file.sniffed_type().map_or_else(
                || file.content_type().trim().to_owned(),
                std::borrow::ToOwned::to_owned,
            );
            let size = i64::try_from(file.size()).unwrap_or(i64::MAX);
            let mut row = UnblockEvidence::new(
                record.id,
                &key,
                file.file_name().trim(),
                &mime,
                size,
                Some(user.id),
            );
            row.insert(&mut tx).await?;
        }
    }
    tx.commit().await?;
    outcome.emit(state).await?;
    state
        .emit(PartyUnblocked {
            party: PartyId(party.id),
            display_reason: event_reason(&records, party.hold_reason.as_deref()),
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::{StateMachine, TransitionError};

    #[test]
    fn unblock_goes_from_blocked_to_active_for_a_user() {
        let machine = Party::machine();
        let transition = machine.find(UNBLOCK).expect("transition");
        assert_eq!(transition.from_states(), ["blocked"]);
        assert_eq!(transition.to_state(), Some("active"));
        assert_eq!(transition.guard_names(), ["note_given"]);
        assert!(!transition.is_system());
    }

    fn record(code: &str, restricted: bool, by: Option<i64>) -> BlockRecord {
        BlockRecord::open(7, code, restricted, "note", by)
    }

    fn request(note: &str) -> UnblockRequest {
        UnblockRequest {
            note: note.into(),
            ..UnblockRequest::default()
        }
    }

    fn pdf() -> Upload {
        Upload::new("match.pdf", "application/pdf", b"%PDF-1.7 x".to_vec())
    }

    #[test]
    fn a_blank_note_is_refused() {
        let records = [record("non_payment", false, Some(1))];
        for note in ["", "  "] {
            assert_eq!(
                refusal(&records, 2, false, &request(note)),
                Some("errors.note_required")
            );
        }
        assert_eq!(refusal(&records, 2, false, &request("paid in full")), None);
    }

    #[test]
    fn whoever_blocked_cannot_unblock_and_a_system_block_has_no_limit() {
        let by_a = [record("non_payment", false, Some(1))];
        assert_eq!(
            refusal(&by_a, 1, false, &request("paid")),
            Some("errors.unblock_other_user")
        );
        assert_eq!(refusal(&by_a, 2, false, &request("paid")), None);
        let by_system = [record("non_payment", false, None)];
        assert_eq!(refusal(&by_system, 1, false, &request("paid")), None);
    }

    #[test]
    fn a_restricted_hold_without_the_permission_looks_like_a_conflict() {
        let records = [record("aml_concern", true, Some(3))];
        let neutral = refusal(&records, 2, false, &request("cleared"));
        assert_eq!(neutral, Some("errors.unblock_other_user"));
        assert_eq!(neutral, refusal(&records, 3, true, &request("cleared")));
        assert_eq!(refusal(&records, 4, true, &request("cleared")), None);
        let text = texts::english("wl_parties.errors.unblock_other_user");
        assert!(!text.contains("sanctions") && !text.contains("aml"));
    }

    #[test]
    fn every_open_record_counts() {
        let records = [
            record("non_payment", false, Some(1)),
            record("aml_concern", true, Some(3)),
        ];
        for (user, can_view) in [(1, false), (3, true), (2, false)] {
            assert_eq!(
                refusal(&records, user, can_view, &request("ok")),
                Some("errors.unblock_other_user")
            );
        }
        assert_eq!(refusal(&records, 4, true, &request("ok")), None);
    }

    #[test]
    fn a_sanctions_hold_needs_a_basis_and_a_file() {
        let records = [record("sanctions", true, Some(3))];
        let mut asked = request("match found false");
        assert_eq!(
            refusal(&records, 4, true, &asked),
            Some("errors.unblock_basis_required")
        );
        asked.basis = Some(UnblockBasis::FalseMatch);
        assert_eq!(
            refusal(&records, 4, true, &asked),
            Some("errors.unblock_evidence_required")
        );
        asked.files.push(pdf());
        assert_eq!(refusal(&records, 4, true, &asked), None);
    }

    #[test]
    fn evidence_files_have_a_size_and_a_type() {
        assert_eq!(file_refusal(&pdf()), None);
        let upload = |name: &str, bytes: usize| Upload::new(name, "x/y", vec![0_u8; bytes]);
        assert_eq!(file_refusal(&upload("a.XLSX", 5)), None);
        assert_eq!(
            file_refusal(&upload("a.pdf", 0)),
            Some("errors.unblock_file_empty")
        );
        assert_eq!(
            file_refusal(&upload("a.pdf", MAX_FILE_BYTES + 1)),
            Some("errors.unblock_file_large")
        );
        assert_eq!(file_refusal(&upload("a.pdf", MAX_FILE_BYTES)), None);
        for name in ["a.exe", "a", "a.html"] {
            assert_eq!(
                file_refusal(&upload(name, 5)),
                Some("errors.unblock_file_type")
            );
        }
        let mut asked = request("ok");
        asked.files.push(upload("a.exe", 5));
        let records = [record("non_payment", false, Some(1))];
        assert_eq!(
            refusal(&records, 2, false, &asked),
            Some("errors.unblock_file_type")
        );
    }

    #[test]
    fn the_history_and_the_event_never_leak_a_restricted_hold() {
        let open = [record("non_payment", false, Some(1))];
        assert_eq!(history_reason(&open, " paid in full "), "paid in full");
        assert_eq!(event_reason(&open, Some("non_payment")), "non_payment");
        assert_eq!(event_reason(&open, None), "on_hold");
        let mixed = [
            record("non_payment", false, Some(1)),
            record("sanctions", true, Some(3)),
        ];
        assert_eq!(history_reason(&mixed, "match found false"), "on_hold");
        assert_eq!(event_reason(&mixed, Some("non_payment")), "on_hold");
    }

    #[test]
    fn evidence_goes_to_the_sanctions_records_else_the_oldest() {
        let mut first = record("non_payment", false, Some(1));
        first.id = 1;
        let mut second = record("sanctions", true, Some(3));
        second.id = 2;
        let records = [first, second];
        let ids = |targets: Vec<&BlockRecord>| targets.iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids(evidence_targets(&records)), [2]);
        assert_eq!(ids(evidence_targets(&records[..1])), [1]);
        assert!(evidence_targets(&[]).is_empty());
    }

    #[renox::test]
    async fn the_guard_refuses_a_blank_note() {
        let machine = Party::machine();
        let unblock = machine.find(UNBLOCK).expect("transition");
        let party = Party::default();
        let run = |note: &str| {
            let input = UnblockInput {
                note: note.into(),
                basis: None,
            };
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
        };
        assert!(
            unblock
                .check_guards(&party, &run("paid in full"))
                .await
                .is_ok()
        );
        for blank in ["", "   "] {
            assert!(matches!(
                unblock.check_guards(&party, &run(blank)).await,
                Err(TransitionError::Guard(_))
            ));
        }
    }
}
