//! The self-pay consent (05 §9 *Self-pay consent*, `CLN.ACL.15`; BL-CLN-01).
//!
//! When the cover fails at check-in the patient may still be treated as
//! self-pay, but only after signing a consent that shows the services and
//! their **estimated** price (each service's self-pay price on the visit day,
//! from the self-pay price list, `SAL.CAT.01`). The bill later uses the price
//! list on the visit day, not the estimate.
//!
//! The patient signs in person on the front-desk tablet (`DOC.SIG.01`).
//! wl-documents has no signature request yet, so the app keeps the minimal
//! record the 04 app keeps for its engagement letter: a [`SelfPayConsent`] with the lines, the estimate, the hash of the
//! text, the signer's name, the time and the IP address. Decision (seam): when
//! `DOC.SIG.01` exists the record gets a `document_id` and the signature is
//! the document's; nothing else changes.
//!
//! A consent is **superseded** (kept, marked) when the outcome becomes
//! *insured* or the appointment is rescheduled or cancelled, and when a
//! service not on it is added during the visit: that needs a new consent
//! (a new signature) before charge capture accepts the service
//! ([`ensure_covered`], [`sign_updated`]).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Json, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::dubai_date;
use wl_kernel::sales::self_pay_prices;

use crate::appointment::Appointment;
use crate::db::refused;
use crate::texts::english;
use crate::visit::Visit;

/// The subject type of a self-pay consent.
pub const CONSENT_SUBJECT_TYPE: &str = "clinics.self_pay_consent";

/// One service on a consent, with its estimated self-pay price.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentLine {
    /// The service code.
    pub code: String,
    /// The self-pay price in fils on the visit day.
    pub price: i64,
}

/// A self-pay consent the patient signed.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_self_pay_consents", index(booking_id))]
pub struct SelfPayConsent {
    /// The primary key.
    pub id: i64,
    /// The booking the visit is for.
    pub booking_id: i64,
    /// The patient.
    pub patient_party_id: Option<i64>,
    /// The pending check-in data it was signed on, until check-in.
    pub pending_id: Option<i64>,
    /// The visit it is kept with, from check-in on.
    pub visit_id: Option<i64>,
    /// The services shown to the patient, each with its estimated price.
    pub lines: Json<Vec<ConsentLine>>,
    /// The estimate: the sum of the lines, in fils.
    pub estimated_fils: i64,
    /// The day the prices were read for (the visit day).
    pub priced_on: NaiveDate,
    /// The hash of the text the patient signed (`DOC.SIG.01`).
    pub document_hash: String,
    /// When the patient signed.
    pub signed_at: DateTime,
    /// The day they signed (Asia/Dubai).
    pub signed_on: NaiveDate,
    /// The signer's name as typed on the tablet.
    pub signer_name: String,
    /// The member of staff who held the tablet.
    pub signer_user_id: Option<i64>,
    /// The IP address of the tablet's browser.
    pub ip_address: Option<String>,
    /// The day a newer consent, or the outcome *insured*, replaced it.
    pub superseded_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SelfPayConsent {
    /// The consent as a [`Subject`]: `clinics.self_pay_consent:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(CONSENT_SUBJECT_TYPE, self.id)
    }

    /// Whether a newer consent or the outcome *insured* replaced it.
    pub fn is_superseded(&self) -> bool {
        self.superseded_on.is_some()
    }

    /// Whether `code` is one of the services on it.
    pub fn lists(&self, code: &str) -> bool {
        self.lines.0.iter().any(|line| line.code == code)
    }

    /// The codes on it.
    pub fn codes(&self) -> Vec<String> {
        self.lines.0.iter().map(|line| line.code.clone()).collect()
    }

    /// The consents of booking `booking`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_booking(db: &Db, booking: i64) -> Result<Vec<SelfPayConsent>> {
        SelfPayConsent::query()
            .where_(SelfPayConsent::BOOKING_ID.eq(booking))
            .order_by(SelfPayConsent::ID)
            .get(db)
            .await
    }

    /// The consent in force for booking `booking`: the latest not superseded.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn current(db: &Db, booking: i64) -> Result<Option<SelfPayConsent>> {
        Ok(SelfPayConsent::of_booking(db, booking)
            .await?
            .into_iter()
            .rev()
            .find(|consent| !consent.is_superseded()))
    }
}

