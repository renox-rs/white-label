//! The money sections of the portal: the party's invoices and statement of
//! account with the dispute (wl-invoicing, BL-INV-31).
//!
//! The pages and the data belong to the modules; the host mounts the routes
//! because the access check is wl-portal's, which the modules do not depend
//! on. Every page first asks [`PortalMenu`] whether the signed-in user's
//! grants for the party (`active` or `read_only`) allow the section; a write
//! action then asks [`PortalGuard`], which refuses a `read_only` grant and,
//! for an action that is new business or money out, a blocked party (a
//! dispute is neither, so a blocked party may dispute and sees no block
//! text, `PTY.ACL.02`).
//!
//! | Route | Needs |
//! |---|---|
//! | `GET /portal/{party}/invoices` | the `invoices` section (`portal.invoices.view`) |
//! | `GET /portal/{party}/invoices/statement` | the same; `?from=&to=`, default this month |
//! | `POST /portal/{party}/invoices/{id}/dispute` | the same, a grant that may write, the client's portal disputes on |

use std::collections::HashMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Deserialize;
use wl_invoicing::portal as invoices;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_portal::{
    PortalAction, PortalGuard, PortalMenu, PortalSection, SectionFuture, SectionRow,
    read_only_notice, register_section,
};

/// The menu order of the invoices (01 tenant portal: after the lease).
const INVOICES_ORDER: i32 = 20;

/// The invoices section of the portal menu; its page is
/// `GET /portal/{party}/invoices` below.
pub struct InvoicesSection;

impl PortalSection for InvoicesSection {
    fn title(&self) -> String {
        "Invoices".to_owned()
    }

    fn permission(&self) -> Option<&str> {
        Some(invoices::VIEW_PERMISSION)
    }

    fn order(&self) -> i32 {
        INVOICES_ORDER
    }

    fn label(&self, lang: &Lang) -> String {
        let found = lang.t("wl_invoicing.portal.title", &[]);
        if found == "wl_invoicing.portal.title" {
            self.title()
        } else {
            found
        }
    }

    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>> {
        Box::pin(async move {
            let lang = state.lang("en");
            let rows = invoices::rows(&state.db, &lang, party, false).await?;
            Ok::<Vec<SectionRow>, Error>(
                rows.iter()
                    .map(|row| {
                        SectionRow::new(&format!("{} ({})", row.number, row.balance), &row.status)
                    })
                    .collect(),
            )
        })
    }
}

/// Registers the money sections (every client: the permission decides who
/// sees them).
pub fn register() {
    register_section(invoices::SECTION, InvoicesSection);
}

/// The menu of `user` for `party` when it holds `section`, else 403.
async fn menu_with(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    party: i64,
    section: &str,
) -> Result<PortalMenu> {
    let menu = PortalMenu::of(state, user.id, PartyId(party), today(), lang)
        .await?
        .ok_or(Error::Forbidden)?;
    if !menu.allows(section) {
        return Err(Error::Forbidden);
    }
    Ok(menu)
}

/// The context the module pages need.
fn context_of(menu: &PortalMenu, lang: &Lang, party: i64) -> invoices::PortalContext {
    invoices::PortalContext {
        home_url: format!("/portal/{party}"),
        can_write: menu.read_only_since.is_none(),
        read_only_notice: menu
            .read_only_since
            .map(|since| read_only_notice(lang, since)),
    }
}

/// `GET /portal/{party}/invoices`.
async fn invoices_index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
) -> Result<View> {
    let menu = menu_with(&state, &user, &lang, party, invoices::SECTION).await?;
    let context = context_of(&menu, &lang, party);
    invoices::invoices_page(&state, &lang, PartyId(party), context).await
}

/// The period of the statement.
#[derive(Deserialize, Default, Debug)]
pub struct Period {
    /// The first day.
    #[serde(default)]
    pub from: Option<String>,
    /// The last day.
    #[serde(default)]
    pub to: Option<String>,
}

/// `GET /portal/{party}/invoices/statement`.
async fn statement(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
    Query(period): Query<Period>,
) -> Result<View> {
    let menu = menu_with(&state, &user, &lang, party, invoices::SECTION).await?;
    let (first, last) = invoices::this_month();
    let parse = |text: &Option<String>| {
        text.as_deref()
            .and_then(|t| NaiveDate::parse_from_str(t.trim(), "%Y-%m-%d").ok())
    };
    let from = parse(&period.from).unwrap_or(first);
    let to = parse(&period.to).unwrap_or(last).max(from);
    let context = context_of(&menu, &lang, party);
    invoices::statement_page(&state, &lang, PartyId(party), from, to, context).await
}

/// `POST /portal/{party}/invoices/{id}/dispute`: `reason`, `amount` and
/// `line_<position>`.
async fn dispute(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((party, id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response> {
    menu_with(&state, &user, &lang, party, invoices::SECTION).await?;
    if !invoices::disputes_allowed(&state) {
        return Err(Error::Forbidden);
    }
    PortalGuard::require(&state, &lang, &user, PartyId(party), PortalAction::Dispute).await?;
    invoices::dispute(&state, &lang, PartyId(party), id, &form).await?;
    let done = lang.t("wl_invoicing.portal.dispute_done", &[]);
    let done = if done == "wl_invoicing.portal.dispute_done" {
        "We received your dispute and will come back to you.".to_owned()
    } else {
        done
    };
    Ok((Toast::success(done), renox::HxRefresh).into_response())
}

/// The routes, for every signed-in user (each handler checks the grant).
pub struct MoneyPortal;

impl Module for MoneyPortal {
    fn name(&self) -> &'static str {
        "money_portal"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/portal/{party}/invoices", invoices_index)
            .name("money_portal.invoices")
            .get("/portal/{party}/invoices/statement", statement)
            .name("money_portal.statement")
            .post("/portal/{party}/invoices/{id}/dispute", dispute)
            .name("money_portal.dispute")
            .require_auth()
    }
}
