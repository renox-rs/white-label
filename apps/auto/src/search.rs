//! The workshop's records in the global search and the *Related* panel
//! (BL-AGT-13, decision 0006): a service advisor finds a job card by its
//! number, the car's plate or its VIN; an insurance coordinator a claim by
//! its number or the insurer's; a fleet account manager a fleet contract by
//! its number; each with its related records.
//!
//! **Plates.** A plate is found however it is typed (`DXB A 12345`,
//! `dxb-a-12345`, `Dubai A 12345`, `A 12345`): each job card carries the
//! plate in those forms, normalised like every identifier. It carries both
//! the plate the car had when the card was opened (`intake_plate`) and the
//! car's plate now, so when a plate moves to another car the job cards of
//! both cars match, the latest first (the index orders equal matches by
//! their last change).
//!
//! A job card carries its workshop's scope when it has one.

use renox::auth::permissions::Scope;
use renox::db::{DbValue, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::{LoadFuture, Relation};
use wl_kernel::search::{SearchDocument, Searchable, index_in, kinds};
use wl_kernel::tickets::TICKET_SUBJECT_TYPE;

use crate::claim::{ClaimStatus, InsuranceClaim};
use crate::fleet_contract::FleetContract;
use crate::job_card::{JobCard, JobCardStatus};
use crate::permissions;
use crate::seed::{
    FLEET_CONTRACT_SUBJECT_TYPE, INSURANCE_CLAIM_SUBJECT_TYPE, JOB_CARD_SUBJECT_TYPE,
    SUPPLIER_CLAIM_SUBJECT_TYPE, VEHICLE_SUBJECT_TYPE,
};
use crate::supplier_claim::SupplierClaim;
use crate::vehicle::{Emirate, Vehicle};

const PARTIES_VIEW: &str = "parties.view";
const INVOICES_VIEW: &str = "invoices.view";
const TICKETS_VIEW: &str = "tickets.view";

/// The short code of an emirate on a plate (`DXB`).
fn short_code(emirate: Emirate) -> &'static str {
    match emirate {
        Emirate::AbuDhabi => "AUH",
        Emirate::Dubai => "DXB",
        Emirate::Sharjah => "SHJ",
        Emirate::Ajman => "AJM",
        Emirate::UmmAlQuwain => "UAQ",
        Emirate::RasAlKhaimah => "RAK",
        Emirate::Fujairah => "FUJ",
    }
}

/// A plate as stored on a job card: the emirate's stored name, the code and
/// the number (`dubai A 12345`).
pub fn plate_key(vehicle: &Vehicle) -> String {
    format!(
        "{} {} {}",
        vehicle.plate_emirate.as_str(),
        vehicle.plate_code,
        vehicle.plate_number
    )
}

/// The forms a plate is typed in: `A 12345`, `DXB A 12345`, `Dubai A 12345`.
fn plate_forms(emirate: Option<Emirate>, code: &str, number: &str) -> Vec<String> {
    let mut forms = vec![format!("{code} {number}")];
    if let Some(emirate) = emirate {
        forms.push(format!("{} {code} {number}", short_code(emirate)));
        forms.push(format!("{} {code} {number}", emirate.label()));
    }
    forms
}

/// The forms of a plate stored by [`plate_key`].
fn stored_plate_forms(key: &str) -> Vec<String> {
    let mut parts = key.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(emirate), Some(code), Some(number)) => {
            plate_forms(Emirate::parse(emirate), code, number)
        }
        _ => Vec::new(),
    }
}

/// The vehicle `id`, read in `tx`.
async fn vehicle_in(tx: &mut Transaction, id: i64) -> Result<Option<Vehicle>> {
    Vehicle::unscoped().where_eq("id", id).first(&mut *tx).await
}

/// `document` with the plate and VIN of `vehicle`.
fn with_vehicle(mut document: SearchDocument, vehicle: Option<&Vehicle>) -> SearchDocument {
    if let Some(vehicle) = vehicle {
        for form in plate_forms(
            Some(vehicle.plate_emirate),
            &vehicle.plate_code,
            &vehicle.plate_number,
        ) {
            document = document.identifier(kinds::PLATE, form);
        }
        if let Some(vin) = &vehicle.vin {
            document = document.identifier(kinds::VIN, vin);
        }
    }
    document
}

