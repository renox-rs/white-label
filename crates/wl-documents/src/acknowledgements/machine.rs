//! The Acknowledgement state machine (wl-documents §4.3, issue #53).
//!
//! ```text
//! [*] request --> pending --acknowledge--> acknowledged
//!                         --replace------> replaced
//!                         --cancel-------> cancelled
//! ```
//!
//! `request` is the creation of a record in `pending`
//! ([`crate::acknowledgements::request`] checks that the reader is required
//! and has no `pending` or `acknowledged` acknowledgement of the version).
//! Every transition is `.system()`: the services in
//! [`crate::acknowledgements`] check who may act (`HRM.ACL.10`: only the
//! reader, or a witness in person) and give the machine what its guards
//! check as the run's input ([`AckRun`]), so both agree.
//!
//! - `acknowledge` needs the version still to be the SOP's current one, the
//!   reader to have opened the version or an accepted translation of it whose
//!   checksum has not changed (or the record to be made in person), and, in
//!   person, a drawn signature and the witness. It stores the evidence.
//! - `replace` and `cancel` store the reason.

use renox::db::{ToDbValue, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use super::model::{AckStatus, METHOD_CLICK, METHOD_DRAWN, SopAcknowledgement};
use crate::texts::english;

/// The reader confirms.
pub const ACKNOWLEDGE: &str = "acknowledge";
/// A newer version was accepted.
pub const REPLACE: &str = "replace";
/// The reader is no longer required.
pub const CANCEL: &str = "cancel";

/// What a run is given: what the guards check and the effects store.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckRun {
    /// The user who acts: the reader, or the witness in person.
    pub by: Option<i64>,
    /// The version is still the SOP's current one.
    pub current: bool,
    /// The reader opened the version or a translation of it (not needed in
    /// person, where the witness shows it).
    pub opened: bool,
    /// The opened file's checksum is unchanged.
    pub checksum_ok: bool,
    /// `click-to-accept` or `drawn`.
    pub method: String,
    /// The file the acknowledgement covers.
    pub document_id: Option<i64>,
    /// Its language.
    pub language: String,
    /// Its checksum.
    pub checksum: String,
    /// The reader's IP address.
    pub ip: Option<String>,
    /// The reader's device.
    pub user_agent: Option<String>,
    /// The drawn signature.
    pub signature: Option<String>,
    /// The witness in person.
    pub witness_id: Option<i64>,
    /// Why, for `replace` and `cancel`.
    pub reason: Option<String>,
}

fn run_of(ctx: &TransitionCtx) -> AckRun {
    ctx.input_as::<AckRun>().unwrap_or_default()
}

type GuardFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<(), String>> + Send>>;

/// A guard that passes when `ok(run)` holds, else says `key`.
fn check(ctx: &TransitionCtx, key: &'static str, ok: fn(&AckRun) -> bool) -> GuardFuture {
    let passes = ok(&run_of(ctx));
    Box::pin(async move {
        if passes {
            Ok(())
        } else {
            Err(english(key, &[]))
        }
    })
}

