//! A recalled part traced to vehicle owners, fleets and customers (08 §7.1
//! settings *Recall*, `BL-AUT-16`).
//!
//! The recall is wl-inventory's: its owner (the Owner / GM or a branch
//! manager, `recalls.manage`) opens it with the item, the supplier and the
//! received date range of the supplier's notice, because parts are not tracked
//! by lot in the MVP. The whole on-hand quantity is held and the trace lists
//! every issue and sale of the part from the first receipt in the range, each
//! line *possible*. Three things are the workshop's:
//!
//! - [`TraceParties`], the app's `TracePartyResolver`: a job card
//!   (`auto.job_card:N`, the cost object of the part's `consume`) is the
//!   vehicle's owner, or the fleet when the vehicle is under a fleet contract
//!   in force (the owner when the contract has ended); a counter sale or a
//!   sales order is its customer (the kernel's `SaleCustomers`); a counter
//!   sale that names nobody is a walk-in sale, which the trace counts per
//!   branch and day.
//! - [`trace`], the list the page shows: the record, the party, the vehicle
//!   of a job card, the day, the evidence and whether the party was told.
//! - [`send`], the notice: one message for each line still to tell, to the
//!   party of the line (to the fleet contact for a fleet), naming only that
//!   party's own vehicle or purchase, never another customer's plate, name or
//!   job; the line is marked `sent` through the kernel's
//!   `LotTrace::set_notice`, and a message that gives up
//!   ([`on_gave_up`]) takes it back to `pending`.
//!
//! Decisions:
//!
//! - The owner on the job card's date is the vehicle's current owner: there is
//!   no ownership history in the MVP.
//! - Parts issued to a job card and put back (`return_in`, `AUT.ACL.21`) are
//!   taken off by the trace itself, so such a job card is not listed.
//! - A line whose party is unknown, and a count of walk-in sales, cannot be
//!   sent; the recall owner waives them with a reason in wl-inventory's recall
//!   page.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::documents::subject_label;
use wl_kernel::inventory::{
    RECALL_SUBJECT_TYPE, TraceFuture, TraceMark, TraceParty, TracePartyResolver, lot_trace,
    provide_trace_party_resolver,
};
use wl_kernel::notify::{
    self, Category, GiveUpReaction, MessageGaveUp, NotificationTypeDef, NotifyChannel,
    NotifyRequest,
};
use wl_kernel::sales::{
    COUNTER_SALE_SUBJECT_TYPE, SALES_ORDER_SUBJECT_TYPE, SaleCustomer, sale_customers,
};
use wl_kernel::{ClientConfig, PartyId, Subject};

use crate::db::refused;
use crate::fleet_contract::FleetContract;
use crate::job_card::{JOB_CARD_SUBJECT_TYPE, JobCard};
use crate::permissions;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// The notification type of the notice to a party that got the part.
pub const RECALL_NOTICE: &str = "auto.recall_notice";

/// The audit action of a notice sent: who told whom, never what was wrong.
pub const SENT: &str = "auto.recall_notice_sent";

/// The Arabic subject of the notice.
const SUBJECT_AR: &str = "إشعار سلامة من {{ workshop }}";

/// The Arabic body of the notice.
const BODY_AR: &str = "{{ workshop }}: قد تكون قطعة قمنا بتوريدها لـ{{ what_ar }} بتاريخ {{ date }} متأثرة بسحب من المورّد. يرجى إحضارها لفحصها.";

/// The party a vehicle's work is for: the fleet when it is under a fleet
/// contract in force, else the owner.
///
/// # Errors
///
/// The database's errors.
pub async fn responsible_party(db: &Db, vehicle: &Vehicle) -> Result<i64> {
    let Some(contract_id) = vehicle.fleet_contract_id else {
        return Ok(vehicle.owner_party_id);
    };
    let Some(contract) = FleetContract::find(db, contract_id).await? else {
        return Ok(vehicle.owner_party_id);
    };
    Ok(if contract.status.is_in_force() {
        contract.fleet_party_id
    } else {
        vehicle.owner_party_id
    })
}

/// The app's `TracePartyResolver`: a job card is its vehicle's owner or fleet,
/// a sale is its customer, a counter sale with no customer is a walk-in sale.
#[derive(Debug, Clone, Copy, Default)]
pub struct TraceParties;

impl TracePartyResolver for TraceParties {
    fn party<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> TraceFuture<'a, TraceParty> {
        Box::pin(async move {
            match subject.subject_type.as_str() {
                JOB_CARD_SUBJECT_TYPE => {
                    let Some(card) = JobCard::find(&state.db, subject.subject_id).await? else {
                        return Ok(TraceParty::Unknown);
                    };
                    match Vehicle::find(&state.db, card.vehicle_id).await? {
                        Some(vehicle) => Ok(TraceParty::Party(PartyId(
                            responsible_party(&state.db, &vehicle).await?,
                        ))),
                        None => Ok(TraceParty::Party(PartyId(card.customer_party_id))),
                    }
                }
                COUNTER_SALE_SUBJECT_TYPE | SALES_ORDER_SUBJECT_TYPE => Ok(
                    match sale_customers(state).customer(state, subject).await? {
                        SaleCustomer::Party(party) => TraceParty::Party(party),
                        SaleCustomer::WalkIn => TraceParty::WalkIn,
                        SaleCustomer::Unknown => TraceParty::Unknown,
                    },
                ),
                _ => Ok(TraceParty::Unknown),
            }
        })
    }
}