async fn job_card_document(tx: &mut Transaction, card: &JobCard) -> Result<SearchDocument> {
    let vehicle = vehicle_in(tx, card.vehicle_id).await?;
    let number = card.number();
    let mut document = SearchDocument::new(&number)
        .secondary(vehicle.as_ref().map(Vehicle::title).unwrap_or_default())
        .identifier(kinds::NUMBER, &number)
        .status(card.status.as_str())
        .voided(card.status == JobCardStatus::Voided);
    if let (Some(kind), Some(id)) = (&card.workshop_type, card.workshop_id) {
        document = document.scope(Scope::new(kind, id));
    }
    if let Some(key) = &card.intake_plate {
        for form in stored_plate_forms(key) {
            document = document.identifier(kinds::PLATE, form);
        }
    }
    if let Some(report) = &card.police_report_no {
        document = document.identifier(kinds::NUMBER, report);
    }
    Ok(with_vehicle(document, vehicle.as_ref()))
}

async fn claim_document(tx: &mut Transaction, claim: &InsuranceClaim) -> Result<SearchDocument> {
    let vehicle = vehicle_in(tx, claim.vehicle_id).await?;
    let number = claim.number();
    let mut document = SearchDocument::new(&number)
        .secondary(vehicle.as_ref().map(Vehicle::title).unwrap_or_default())
        .identifier(kinds::NUMBER, &number)
        .identifier(kinds::NUMBER, &claim.claim_no)
        .identifier(kinds::NUMBER, &claim.policy_no)
        .status(claim.status.as_str())
        .voided(claim.status == ClaimStatus::Voided);
    if let Some(report) = &claim.police_report_no {
        document = document.identifier(kinds::NUMBER, report);
    }
    Ok(with_vehicle(document, vehicle.as_ref()))
}

fn supplier_claim_document(claim: &SupplierClaim) -> SearchDocument {
    let number = claim.number();
    let mut document = SearchDocument::new(&number)
        .secondary(&claim.part_description)
        .identifier(kinds::NUMBER, &number)
        .identifier(kinds::NUMBER, &claim.ticket_number)
        .status(claim.status.as_str());
    if let Some(reference) = &claim.supplier_reference {
        document = document.identifier(kinds::NUMBER, reference);
    }
    if let Some(code) = &claim.item_code {
        document = document.identifier(kinds::SKU, code);
    }
    document
}

fn fleet_contract_document(contract: &FleetContract) -> SearchDocument {
    let number = contract.number();
    SearchDocument::new(format!("{number} · {}", contract.title))
        .secondary(format!("{} to {}", contract.start_date, contract.end_date))
        .identifier(kinds::NUMBER, &number)
        .status(contract.status.as_str())
}

/// The `(from, to)` pairs of `statement` (ending in `IN `, selecting
/// `from_id` and `to_id`) for `ids`.
fn pairs_of<'a>(
    db: &'a Db,
    ids: &'a [i64],
    statement: &'static str,
    from_type: &'static str,
    to_type: &'static str,
) -> LoadFuture<'a> {
    Box::pin(async move {
        let placeholders = vec!["?"; ids.len()].join(", ");
        let rows = sql(format!("{statement}({placeholders})"))
            .bind_all(ids.iter().map(|id| DbValue::Integer(*id)))
            .fetch_all(db)
            .await?;
        let mut pairs = Vec::with_capacity(rows.len());
        for row in &rows {
            let from: i64 = row.try_get("from_id")?;
            let to: i64 = row.try_get("to_id")?;
            if from != to || from_type != to_type {
                pairs.push((Subject::new(from_type, from), Subject::new(to_type, to)));
            }
        }
        Ok(pairs)
    })
}

