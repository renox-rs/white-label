//! A block guard for the ticketing tests: the module does not depend on
//! `wl-parties`, so the tests provide a `BlockGuard` with the set of blocked
//! vendors the test fills. For a viewer it would show the block reason, so a
//! test sees that the module asks as the system and shows the neutral text
//! only.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::block_guard::{
    ActionKind, BlockFuture, BlockGuard, BlockReasonLine, BlockRefusal, GuardOutcome,
};

/// The reason a holder of `parties.block_reason.view` would see.
pub const REAL_REASON: &str = "fraud_suspected";

/// The blocked vendors.
#[derive(Clone, Default)]
pub struct Blocks(Arc<Mutex<BTreeSet<i64>>>);

impl Blocks {
    /// Blocks `party`.
    pub fn block(&self, party: PartyId) {
        self.0.lock().unwrap().insert(party.0);
    }

    /// Lifts the block of `party`.
    pub fn unblock(&self, party: PartyId) {
        self.0.lock().unwrap().remove(&party.0);
    }
}

impl BlockGuard for Blocks {
    fn check_in<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        viewer: Option<&'a User>,
        party: PartyId,
        kind: ActionKind,
    ) -> BlockFuture<'a, GuardOutcome> {
        let blocked = kind.is_guarded() && self.0.lock().unwrap().contains(&party.0);
        let detailed = viewer.is_some();
        Box::pin(async move {
            Ok(if !blocked {
                GuardOutcome::Allowed
            } else if detailed {
                GuardOutcome::Blocked(BlockRefusal::detailed(
                    party,
                    kind,
                    vec![BlockReasonLine {
                        label: REAL_REASON.to_owned(),
                        link: format!("/parties/{}/blocks/1", party.0),
                    }],
                    "Compliance",
                ))
            } else {
                GuardOutcome::Blocked(BlockRefusal::neutral(party, kind, "Compliance"))
            })
        })
    }
}
