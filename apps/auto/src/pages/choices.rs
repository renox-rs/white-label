//! The pick lists of the forms that named a record of another module by a
//! typed id: the payer agreement of an insurer, the zero-rated invoice of an
//! export sale, the return to the supplier of a warranty claim, and the fleet
//! contract of a vehicle. Each answers as the person types (`?q=`) or with the
//! labels of the values the field holds (`?values=`), like the vehicle and the
//! party pickers.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /auto/fleet-contract-options` | `auto.fleet_contract_options` | `vehicles.manage` |
//! | `GET /auto/payer-agreement-options` | `auto.payer_agreement_options` | `insurance.manage` |
//! | `GET /auto/export-invoice-options` | `auto.export_invoice_options` | `export_sales.manage` |
//! | `GET /auto/supplier-return-options` | `auto.supplier_return_options` | `supplier_claims.manage` |
//!
//! The other module's rows are read with SQL on its tables (the contracts of
//! the kernel have no search for them); a deployment without the module has
//! no rows there and the list is empty.

use renox::prelude::*;
use renox::select::{OptionQuery, SelectOption};

use super::{party_name, require};
use crate::fleet_contract::{FleetContract, FleetStatus};
use crate::permissions;

/// The routes of the pick lists.
pub fn routes() -> Routes {
    Routes::new()
        .get("/auto/fleet-contract-options", fleet_contracts)
        .name("auto.fleet_contract_options")
        .get("/auto/payer-agreement-options", payer_agreements)
        .name("auto.payer_agreement_options")
        .get("/auto/export-invoice-options", export_invoices)
        .name("auto.export_invoice_options")
        .get("/auto/supplier-return-options", supplier_returns)
        .name("auto.supplier_return_options")
        .require_auth()
}

/// `%text%` for a `LIKE`, the wildcards of the text itself taken out.
fn pattern(text: &str) -> String {
    let clean: String = text
        .chars()
        .filter(|c| !matches!(c, '%' | '_' | '\\'))
        .collect();
    format!("%{}%", clean.trim().to_lowercase())
}

/// The name of payer agreement `id` of Invoicing, when there is one.
pub(crate) async fn payer_agreement_name(db: &Db, id: i64) -> Option<String> {
    renox::db::sql("SELECT name FROM wl_invoicing_payer_agreements WHERE id = ?")
        .bind(id)
        .scalar_optional::<String>(db)
        .await
        .ok()
        .flatten()
}

/// `GET /auto/fleet-contract-options`: the contracts a vehicle can be put
/// under (not the ended ones), as `FC-0007 · name`.
async fn fleet_contracts(
    State(state): State<AppState>,
    user: AuthUser,
    query: OptionQuery,
) -> Result<Json<Vec<SelectOption>>> {
    require(&user, permissions::VEHICLES_MANAGE)?;
    let found = if query.is_lookup() {
        let mut found = Vec::new();
        for id in query.values_as::<i64>() {
            found.extend(FleetContract::find(&state.db, id).await?);
        }
        found
    } else {
        let wanted = query.q.trim().to_lowercase();
        let mut all = FleetContract::query()
            .order_by_desc(FleetContract::ID)
            .get(&state.db)
            .await?;
        all.retain(|contract| {
            contract.status != FleetStatus::Ended
                && format!("{} {}", contract.number(), contract.title)
                    .to_lowercase()
                    .contains(&wanted)
        });
        all.truncate(20);
        all
    };
    Ok(Json(
        found
            .into_iter()
            .map(|contract| {
                SelectOption::new(
                    contract.id,
                    format!("{} · {}", contract.number(), contract.title),
                )
            })
            .collect(),
    ))
}

/// `GET /auto/payer-agreement-options`: the active payer agreements of
/// Invoicing, by name.
async fn payer_agreements(
    State(state): State<AppState>,
    user: AuthUser,
    query: OptionQuery,
) -> Result<Json<Vec<SelectOption>>> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let rows: Vec<(i64, String)> = if query.is_lookup() {
        let mut rows = Vec::new();
        for id in query.values_as::<i64>() {
            rows.extend(
                renox::db::sql("SELECT id, name FROM wl_invoicing_payer_agreements WHERE id = ?")
                    .bind(id)
                    .fetch_as::<(i64, String)>(&state.db)
                    .await
                    .unwrap_or_default(),
            );
        }
        rows
    } else {
        renox::db::sql(
            "SELECT id, name FROM wl_invoicing_payer_agreements \
             WHERE active = ? AND LOWER(name) LIKE ? ORDER BY name LIMIT 20",
        )
        .bind(true)
        .bind(pattern(&query.q))
        .fetch_as::<(i64, String)>(&state.db)
        .await
        .unwrap_or_default()
    };
    Ok(Json(
        rows.into_iter()
            .map(|(id, name)| SelectOption::new(id, name))
            .collect(),
    ))
}

