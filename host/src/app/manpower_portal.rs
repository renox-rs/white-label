//! The 09 Manpower portal sections (BL-MPW-28, `MPW.ACL.01`): a client user
//! sees the requests, the workers on site and the timesheets of its own
//! company, each with the status in words and nothing about a rate, a
//! salary, a document, a block or a hold reason. The rows are the app's
//! (`manpower::portal`); the host turns them into wl-portal sections because
//! the app does not depend on the portal crate. The full pages are the app's
//! (`/portal/manpower/...`).

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_portal::{PortalSection, SectionFuture, SectionRow, register_section};

/// Which rows a section lists.
#[derive(Clone, Copy)]
enum Kind {
    Requests,
    Deployments,
    Timesheets,
}

/// A manpower section.
struct Section(Kind);

impl PortalSection for Section {
    fn title(&self) -> String {
        match self.0 {
            Kind::Requests => manpower::portal::requests_title(),
            Kind::Deployments => manpower::portal::deployments_title(),
            Kind::Timesheets => manpower::portal::timesheets_title(),
        }
    }

    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>> {
        Box::pin(async move {
            let rows = match self.0 {
                Kind::Requests => manpower::portal::requests(state, party).await?,
                Kind::Deployments => manpower::portal::deployments(state, party).await?,
                Kind::Timesheets => manpower::portal::timesheet_rows(state, party).await?,
            };
            Ok::<Vec<SectionRow>, Error>(
                rows.iter()
                    .map(|row| SectionRow::new(&row.label, &row.status))
                    .collect(),
            )
        })
    }
}

/// Registers the sections for the Manpower client.
pub fn register(client_app: &str) {
    if client_app == "manpower" {
        register_section(manpower::portal::REQUESTS_SECTION, Section(Kind::Requests));
        register_section(
            manpower::portal::DEPLOYMENTS_SECTION,
            Section(Kind::Deployments),
        );
        register_section(
            manpower::portal::TIMESHEETS_SECTION,
            Section(Kind::Timesheets),
        );
    }
}