/// One line of the list: a job card or a sale that got the part, or the count
/// of the walk-in sales of one branch and day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRow {
    /// The line of the recall, `inventory.recall_line:7`.
    pub line: Subject,
    /// `JOB-00150`, `SO-15`; empty for a count of walk-in sales.
    pub record: String,
    /// The vehicle's plate for a job card.
    pub vehicle: Option<String>,
    /// The vehicle owner, fleet or customer behind the line.
    pub party: Option<i64>,
    /// Who the notice goes to: the party, or the fleet's contact.
    pub addressee: Option<i64>,
    /// The day of the fit or the sale (Asia/Dubai).
    pub on: Option<NaiveDate>,
    /// The branch (the location the part left), for a count of walk-in sales.
    pub branch: String,
    /// The sales counted, for a count of walk-in sales; else 1.
    pub sales: i64,
    /// Whether the line is a count of walk-in sales.
    pub walk_in: bool,
    /// How sure the trace is: *possible* for a part not tracked by lot.
    pub mark: TraceMark,
    /// `pending`, `sent` or `waived`.
    pub notice: String,
}

/// The fleet's contact when `vehicle` is under a fleet contract in force and
/// `party` is the fleet; `None` otherwise.
async fn fleet_contact(db: &Db, vehicle: &Vehicle, party: i64) -> Result<Option<i64>> {
    let Some(contract_id) = vehicle.fleet_contract_id else {
        return Ok(None);
    };
    let Some(contract) = FleetContract::find(db, contract_id).await? else {
        return Ok(None);
    };
    if contract.status.is_in_force() && contract.fleet_party_id == party {
        Ok(Some(contract.contact_party_id.unwrap_or(party)))
    } else {
        Ok(None)
    }
}

/// The name of a sale for people: its registered label (`SO-00015`), else the
/// kind and number.
async fn sale_name(db: &Db, subject: &Subject) -> String {
    match subject_label(db, subject).await {
        Ok(Some(label)) if !label.is_empty() => label,
        _ => {
            let kind = subject
                .subject_type
                .rsplit('.')
                .next()
                .unwrap_or(&subject.subject_type)
                .replace('_', " ");
            format!("{kind} #{}", subject.subject_id)
        }
    }
}

/// The lines of recall `recall`, oldest day first, with the party and the
/// vehicle of each. The parties come from [`TraceParties`], which wl-inventory
/// called when it traced.
///
/// # Errors
///
/// The database's and the trace's errors.
pub async fn trace(state: &AppState, recall: i64) -> Result<Vec<TraceRow>> {
    let holders = lot_trace(state)
        .holders(state, &Subject::new(RECALL_SUBJECT_TYPE, recall))
        .await?;
    let mut rows = Vec::new();
    for holder in holders {
        let walk_in = holder.kind == "walk_in";
        let mut record = String::new();
        let mut vehicle: Option<Vehicle> = None;
        if let Some(subject) = &holder.subject {
            if subject.subject_type == JOB_CARD_SUBJECT_TYPE {
                if let Some(card) = JobCard::find(&state.db, subject.subject_id).await? {
                    record = card.number();
                    vehicle = Vehicle::find(&state.db, card.vehicle_id).await?;
                }
            } else {
                record = sale_name(&state.db, subject).await;
            }
        }
        let party = holder.party.map(|party| party.0);
        let addressee = match (party, &vehicle) {
            (Some(party), Some(vehicle)) => Some(
                fleet_contact(&state.db, vehicle, party)
                    .await?
                    .unwrap_or(party),
            ),
            (party, _) => party,
        };
        let branch = match (&holder.location, walk_in) {
            (Some(location), true) => sale_name(&state.db, location).await,
            _ => String::new(),
        };
        rows.push(TraceRow {
            line: holder.line.clone(),
            record,
            vehicle: vehicle.as_ref().map(Vehicle::plate),
            party,
            addressee,
            on: holder.on,
            branch,
            sales: holder.sales,
            walk_in,
            mark: holder.mark,
            notice: holder.notice.clone(),
        });
    }
    rows.sort_by(|a, b| {
        (a.on, &a.record, a.line.subject_id).cmp(&(b.on, &b.record, b.line.subject_id))
    });
    Ok(rows)
}

/// The notification type of the notice: an email in English and Arabic; a
/// message that gives up gives the branch manager a task.
pub fn notice_types() -> Vec<NotificationTypeDef> {
    let subject = english("auto.recall.mail.subject", &[]);
    let body = english("auto.recall.mail.body", &[]);
    vec![
        NotificationTypeDef::new(RECALL_NOTICE, "auto")
            .group("recalls")
            .category(Category::Transactional)
            .give_up(&[GiveUpReaction::StaffTask])
            .staff_role(permissions::BRANCH_MANAGER_ROLE)
            .template(NotifyChannel::Email, "en", Some(&subject), &body)
            .template(NotifyChannel::Email, "ar", Some(SUBJECT_AR), BODY_AR),
    ]
}

