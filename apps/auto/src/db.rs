//! Small database helpers shared by the services and the machines.

use renox::prelude::*;

/// A 422 with `message`, like a refused guard.
pub(crate) fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}
