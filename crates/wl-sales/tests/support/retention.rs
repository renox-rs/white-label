//! What the segment and retention tests need: the party directory and the
//! consents of the other modules answered by hand, a notifier that records
//! broadcasts (the real one is in `retention_real.rs`), and sales written as
//! fixtures.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::broadcast::{BroadcastId, BroadcastReport, BroadcastRequest};
use wl_kernel::consents::{ConsentFuture, Consents};
use wl_kernel::notify::{
    ContactKind, MessageId, Notifier, NotifyChannel, NotifyFuture, NotifyRequest,
    PartyContactPoint, PartyContacts,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::portal::{PortalFuture, PortalUser, PortalUsers};
use wl_kernel::{PartyId, Subject};
use wl_sales::sale_line::KIND_ITEM;
use wl_sales::{CounterSale, CounterSaleStatus, SaleLine};

use super::Booted;

/// The parties: every party is active until a test sets its status.
#[derive(Clone, Default)]
pub struct Directory {
    status: Arc<Mutex<HashMap<i64, PartyStatus>>>,
}

impl Directory {
    /// Sets the state of `party`.
    pub fn set(&self, party: i64, status: PartyStatus) {
        self.status.lock().unwrap().insert(party, status);
    }

    fn status_of(&self, party: PartyId) -> PartyStatus {
        self.status
            .lock()
            .unwrap()
            .get(&party.0)
            .copied()
            .unwrap_or(PartyStatus::Active)
    }
}

impl PartyDirectory for Directory {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            Ok(Some(PartySummary {
                id: party,
                code: format!("P-{:05}", party.0),
                kind: Default::default(),
                display_name: format!("Party {}", party.0),
                status: self.status_of(party),
                display_block_reason: None,
            }))
        })
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
        Box::pin(async move { Ok(Some(format!("Party {}", party.0))) })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async move { Ok(Some(self.status_of(party))) })
    }

    fn contacts<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<PartyContacts>> {
        Box::pin(async move {
            Ok(Some(PartyContacts {
                party,
                language: "en".into(),
                preferred_channel: None,
                points: vec![PartyContactPoint {
                    id: party.0,
                    kind: ContactKind::Mobile,
                    address: format!("+9715{:08}", party.0),
                    primary: true,
                    verified: true,
                    valid: true,
                }],
            }))
        })
    }
}

/// Marketing consent: the parties that gave it, or everybody.
#[derive(Clone, Default)]
pub struct Consent {
    given: Arc<Mutex<HashSet<i64>>>,
    everyone: Arc<AtomicBool>,
}

impl Consent {
    /// `party` gave marketing consent on every channel.
    pub fn give(&self, party: i64) {
        self.given.lock().unwrap().insert(party);
    }

    /// Everybody has consented.
    pub fn everyone(&self) {
        self.everyone.store(true, Ordering::SeqCst);
    }
}

impl Consents for Consent {
    fn has_consent<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        purpose: &'a str,
        _channel: NotifyChannel,
    ) -> ConsentFuture<'a, bool> {
        let yes = purpose == "marketing"
            && (self.everyone.load(Ordering::SeqCst) || self.given.lock().unwrap().contains(&party.0));
        Box::pin(async move { Ok(yes) })
    }

    fn withdraw<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _purpose: &'a str,
        _channel: NotifyChannel,
        _source: &'a str,
    ) -> ConsentFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}

/// Who acts for which party in the portal.
#[derive(Clone, Default)]
pub struct Portal {
    grants: Arc<Mutex<Vec<(i64, i64)>>>,
}

impl Portal {
    /// The user `user` acts for the party `party`.
    pub fn grant(&self, user: i64, party: i64) {
        self.grants.lock().unwrap().push((user, party));
    }
}

impl PortalUsers for Portal {
    fn acting_for<'a>(
        &'a self,
        _db: &'a Db,
        user: i64,
        party: PartyId,
        _on: renox::chrono::NaiveDate,
    ) -> PortalFuture<'a, Option<PortalUser>> {
        let found = self
            .grants
            .lock()
            .unwrap()
            .iter()
            .any(|(u, p)| *u == user && *p == party.0);
        Box::pin(async move {
            Ok(found.then(|| PortalUser {
                user_id: user,
                person: party,
                acting_for: party,
                roles: vec!["owner".to_owned()],
            }))
        })
    }
}

/// The directory, the consents and the portal grants as a module.
#[derive(Clone, Default)]
pub struct Parties {
    /// The parties.
    pub directory: Directory,
    /// The consents.
    pub consent: Consent,
    /// The portal grants.
    pub portal: Portal,
    /// The `PointsTransferred` events a merge emitted.
    pub transferred: Arc<Mutex<Vec<wl_kernel::sales::PointsTransferred>>>,
}