/// The workshop's name for the notice.
fn workshop_name(state: &AppState) -> String {
    state
        .provided::<std::sync::Arc<ClientConfig>>()
        .map(|config| config.client.name.clone())
        .unwrap_or_default()
}

/// What a notice is about, in English and Arabic: the party's own vehicle (or
/// a fleet vehicle), or its own purchase.
fn what_of(row: &TraceRow, fleet: bool) -> (String, String) {
    match &row.vehicle {
        Some(plate) if fleet => (
            english("auto.recall.what_fleet_vehicle", &[("plate", plate)]),
            format!("المركبة {plate} ضمن أسطولكم"),
        ),
        Some(plate) => (
            english("auto.recall.what_vehicle", &[("plate", plate)]),
            format!("مركبتكم {plate}"),
        ),
        None => (
            english("auto.recall.what_sale", &[("record", &row.record)]),
            format!("مشترياتكم {}", row.record),
        ),
    }
}

/// `by` (a holder of `recalls.manage`) tells the party of each line of recall
/// `recall_id` still `pending` with a known party. Returns how many notices
/// went out.
///
/// # Errors
///
/// `Forbidden` without the permission; a refusal when nobody is left to tell;
/// the notifier's errors (a line whose notice failed stays `pending`).
pub async fn send(state: &AppState, by: &AuthUser, recall_id: i64) -> Result<usize> {
    if !by.has_permission(permissions::RECALLS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let rows = trace(state, recall_id).await?;
    let to_tell: Vec<&TraceRow> = rows
        .iter()
        .filter(|row| row.notice == "pending" && !row.walk_in && row.addressee.is_some())
        .collect();
    if to_tell.is_empty() {
        return Err(refused(english("auto.recall.problem.nothing", &[])));
    }
    let workshop = workshop_name(state);
    let tracer = lot_trace(state);
    let mut sent = 0;
    for row in to_tell {
        let addressee = row.addressee.unwrap_or_default();
        // The fleet's contact is told about a fleet vehicle.
        let fleet = row.party.is_some_and(|party| party != addressee);
        let request = {
            let (what, what_ar) = what_of(row, fleet);
            NotifyRequest::new(RECALL_NOTICE, row.line.clone())
                .subject_label(&row.record)
                .to_party(PartyId(addressee))
                .data(json!({
                    "workshop": workshop,
                    "what": what,
                    "what_ar": what_ar,
                    "date": row.on.map(|day| day.to_string()).unwrap_or_default(),
                }))
        };
        notify::notifier(state).notify(state, request).await?;
        tracer
            .set_notice(state, &row.line, true, Some(by.id))
            .await?;
        renox::audit::record(
            &state.db,
            renox::audit::Entry::new(SENT)
                .user(by.id)
                .subject(row.line.subject_type.as_str(), row.line.subject_id)
                .data(json!({ "recall": recall_id, "party": addressee })),
        )
        .await?;
        sent += 1;
    }
    Ok(sent)
}

/// `MessageGaveUp` of a recall notice: the line is `pending` again (the staff
/// task is wl-notify's give-up reaction).
///
/// # Errors
///
/// The database's errors.
pub async fn on_gave_up(state: &AppState, event: &MessageGaveUp) -> Result {
    if event.type_key != RECALL_NOTICE {
        return Ok(());
    }
    lot_trace(state)
        .set_notice(state, &event.subject, false, None)
        .await?;
    Ok(())
}

/// Registers the resolver, the notification type and the listener of the
/// give-up.
pub(crate) fn register(app: &mut Registry) {
    provide_trace_party_resolver(app, TraceParties);
    for def in notice_types() {
        notify::register_type(def);
    }
    app.listen(
        |event: MessageGaveUp, state: AppState| async move { on_gave_up(&state, &event).await },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notice_is_one_email_type_in_both_languages() {
        let types = notice_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].key, RECALL_NOTICE);
        assert!(english("auto.recall.mail.body", &[]).contains("{{ what }}"));
    }

    #[test]
    fn a_notice_names_the_vehicle_or_the_purchase_of_its_own_party() {
        let row = TraceRow {
            line: Subject::new("inventory.recall_line", 1),
            record: "JOB-00150".to_owned(),
            vehicle: Some("Dubai A 12345".to_owned()),
            party: Some(7),
            addressee: Some(7),
            on: None,
            branch: String::new(),
            sales: 1,
            walk_in: false,
            mark: TraceMark::Possible,
            notice: "pending".to_owned(),
        };
        assert_eq!(what_of(&row, false).0, "your vehicle Dubai A 12345");
        assert_eq!(
            what_of(&row, true).0,
            "the vehicle Dubai A 12345 of your fleet"
        );
        let sale = TraceRow {
            vehicle: None,
            record: "SO-15".to_owned(),
            ..row
        };
        assert_eq!(what_of(&sale, false).0, "your purchase SO-15");
    }
}
