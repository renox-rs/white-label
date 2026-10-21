//! Drafting a landed cost adjustment from a late supplier bill (wl-inventory
//! §4 *Landed cost*, §5.8 step 1, §6 *Consumes*, issue #229). When
//! wl-invoicing approves a supplier bill with lines coded to a landed cost
//! ([`SupplierBillApproved::landed_costs`]), nothing was posted for them; the
//! owner of the landed cost takes over:
//!
//! - a landed cost that is `allocated` gets a **draft** adjustment, one per
//!   bill (a second event for the same bill does nothing), with one cost line
//!   per bill line coded to it and the bill's reference, dated today; a
//!   second, different bill makes its own draft even while an earlier one is
//!   unposted, and drafts are never merged. The holders of the `procurement`
//!   role are told. Nothing is posted: the purchaser posts it
//!   ([`crate::landed_adjust::adjust`]), and `STK.ACL.06` applies then;
//! - a landed cost that is still `draft` gets the bill line as a cost line of
//!   the draft, once per bill line, and no adjustment;
//! - any other landed cost (cancelled, unknown) is left alone.
//!
//! Decisions: the amount and reference of a line are read from the bill line
//! through the kernel's `SupplierBills` when wl-invoicing is there (the line
//! amount in AED and the supplier's invoice number), else the event's amount
//! and `Bill <id>` are used. The kind of a cost line is told from the bill
//! line's description (*duty*, *clearing*, *insur...*), else freight.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::documents::today;
use wl_kernel::inventory::LANDED_COST_SUBJECT_TYPE;
use wl_kernel::invoicing::{SupplierBillApproved, supplier_bills};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::chase::PROCUREMENT_ROLE;
use crate::landed_adjust::{LandedCostAdjustment, NewAdjustment, create_adjustment};
use crate::landed_cost::{CostKind, CostLine, LandedCost, LandedCostStatus, NewCostLine};
use crate::texts::english;

/// The notification type of the notice to the purchaser.
pub const NOTICE: &str = "inventory.landed_cost_adjustment_drafted";

/// The notification type of the notice: staff, transactional, by e-mail.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "inventory")
        .group("landed_costs")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.landed.drafted_subject")),
            english("wl_inventory.landed.drafted_body"),
        )
}

/// Registers the notification type with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
}

/// The kind of a cost line told from the bill line's description.
pub fn kind_of(description: &str) -> CostKind {
    let text = description.to_lowercase();
    if text.contains("duty") || text.contains("customs") {
        CostKind::Duty
    } else if text.contains("clearing") || text.contains("clearance") {
        CostKind::Clearing
    } else if text.contains("insur") {
        CostKind::Insurance
    } else {
        CostKind::Freight
    }
}

/// The ids of the users who hold `role` globally.
async fn holders(db: &Db, role: &str) -> Result<Vec<i64>> {
    Ok(renox::auth::permissions::users_with_role_in(
        db,
        role,
        &renox::auth::permissions::Scope::global(),
    )
    .await?
    .iter()
    .map(|user| user.id)
    .collect())
}

/// A cost line for bill line `line` of bill `event.bill`: its amount, kind and
/// the bill's reference.
async fn cost_line_of(
    state: &AppState,
    event: &SupplierBillApproved,
    line: &wl_kernel::LandedCostLine,
) -> Result<NewCostLine> {
    let found = supplier_bills(state).line(&state.db, line.line).await?;
    let (amount, reference, description) = match found {
        Some(found) => (found.amount_aed, found.bill_number, found.description),
        None => (line.amount, format!("Bill {}", event.bill.0), String::new()),
    };
    let mut cost = NewCostLine::of(kind_of(&description), amount).bill(&reference);
    cost.description = description;
    cost.bill_line_id = Some(line.line.0);
    Ok(cost)
}

