//! Where a sales order is delivered, and the day a line is promised for
//! (wl-sales §3 *Sales order*, §6 `DeliveryDestination`, `PromisedLines`,
//! `OrderLineRules`; BL-MFG-11 points 6 and 7 of the 07 vertical).
//!
//! - **The delivery.** An order carries a delivery address (free text), an
//!   emirate, a country (ISO, `AE` for the UAE) and the answer *domestic* or
//!   *export*, which is the VAT category of the address: a domestic delivery
//!   is standard rated, an export is zero rated with evidence. The answer is
//!   what the person chose; without a choice a country decides it (`AE` is
//!   domestic, any other is export); a choice that contradicts the country is
//!   refused. A draft's delivery is changed from its page ([`set`]); a placed
//!   order's through `amend` ([`crate::amend::AmendInput::delivery`]).
//! - **Order line rules.** At `submit` and at `amend`, [`ensure_lines_allowed`]
//!   asks the app's kernel [`OrderLineRules`] about each item line (the
//!   vertical says whether a line is allowed on a *domestic* order: 07 refuses
//!   an *export only* item) and refuses the order with a 422 that names the
//!   items. Without a provider the default accepts every line. Decision: an
//!   order that does not say where it goes counts as **domestic** for this
//!   check (07 §3.3: *domestic delivery: the VAT category of the delivery
//!   address is not export*), so the ban cannot be passed by leaving the
//!   delivery blank. An amendment checks the lines it adds or enlarges, and
//!   every line when it changes the delivery.
//! - **Providers.** [`Destination`] is the kernel's [`DeliveryDestination`]
//!   (`None` for a record that is not a sales order or an order that does not
//!   say); [`PromisedDays`] is the kernel's [`PromisedLines`]: the item lines of
//!   `confirmed` and `partially_fulfilled` orders with an open quantity, that
//!   no dispatch picked yet, promised on or after the day asked, oldest first.
//!   An item is found by its code through the stock service.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::{LOCATION_SUBJECT_TYPE, stock_service};
use wl_kernel::sales::{
    DeliveryDestination, PromisedLine, PromisedLines, SALES_ORDER_SUBJECT_TYPE, SalesFuture,
    order_line_rules,
};

use crate::sale_line::{KIND_ITEM, SaleLine};
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::texts::english;

/// A delivery inside the UAE (standard rated).
pub const DOMESTIC: &str = "domestic";

/// A delivery abroad (zero rated export).
pub const EXPORT: &str = "export";

/// The country code of the UAE.
pub const HOME_COUNTRY: &str = "AE";

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The delivery of an order, as entered. Blank text is no answer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Delivery {
    /// The delivery address, as typed.
    pub address: Option<String>,
    /// The emirate.
    pub emirate: Option<String>,
    /// The country, ISO code, any case.
    pub country: Option<String>,
    /// `domestic` or `export`; blank to let the country decide.
    pub destination: Option<String>,
}

impl Delivery {
    /// A delivery with nothing said yet.
    pub fn new() -> Self {
        Delivery::default()
    }

    /// Entered from the text of a form: blank text is no answer.
    pub fn of(address: &str, emirate: &str, country: &str, destination: &str) -> Self {
        let value = |text: &str| (!text.trim().is_empty()).then(|| text.trim().to_owned());
        Delivery {
            address: value(address),
            emirate: value(emirate),
            country: value(country).map(|c| c.to_uppercase()),
            destination: value(destination).map(|d| d.to_lowercase()),
        }
    }

    /// To `address`.
    pub fn address(mut self, address: &str) -> Self {
        self.address = Some(address.trim().to_owned()).filter(|a| !a.is_empty());
        self
    }

    /// In `emirate`.
    pub fn emirate(mut self, emirate: &str) -> Self {
        self.emirate = Some(emirate.trim().to_owned()).filter(|a| !a.is_empty());
        self
    }

    /// In the country `country` (ISO code).
    pub fn country(mut self, country: &str) -> Self {
        self.country = Some(country.trim().to_uppercase()).filter(|a| !a.is_empty());
        self
    }

