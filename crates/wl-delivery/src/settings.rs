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

/// What happens to a perishable delivery after a failed attempt (`WRK.RUL.16`,
/// §7): *return* (the goods go back to the origin, which decides reuse or
/// wastage) or *discard* (destroyed with a photo, wastage recorded).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PerishableAction {
    /// Back to the origin.
    Return,
    /// Destroyed with a photo.
    Discard,
}

impl PerishableAction {
    /// The stored name: `return` or `discard`.
    pub fn as_str(self) -> &'static str {
        match self {
            PerishableAction::Return => "return",
            PerishableAction::Discard => "discard",
        }
    }
}

/// The perishable policy of one source type: what follows a failed visit and
/// what follows a run `abort` (06: *return* for own-rider deliveries, and
/// *discard* only when the rider cannot bring the food back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerishablePolicy {
    /// After a failed visit.
    pub on_failed_visit: PerishableAction,
    /// After the run was aborted.
    pub on_abort: PerishableAction,
}

impl Default for PerishablePolicy {
    /// The module default: *discard* (cooked food) in both cases.
    fn default() -> Self {
        PerishablePolicy {
            on_failed_visit: PerishableAction::Discard,
            on_abort: PerishableAction::Discard,
        }
    }
}

/// The wastage value above which a discard needs a second person (`STK.ACL.02`,
/// §7: AED 1,000; 06: AED 200), in fils.
pub const DEFAULT_DISCARD_APPROVAL_FILS: i64 = 100_000;

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
    /// Whether the receiving driver counts the cash at a hand-over (§7:
    /// on); off, the custody amount moves as recorded.
    hand_over_counts_cash: bool,
    /// The abort reasons a vertical adds to breakdown, accident, driver
    /// unwell, weather and other (03: `driver_unreachable`).
    extra_abort_reasons: Vec<String>,
    /// The role that gets the dispatcher's tasks (a missing item at run
    /// close); the company admin when nobody holds it.
    dispatcher_role: String,
    /// The perishable policy per source type; missing takes the default.
    perishable: BTreeMap<String, PerishablePolicy>,
    /// The wastage value above which a discard needs a second person.
    discard_approval_fils: i64,
    /// The delivery promise per source type, in minutes: the perishable
    /// hand-over window when the source gives none.
    promise: BTreeMap<String, i64>,
    /// Whether `WRK.RUL.08` reattempts or returns a failed non-perishable
    /// delivery by itself (off: a dispatcher does it, and 03 shipments follow
    /// their own rule).
    attempt_rule: bool,
}

/// The role of the dispatcher's tasks unless a client names another.
pub const DEFAULT_DISPATCHER_ROLE: &str = "dispatcher";

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
            hand_over_counts_cash: true,
            extra_abort_reasons: Vec::new(),
            dispatcher_role: DEFAULT_DISPATCHER_ROLE.to_owned(),
            perishable: BTreeMap::new(),
            discard_approval_fils: DEFAULT_DISCARD_APPROVAL_FILS,
            promise: BTreeMap::new(),
            attempt_rule: false,
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

    /// Makes the receiving driver count the cash at a hand-over (`true`, §7)
    /// or lets the custody amount move as recorded (`false`).
    pub fn set_hand_over_counts_cash(&mut self, on: bool) {
        self.hand_over_counts_cash = on;
    }

    /// Adds abort reasons beyond the five of the spec.
    pub fn set_extra_abort_reasons(&mut self, reasons: &[&str]) {
        self.extra_abort_reasons = reasons.iter().map(|r| (*r).to_owned()).collect();
    }

    /// Sets the role that gets the dispatcher's tasks.
    pub fn set_dispatcher_role(&mut self, role: &str) {
        self.dispatcher_role = role.to_owned();
    }

    /// Sets what follows a failed visit and a run `abort` for perishable
    /// deliveries of `source_type` (`WRK.RUL.16`).
    pub fn set_perishable_policy(
        &mut self,
        source_type: &str,
        on_failed_visit: PerishableAction,
        on_abort: PerishableAction,
    ) {
        self.perishable.insert(
            source_type.to_owned(),
            PerishablePolicy {
                on_failed_visit,
                on_abort,
            },
        );
    }

    /// The perishable policy of `source_type`: the client's, else the module
    /// default (*discard*). The rule reads it at the failed attempt.
    pub fn perishable_policy(&self, source_type: &str) -> PerishablePolicy {
        self.perishable
            .get(source_type)
            .copied()
            .unwrap_or_default()
    }

    /// Sets the wastage value, in fils, above which a discard needs a second
    /// person (`STK.ACL.02`).
    pub fn set_discard_approval_fils(&mut self, fils: i64) {
        self.discard_approval_fils = fils.max(0);
    }

    /// The wastage value above which a discard needs a second person.
    pub fn discard_approval_fils(&self) -> i64 {
        self.discard_approval_fils
    }

    /// Sets the delivery promise of `source_type` in minutes: the perishable
    /// hand-over window when the source's request gives none.
    pub fn set_promise_minutes(&mut self, source_type: &str, minutes: i64) {
        self.promise.insert(source_type.to_owned(), minutes.max(1));
    }

    /// The delivery promise of `source_type` in minutes, if set.
    pub fn promise_minutes(&self, source_type: &str) -> Option<i64> {
        self.promise.get(source_type).copied()
    }

    /// Turns `WRK.RUL.08` on: a failed non-perishable delivery is reattempted
    /// while attempts remain and its reason allows, else returned once the
    /// goods are back.
    pub fn set_attempt_rule(&mut self, on: bool) {
        self.attempt_rule = on;
    }

    /// Whether `WRK.RUL.08` acts by itself.
    pub fn attempt_rule(&self) -> bool {
        self.attempt_rule
    }

    /// The role that gets the dispatcher's tasks.
    pub fn dispatcher_role(&self) -> &str {
        &self.dispatcher_role
    }

    /// Whether the receiving driver counts the cash at a hand-over.
    pub fn hand_over_counts_cash(&self) -> bool {
        self.hand_over_counts_cash
    }

    /// The abort reason codes: the five of the spec, then the vertical's.
    pub fn abort_reasons(&self) -> Vec<String> {
        ["breakdown", "accident", "driver_unwell", "weather", "other"]
            .iter()
            .map(|r| (*r).to_owned())
            .chain(self.extra_abort_reasons.iter().cloned())
            .collect()
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
        assert!(settings.hand_over_counts_cash());
        assert_eq!(settings.abort_reasons().len(), 5);
    }

    #[test]
    fn a_vertical_adds_abort_reasons_after_the_five() {
        let mut settings = DeliverySettings::default();
        settings.set_extra_abort_reasons(&["driver_unreachable"]);
        let reasons = settings.abort_reasons();
        assert_eq!(reasons.len(), 6);
        assert_eq!(reasons[5], "driver_unreachable");
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
