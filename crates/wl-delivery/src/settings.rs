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
use wl_kernel::delivery::PodMethod;

/// The proof required when the setting names nothing for a source type:
/// photo and signature, both needed (§7).
pub const DEFAULT_METHODS: [PodMethod; 2] = [PodMethod::Photo, PodMethod::Signature];

/// The days untraced before a delivery is marked lost (`WRK.RUL.17`, §7).
pub const DEFAULT_UNTRACED_DAYS: i64 = 7;

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
}

impl Default for DeliverySettings {
    fn default() -> Self {
        DeliverySettings {
            partial: BTreeMap::new(),
            proof: BTreeMap::new(),
            cod_tolerance: 0,
            untraced_days: DEFAULT_UNTRACED_DAYS,
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
