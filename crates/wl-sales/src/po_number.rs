//! The customer PO number of an order (wl-sales §3 *Sales order*, §4.2
//! *Customer PO number*, §7 *Unique customer PO number per customer*;
//! BL-SAL-18).
//!
//! With the setting on (07 Manufacturing), `submit` refuses a customer PO
//! reference that another order of the same customer already uses, compared
//! trimmed and case-insensitive together with its call-off number (a call-off
//! against a blanket PO carries its own). Cancelled and rejected orders still
//! count; the same number for another customer is fine; an order without a
//! PO reference is not checked. The refusal names the existing order by its
//! number and status only, which is also what a buyer on the portal is shown.
//!
//! Decisions:
//! - The key is a row ([`PoKey`]) written by the `submit` transition in its
//!   own transaction, with a unique index on (customer, reference, call-off),
//!   so of two concurrent submits only one passes; a lost race is answered
//!   like any other duplicate.
//! - A draft's reference is editable ([`set_reference`]) and checked when
//!   changed, as an early warning; the key is claimed at `submit`.
//! - `amend` (BL-SAL-05) changes the key of a submitted order; it is not in
//!   this branch. It calls [`ensure_free`] with the order it amends as the
//!   exception and moves the key, as `submit` does for a draft.
//! - The portal upload screen of 07 (BL-MFG-06) shows [`buyer_refusal`]: the
//!   number and the status of the existing order, nothing else.

use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::config_of;
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::texts::english;

/// The stored key of a PO reference and its call-off number: trimmed and
/// lower case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// The customer PO reference.
    pub po_key: String,
    /// The call-off number, empty when the order has none.
    pub call_off_key: String,
}

impl Reference {
    /// The key of `customer_po` and `call_off`; `None` when there is no PO
    /// reference (such an order is not checked).
    pub fn of(customer_po: &str, call_off: &str) -> Option<Reference> {
        let po_key = key_of(customer_po);
        (!po_key.is_empty()).then(|| Reference {
            po_key,
            call_off_key: key_of(call_off),
        })
    }
}

/// `text` trimmed and lower case.
pub fn key_of(text: &str) -> String {
    text.trim().to_lowercase()
}

