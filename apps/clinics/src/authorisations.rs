//! The services of the prior authorisation (05 §7.1 *Prior authorisation*,
//! `CLN.INS.03`; base for BL-CLN-02 and BL-CLN-03).
//!
//! Every status change is a transition of
//! [`crate::authorisation_machine`]: the services here gather what the
//! guards need, run it and return the record read again. Who may call them is
//! decided by the pages (`insurance.manage`); a service refuses only what is
//! wrong.
//!
//! - [`create`], [`request`], [`approve`], [`approve_partially`], [`deny`] and
//!   [`cancel`] are the billing coordinator's.
//! - [`consume`] is charge capture's (system): units used, `used` when none
//!   are left. Charge capture is not built yet; nothing calls it in
//!   production, and nothing is consumed at check-in.
//! - [`expire_due`] is the daily run's (system): the authorisations whose
//!   last valid day has passed.
//! - [`require_for`] keeps the payer's list of services that need an
//!   authorisation (the payer contract's prior-auth list, minimal).
//! - [`covering`] finds the authorisations that cover a service on a day, the
//!   question the flags and the check-in ask.
//!
//! The patient is told when an authorisation is denied (notification
//! `clinic.prior_auth_denied`, no clinical reason, the self-pay option
//! offered) from a listener on the committed transition.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Json};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{TransitionError, Transitioned};
use wl_kernel::PartyId;
use wl_kernel::documents::dubai_date;
use wl_kernel::notify::{self, GROUP_BOOKINGS, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::parties::directory;

use crate::authorisation::{AuthStatus, PriorAuthService, PriorAuthorisation};
use crate::authorisation_machine::{
    APPROVE, APPROVE_PARTIALLY, AuthInput, CANCEL, CONSUME, CONSUME_LAST, CONSUME_PARTIAL, DENY,
    EXPIRE, REQUEST,
};
use crate::db::refused;
use crate::policy::InsurancePolicy;
use crate::texts::english;

/// The scheduled task, and the command, of the daily run.
pub const TASK: &str = "clinics:prior_authorisations";
/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";
/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";
/// The notification type that tells the patient an authorisation was denied.
pub const DENIED_NOTICE: &str = "clinic.prior_auth_denied";

/// A prior authorisation to prepare.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewAuthorisation {
    /// The patient's policy it is for.
    pub policy: i64,
    /// The service codes it covers.
    pub services: Vec<String>,
    /// The units asked for.
    pub units: i64,
}

impl NewAuthorisation {
    /// An authorisation of `units` on `policy`, with no service yet.
    pub fn new(policy: i64, units: i64) -> Self {
        NewAuthorisation {
            policy,
            services: Vec::new(),
            units,
        }
    }

    /// Covering the service `code`.
    pub fn service(mut self, code: &str) -> Self {
        self.services.push(code.to_owned());
        self
    }
}

/// Prepares an authorisation (`draft`) for a policy: the patient and the
/// payer are the policy's.
///
/// # Errors
///
/// A 404 for an unknown policy; a 422 for fewer than one unit.
pub async fn create(
    state: &AppState,
    by: Option<i64>,
    new: NewAuthorisation,
) -> Result<PriorAuthorisation> {
    let policy = InsurancePolicy::find_or_404(&state.db, new.policy).await?;
    if new.units < 1 {
        return Err(refused(english("clinics.authorisation.problem.units_asked", &[])));
    }
    let mut services: Vec<String> = Vec::new();
    for code in &new.services {
        let code = code.trim();
        if !code.is_empty() && !services.iter().any(|s| s == code) {
            services.push(code.to_owned());
        }
    }
    let mut authorisation = PriorAuthorisation {
        policy_id: policy.id,
        patient_party_id: policy.patient_party_id,
        payer_party_id: policy.payer_party_id,
        services: Json(services),
        units_requested: new.units,
        created_by: by,
        ..PriorAuthorisation::default()
    };
    authorisation.save(&state.db).await?;
    Ok(authorisation)
}

/// Runs the transition `name` of the user `by` with `input`.
async fn run(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    name: &str,
    mut input: AuthInput,
) -> Result<PriorAuthorisation> {
    let authorisation = PriorAuthorisation::find_or_404(&state.db, id).await?;
    input.by = Some(by.id);
    authorisation
        .transition(name)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    PriorAuthorisation::find_or_404(&state.db, id).await
}