/// Sets the columns of acknowledgement `id` outside the machine's guard and
/// stamps `updated_at`.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    SopAcknowledgement::query()
        .where_(SopAcknowledgement::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

impl StateMachine for SopAcknowledgement {
    type State = AckStatus;
    const COLUMN: renox::db::Col<SopAcknowledgement, AckStatus> = SopAcknowledgement::STATUS;

    fn machine() -> Machine<Self> {
        use AckStatus::{Acknowledged, Cancelled, Pending, Replaced};
        Machine::new()
            .initial([Pending])
            .state(Pending)
            .tone("warning")
            .state(Acknowledged)
            .tone("success")
            .terminal()
            .state(Replaced)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(ACKNOWLEDGE)
                    .label("Acknowledge")
                    .tone("success")
                    .system()
                    .from([Pending])
                    .to(Acknowledged)
                    .guard("still_current", |_a: &SopAcknowledgement, ctx| {
                        check(ctx, "wl_documents.acks.refused.superseded", |r| r.current)
                    })
                    .guard("opened", |_a: &SopAcknowledgement, ctx| {
                        check(ctx, "wl_documents.acks.refused.not_opened", |r| {
                            r.opened || r.method == METHOD_DRAWN
                        })
                    })
                    .guard("checksum_unchanged", |_a: &SopAcknowledgement, ctx| {
                        check(ctx, "wl_documents.acks.refused.checksum", |r| r.checksum_ok)
                    })
                    .guard("evidence", |_a: &SopAcknowledgement, ctx| {
                        check(ctx, "wl_documents.acks.refused.evidence", |r| {
                            if r.method == METHOD_DRAWN {
                                r.signature.as_deref().is_some_and(|s| !s.trim().is_empty())
                                    && r.witness_id.is_some()
                            } else {
                                r.method == METHOD_CLICK
                            }
                        })
                    })
                    .effect(|ack: &SopAcknowledgement, ctx, tx| {
                        let (id, run) = (ack.id, run_of(ctx));
                        Box::pin(async move {
                            let at = Some(now());
                            set(
                                tx,
                                id,
                                &[
                                    ("acknowledged_at", &at),
                                    ("acknowledged_by", &run.by),
                                    ("ack_document_id", &run.document_id),
                                    ("language", &run.language),
                                    ("checksum", &run.checksum),
                                    ("method", &run.method),
                                    ("ip_address", &run.ip),
                                    ("user_agent", &run.user_agent),
                                    ("signature", &run.signature),
                                    ("witness_id", &run.witness_id),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(ending(REPLACE, "Replace", Replaced))
            .transition(ending(CANCEL, "Cancel", Cancelled))
    }
}

/// `replace` or `cancel`: pending to `to`, storing the reason.
fn ending(name: &'static str, label: &'static str, to: AckStatus) -> Transition<SopAcknowledgement> {
    Transition::new(name)
        .label(label)
        .tone("neutral")
        .system()
        .from([AckStatus::Pending])
        .to(to)
        .effect(|ack: &SopAcknowledgement, ctx, tx| {
            let (id, reason) = (ack.id, run_of(ctx).reason);
            Box::pin(async move { set(tx, id, &[("ended_reason", &reason)]).await })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(run: &AckRun) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(run).unwrap())
    }

    fn own_session() -> AckRun {
        AckRun {
            by: Some(3),
            current: true,
            opened: true,
            checksum_ok: true,
            method: METHOD_CLICK.into(),
            document_id: Some(1),
            language: "en".into(),
            checksum: "abc".into(),
            ..AckRun::default()
        }
    }

    #[test]
    fn the_machine_has_the_three_transitions_of_the_spec() {
        let machine = SopAcknowledgement::machine();
        assert_eq!(machine.initial_states(), ["pending"]);
        let table = [
            (ACKNOWLEDGE, "acknowledged"),
            (REPLACE, "replaced"),
            (CANCEL, "cancelled"),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, to) in table {
            let t = machine.find(name).expect(name);
            assert!(t.is_system(), "{name}");
            assert_eq!(t.from_states(), ["pending"], "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
    }

    #[renox::test]
    async fn acknowledging_needs_the_current_version_opened_with_its_checksum() {
        let machine = SopAcknowledgement::machine();
        let ack = SopAcknowledgement::default();
        let acknowledge = machine.find(ACKNOWLEDGE).unwrap();
        assert!(acknowledge.check_guards(&ack, &ctx(&own_session())).await.is_ok());
        for broken in [
            AckRun { current: false, ..own_session() },
            AckRun { opened: false, ..own_session() },
            AckRun { checksum_ok: false, ..own_session() },
            AckRun { method: String::new(), ..own_session() },
        ] {
            assert!(acknowledge.check_guards(&ack, &ctx(&broken)).await.is_err(), "{broken:?}");
        }
    }

    #[renox::test]
    async fn in_person_needs_the_drawn_signature_and_the_witness_but_not_the_opening() {
        let machine = SopAcknowledgement::machine();
        let ack = SopAcknowledgement::default();
        let acknowledge = machine.find(ACKNOWLEDGE).unwrap();
        let in_person = AckRun {
            opened: false,
            method: METHOD_DRAWN.into(),
            signature: Some("data:image/png;base64,AAAA".into()),
            witness_id: Some(9),
            ..own_session()
        };
        assert!(acknowledge.check_guards(&ack, &ctx(&in_person)).await.is_ok());
        for broken in [
            AckRun { signature: None, ..in_person.clone() },
            AckRun { signature: Some("  ".into()), ..in_person.clone() },
            AckRun { witness_id: None, ..in_person.clone() },
        ] {
            assert!(acknowledge.check_guards(&ack, &ctx(&broken)).await.is_err(), "{broken:?}");
        }
    }
}