/// Declares the searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<JobCard>(JOB_CARD_SUBJECT_TYPE)
        .permission(permissions::JOB_CARDS_VIEW)
        .label("Job cards")
        .link("/auto/job-cards/{id}")
        .document_in(|tx, card| Box::pin(job_card_document(tx, card)))
        .register();
    Searchable::of::<InsuranceClaim>(INSURANCE_CLAIM_SUBJECT_TYPE)
        .permission(permissions::INSURANCE_MANAGE)
        .label("Insurance claims")
        .link("/auto/claims/{id}")
        .document_in(|tx, claim| Box::pin(claim_document(tx, claim)))
        .register();
    Searchable::of::<SupplierClaim>(SUPPLIER_CLAIM_SUBJECT_TYPE)
        .permission(permissions::SUPPLIER_CLAIMS_MANAGE)
        .label("Supplier warranty claims")
        .link("/auto/supplier-claims/{id}")
        .document(supplier_claim_document)
        .register();
    Searchable::of::<FleetContract>(FLEET_CONTRACT_SUBJECT_TYPE)
        .permission(permissions::FLEET_CONTRACTS_MANAGE)
        .label("Fleet contracts")
        .link("/auto/fleet/{id}")
        .document(fleet_contract_document)
        .register();

    // A job card: its vehicle, the car's other job cards, its claims, the
    // warranty claims of parts it fitted, its customer and invoice.
    Relation::belongs_to(
        JOB_CARD_SUBJECT_TYPE,
        "vehicle",
        VEHICLE_SUBJECT_TYPE,
        "auto_job_cards",
        "vehicle_id",
    )
    .label("Vehicle")
    .permission(permissions::VEHICLES_VIEW)
    .link("/auto/vehicles/{id}")
    .register();
    Relation::new(
        JOB_CARD_SUBJECT_TYPE,
        "vehicle_history",
        JOB_CARD_SUBJECT_TYPE,
    )
    .label("Other job cards of the car")
    .load(|db, ids| {
        pairs_of(
            db,
            ids,
            "SELECT a.id AS from_id, b.id AS to_id FROM auto_job_cards a \
                 JOIN auto_job_cards b ON b.vehicle_id = a.vehicle_id WHERE a.id IN ",
            JOB_CARD_SUBJECT_TYPE,
            JOB_CARD_SUBJECT_TYPE,
        )
    })
    .register();
    Relation::has_many(
        JOB_CARD_SUBJECT_TYPE,
        "insurance_claims",
        INSURANCE_CLAIM_SUBJECT_TYPE,
        "auto_insurance_claims",
        "job_card_id",
    )
    .label("Insurance claims")
    .register();
    Relation::has_many(
        JOB_CARD_SUBJECT_TYPE,
        "supplier_claims",
        SUPPLIER_CLAIM_SUBJECT_TYPE,
        "auto_supplier_claims",
        "fitted_job_card_id",
    )
    .label("Warranty claims of parts it fitted")
    .register();
    Relation::belongs_to(
        JOB_CARD_SUBJECT_TYPE,
        "customer",
        PARTY_SUBJECT_TYPE,
        "auto_job_cards",
        "customer_party_id",
    )
    .label("Customer")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();
    Relation::belongs_to(
        JOB_CARD_SUBJECT_TYPE,
        "invoice",
        INVOICE_SUBJECT_TYPE,
        "auto_job_cards",
        "invoice_id",
    )
    .label("Invoice")
    .permission(INVOICES_VIEW)
    .link("/invoices/{id}")
    .register();

    // An insurance claim: its job card, its vehicle and its customer.
    Relation::belongs_to(
        INSURANCE_CLAIM_SUBJECT_TYPE,
        "job_card",
        JOB_CARD_SUBJECT_TYPE,
        "auto_insurance_claims",
        "job_card_id",
    )
    .label("Job card")
    .register();
    Relation::belongs_to(
        INSURANCE_CLAIM_SUBJECT_TYPE,
        "vehicle",
        VEHICLE_SUBJECT_TYPE,
        "auto_insurance_claims",
        "vehicle_id",
    )
    .label("Vehicle")
    .permission(permissions::VEHICLES_VIEW)
    .link("/auto/vehicles/{id}")
    .register();
    Relation::belongs_to(
        INSURANCE_CLAIM_SUBJECT_TYPE,
        "customer",
        PARTY_SUBJECT_TYPE,
        "auto_insurance_claims",
        "customer_party_id",
    )
    .label("Customer")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();

    // A supplier warranty claim: the job cards that fitted and replaced the
    // part, the supplier and the claim's ticket.
    Relation::belongs_to(
        SUPPLIER_CLAIM_SUBJECT_TYPE,
        "fitted_job_card",
        JOB_CARD_SUBJECT_TYPE,
        "auto_supplier_claims",
        "fitted_job_card_id",
    )
    .label("Job card that fitted the part")
    .register();
    Relation::belongs_to(
        SUPPLIER_CLAIM_SUBJECT_TYPE,
        "replacement_job_card",
        JOB_CARD_SUBJECT_TYPE,
        "auto_supplier_claims",
        "replacement_job_card_id",
    )
    .label("Job card that replaced it")
    .register();
    Relation::belongs_to(
        SUPPLIER_CLAIM_SUBJECT_TYPE,
        "supplier",
        PARTY_SUBJECT_TYPE,
        "auto_supplier_claims",
        "supplier_party_id",
    )
    .label("Supplier")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();
    Relation::belongs_to(
        SUPPLIER_CLAIM_SUBJECT_TYPE,
        "ticket",
        TICKET_SUBJECT_TYPE,
        "auto_supplier_claims",
        "ticket_id",
    )
    .label("Ticket")
    .permission(TICKETS_VIEW)
    .link("/tickets/{id}")
    .register();

    // A fleet contract: its vehicles, their job cards and the fleet.
    Relation::has_many(
        FLEET_CONTRACT_SUBJECT_TYPE,
        "vehicles",
        VEHICLE_SUBJECT_TYPE,
        "auto_vehicles",
        "fleet_contract_id",
    )
    .label("Vehicles")
    .permission(permissions::VEHICLES_VIEW)
    .link("/auto/vehicles/{id}")
    .register();
    Relation::new(
        FLEET_CONTRACT_SUBJECT_TYPE,
        "job_cards",
        JOB_CARD_SUBJECT_TYPE,
    )
    .label("Job cards")
    .load(|db, ids| {
        pairs_of(
            db,
            ids,
            "SELECT v.fleet_contract_id AS from_id, j.id AS to_id FROM auto_vehicles v \
                 JOIN auto_job_cards j ON j.vehicle_id = v.id WHERE v.fleet_contract_id IN ",
            FLEET_CONTRACT_SUBJECT_TYPE,
            JOB_CARD_SUBJECT_TYPE,
        )
    })
    .register();
    Relation::belongs_to(
        FLEET_CONTRACT_SUBJECT_TYPE,
        "fleet",
        PARTY_SUBJECT_TYPE,
        "auto_fleet_contracts",
        "fleet_party_id",
    )
    .label("Fleet customer")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();
}

