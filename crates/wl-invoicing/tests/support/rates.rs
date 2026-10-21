//! What the rate card tests (BL-MPW-07, #421) share: the portal grants of the
//! client company, the timesheets that billing or payroll locked, and a
//! contract with its card.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::rate_card::{RateCard, RateCardVersion, Unit};
use wl_invoicing::rate_cards::{self, LineInput};
use wl_kernel::attendance::LabourFuture;
use wl_kernel::portal::{PortalFuture, PortalUser, PortalUsers};
use wl_kernel::timesheets::Timesheets;
use wl_kernel::{PartyId, Subject};

use super::{BLUE_DUNES, Booted};

/// The portal grants: who acts for which company, with which roles.
#[derive(Clone, Default)]
pub struct Portal(Arc<Mutex<Vec<PortalUser>>>);

impl Portal {
    /// `user` acts for `party` with the portal `roles`.
    pub fn grant(&self, user: &User, party: PartyId, roles: &[&str]) {
        self.0.lock().unwrap().push(PortalUser {
            user_id: user.id,
            person: PartyId(9_000 + user.id),
            acting_for: party,
            roles: roles.iter().map(|r| (*r).to_owned()).collect(),
        });
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
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|grant| grant.user_id == user && grant.acting_for == party)
                .cloned())
        })
    }
}

/// The timesheets that are `locked`.
#[derive(Clone, Default)]
pub struct Sheets(Arc<Mutex<BTreeSet<i64>>>);

impl Sheets {
    /// Timesheet `id` is `locked`.
    pub fn lock(&self, id: i64) {
        self.0.lock().unwrap().insert(id);
    }
}

impl Timesheets for Sheets {
    fn is_locked<'a>(
        &'a self,
        _state: &'a AppState,
        timesheet: &'a Subject,
    ) -> LabourFuture<'a, bool> {
        let locked = timesheet.subject_type == "attendance.timesheet"
            && self.0.lock().unwrap().contains(&timesheet.subject_id);
        Box::pin(async move { Ok(locked) })
    }
}

/// A day, from `YYYY-MM-DD`.
pub fn d(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// The contract the cards price.
pub fn contract() -> Subject {
    Subject::new("manpower.client_contract", 4)
}

/// A timesheet.
pub fn sheet(id: i64) -> Subject {
    Subject::new("attendance.timesheet", id)
}

/// The guard line of AED `rate` an hour on the day shift.
pub fn guard(rate: i64) -> LineInput {
    LineInput::new("guard", "day", Unit::Hour, rate * 100)
}

/// The card of Blue Dunes for [`contract`].
pub async fn card(t: &Booted) -> RateCard {
    rate_cards::create_card(t.db(), None, BLUE_DUNES, &contract())
        .await
        .unwrap()
}

/// A version of `card` effective from `from` with `line`, made today.
pub async fn version(t: &Booted, card: &RateCard, from: &str, line: LineInput) -> RateCardVersion {
    let version = rate_cards::new_version(t.db(), None, card.id, d(from), false)
        .await
        .unwrap();
    rate_cards::add_line(t.db(), version.id, line)
        .await
        .unwrap();
    version
}

/// Runs `work` on `day` (Asia/Dubai, midday): the clock moves forward from the
/// real one.
pub async fn on<F: std::future::Future>(t: &Booted, day: &str, work: F) -> F::Output {
    t.travel_to(day, 8, 0);
    t.app.at_travelled_time(work).await
}

/// `POST /rate-card-versions/{id}/accept` as `by`, with `fields`.
pub async fn accept_as(
    t: &Booted,
    version: &RateCardVersion,
    by: &User,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    t.post_as(
        &format!("/rate-card-versions/{}/accept", version.id),
        by,
        fields,
    )
    .await
}

/// The version as it is stored now.
pub async fn reload(t: &Booted, version: &RateCardVersion) -> RateCardVersion {
    RateCardVersion::find_or_404(t.db(), version.id)
        .await
        .unwrap()
}

/// A member of staff who may make versions and record the client's signed
/// acceptance.
pub async fn account_manager(t: &Booted, name: &str) -> User {
    t.user(
        name,
        &[
            wl_invoicing::permissions::RATE_CARDS_VIEW,
            wl_invoicing::permissions::RATE_CARDS_MANAGE,
        ],
    )
    .await
}

/// The MD: holds `rate_cards.approve`.
pub async fn md(t: &Booted, name: &str) -> User {
    t.user(name, &[wl_invoicing::permissions::RATE_CARDS_APPROVE])
        .await
}

/// A user of the client company's portal with the role `approver`.
pub async fn client_user(t: &Booted, name: &str) -> User {
    let user = t.user(name, &[]).await;
    t.portal.grant(&user, BLUE_DUNES, &["approver"]);
    user
}

/// Accepts `version` on behalf of the client through staff's route.
pub async fn accept_signed(t: &Booted, version: &RateCardVersion, by: &User) {
    accept_as(t, version, by, &[("document", "signed-acceptance.pdf")])
        .await
        .assert_status(303);
}