/// Handles the approval of a supplier bill: drafts the adjustments of the
/// allocated landed costs its lines are coded to, and adds the lines coded
/// to a draft landed cost to it. Repeating it changes nothing.
///
/// # Errors
///
/// A database error, or the notifier's.
pub async fn on_bill_approved(state: &AppState, event: &SupplierBillApproved) -> Result {
    let mut by_landed: BTreeMap<i64, Vec<&wl_kernel::LandedCostLine>> = BTreeMap::new();
    for line in &event.landed_costs {
        if line.landed_cost.subject_type == LANDED_COST_SUBJECT_TYPE {
            by_landed
                .entry(line.landed_cost.subject_id)
                .or_default()
                .push(line);
        }
    }
    for (landed_cost_id, lines) in by_landed {
        let Some(landed) = LandedCost::find(&state.db, landed_cost_id).await? else {
            continue;
        };
        match landed.status {
            LandedCostStatus::Allocated => draft_adjustment(state, event, &landed, &lines).await?,
            LandedCostStatus::Draft => add_to_draft(state, event, &landed, &lines).await?,
            LandedCostStatus::Cancelled => {}
        }
    }
    Ok(())
}

/// One draft adjustment of `landed` for the lines of bill `event.bill`, unless
/// the bill has one already; the purchaser is told.
async fn draft_adjustment(
    state: &AppState,
    event: &SupplierBillApproved,
    landed: &LandedCost,
    lines: &[&wl_kernel::LandedCostLine],
) -> Result {
    let known = LandedCostAdjustment::query()
        .where_(LandedCostAdjustment::LANDED_COST_ID.eq(landed.id))
        .where_(LandedCostAdjustment::BILL_ID.eq(event.bill.0))
        .exists(&state.db)
        .await?;
    if known {
        return Ok(());
    }
    let mut costs = Vec::new();
    for line in lines {
        costs.push(cost_line_of(state, event, line).await?);
    }
    let reference = costs
        .first()
        .and_then(|cost| cost.bill_reference.clone())
        .unwrap_or_else(|| format!("Bill {}", event.bill.0));
    let total: i64 = costs.iter().map(|cost| cost.amount).sum();
    let adjustment = create_adjustment(
        &state.db,
        None,
        landed.id,
        NewAdjustment {
            bill_reference: reference.clone(),
            bill_id: Some(event.bill.0),
            posting_date: today(),
            lines: costs,
        },
    )
    .await?;
    let people = holders(&state.db, PROCUREMENT_ROLE).await?;
    if people.is_empty() {
        return Ok(());
    }
    let number = landed.number.clone().unwrap_or_default();
    let mut request = NotifyRequest::new(NOTICE, adjustment.subject())
        .subject_label(&number)
        .data(json!({
            "number": number,
            "bill": reference,
            "amount": crate::po_revise::money_text(total),
            "link": format!("/landed-costs/{}/adjustments/{}", landed.id, adjustment.id),
        }));
    for id in people {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Adds the bill lines to the cost lines of the draft `landed`, those not
/// there yet.
async fn add_to_draft(
    state: &AppState,
    event: &SupplierBillApproved,
    landed: &LandedCost,
    lines: &[&wl_kernel::LandedCostLine],
) -> Result {
    let mut position = i64::try_from(
        CostLine::query()
            .where_(CostLine::LANDED_COST_ID.eq(landed.id))
            .count(&state.db)
            .await?,
    )
    .unwrap_or(0);
    for line in lines {
        let there = CostLine::query()
            .where_(CostLine::LANDED_COST_ID.eq(landed.id))
            .where_(CostLine::BILL_LINE_ID.eq(line.line.0))
            .exists(&state.db)
            .await?;
        if there {
            continue;
        }
        let cost = cost_line_of(state, event, line).await?;
        position += 1;
        let mut row = CostLine {
            landed_cost_id: landed.id,
            adjustment_id: None,
            position,
            kind: cost.kind,
            description: cost.description,
            amount: cost.amount,
            bill_line_id: cost.bill_line_id,
            bill_reference: cost.bill_reference,
            ..CostLine::default()
        };
        row.insert(&state.db).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_is_told_from_the_description() {
        assert_eq!(kind_of("Customs duty 5%"), CostKind::Duty);
        assert_eq!(kind_of("Port CLEARING fee"), CostKind::Clearing);
        assert_eq!(kind_of("Marine insurance"), CostKind::Insurance);
        assert_eq!(kind_of("Sea freight"), CostKind::Freight);
        assert_eq!(kind_of(""), CostKind::Freight);
    }
}