/// `GET /auto/export-invoice-options`: the issued zero-rated invoices not yet
/// registered as an export sale, as `INV-2026-0007 · customer · AED 25,000.00`.
async fn export_invoices(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    query: OptionQuery,
) -> Result<Json<Vec<SelectOption>>> {
    require(&user, permissions::EXPORT_SALES_MANAGE)?;
    let rows: Vec<(i64, String, i64, i64, String)> = if query.is_lookup() {
        let mut rows = Vec::new();
        for id in query.values_as::<i64>() {
            rows.extend(
                renox::db::sql(
                    "SELECT id, COALESCE(number, ''), party_id, gross_total, currency \
                     FROM wl_invoicing_invoices WHERE id = ?",
                )
                .bind(id)
                .fetch_as::<(i64, String, i64, i64, String)>(&state.db)
                .await
                .unwrap_or_default(),
            );
        }
        rows
    } else {
        renox::db::sql(
            "SELECT i.id, i.number, i.party_id, i.gross_total, i.currency \
             FROM wl_invoicing_invoices i \
             WHERE i.number IS NOT NULL AND i.status NOT IN ('draft', 'credited') \
             AND i.vat_total = 0 AND LOWER(i.number) LIKE ? \
             AND NOT EXISTS (SELECT 1 FROM auto_export_sales e WHERE e.invoice_id = i.id) \
             ORDER BY i.id DESC LIMIT 20",
        )
        .bind(pattern(&query.q))
        .fetch_as::<(i64, String, i64, i64, String)>(&state.db)
        .await
        .unwrap_or_default()
    };
    let mut options = Vec::new();
    for (id, number, party, gross, currency) in rows {
        let customer = party_name(&state, &lang.locale, party).await;
        let total = if currency == crate::pages::lists::CURRENCY {
            crate::money_text(gross)
        } else {
            format!("{currency} {gross}")
        };
        options.push(SelectOption::new(
            id,
            format!("{number} · {customer} · {total}"),
        ));
    }
    Ok(Json(options))
}

/// `GET /auto/supplier-return-options`: the returns to a supplier, by number.
async fn supplier_returns(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    query: OptionQuery,
) -> Result<Json<Vec<SelectOption>>> {
    require(&user, permissions::SUPPLIER_CLAIMS_MANAGE)?;
    let rows: Vec<(i64, String, i64)> = if query.is_lookup() {
        let mut rows = Vec::new();
        for id in query.values_as::<i64>() {
            rows.extend(
                renox::db::sql(
                    "SELECT id, COALESCE(number, ''), supplier_party_id \
                     FROM wl_inventory_supplier_returns WHERE id = ?",
                )
                .bind(id)
                .fetch_as::<(i64, String, i64)>(&state.db)
                .await
                .unwrap_or_default(),
            );
        }
        rows
    } else {
        renox::db::sql(
            "SELECT id, number, supplier_party_id FROM wl_inventory_supplier_returns \
             WHERE number IS NOT NULL AND LOWER(number) LIKE ? ORDER BY id DESC LIMIT 20",
        )
        .bind(pattern(&query.q))
        .fetch_as::<(i64, String, i64)>(&state.db)
        .await
        .unwrap_or_default()
    };
    let mut options = Vec::new();
    for (id, number, supplier) in rows {
        let name = party_name(&state, &lang.locale, supplier).await;
        options.push(SelectOption::new(id, format!("{number} · {name}")));
    }
    Ok(Json(options))
}

#[cfg(test)]
mod tests {
    use super::pattern;

    #[test]
    fn a_pattern_has_no_wildcards_of_its_own() {
        assert_eq!(pattern(" INV_20%26 "), "%inv2026%");
        assert_eq!(pattern(""), "%%");
    }
}
