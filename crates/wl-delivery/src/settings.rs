//! The per-client settings of wl-delivery (§7) that the lifecycles read:
//! the proof required and whether partial acceptance is on, per source type,
//! and the cash difference tolerance.
//!
//! They are copied onto a delivery when it is created (the required proof,
//! partial acceptance), so a later change of the setting does not change a
//! delivery already planned. The app sets them with the builder methods of
//! [`WlDelivery`](crate::WlDelivery).

use std::collections::BTreeMap;
use std::sync::Arc;

use renox::prelude::*;
use wl_kernel::delivery::{CodAbsorber, PodMethod};

/// The proof required when the setting names nothing for a source type:
/// photo and signature, both needed (§7).
pub const DEFAULT_METHODS: [PodMethod; 2] = [PodMethod::Photo, PodMethod::Signature];

/// The days untraced before a delivery is marked lost (`WRK.RUL.17`, §7).
pub const DEFAULT_UNTRACED_DAYS: i64 = 7;

/// Minutes an approver has to answer a cash difference (`WRK.ACL.09`, §7).
pub const DEFAULT_COD_APPROVAL_MINUTES: i64 = 10;

/// Who may absorb a cash difference when the setting names nobody: all four
/// (03 leaves out the customer).
pub const ALL_ABSORBERS: [CodAbsorber; 4] = [
    CodAbsorber::Customer,
    CodAbsorber::Client,
    CodAbsorber::Business,
    CodAbsorber::Driver,
];

/// The settings. `Default` is the §7 defaults.
#[derive(Debug, Clone)]
pub struct DeliverySettings {
    /// Partial acceptance per source type (`inventory.dispatch_note`);
    /// missing means on.
    partial: BTreeMap<String, bool>,
    /// The proof per source type: the methods and whether any one is enough.
    proof: BTreeMap<String, (Vec<PodMethod>, bool)>,
    /// The cash difference accepted without a reason, in fils (§7: 0).
    cod_tolerance: i64,
    /// The days a delivery may go untraced before `WRK.RUL.17` marks it lost
    /// (§7: 7).
    untraced_days: i64,
    /// Whether a short payment above the tolerance may be approved (§7:
    /// allowed with approval); off means it is always a failed attempt.
    cod_approval: bool,
    /// Minutes an approver has to answer before the request lapses (§7: 10).
    cod_approval_minutes: i64,
    /// Who may absorb a difference; the approver picks one of these.
    cod_absorbers: Vec<CodAbsorber>,
    /// Whether the driver's cash period leaves the escalation of the
    /// payments module to a vertical rule (03: `LOG.RUL.13`).
    replace_cash_escalation: bool,
}

impl Default for DeliverySettings {
    fn default() -> Self {
        DeliverySettings {
            partial: BTreeMap::new(),
            proof: BTreeMap::new(),
            cod_tolerance: 0,
            untraced_days: DEFAULT_UNTRACED_DAYS,
            cod_approval: true,
            cod_approval_minutes: DEFAULT_COD_APPROVAL_MINUTES,
            cod_absorbers: ALL_ABSORBERS.to_vec(),
            replace_cash_escalation: false,
        }
    }
}

impl DeliverySettings {
    /// Sets partial acceptance for `source_type`.
    pub fn set_partial_acceptance(&mut self, source_type: &str, on: bool) {
        self.partial.insert(source_type.to_owned(), on);
    }

    /// Sets the proof for `source_type`: `methods`, and whether any one of
    /// them is enough (`any`) or each is needed.
    pub fn set_required_proof(&mut self, source_type: &str, methods: &[PodMethod], any: bool) {
        self.proof
            .insert(source_type.to_owned(), (methods.to_vec(), any));
    }

    /// Sets the cash difference accepted without a reason, in fils.
    pub fn set_cod_tolerance(&mut self, fils: i64) {
        self.cod_tolerance = fils.max(0);
    }

    /// Sets the days a delivery may go untraced before it is marked lost.
    pub fn set_untraced_days(&mut self, days: i64) {
        self.untraced_days = days.max(1);
    }

    /// Allows (`true`) or forbids a short payment above the tolerance with
    /// an approval; forbidden, it is always a failed attempt.
    pub fn set_cod_approval(&mut self, allowed: bool) {
        self.cod_approval = allowed;
    }

