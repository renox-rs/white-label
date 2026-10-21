//! A shipment is voidable (03 Logistics §7.1 row `void`, `PLT.ACL.09`;
//! BL-LOG-01): a duplicate or a booking entered in error, found while it is
//! still `created`. The kernel's void pattern gives the `void` and
//! `void_approved` transitions, the reason, the Void record that keeps the
//! number, and the `RecordVoided` event; the machine adds that no parcel was
//! scanned and that a client user voids only what they booked
//! ([`crate::shipment_machine`]).
//!
//! A `created` shipment has posted no money and moved no stock, so there is
//! nothing to reverse and no approval. The label is invalidated by the state:
//! every scan of a voided shipment's label is refused ([`crate::scan`]). Its
//! delivery is cancelled after the commit ([`crate::events`]), and it stays
//! off every invoice ([`crate::Shipment::billable`]).

use wl_kernel::{SubjectMachine, Voidable};

use crate::permissions;
use crate::shipment::{SUBJECT_TYPE, Shipment};

/// Registers the shipment with the kernel's machines (so `void_approved` can
/// be run by name) and as a voidable record that keeps its number.
pub(crate) fn register() {
    SubjectMachine::of::<Shipment>(SUBJECT_TYPE).register();
    renox::state_machine::register::<Shipment>();
    Voidable::of::<Shipment>(SUBJECT_TYPE)
        .permission(permissions::SHIPMENTS_VOID)
        .number("shipment", |shipment: &Shipment| {
            Some(shipment.number.clone())
        })
        .register();
}
