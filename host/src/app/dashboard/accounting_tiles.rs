//! The dashboard of a client user of an accounting firm (04): the portal has
//! no home page of its own, so a client signing in finds here what waits for
//! it (a return to approve, a letter to sign), the KPI packs it may read and
//! the portal sections of each company it acts for.

use accounting::{EngagementLetter, Return, ReturnStatus};
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_portal::{GrantStatus, PortalAccess};

/// One link of the block.
#[derive(Debug, Clone, Serialize)]
pub(super) struct PortalLink {
    /// What the link says.
    pub title: String,
    /// Where it goes.
    pub url: String,
    /// The icon beside it.
    pub icon: &'static str,
}

/// The links for `user` as a client user of an accounting firm; none for
/// anyone without an active grant, and on a deployment of another vertical.
pub(super) async fn portal_links(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
) -> Result<Vec<PortalLink>> {
    if super::app_of(state) != "accounting" {
        return Ok(Vec::new());
    }
    let mut links = Vec::new();
    let mut parties: Vec<i64> = Vec::new();
    for grant in PortalAccess::grants_for(&state.db, user.id).await? {
        if grant.status == GrantStatus::Active && !parties.contains(&grant.party_id) {
            parties.push(grant.party_id);
        }
    }
    for party in parties {
        let client = PartyId(party);
        let name = wl_parties::Party::find(&state.db, party)
            .await?
            .map(|p| p.legal_name_en)
            .unwrap_or_default();
        for ret in Return::of_client(&state.db, client).await? {
            if ret.status == ReturnStatus::ClientApproval
                && accounting::returns::is_client_approver(state, user, &ret).await?
            {
                links.push(PortalLink {
                    title: lang.t(
                        "dashboard.portal.accounting.approve_return",
                        &[
                            ("number", &ret.number()),
                            ("period", &ret.period_end.format("%-d %b %Y").to_string()),
                        ],
                    ),
                    url: format!("/accounting/portal/returns/{}", ret.id),
                    icon: "circle-check",
                });
            }
        }
        for engagement in accounting::Engagement::of_client(&state.db, client).await? {
            if engagement.letter_signed {
                continue;
            }
            if let Some(letter) = EngagementLetter::current(&state.db, engagement.id).await?
                && letter.signed_at.is_none()
            {
                links.push(PortalLink {
                    title: lang.t(
                        "dashboard.portal.accounting.sign_letter",
                        &[("number", &engagement.number())],
                    ),
                    url: format!("/accounting/portal/engagements/{}/letter", engagement.id),
                    icon: "pencil",
                });
            }
        }
        for pack in accounting::KpiPack::query()
            .where_(accounting::KpiPack::CLIENT_PARTY_ID.eq(party))
            .where_(accounting::KpiPack::STATUS.eq(accounting::KpiStatus::Published))
            .order_by_desc(accounting::KpiPack::PERIOD_END)
            .get(&state.db)
            .await?
        {
            links.push(PortalLink {
                title: lang.t(
                    "dashboard.portal.accounting.kpi_pack",
                    &[("period", &pack.period_end.format("%B %Y").to_string())],
                ),
                url: format!("/accounting/portal/kpi-packs/{}", pack.id),
                icon: "chart-line",
            });
        }
        for (section, key, icon) in [
            (
                accounting::portal::SECTION,
                "dashboard.portal.accounting.engagements",
                "handshake",
            ),
            (
                accounting::portal::RETURNS_SECTION,
                "dashboard.portal.accounting.returns",
                "file-text",
            ),
            (
                accounting::portal::KPI_SECTION,
                "dashboard.portal.accounting.kpi_packs",
                "chart-column",
            ),
        ] {
            links.push(PortalLink {
                title: lang.t(key, &[("client", &name)]),
                url: format!("/portal/{party}/{section}"),
                icon,
            });
        }
    }
    Ok(links)
}
