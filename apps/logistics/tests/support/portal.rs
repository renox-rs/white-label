//! A portal for the logistics tests: the app does not depend on `wl-portal`,
//! so the tests provide the kernel's `PortalUsers` contract from a table of
//! the users who act for a client company.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::portal::{PortalFuture, PortalUser, PortalUsers};

/// Which company each client user acts for.
#[derive(Clone, Default)]
pub struct Portal(Arc<Mutex<BTreeMap<i64, i64>>>);

impl Portal {
    /// `user_id` acts for the client company `party`.
    pub fn grant(&self, user_id: i64, party: i64) {
        self.0.lock().unwrap().insert(user_id, party);
    }
}

impl PortalUsers for Portal {
    fn acting_for<'a>(
        &'a self,
        _db: &'a Db,
        user: i64,
        party: PartyId,
        _on: NaiveDate,
    ) -> PortalFuture<'a, Option<PortalUser>> {
        let found = self.0.lock().unwrap().get(&user).copied();
        Box::pin(async move {
            Ok(found.filter(|acts| *acts == party.0).map(|acts| PortalUser {
                user_id: user,
                person: PartyId(1000 + user),
                acting_for: PartyId(acts),
                roles: vec!["owner".to_owned()],
            }))
        })
    }
}