    /// Sets the minutes an approver has to answer a cash difference.
    pub fn set_cod_approval_minutes(&mut self, minutes: i64) {
        self.cod_approval_minutes = minutes.max(1);
    }

    /// Sets who may absorb a cash difference.
    pub fn set_cod_absorbers(&mut self, absorbers: &[CodAbsorber]) {
        self.cod_absorbers = absorbers.to_vec();
    }

    /// Makes the driver's cash period leave the escalation to a vertical
    /// rule.
    pub fn set_replace_cash_escalation(&mut self, replaced: bool) {
        self.replace_cash_escalation = replaced;
    }

    /// Whether a short payment above the tolerance may be approved.
    pub fn cod_approval(&self) -> bool {
        self.cod_approval
    }

    /// Minutes an approver has to answer a cash difference.
    pub fn cod_approval_minutes(&self) -> i64 {
        self.cod_approval_minutes
    }

    /// Who may absorb a cash difference.
    pub fn cod_absorbers(&self) -> &[CodAbsorber] {
        &self.cod_absorbers
    }

    /// Whether `absorber` may be chosen.
    pub fn offers_absorber(&self, absorber: CodAbsorber) -> bool {
        self.cod_absorbers.contains(&absorber)
    }

    /// Whether the driver's cash period leaves the escalation to a vertical
    /// rule.
    pub fn replace_cash_escalation(&self) -> bool {
        self.replace_cash_escalation
    }

    /// The days a delivery may go untraced before it is marked lost.
    pub fn untraced_days(&self) -> i64 {
        self.untraced_days
    }

    /// Whether a delivery for `source_type` may be partly accepted.
    pub fn partial_acceptance(&self, source_type: &str) -> bool {
        self.partial.get(source_type).copied().unwrap_or(true)
    }

    /// The methods required for `source_type` and whether any one is enough.
    pub fn required_proof(&self, source_type: &str) -> (Vec<PodMethod>, bool) {
        self.proof
            .get(source_type)
            .cloned()
            .unwrap_or_else(|| (DEFAULT_METHODS.to_vec(), false))
    }

    /// The cash difference accepted without a reason, in fils.
    pub fn cod_tolerance(&self) -> i64 {
        self.cod_tolerance
    }
}

/// The app's settings, or the defaults when none were provided.
pub fn settings(state: &AppState) -> Arc<DeliverySettings> {
    state
        .provided::<DeliverySettings>()
        .unwrap_or_else(|| Arc::new(DeliverySettings::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_photo_and_signature_with_partial_acceptance_on() {
        let settings = DeliverySettings::default();
        assert!(settings.partial_acceptance("sales.order"));
        assert_eq!(
            settings.required_proof("sales.order"),
            (vec![PodMethod::Photo, PodMethod::Signature], false)
        );
        assert_eq!(settings.cod_tolerance(), 0);
        assert_eq!(settings.untraced_days(), 7);
        assert!(settings.cod_approval());
        assert_eq!(settings.cod_approval_minutes(), 10);
        assert!(settings.offers_absorber(CodAbsorber::Customer));
        assert!(!settings.replace_cash_escalation());
    }

    #[test]
    fn the_cash_difference_settings_can_be_changed() {
        let mut settings = DeliverySettings::default();
        settings.set_cod_approval(false);
        settings.set_cod_absorbers(&[CodAbsorber::Client, CodAbsorber::Driver]);
        assert!(!settings.cod_approval());
        assert!(!settings.offers_absorber(CodAbsorber::Customer));
        assert!(settings.offers_absorber(CodAbsorber::Client));
    }

    #[test]
    fn a_source_type_can_turn_partial_acceptance_off_and_change_the_proof() {
        let mut settings = DeliverySettings::default();
        settings.set_partial_acceptance("logistics.shipment", false);
        settings.set_required_proof("logistics.shipment", &[PodMethod::Otp], true);
        assert!(!settings.partial_acceptance("logistics.shipment"));
        assert!(settings.partial_acceptance("sales.order"));
        assert_eq!(
            settings.required_proof("logistics.shipment"),
            (vec![PodMethod::Otp], true)
        );
    }
}