/// The key one submitted order holds on its customer's PO number.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_order_po_keys",
    unique(order_id),
    unique(party_id, po_key, call_off_key)
)]
pub struct PoKey {
    /// The primary key.
    pub id: i64,
    /// The order that holds the number.
    #[model(references = "wl_sales_sales_orders")]
    pub order_id: i64,
    /// The customer.
    pub party_id: i64,
    /// The reference, trimmed and lower case.
    pub po_key: String,
    /// The call-off number, trimmed and lower case; empty for none.
    pub call_off_key: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The order of `party` that holds `reference`, other than `except`.
pub async fn holder_of(
    db: &Db,
    party: i64,
    reference: &Reference,
    except: Option<i64>,
) -> Result<Option<SalesOrder>> {
    let key = PoKey::query()
        .where_(PoKey::PARTY_ID.eq(party))
        .where_(PoKey::PO_KEY.eq(reference.po_key.clone()))
        .where_(PoKey::CALL_OFF_KEY.eq(reference.call_off_key.clone()))
        .first(db)
        .await?;
    match key {
        Some(key) if Some(key.order_id) != except => SalesOrder::find(db, key.order_id).await,
        _ => Ok(None),
    }
}

/// The refusal for a PO number that `holder` already carries: the number and
/// the status of its order, in English.
fn duplicate(holder: &SalesOrder, customer_po: &str, call_off: &str) -> Error {
    let status = english(
        &format!("wl_sales.orders.status.{}", holder.status.as_str()),
        &[],
    );
    let (po, number, call) = (customer_po.trim(), &holder.number, call_off.trim());
    let message = if call.is_empty() {
        english(
            "wl_sales.po.duplicate",
            &[("po", &po), ("number", number), ("status", &status)],
        )
    } else {
        english(
            "wl_sales.po.duplicate_call_off",
            &[
                ("po", &po),
                ("call_off", &call),
                ("number", number),
                ("status", &status),
            ],
        )
    };
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// 422 naming the existing order when another order of `party` (not
/// `except`, the order being amended) already uses the PO reference and
/// call-off number; nothing for an empty reference. Checks regardless of the
/// setting: the callers ask [`is_on`] first.
pub async fn ensure_free(
    db: &Db,
    party: i64,
    customer_po: &str,
    call_off: &str,
    except: Option<i64>,
) -> Result {
    let Some(reference) = Reference::of(customer_po, call_off) else {
        return Ok(());
    };
    match holder_of(db, party, &reference, except).await? {
        Some(holder) => Err(duplicate(&holder, customer_po, call_off)),
        None => Ok(()),
    }
}

/// Whether the setting *Unique customer PO number per customer* is on.
pub fn is_on(state: &AppState) -> bool {
    config_of(state).unique_customer_po
}

/// The key `submit` must claim for `order`: checks the number is free (422
/// naming the existing order) and returns the reference to store; `None`
/// when the setting is off or the order has no PO reference.
pub async fn claim_for(state: &AppState, order: &SalesOrder) -> Result<Option<Reference>> {
    if !is_on(state) {
        return Ok(None);
    }
    let customer_po = order.customer_po.as_deref().unwrap_or_default();
    let call_off = order.call_off.as_deref().unwrap_or_default();
    ensure_free(
        &state.db,
        order.party_id,
        customer_po,
        call_off,
        Some(order.id),
    )
    .await?;
    Ok(Reference::of(customer_po, call_off))
}

/// Answers a failed `submit` that may have lost the race for the unique key:
/// the refusal naming the order that holds the number, else the original
/// error.
pub async fn explain_race(state: &AppState, order: &SalesOrder, error: Error) -> Error {
    if !is_on(state) {
        return error;
    }
    let customer_po = order.customer_po.as_deref().unwrap_or_default();
    let call_off = order.call_off.as_deref().unwrap_or_default();
    match ensure_free(
        &state.db,
        order.party_id,
        customer_po,
        call_off,
        Some(order.id),
    )
    .await
    {
        Err(refusal) => refusal,
        Ok(()) => error,
    }
}

/// Changes the PO reference and call-off number of the draft order `id`.
/// With the setting on, a number another order of the customer holds is
/// refused.
///
/// # Errors
///
/// 422 when the order is not a draft, or the number is already used.
pub async fn set_reference(
    state: &AppState,
    id: i64,
    customer_po: &str,
    call_off: &str,
) -> Result<SalesOrder> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    if !order.is_draft() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("wl_sales.po.not_draft", &[]),
        ));
    }
    if is_on(state) {
        ensure_free(
            &state.db,
            order.party_id,
            customer_po,
            call_off,
            Some(order.id),
        )
        .await?;
    }
    let value = |text: &str| (!text.trim().is_empty()).then(|| text.trim().to_owned());
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(id))
        .update(
            &state.db,
            &[
                ("customer_po", &value(customer_po)),
                ("call_off", &value(call_off)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    SalesOrder::find_or_404(&state.db, id).await
}

/// What a buyer on the portal is told when the PO they upload is a duplicate:
/// the existing order's number and status, nothing else (no prices, no
/// notes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuyerRefusal {
    /// The existing order's number.
    pub number: String,
    /// Its status, in words.
    pub status: String,
}

impl BuyerRefusal {
    /// The sentence for the buyer.
    pub fn text(&self) -> String {
        english(
            "wl_sales.po.portal_duplicate",
            &[("number", &self.number), ("status", &self.status)],
        )
    }
}

/// The refusal a buyer of `party` gets for uploading the PO `customer_po`
/// with `call_off`, if one of the customer's own orders already holds it.
/// Only that customer's orders are looked at.
pub async fn buyer_refusal(
    state: &AppState,
    party: i64,
    customer_po: &str,
    call_off: &str,
) -> Result<Option<BuyerRefusal>> {
    if !is_on(state) {
        return Ok(None);
    }
    let Some(reference) = Reference::of(customer_po, call_off) else {
        return Ok(None);
    };
    Ok(holder_of(&state.db, party, &reference, None)
        .await?
        .map(|holder| BuyerRefusal {
            number: holder.number.clone(),
            status: english(
                &format!("wl_sales.orders.status.{}", holder.status.as_str()),
                &[],
            ),
        }))
}

/// The order of its customer that already holds the PO number of the draft
/// `order`, if the setting is on: what the order page warns about.
pub async fn clash_of(state: &AppState, order: &SalesOrder) -> Result<Option<SalesOrder>> {
    if !is_on(state) || order.status != SalesOrderStatus::Draft {
        return Ok(None);
    }
    let Some(reference) = Reference::of(
        order.customer_po.as_deref().unwrap_or_default(),
        order.call_off.as_deref().unwrap_or_default(),
    ) else {
        return Ok(None);
    };
    holder_of(&state.db, order.party_id, &reference, Some(order.id)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_is_trimmed_and_lower_case_and_empty_means_unchecked() {
        let reference = Reference::of("  PO-8812 ", "").unwrap();
        assert_eq!(reference.po_key, "po-8812");
        assert_eq!(reference.call_off_key, "");
        assert_eq!(
            Reference::of("BL-1", " CO-1 ").unwrap().call_off_key,
            "co-1"
        );
        assert!(Reference::of("   ", "CO-1").is_none());
    }

    #[test]
    fn the_refusal_names_the_order_and_its_status_only() {
        let holder = SalesOrder {
            number: "SO-2026-0070".into(),
            status: SalesOrderStatus::Cancelled,
            ..SalesOrder::default()
        };
        let Error::Status(code, message) = duplicate(&holder, "po-8812 ", "") else {
            panic!("a status error");
        };
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("SO-2026-0070") && message.contains("Cancelled"));
        assert!(message.contains("po-8812"));
        let Error::Status(_, with_call_off) = duplicate(&holder, "BL-1", "CO-1") else {
            panic!("a status error");
        };
        assert!(with_call_off.contains("CO-1"));
    }
}