/// `request`: sent to the payer, with the payer's reference if it gave one.
///
/// # Errors
///
/// The machine's: a 403 without `insurance.manage`, a 422 when no service or
/// unit is named or the authorisation is not a draft.
pub async fn request(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reference: Option<&str>,
    on: NaiveDate,
) -> Result<PriorAuthorisation> {
    let input = AuthInput {
        reference: reference.map(str::to_owned),
        on: Some(on),
        ..AuthInput::default()
    };
    run(state, by, id, REQUEST, input).await
}

/// `approve`: approved for the units asked for, valid through `valid_until`.
///
/// # Errors
///
/// The machine's: a 422 without a reference or a validity date, or with a
/// validity date before `on`.
pub async fn approve(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reference: Option<&str>,
    valid_until: Option<NaiveDate>,
    on: NaiveDate,
) -> Result<PriorAuthorisation> {
    let input = AuthInput {
        reference: reference.map(str::to_owned),
        valid_until,
        on: Some(on),
        ..AuthInput::default()
    };
    run(state, by, id, APPROVE, input).await
}

/// `approve_partially`: approved for `units`, fewer than were asked for.
///
/// # Errors
///
/// The machine's: a 422 unless `units` is between one and the units asked for
/// less one, or without a reference or a validity date.
pub async fn approve_partially(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    units: i64,
    reference: Option<&str>,
    valid_until: Option<NaiveDate>,
    on: NaiveDate,
) -> Result<PriorAuthorisation> {
    let input = AuthInput {
        reference: reference.map(str::to_owned),
        valid_until,
        units: Some(units),
        on: Some(on),
        ..AuthInput::default()
    };
    run(state, by, id, APPROVE_PARTIALLY, input).await
}

/// `deny`: the payer refused; `reason` is recorded (it never reaches the
/// patient).
///
/// # Errors
///
/// The machine's: a 422 without a reason.
pub async fn deny(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    on: NaiveDate,
) -> Result<PriorAuthorisation> {
    let input = AuthInput {
        reason: Some(reason.to_owned()),
        on: Some(on),
        ..AuthInput::default()
    };
    run(state, by, id, DENY, input).await
}

/// `cancel`: no longer needed.
///
/// # Errors
///
/// The machine's: a 422 unless it is a draft or requested.
pub async fn cancel(state: &AppState, by: &AuthUser, id: i64) -> Result<PriorAuthorisation> {
    run(state, by, id, CANCEL, AuthInput::default()).await
}

/// Charge capture used `units` of authorisation `id`: `used` when none are
/// left, else it stays `approved` or `partially_approved`.
///
/// # Errors
///
/// A 422 unless `units` is between one and the units left.
pub async fn consume(
    state: &AppState,
    id: i64,
    units: i64,
    at: DateTime,
) -> Result<PriorAuthorisation> {
    let authorisation = PriorAuthorisation::find_or_404(&state.db, id).await?;
    let name = if units == authorisation.units_left() {
        CONSUME_LAST
    } else if authorisation.status == AuthStatus::PartiallyApproved {
        CONSUME_PARTIAL
    } else {
        CONSUME
    };
    let input = AuthInput {
        units: Some(units),
        on: Some(dubai_date(at)),
        ..AuthInput::default()
    };
    authorisation
        .transition(name)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    PriorAuthorisation::find_or_404(&state.db, id).await
}