    /// Delivered inside the UAE.
    pub fn domestic(mut self) -> Self {
        self.destination = Some(DOMESTIC.to_owned());
        self
    }

    /// Delivered abroad.
    pub fn export(mut self) -> Self {
        self.destination = Some(EXPORT.to_owned());
        self
    }

    /// Whether the delivery says nothing at all.
    pub fn is_blank(&self) -> bool {
        self.address.is_none()
            && self.emirate.is_none()
            && self.country.is_none()
            && self.destination.is_none()
    }

    /// The answer the delivery gives, [`DOMESTIC`] or [`EXPORT`]: the choice
    /// made, else the country's, else none.
    ///
    /// # Errors
    ///
    /// 422 for a choice other than domestic or export, or one that
    /// contradicts the country.
    pub fn answer(&self) -> Result<Option<&'static str>> {
        let by_country = self.country.as_deref().map(|country| {
            if country.eq_ignore_ascii_case(HOME_COUNTRY) {
                DOMESTIC
            } else {
                EXPORT
            }
        });
        let chosen = match self.destination.as_deref() {
            None => None,
            Some(DOMESTIC) => Some(DOMESTIC),
            Some(EXPORT) => Some(EXPORT),
            Some(_) => return Err(refused(english("wl_sales.delivery.problem.choice", &[]))),
        };
        match (chosen, by_country) {
            (Some(DOMESTIC), Some(EXPORT)) => Err(refused(english(
                "wl_sales.delivery.problem.domestic_abroad",
                &[("country", &self.country.clone().unwrap_or_default())],
            ))),
            (Some(EXPORT), Some(DOMESTIC)) => Err(refused(english(
                "wl_sales.delivery.problem.export_home",
                &[],
            ))),
            (Some(chosen), _) => Ok(Some(chosen)),
            (None, found) => Ok(found),
        }
    }

    /// The delivery an order holds.
    pub fn of_order(order: &SalesOrder) -> Self {
        Delivery {
            address: order.delivery_address.clone(),
            emirate: order.delivery_emirate.clone(),
            country: order.delivery_country.clone(),
            destination: order.delivery_destination.clone(),
        }
    }
}

/// Whether the order delivers abroad: `Some(true)` for an export,
/// `Some(false)` for a domestic delivery, `None` when it does not say.
pub fn is_export(order: &SalesOrder) -> Option<bool> {
    order
        .delivery_destination
        .as_deref()
        .map(|destination| destination == EXPORT)
}

/// Writes `delivery` on the draft order `id` after checking it.
///
/// # Errors
///
/// 422 when the order is not a draft or the delivery contradicts itself.
pub async fn set(state: &AppState, id: i64, delivery: Delivery) -> Result<SalesOrder> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    if !order.is_draft() {
        return Err(refused(english("wl_sales.delivery.problem.not_draft", &[])));
    }
    let answer = delivery.answer()?;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(id))
        .update(
            &state.db,
            &[
                ("delivery_address", &delivery.address),
                ("delivery_emirate", &delivery.emirate),
                ("delivery_country", &delivery.country),
                ("delivery_destination", &answer.map(str::to_owned)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    SalesOrder::find_or_404(&state.db, id).await
}

/// Asks the vertical's [`OrderLineRules`](wl_kernel::sales::OrderLineRules)
/// about the item lines `codes` of an order delivered abroad (`export`) or
/// not. Nothing is asked of an export, or when no item has such a code; the
/// words of every refused line are the 422 (one per line).
///
/// # Errors
///
/// 422 naming the refused items.
pub async fn ensure_lines_allowed<'a>(
    state: &AppState,
    export: Option<bool>,
    codes: impl IntoIterator<Item = &'a str>,
) -> Result {
    if export == Some(true) {
        return Ok(());
    }
    let rules = order_line_rules(state);
    let stock = stock_service(state);
    let place = Subject::new(LOCATION_SUBJECT_TYPE, 0);
    let mut seen: Vec<&str> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    for code in codes {
        let code = code.trim();
        if code.is_empty() || seen.contains(&code) {
            continue;
        }
        seen.push(code);
        let Some(found) = stock.choices(state, code, &place).await? else {
            continue;
        };
        if let Some(refusal) = rules.refuse_line(state, &found.item, true).await? {
            words.push(refusal);
        }
    }
    if words.is_empty() {
        Ok(())
    } else {
        Err(refused(words.join("\n")))
    }
}

/// The item lines of `lines` that still count on the order: not cancelled in
/// full.
pub fn live_codes(lines: &[SaleLine]) -> Vec<&str> {
    lines
        .iter()
        .filter(|line| line.kind == KIND_ITEM && line.cancelled_milli < line.quantity_milli)
        .map(|line| line.item_code.as_str())
        .collect()
}

/// The kernel's [`DeliveryDestination`].
pub(crate) struct Destination;

impl DeliveryDestination for Destination {
    fn is_export<'a>(
        &'a self,
        state: &'a AppState,
        order: &'a Subject,
    ) -> SalesFuture<'a, Option<bool>> {
        Box::pin(async move {
            if order.subject_type != SALES_ORDER_SUBJECT_TYPE {
                return Ok(None);
            }
            Ok(SalesOrder::find(&state.db, order.subject_id)
                .await?
                .and_then(|found| is_export(&found)))
        })
    }
}

