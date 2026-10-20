//! The public web form of an enquiry (wl-sales §4.1, issue #139 point 1): a
//! link `…/enquiries?c=SPRING26` carries the campaign code, and the posted
//! form becomes a lead through [`lead::capture`](crate::lead::capture) with
//! the source the campaign implies (or *website* with no campaign). The form
//! needs no login; a second submission for an open lead of the same party is
//! a note on it, which `capture` decides.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /enquiries?c=CODE` | `wl_sales.enquiry` | nothing (public) |
//! | `GET /enquiries/thanks` | `wl_sales.enquiry.thanks` | nothing (public) |

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::Deserialize;

use crate::lead::{self, Captured, NewLead};

/// The query of the form link.
#[derive(Debug, Deserialize)]
struct EnquiryQuery {
    /// The campaign code of the link.
    c: Option<String>,
}

/// The public routes.
pub fn routes() -> Routes {
    Routes::new()
        .post("/enquiries", submit)
        .name("wl_sales.enquiry")
        .get("/enquiries/thanks", thanks)
        .name("wl_sales.enquiry.thanks")
}

/// `POST /enquiries?c=CODE`: captures the lead and thanks the visitor.
async fn submit(
    State(state): State<AppState>,
    Query(query): Query<EnquiryQuery>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let name = form.get("name").map_or("", |v| v.trim()).to_owned();
    let interest = form.get("interest").map_or("", |v| v.trim()).to_owned();
    if name.is_empty() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            crate::texts::english("wl_sales.enquiry.name_required", &[]),
        ));
    }
    let mut input = NewLead::web_form(None, &name).interest(&interest);
    if let Some(code) = query.c.as_deref() {
        input = input.campaign_code(code);
    }
    match lead::capture(&state, input, wl_kernel::documents::today()).await? {
        Ok(Captured::Created(_) | Captured::Noted(_)) => {
            Ok(Redirect::route("wl_sales.enquiry.thanks", &[])?.into_response())
        }
        Err(refused) => Err(refused.into_error()),
    }
}

/// `GET /enquiries/thanks`.
async fn thanks() -> Result<Response> {
    Ok(crate::texts::english("wl_sales.enquiry.thanks", &[]).into_response())
}