/// `expire` for every usable authorisation whose last valid day is before the
/// day of `at`. Returns the ids that expired.
///
/// # Errors
///
/// The database's and the machine's errors.
pub async fn expire_due(state: &AppState, at: DateTime) -> Result<Vec<i64>> {
    let today = dubai_date(at);
    let all = PriorAuthorisation::query()
        .where_(PriorAuthorisation::VALID_UNTIL.lt(today))
        .order_by(PriorAuthorisation::ID)
        .get(&state.db)
        .await?;
    let mut expired = Vec::new();
    for authorisation in all.into_iter().filter(|a| a.status.is_usable()) {
        let input = AuthInput {
            on: Some(today),
            ..AuthInput::default()
        };
        match authorisation
            .transition(EXPIRE)
            .by_system()
            .input(&input)
            .run(state)
            .await
        {
            Ok(_) => expired.push(authorisation.id),
            // Another run got there first.
            Err(TransitionError::NotFrom(_)) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    Ok(expired)
}

/// Puts the service `code` on the payer's list of services that need a prior
/// authorisation; a second call changes nothing.
///
/// # Errors
///
/// A 422 for a blank code.
pub async fn require_for(state: &AppState, payer: PartyId, code: &str) -> Result<PriorAuthService> {
    let code = code.trim();
    if code.is_empty() {
        return Err(refused(english("clinics.authorisation.problem.service_code", &[])));
    }
    let existing = PriorAuthService::query()
        .where_(PriorAuthService::PAYER_PARTY_ID.eq(payer.0))
        .where_(PriorAuthService::SERVICE_CODE.eq(code))
        .first(&state.db)
        .await?;
    if let Some(found) = existing {
        return Ok(found);
    }
    let mut row = PriorAuthService {
        payer_party_id: payer.0,
        service_code: code.to_owned(),
        ..PriorAuthService::default()
    };
    row.save(&state.db).await?;
    Ok(row)
}

/// Takes the service `code` off the payer's list.
///
/// # Errors
///
/// The database's errors.
pub async fn stop_requiring(state: &AppState, payer: PartyId, code: &str) -> Result<u64> {
    PriorAuthService::query()
        .where_(PriorAuthService::PAYER_PARTY_ID.eq(payer.0))
        .where_(PriorAuthService::SERVICE_CODE.eq(code.trim()))
        .delete(&state.db)
        .await
}

/// The authorisations of `patient` with `payer` that cover the service
/// `code` on `day`: usable, valid through `day`, with units left, leaving out
/// the authorisation `except`. Oldest first.
///
/// # Errors
///
/// The database's errors.
pub async fn covering(
    db: &Db,
    patient: i64,
    payer: i64,
    code: &str,
    day: NaiveDate,
    except: Option<i64>,
) -> Result<Vec<PriorAuthorisation>> {
    let all = PriorAuthorisation::usable_of(db, patient, payer).await?;
    Ok(all
        .into_iter()
        .filter(|a| {
            Some(a.id) != except && a.covers(code) && a.is_valid_on(day) && a.units_left() > 0
        })
        .collect())
}

/// The parties a message about `patient` goes to: its guardians when it has
/// any (`PTY.REL.01`), else the patient.
pub async fn addressees(state: &AppState, patient: PartyId) -> Vec<PartyId> {
    let guardians = directory(state)
        .guardians(&state.db, patient)
        .await
        .unwrap_or_default();
    if guardians.is_empty() {
        vec![patient]
    } else {
        guardians
    }
}

/// `deny` committed: the patient is told, with no clinical reason, and the
/// self-pay option is offered.
///
/// # Errors
///
/// The notifier's errors.
pub async fn on_denied(state: &AppState, id: i64) -> Result {
    let authorisation = PriorAuthorisation::find_or_404(&state.db, id).await?;
    for party in addressees(state, PartyId(authorisation.patient_party_id)).await {
        let request = NotifyRequest::new(DENIED_NOTICE, authorisation.subject())
            .subject_label(&authorisation.number())
            .to_party(party)
            .data(json!({ "number": authorisation.number() }));
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// What the daily run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Daily {
    /// The authorisations that expired.
    pub expired: Vec<i64>,
    /// The authorisations whose look-ahead ran (`CLN.RUL.09`).
    pub looked_ahead: Vec<i64>,
}

/// The daily run: the authorisations whose validity ended expire (which
/// flags the appointments that relied on them), then the look-ahead of
/// `CLN.RUL.09` runs for the ones that end in three days.
///
/// # Errors
///
/// The database's and the machine's errors.
pub async fn run_daily(state: &AppState, at: DateTime) -> Result<Daily> {
    let expired = expire_due(state, at).await?;
    let looked_ahead = crate::prior_auth_rules::look_ahead(state, at).await?;
    Ok(Daily {
        expired,
        looked_ahead,
    })
}

/// The notification types of the authorisation.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let subject = english("clinics.authorisation.denied_subject", &[]);
    let body = english("clinics.authorisation.denied_body", &[]);
    vec![
        NotificationTypeDef::new(DENIED_NOTICE, "clinics")
            .group(GROUP_BOOKINGS)
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    ]
}

/// Registers the machine, the notification type, the listener and the daily
/// run.
pub(crate) fn register(app: &mut Registry) {
    crate::authorisation_machine::register();
    for def in notification_types() {
        notify::register_type(def);
    }
    app.listen(
        |event: Transitioned<PriorAuthorisation>, state: AppState| async move {
            if event.transition == DENY {
                on_denied(&state, event.key).await?;
            }
            Ok(())
        },
    );
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_daily(&state, renox::db::now()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Expire the prior authorisations whose validity ended and flag the appointments that relied on them (CLN.RUL.09)",
        |_args, state: AppState| async move {
            let daily = run_daily(&state, renox::db::now()).await?;
            println!(
                "{} prior authorisations expired, {} looked ahead.",
                daily.expired.len(),
                daily.looked_ahead.len()
            );
            Ok(())
        },
    );
}
