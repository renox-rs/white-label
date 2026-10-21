//! The client's portal window after its contract ends (03 Logistics §3.3
//! `POR.ACL.01`; BL-LOG-26): its portal users keep read-only access to
//! shipments, invoices and COD statements for
//! [`crate::defaults::PORTAL_CLOSES_AFTER_DAYS`] days, then the portal closes.
//!
//! The end of the contract is a date on the client's terms
//! ([`end_contract`], `ClientTerms::contract_ended_on`; the contract itself is
//! the rate card agreement of `FIN.RAT.01`, which is not built). The window is
//! worked out from that date ([`access`]) and applied where a client user
//! acts for the company ([`crate::booking::acts_for`], which is for acting,
//! and [`crate::booking::sees_for`], which is for looking): a closed portal
//! gives neither, a read-only one only the second.

use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::PartyId;
use wl_kernel::documents::today;

use crate::defaults::PORTAL_CLOSES_AFTER_DAYS;
use crate::freight::date_in;
use crate::pages::{Fields, field};
use crate::permissions;
use crate::reweigh::{self, ClientTerms};
use crate::texts::t;

/// What a client's portal users may do on a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// The contract runs (or none ended): look and act.
    Open,
    /// The contract ended: look only.
    ReadOnly,
    /// The contract ended more than the window ago: nothing.
    Closed,
}

/// The portal access of `client` on `on`: open until the contract's end date,
/// read-only for the 90 days after it, closed from the day after those.
///
/// # Errors
///
/// Database errors.
pub async fn access(db: &Db, client: PartyId, on: NaiveDate) -> Result<Access> {
    let terms = reweigh::terms(db, client).await?;
    Ok(access_of(terms.contract_ended_on, on))
}

/// [`access`] for a known end date.
pub fn access_of(ended: Option<NaiveDate>, on: NaiveDate) -> Access {
    match ended {
        None => Access::Open,
        Some(ended) if on <= ended => Access::Open,
        Some(ended) if on <= ended + TimeDelta::days(PORTAL_CLOSES_AFTER_DAYS) => Access::ReadOnly,
        Some(_) => Access::Closed,
    }
}

/// `user` (`shipments.manage`) records the day the contract of `client`
/// ended, or clears it with `None` (the contract was renewed).
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`.
pub async fn end_contract(
    state: &AppState,
    user: &AuthUser,
    client: PartyId,
    ended: Option<NaiveDate>,
) -> std::result::Result<ClientTerms, TransitionError> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let found = ClientTerms::query()
        .where_(ClientTerms::CLIENT_PARTY_ID.eq(client.0))
        .first(&state.db)
        .await?;
    match found {
        Some(mut terms) => {
            terms.contract_ended_on = ended;
            terms.save(&state.db).await?;
        }
        None => {
            let mut terms = ClientTerms {
                client_party_id: client.0,
                contract_ended_on: ended,
                ..ClientTerms::default()
            };
            terms.insert(&state.db).await?;
        }
    }
    Ok(reweigh::terms(&state.db, client).await?)
}

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .post("/clients/{client}/contract-end", contract_end)
        .name("logistics.contract_end")
        .require_auth()
}

/// `POST /clients/{client}/contract-end`: `date` (`2026-11-01`; blank takes
/// the end back).
async fn contract_end(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(client): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let text = field(&form, "date");
    let ended = if text.trim().is_empty() {
        None
    } else {
        Some(date_in(&text).ok_or_else(|| {
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, t(&lang, "logistics.bad_input"))
        })?)
    };
    end_contract(&state, &user, PartyId(client), ended)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.contract_ended")),
        Redirect::route("logistics.shipments", &[])?,
    )
        .into_response())
}

/// Whether the portal of `client` is open for acting today.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn open_today(db: &Db, client: PartyId) -> Result<bool> {
    Ok(access(db, client, today()).await? == Access::Open)
}

/// Whether the portal of `client` still shows its records today.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn visible_today(db: &Db, client: PartyId) -> Result<bool> {
    Ok(access(db, client, today()).await? != Access::Closed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    #[test]
    fn a_contract_ended_on_1_november_is_read_only_until_30_january_and_closed_after() {
        let ended = Some(day("2026-11-01"));
        assert_eq!(access_of(ended, day("2026-11-01")), Access::Open);
        assert_eq!(access_of(ended, day("2026-11-02")), Access::ReadOnly);
        assert_eq!(access_of(ended, day("2027-01-30")), Access::ReadOnly);
        assert_eq!(access_of(ended, day("2027-01-31")), Access::Closed);
        assert_eq!(access_of(None, day("2030-01-01")), Access::Open);
    }
}