/// Writes the document of `subject` again in a transaction of its own, for
/// a record saved outside one.
///
/// # Errors
///
/// A database error.
pub async fn index(db: &Db, subject: &Subject) -> Result {
    let mut tx = db.begin().await?;
    index_in(&mut tx, subject).await?;
    tx.commit().await?;
    Ok(())
}

/// Writes the documents of the job cards and claims of vehicle `id` again
/// (its plate or VIN changed).
///
/// # Errors
///
/// A database error.
pub async fn index_vehicle(db: &Db, id: i64) -> Result {
    let mut tx = db.begin().await?;
    for card in JobCard::query()
        .where_eq("vehicle_id", id)
        .get(&mut tx)
        .await?
    {
        index_in(&mut tx, &Subject::new(JOB_CARD_SUBJECT_TYPE, card.id)).await?;
    }
    for claim in InsuranceClaim::query()
        .where_eq("vehicle_id", id)
        .get(&mut tx)
        .await?
    {
        index_in(
            &mut tx,
            &Subject::new(INSURANCE_CLAIM_SUBJECT_TYPE, claim.id),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Writes the document of a record of `M` again once a transition run with
/// `run` has committed (its status changed).
fn reindex_after<M>(app: &mut Registry, subject_type: &'static str)
where
    M: StateMachine<Key = i64> + Send + Sync + 'static,
{
    app.listen(move |event: Transitioned<M>, state: AppState| async move {
        index(&state.db, &Subject::new(subject_type, event.key)).await
    });
}

/// Keeps the documents current after transitions.
pub(crate) fn listen(app: &mut Registry) {
    reindex_after::<JobCard>(app, JOB_CARD_SUBJECT_TYPE);
    reindex_after::<InsuranceClaim>(app, INSURANCE_CLAIM_SUBJECT_TYPE);
    reindex_after::<SupplierClaim>(app, SUPPLIER_CLAIM_SUBJECT_TYPE);
    reindex_after::<FleetContract>(app, FLEET_CONTRACT_SUBJECT_TYPE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::search::normalize_identifier;

    #[test]
    fn a_plate_is_one_identifier_however_it_is_typed() {
        let forms = stored_plate_forms("dubai A 12345");
        let normalised: Vec<String> = forms
            .iter()
            .map(|form| normalize_identifier(kinds::PLATE, form))
            .collect();
        for typed in ["DXB A 12345", "dxb-a-12345", "Dubai A 12345", "a 12345"] {
            assert!(
                normalised.contains(&normalize_identifier(kinds::PLATE, typed)),
                "{typed}"
            );
        }
    }
}
