//! A party directory for the logistics tests that need the real wl-payments:
//! the clients of the tests are known parties, and one can be blocked.

#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::notify::{ContactKind, PartyContactPoint, PartyContacts};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyKind, PartyStatus, PartySummary};

/// The primary e-mail on record of the clients.
pub const KNOWN_EMAIL: &str = "accounts@souq.example";

/// The directory: the clients 501 and 502 exist; the blocked ones answer
/// `Blocked`.
#[derive(Clone, Default)]
pub struct Parties {
    blocked: Arc<Mutex<HashSet<i64>>>,
}

impl Parties {
    /// Blocks `party`.
    pub fn block(&self, party: PartyId) {
        self.blocked.lock().unwrap().insert(party.0);
    }

    /// Unblocks `party`.
    pub fn unblock(&self, party: PartyId) {
        self.blocked.lock().unwrap().remove(&party.0);
    }

    fn summary(&self, party: PartyId) -> Option<PartySummary> {
        if !(501..=502).contains(&party.0) {
            return None;
        }
        let blocked = self.blocked.lock().unwrap().contains(&party.0);
        Some(PartySummary {
            id: party,
            code: format!("P-{:05}", party.0),
            kind: PartyKind::Company,
            display_name: format!("Party {}", party.0),
            status: if blocked {
                PartyStatus::Blocked
            } else {
                PartyStatus::Active
            },
            display_block_reason: None,
        })
    }
}

impl PartyDirectory for Parties {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move { Ok(self.summary(party)) })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        _code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async move { Ok(self.summary(party).map(|p| p.display_name)) })
    }

    fn contacts<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<PartyContacts>> {
        let known = self.summary(party).is_some();
        Box::pin(async move {
            Ok(known.then(|| PartyContacts {
                party,
                language: "en".to_owned(),
                preferred_channel: None,
                points: vec![PartyContactPoint {
                    id: party.0,
                    kind: ContactKind::Email,
                    address: KNOWN_EMAIL.to_owned(),
                    primary: true,
                    verified: true,
                    valid: true,
                }],
            }))
        })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async move { Ok(self.summary(party).map(|p| p.status)) })
    }
}