/// The hash of `text` (FNV-1a, in hex): what the signature is over until
/// wl-documents generates the PDF. The same function as the 04 letters use.
pub fn document_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The text the patient signs: the services and the estimate.
pub fn consent_text(booking: i64, priced_on: NaiveDate, lines: &[ConsentLine]) -> String {
    let rows: Vec<String> = lines
        .iter()
        .map(|line| format!("{}:{}", line.code, line.price))
        .collect();
    let total: i64 = lines.iter().map(|line| line.price).sum();
    format!(
        "SELF-PAY|{booking}|{priced_on}|{}|{total}",
        rows.join(",")
    )
}

/// The lines for `codes` priced on `on`, each from the self-pay price list.
///
/// # Errors
///
/// A 422 "no self-pay price for <service>" for the first code the list has
/// no price for on that day; the price list's errors.
pub async fn price_lines(
    state: &AppState,
    codes: &[String],
    on: NaiveDate,
) -> Result<Vec<ConsentLine>> {
    let prices = self_pay_prices(state);
    let mut lines = Vec::with_capacity(codes.len());
    for code in codes {
        match prices.price_on(&state.db, code, on).await? {
            Some(price) => lines.push(ConsentLine {
                code: code.clone(),
                price,
            }),
            None => {
                return Err(refused(english(
                    "clinics.consent.problem.no_price",
                    &[("service", code)],
                )));
            }
        }
    }
    Ok(lines)
}

/// Adds `extra` to `first`, once each, in order.
fn with_codes(first: &str, extra: &[String]) -> Vec<String> {
    let mut codes = vec![first.to_owned()];
    for code in extra {
        let code = code.trim();
        if !code.is_empty() && !codes.iter().any(|c| c == code) {
            codes.push(code.to_owned());
        }
    }
    codes
}

/// Records the consent the patient signed for booking `appointment`'s
/// services (its own and `extra`), priced on the visit day, superseding the
/// consent in force. Returns it unlinked: the caller attaches it to the
/// pending check-in data or to the visit.
///
/// # Errors
///
/// A 422 when the signer's name is blank or a service has no self-pay price.
pub async fn sign(
    state: &AppState,
    by: Option<i64>,
    appointment: &Appointment,
    signer_name: &str,
    ip: Option<&str>,
    extra: &[String],
    at: DateTime,
) -> Result<SelfPayConsent> {
    if signer_name.trim().is_empty() {
        return Err(refused(english("clinics.consent.problem.signer", &[])));
    }
    let codes = with_codes(&appointment.service_code, extra);
    let lines = price_lines(state, &codes, appointment.visit_date).await?;
    let text = consent_text(appointment.booking_id, appointment.visit_date, &lines);
    supersede_all(state, appointment.booking_id, dubai_date(at)).await?;
    let mut consent = SelfPayConsent {
        booking_id: appointment.booking_id,
        patient_party_id: appointment.patient_party_id,
        estimated_fils: lines.iter().map(|line| line.price).sum(),
        lines: Json(lines),
        priced_on: appointment.visit_date,
        document_hash: document_hash(&text),
        signed_at: at,
        signed_on: dubai_date(at),
        signer_name: signer_name.trim().to_owned(),
        signer_user_id: by,
        ip_address: ip.map(str::to_owned),
        ..SelfPayConsent::default()
    };
    consent.save(&state.db).await?;
    Ok(consent)
}

/// Marks every consent of the booking that is in force as superseded on `on`.
///
/// # Errors
///
/// The database's errors.
pub async fn supersede_all(state: &AppState, booking: i64, on: NaiveDate) -> Result<u64> {
    SelfPayConsent::query()
        .where_(SelfPayConsent::BOOKING_ID.eq(booking))
        .where_(SelfPayConsent::SUPERSEDED_ON.is_null())
        .update(
            &state.db,
            &[("superseded_on", &Some(on)), ("updated_at", &now())],
        )
        .await
}

