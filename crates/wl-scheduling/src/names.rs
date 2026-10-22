//! The names of the service types, for a vertical that holds only the code of
//! a booking's service (wl-kernel `ServiceNames`).

use std::collections::BTreeMap;

use renox::prelude::*;
use wl_kernel::scheduling::{SchedulingFuture, ServiceNames};

use crate::resource::ServiceType;

/// The name of `service` in `locale`: the Arabic one when the page is Arabic
/// and the type has it, else the English one.
#[must_use]
pub fn name_in(service: &ServiceType, locale: &str) -> String {
    match (locale, service.name_ar.as_deref()) {
        ("ar", Some(arabic)) if !arabic.trim().is_empty() => arabic.to_owned(),
        _ => service.name_en.clone(),
    }
}

/// The service types of this module as the app's service names.
pub struct ServiceTypeNames;

impl ServiceNames for ServiceTypeNames {
    fn names<'a>(
        &'a self,
        state: &'a AppState,
        locale: &'a str,
    ) -> SchedulingFuture<'a, BTreeMap<String, String>> {
        Box::pin(async move {
            Ok(ServiceType::query()
                .get(&state.db)
                .await?
                .into_iter()
                .map(|service| (service.code.clone(), name_in(&service, locale)))
                .collect())
        })
    }
}
