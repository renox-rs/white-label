//! The buyer's PO upload in the portal (07 Manufacturing §4.1 row "The same
//! customer PO...", §4.5 step 2; BL-MFG-06, #387; wl-sales `BL-SAL-18`).
//!
//! The buyer enters the customer PO number, a call-off number when the PO is
//! a blanket one, and a note, at `/portal/manufacturing/orders`. Before
//! anything is kept the number is asked of wl-sales through the kernel's
//! `CustomerPoCheck` (the setting *Unique customer PO number per customer*,
//! on for this vertical in the host): when an order of the customer already
//! holds it, trimmed and case-insensitive with its call-off number (a
//! cancelled order still holds it), the upload is refused and the buyer is
//! shown the number and the status of that order, nothing else - no price, no
//! note, nobody else's order. The same PO number with a new call-off number
//! is accepted, and so is the same number for another customer.
//!
//! An accepted upload is a [`PoUpload`] with a unique key on (customer, PO,
//! call-off), so two uploads at once keep one, and sales is told to enter the
//! order. It creates no order and no reservation; the order entry in
//! wl-sales checks the same number again at `submit`.
//!
//! Decisions: the portal grant decides which customer the buyer acts for (the
//! first one, or the one the form names when the buyer acts for several); a
//! person acting for none gets 403.

use std::collections::BTreeMap;

use renox::axum::http::HeaderMap;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::portal::portal_users;
use wl_kernel::sales::customer_po_check;
use wl_kernel::{PartyId, Subject};

use crate::db::refused;
use crate::notices;
use crate::permissions;
use crate::screens::{Texts, date_text, done};
use crate::texts::{english, screen_texts, t};

/// The portal page of the uploads.
pub const URL: &str = "/portal/manufacturing/orders";

/// A customer PO a buyer uploaded.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manufacturing_po_uploads",
    unique(party_id, po_key, call_off_key)
)]
pub struct PoUpload {
    /// The primary key.
    pub id: i64,
    /// The customer.
    pub party_id: i64,
    /// The PO number as entered.
    pub po: String,
    /// The call-off number as entered; empty for none.
    pub call_off: String,
    /// The PO number, trimmed and lower case.
    pub po_key: String,
    /// The call-off number, trimmed and lower case.
    pub call_off_key: String,
    /// The buyer's note.
    pub note: String,
    /// The buyer.
    pub uploaded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

fn key_of(text: &str) -> String {
    text.trim().to_lowercase()
}

/// The words that refuse a duplicate: the number and status of the order.
fn duplicate(po: &str, call_off: &str, number: &str, status: &str) -> Error {
    let message = if call_off.trim().is_empty() {
        english(
            "manufacturing.po.duplicate",
            &[("po", &po.trim()), ("number", &number), ("status", &status)],
        )
    } else {
        english(
            "manufacturing.po.duplicate_call_off",
            &[
                ("po", &po.trim()),
                ("call_off", &call_off.trim()),
                ("number", &number),
                ("status", &status),
            ],
        )
    };
    refused(message)
}

/// Keeps the PO `po` / `call_off` of `customer` uploaded by `by`.
///
/// # Errors
///
/// A 422 for an empty number, an order of the customer that holds the number
/// (named by its number and status only) or an upload of it already.
pub async fn upload(
    state: &AppState,
    by: &AuthUser,
    customer: PartyId,
    po: &str,
    call_off: &str,
    note: &str,
) -> Result<PoUpload> {
    if po.trim().is_empty() {
        return Err(refused(english("manufacturing.po.empty", &[])));
    }
    if let Some(holder) = customer_po_check(state)
        .holder(state, customer, po, call_off)
        .await?
    {
        return Err(duplicate(po, call_off, &holder.number, &holder.status));
    }
    let (po_key, call_off_key) = (key_of(po), key_of(call_off));
    let known = PoUpload::query()
        .where_(PoUpload::PARTY_ID.eq(customer.0))
        .where_(PoUpload::PO_KEY.eq(po_key.clone()))
        .where_(PoUpload::CALL_OFF_KEY.eq(call_off_key.clone()))
        .first(&state.db)
        .await?;
    if known.is_some() {
        return Err(refused(english(
            "manufacturing.po.uploaded",
            &[("po", &po.trim())],
        )));
    }
    let saved = PoUpload::create(
        &state.db,
        PoUpload {
            party_id: customer.0,
            po: po.trim().to_owned(),
            call_off: call_off.trim().to_owned(),
            po_key,
            call_off_key,
            note: note.trim().to_owned(),
            uploaded_by: Some(by.id),
            ..PoUpload::default()
        },
    )
    .await
    .map_err(|error| {
        if error.is_unique_violation() {
            refused(english("manufacturing.po.uploaded", &[("po", &po.trim())]))
        } else {
            error
        }
    })?;
    let sales = notices::holders(state, permissions::SALES_ROLE, 0).await?;
    notices::tell(
        state,
        Subject::new("manufacturing.po_upload", saved.id),
        &saved.po,
        &sales,
        &english("manufacturing.po.told_title", &[("po", &saved.po)]),
        &english(
            "manufacturing.po.told_body",
            &[("po", &saved.po), ("call_off", &saved.call_off)],
        ),
    )
    .await?;
    Ok(saved)
}

/// The companies `user` acts for, or 403.
async fn parties(state: &AppState, user: &AuthUser) -> Result<Vec<PartyId>> {
    let mut found: Vec<PartyId> = Vec::new();
    for grant in portal_users(state)
        .parties_of(&state.db, user.id, today())
        .await?
    {
        if !found.contains(&grant.acting_for) {
            found.push(grant.acting_for);
        }
    }
    if found.is_empty() {
        return Err(Error::Forbidden);
    }
    Ok(found)
}

/// An upload of the buyer's company.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Row {
    po: String,
    call_off: String,
    note: String,
    day: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/portal_orders.html")]
pub(crate) struct OrdersPage {
    tx: Texts,
    rows: Vec<Row>,
}

/// `GET /portal/manufacturing/orders`.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
) -> Result<Response> {
    let mut rows = Vec::new();
    for party in parties(&state, &user).await? {
        for found in PoUpload::query()
            .where_(PoUpload::PARTY_ID.eq(party.0))
            .order_by_desc(PoUpload::ID)
            .get(&state.db)
            .await?
        {
            rows.push(Row {
                po: found.po,
                call_off: if found.call_off.is_empty() {
                    t(&lang, "manufacturing.screen.not_set")
                } else {
                    found.call_off
                },
                note: found.note,
                day: found.created_at.map_or_else(String::new, |at| {
                    date_text(wl_kernel::documents::dubai_date(at))
                }),
            });
        }
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        return Ok(Json(&rows).into_response());
    }
    Ok(View::typed(OrdersPage {
        tx: screen_texts(&lang),
        rows,
    })
    .into_response())
}

/// `POST /portal/manufacturing/orders`: `po`, `call_off`, `note`.
pub(crate) async fn post(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let acting = parties(&state, &user).await?;
    let customer = form
        .get("party_id")
        .and_then(|id| id.trim().parse::<i64>().ok())
        .map(PartyId)
        .filter(|party| acting.contains(party))
        .unwrap_or(acting[0]);
    let get = |name: &str| form.get(name).map_or("", |value| value.trim());
    upload(
        &state,
        &user,
        customer,
        get("po"),
        get("call_off"),
        get("note"),
    )
    .await?;
    Ok(done(&lang, URL, "manufacturing.screen.po.done"))
}