impl Module for Parties {
    fn name(&self) -> &'static str {
        "parties_fake"
    }

    fn register(&self, app: &mut Registry) {
        wl_kernel::parties::provide_directory(app, self.directory.clone());
        wl_kernel::consents::provide_consents(app, self.consent.clone());
        wl_kernel::portal::provide_portal_users(app, self.portal.clone());
        let log = self.transferred.clone();
        app.listen(
            move |event: wl_kernel::sales::PointsTransferred, _state: AppState| {
                let log = log.clone();
                async move {
                    log.lock().unwrap().push(event);
                    Ok(())
                }
            },
        );
    }
}

/// A notifier that records the broadcasts asked for and answers with the
/// reports a test sets.
#[derive(Clone, Default)]
pub struct Gate {
    /// The broadcasts asked for.
    pub requests: Arc<Mutex<Vec<BroadcastRequest>>>,
    /// The broadcasts cancelled, with the reason given.
    pub cancelled: Arc<Mutex<Vec<(i64, String)>>>,
    reports: Arc<Mutex<HashMap<i64, BroadcastReport>>>,
    refusal: Arc<Mutex<Option<String>>>,
    serial: Arc<AtomicI64>,
    /// The notices sent through `notify`.
    pub notices: Arc<Mutex<Vec<NotifyRequest>>>,
}

impl Gate {
    /// Makes the next broadcasts fail with `message` (or work again).
    pub fn refuse(&self, message: Option<&str>) {
        *self.refusal.lock().unwrap() = message.map(str::to_owned);
    }

    /// Sets what the report of broadcast `id` says.
    pub fn report(&self, id: i64, report: BroadcastReport) {
        self.reports.lock().unwrap().insert(id, report);
    }

    /// The requests so far.
    pub fn asked(&self) -> Vec<BroadcastRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Notifier for Gate {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.notices.lock().unwrap().push(request);
            Ok(vec![MessageId(1)])
        })
    }

    fn broadcast<'a>(
        &'a self,
        _state: &'a AppState,
        request: BroadcastRequest,
    ) -> NotifyFuture<'a, BroadcastId> {
        Box::pin(async move {
            if let Some(message) = self.refusal.lock().unwrap().clone() {
                return Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message));
            }
            self.requests.lock().unwrap().push(request);
            Ok(BroadcastId(self.serial.fetch_add(1, Ordering::SeqCst) + 1))
        })
    }

    fn cancel_broadcast<'a>(
        &'a self,
        _state: &'a AppState,
        id: BroadcastId,
        reason: &'a str,
    ) -> NotifyFuture<'a, bool> {
        Box::pin(async move {
            self.cancelled.lock().unwrap().push((id.0, reason.to_owned()));
            Ok(true)
        })
    }

    fn broadcast_report<'a>(
        &'a self,
        _state: &'a AppState,
        id: BroadcastId,
    ) -> NotifyFuture<'a, Option<BroadcastReport>> {
        Box::pin(async move { Ok(self.reports.lock().unwrap().get(&id.0).cloned()) })
    }
}

/// The fakes together, as one module.
#[derive(Clone, Default)]
pub struct Stage {
    /// The parties and consents.
    pub parties: Parties,
    /// The notifier.
    pub gate: Gate,
}

impl Module for Stage {
    fn name(&self) -> &'static str {
        "retention_stage"
    }

    fn register(&self, app: &mut Registry) {
        self.parties.register(app);
        wl_kernel::notify::provide_notifier(app, self.gate.clone());
    }
}

/// The number of the next fixture sale.
static SERIAL: AtomicI64 = AtomicI64::new(1);

/// Writes a completed counter sale of `party` at `at` worth `net` fils before
/// VAT, with one line of `item`.
pub async fn sale_at(t: &Booted, party: i64, at: DateTime, net: i64, item: &str) -> CounterSale {
    let n = SERIAL.fetch_add(1, Ordering::SeqCst);
    let mut sale = CounterSale {
        number: format!("FIX-{n:05}"),
        party_id: Some(party),
        cashier_user_id: 1,
        currency: "AED".into(),
        net_total: net,
        vat_total: net / 20,
        gross_total: net + net / 20,
        completed_at: Some(at),
        ..CounterSale::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    sale.insert_in_state(&mut tx, CounterSaleStatus::Completed, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    SaleLine {
        subject_type: sale.subject().subject_type,
        subject_id: sale.id,
        line_no: 1,
        item_code: item.to_owned(),
        description: item.to_owned(),
        kind: KIND_ITEM.to_owned(),
        quantity_milli: 1000,
        unit_price: net,
        ..SaleLine::default()
    }
    .insert(t.db())
    .await
    .unwrap();
    sale
}

/// The subject of campaign `id`.
pub fn campaign_subject(id: i64) -> Subject {
    Subject::new("sales.campaign", id)
}
