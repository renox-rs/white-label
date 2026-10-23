//! The names of the service types, for a vertical that holds only the code of
//! a booking's service (wl-kernel `ServiceNames`).

use std::collections::BTreeMap;

use renox::prelude::*;
use wl_kernel::scheduling::{
    ResourceChoice, ResourceChoiceKind, ResourceNames, SchedulingFuture, ServiceNames,
};

use crate::resource::{Resource, ResourceType, ServiceType};

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

/// The resources of this module as the app's resource names.
pub struct ResourceTypeNames;

impl ResourceNames for ResourceTypeNames {
    fn names<'a>(
        &'a self,
        state: &'a AppState,
        ids: &'a [i64],
    ) -> SchedulingFuture<'a, BTreeMap<i64, String>> {
        Box::pin(async move {
            if ids.is_empty() {
                return Ok(BTreeMap::new());
            }
            Ok(Resource::query()
                .where_(Resource::ID.is_in(ids.to_vec()))
                .get(&state.db)
                .await?
                .into_iter()
                .map(|resource| (resource.id, resource.name))
                .collect())
        })
    }

    fn choices<'a>(
        &'a self,
        state: &'a AppState,
        kind: ResourceChoiceKind,
    ) -> SchedulingFuture<'a, Vec<ResourceChoice>> {
        Box::pin(async move {
            let wanted = match kind {
                ResourceChoiceKind::Person => ResourceType::Person,
                ResourceChoiceKind::Room => ResourceType::Room,
            };
            Ok(Resource::query()
                .where_(Resource::RESOURCE_TYPE.eq(wanted))
                .where_(Resource::ACTIVE.eq(true))
                .order_by(Resource::NAME)
                .get(&state.db)
                .await?
                .into_iter()
                .map(|resource| ResourceChoice {
                    id: resource.id,
                    name: resource.name,
                    employee_id: resource.employee_id,
                    capacity: resource.capacity,
                })
                .collect())
        })
    }
}
