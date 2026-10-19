//! Integration health and the manual fallback (wl-kernel §4.5, §5.14,
//! `PLT.RUL.05`).
//!
//! Each external system a module calls is registered as a connection
//! ([`IntegrationDef`]) with the screens it affects, a manual fallback and
//! the role responsible for it. This part is the contract: the registry,
//! the call a module hands over ([`Outbound`]), what its handler answers
//! ([`Delivery`]), the retry wait ([`retry_wait`]: 1, 2, 4 and 8 minutes,
//! then 15 minutes, with no attempt limit) and the event
//! ([`IntegrationStatusChanged`]).

pub mod def;
pub mod events;

pub use def::{
    DEFAULT_DOWN_AFTER_MINUTES, Delivery, DueFn, IntegrationDef, Outbound, SendFn, all, call_failed,
    down_at, get, register, retry_wait,
};
pub use events::IntegrationStatusChanged;
