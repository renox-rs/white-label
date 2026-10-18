//! Voiding a record entered in error (wl-kernel §5.12, `PLT.ACL.09`).
//!
//! A module registers its entity as voidable with [`Voidable::of`]: the
//! permission and scope, the number it keeps, whether the record was given
//! to a third party, and one reverse handler per kind of effect. This part is
//! the base: the [`VoidRecord`] (table `wl_voids`, one per subject), the
//! [`RecordVoided`] event, the [`Effect`] a handler reverses, and the
//! registry with its boot checks. [`run`] is the void itself: the two
//! transitions, [`void`] and the helpers modules use.

pub mod approval;
pub mod events;
pub mod record;
pub mod register;
pub mod run;

use renox::prelude::*;

pub use events::RecordVoided;
pub use record::{Effect, EffectKind, ReverseError, VoidRecord};
pub use register::{
    Reverse, Voidable, VoidableBuilder, check_machine, get, register, subject_types,
};
pub use run::{
    VoidOutcome, ensure_editable, exclude_voided, is_voided, transitions, void, void_numbers,
};

/// The state a voided record is in.
pub const VOIDED_STATE: &str = "voided";
/// The transition a user runs to void a record.
pub const VOID: &str = "void";
/// The input key the interceptor writes the posted count to; the default
/// void policy's condition reads it.
pub const POSTED_KEY: &str = "posted";
/// The transition the kernel runs as the system once a void is approved.
pub const VOID_APPROVED: &str = "void_approved";

/// Registers the void model.
pub(crate) fn register_models(app: &mut Registry) {
    app.model::<VoidRecord>();
}