/// The kernel's [`PromisedLines`].
pub(crate) struct PromisedDays;

impl PromisedLines for PromisedDays {
    fn of_item<'a>(
        &'a self,
        state: &'a AppState,
        item: &'a Subject,
        from: NaiveDate,
    ) -> SalesFuture<'a, Vec<PromisedLine>> {
        Box::pin(async move {
            let Some(label) = stock_service(state).item_label(state, item).await? else {
                return Ok(Vec::new());
            };
            let lines = SaleLine::query()
                .where_(SaleLine::SUBJECT_TYPE.eq(SALES_ORDER_SUBJECT_TYPE.to_owned()))
                .where_(SaleLine::ITEM_CODE.eq(label.code))
                .order_by(SaleLine::ID)
                .get(&state.db)
                .await?;
            let mut found: Vec<PromisedLine> = Vec::new();
            let mut orders: Vec<SalesOrder> = Vec::new();
            for line in lines {
                let Some(promised) = line.promised_on.filter(|day| *day >= from) else {
                    continue;
                };
                if line.kind != KIND_ITEM || line.open_milli() <= 0 {
                    continue;
                }
                if !orders.iter().any(|order| order.id == line.subject_id)
                    && let Some(order) = SalesOrder::find(&state.db, line.subject_id).await?
                {
                    orders.push(order);
                }
                let Some(order) = orders.iter().find(|order| order.id == line.subject_id) else {
                    continue;
                };
                if !matches!(
                    order.status,
                    SalesOrderStatus::Confirmed | SalesOrderStatus::PartiallyFulfilled
                ) {
                    continue;
                }
                // Picked goods are on their way: only lines no dispatch took.
                let progress =
                    crate::progress::of_lines(state, order, std::slice::from_ref(&line)).await?;
                if progress.get(&line.id) != Some(&"not_dispatched") {
                    continue;
                }
                found.push(PromisedLine {
                    order: order.order_id(),
                    line: line.id,
                    number: Some(order.number.clone()),
                    item: item.clone(),
                    promised,
                });
            }
            found.sort_by_key(|line| (line.promised, line.order.0, line.line));
            Ok(found)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_country_decides_when_nothing_is_chosen() {
        let home = Delivery::new().country("ae");
        assert_eq!(home.answer().unwrap(), Some(DOMESTIC));
        let abroad = Delivery::new().country("om");
        assert_eq!(abroad.answer().unwrap(), Some(EXPORT));
        assert_eq!(Delivery::new().answer().unwrap(), None);
    }

    #[test]
    fn a_choice_that_contradicts_the_country_is_refused() {
        assert!(Delivery::new().country("OM").domestic().answer().is_err());
        assert!(Delivery::new().country("AE").export().answer().is_err());
        assert_eq!(
            Delivery::new().export().answer().unwrap(),
            Some(EXPORT),
            "an export needs no country"
        );
        assert!(Delivery::of("", "", "", "elsewhere").answer().is_err());
    }
}