/// Charge capture asks before it accepts `code` on the visit: a `self_pay`
/// visit takes only the services on its consent in force; a service not on it
/// needs an updated consent first ([`sign_updated`]). A visit with another
/// outcome takes any service.
///
/// # Errors
///
/// A 422 "<service> is not on the self-pay consent; ask the patient to sign an
/// updated consent" for a self-pay visit; a 404 for an unknown visit.
pub async fn ensure_covered(state: &AppState, visit: i64, code: &str) -> Result {
    let visit = Visit::find_or_404(&state.db, visit).await?;
    if visit.coverage_outcome != Some(crate::coverage::CoverageOutcome::SelfPay) {
        return Ok(());
    }
    let current = match visit.consent_id {
        Some(id) => SelfPayConsent::find(&state.db, id).await?,
        None => None,
    };
    if current
        .as_ref()
        .is_some_and(|consent| !consent.is_superseded() && consent.lists(code))
    {
        return Ok(());
    }
    Err(refused(english(
        "clinics.consent.problem.not_on_consent",
        &[("service", &code)],
    )))
}

/// The patient signs an updated consent during the visit, adding `extra`
/// services to those already on it; the visit keeps the new consent and the
/// old one is marked superseded. Only a `self_pay` visit that has not ended
/// takes one.
///
/// # Errors
///
/// A 422 for a visit that is not a self-pay visit in progress, a blank name
/// or a service with no self-pay price.
pub async fn sign_updated(
    state: &AppState,
    by: Option<i64>,
    visit: i64,
    signer_name: &str,
    ip: Option<&str>,
    extra: &[String],
    at: DateTime,
) -> Result<SelfPayConsent> {
    let visit = Visit::find_or_404(&state.db, visit).await?;
    if visit.coverage_outcome != Some(crate::coverage::CoverageOutcome::SelfPay)
        || !visit.status.is_open()
    {
        return Err(refused(english("clinics.consent.problem.not_self_pay", &[])));
    }
    let appointment = Appointment::of_booking(&state.db, visit.booking_id)
        .await?
        .ok_or(Error::NotFound)?;
    let mut codes = match visit.consent_id {
        Some(id) => SelfPayConsent::find(&state.db, id)
            .await?
            .map(|consent| consent.codes())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    codes.extend(extra.iter().cloned());
    let rest: Vec<String> = codes
        .into_iter()
        .filter(|code| *code != appointment.service_code)
        .collect();
    let consent = sign(state, by, &appointment, signer_name, ip, &rest, at).await?;
    SelfPayConsent::query()
        .where_(SelfPayConsent::ID.eq(consent.id))
        .update(
            &state.db,
            &[("visit_id", &Some(visit.id)), ("updated_at", &now())],
        )
        .await?;
    Visit::query()
        .where_(Visit::ID.eq(visit.id))
        .update(
            &state.db,
            &[("consent_id", &Some(consent.id)), ("updated_at", &now())],
        )
        .await?;
    SelfPayConsent::find_or_404(&state.db, consent.id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_and_its_hash_follow_the_services_and_prices() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        let lines = vec![
            ConsentLine {
                code: "consultation".into(),
                price: 45_000,
            },
            ConsentLine {
                code: "x_ray".into(),
                price: 12_000,
            },
        ];
        let text = consent_text(7, day, &lines);
        assert_eq!(text, "SELF-PAY|7|2026-11-10|consultation:45000,x_ray:12000|57000");
        assert_eq!(document_hash(&text), document_hash(&text));
        assert_ne!(document_hash(&text), document_hash("SELF-PAY|8"));
    }

    #[test]
    fn a_code_is_listed_once() {
        let codes = with_codes("consultation", &["x_ray".into(), " consultation ".into(), "".into()]);
        assert_eq!(codes, ["consultation", "x_ray"]);
    }
}
