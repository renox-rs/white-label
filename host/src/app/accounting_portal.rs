//! The 04 Accounting portal section (`ACC.ACL.14`): a client user sees each
//! engagement of its company with its status only, *On hold* or *Ended*
//! included, and nothing about a block. The rows are the app's
//! (`accounting::portal`); the host turns them into a wl-portal section,
//! because the app does not depend on the portal crate.

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_portal::{PortalSection, SectionFuture, SectionRow, register_section};

/// The engagements section.
pub struct Engagements;

impl PortalSection for Engagements {
    fn title(&self) -> String {
        accounting::portal::title()
    }

    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>> {
        Box::pin(async move {
            let rows = accounting::portal::rows(state, party).await?;
            Ok::<Vec<SectionRow>, Error>(
                rows.iter()
                    .map(|row| SectionRow::new(&row.label, &row.status))
                    .collect(),
            )
        })
    }
}

/// The returns section: a client user sees its company's returns, and the
/// deadline of one the firm no longer files as its own (BL-ACC-17); read-only
/// after the engagement ends, closed 90 days later (`POR.ACL.01`).
pub struct Returns;

impl PortalSection for Returns {
    fn title(&self) -> String {
        accounting::portal::returns_title()
    }

    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>> {
        Box::pin(async move {
            let rows = accounting::portal::returns(state, party).await?;
            Ok::<Vec<SectionRow>, Error>(
                rows.iter()
                    .map(|row| SectionRow::new(&row.label, &row.status))
                    .collect(),
            )
        })
    }
}

/// The KPI packs section: the latest published version of each period, shown
/// as being revised while a newer draft exists (BL-ACC-20); a withdrawn pack
/// is gone at once (BL-ACC-21).
pub struct KpiPacks;

impl PortalSection for KpiPacks {
    fn title(&self) -> String {
        accounting::portal::kpi_title()
    }

    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>> {
        Box::pin(async move {
            let rows = accounting::portal::kpi_packs(state, party).await?;
            Ok::<Vec<SectionRow>, Error>(
                rows.iter()
                    .map(|row| SectionRow::new(&row.label, &row.status))
                    .collect(),
            )
        })
    }
}

/// Registers the sections for the Accounting client.
pub fn register(client_app: &str) {
    if client_app == "accounting" {
        register_section(accounting::portal::SECTION, Engagements);
        register_section(accounting::portal::RETURNS_SECTION, Returns);
        register_section(accounting::portal::KPI_SECTION, KpiPacks);
    }
}
