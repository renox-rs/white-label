//! What the tenant and owner portals show of 01 (`POR.SLF.01`, `POR.SLF.02`,
//! `PRP.ACL.01`, `PRP.ACL.02`; BL-PRP-30, BL-PRP-29).
//!
//! The portal itself is wl-portal's: the host registers the sections with it
//! and checks the user's grants for the party. This module gives the rows of
//! one party, filtered to its own records only, so another party's lease,
//! cheque, document or statement never shows, even by URL:
//!
//! - [`tenant_leases`]: each lease version where the party is the tenant
//!   (unit, building, term, annual rent, payment plan, status, Ejari number).
//!   Drafts are not shown; `pending_signature` shows as *Waiting for
//!   signature*. The legal case flag and internal notes are never shown.
//! - [`tenant_cheques`]: the cheques of those leases, with the status
//!   wl-payments stored.
//! - [`TENANT_DOCUMENT_KINDS`]: the document kinds of a lease the tenant may
//!   see (the contract, the Ejari certificate, the acceptance of changed
//!   terms); the host lists and serves them from wl-documents.
//! - [`owner_statements`]: the published and settled statements of the
//!   agreements of an owner.
//!
//! The party's block is never read here (`PTY.ACL.02`): a blocked party sees
//! the same rows, with the statuses the modules stored.

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::cheques::ChequeStatus;

use crate::agreement::ManagementAgreement;
use crate::building::Building;
use crate::charge::RENT;
use crate::lease::{Lease, LeaseStatus};
use crate::money_text;
use crate::statement::{OwnerStatement, StatementStatus};
use crate::unit::Unit;

/// The document kinds of a lease its tenant sees in the portal: the signed
/// contract, the generated contract (BL-PRP-31), the Ejari or Tawtheeq
/// certificate and the acceptance of changed terms. Internal records (the
/// abandonment inspection, legal documents) are not among them.
pub const TENANT_DOCUMENT_KINDS: &[&str] = &[
    "signed_tenancy_contract",
    crate::generated::TENANCY_CONTRACT_KIND,
    "ejari_certificate",
    crate::terms::TENANT_ACCEPTANCE_KIND,
];

/// One row of a portal section: what it is and the status as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRow {
    /// What it is.
    pub label: String,
    /// Its status, in words.
    pub status: String,
}

/// A lease status in the tenant's words.
pub fn lease_status_text(status: LeaseStatus) -> &'static str {
    match status {
        LeaseStatus::Draft => "Draft",
        LeaseStatus::PendingSignature => "Waiting for signature",
        LeaseStatus::Active => "Active",
        LeaseStatus::RenewalDue => "Renewal due",
        LeaseStatus::Expired => "Expired",
        LeaseStatus::NoticeGiven => "Notice given",
        LeaseStatus::Renewed => "Renewed",
        LeaseStatus::Amended => "Amended",
        LeaseStatus::Terminated => "Terminated",
        LeaseStatus::Cancelled => "Cancelled",
    }
}

/// A cheque status as wl-payments stored it, in words.
pub fn cheque_status_text(status: ChequeStatus) -> &'static str {
    match status {
        ChequeStatus::Received => "Received",
        ChequeStatus::DueForDeposit => "Due for deposit",
        ChequeStatus::Deposited => "Deposited",
        ChequeStatus::Cleared => "Cleared",
        ChequeStatus::Bounced => "Bounced",
        ChequeStatus::LegalCase => "Legal case",
        ChequeStatus::Replaced => "Replaced",
        ChequeStatus::WrittenOff => "Written off",
        ChequeStatus::Returned => "Returned",
    }
}

/// The lease versions of the tenant `party` the portal shows, oldest first:
/// every version but drafts.
///
/// # Errors
///
/// The database's errors.
pub async fn leases_of(db: &Db, party: PartyId) -> Result<Vec<Lease>> {
    Ok(Lease::query()
        .where_(Lease::TENANT_PARTY_ID.eq(party.0))
        .order_by(Lease::ROOT_ID)
        .order_by(Lease::VERSION)
        .get(db)
        .await?
        .into_iter()
        .filter(|lease| lease.status != LeaseStatus::Draft)
        .collect())
}

/// The *Lease* rows of the tenant `party`.
///
/// # Errors
///
/// The database's errors.
pub async fn tenant_leases(db: &Db, party: PartyId) -> Result<Vec<PortalRow>> {
    let mut rows = Vec::new();
    for lease in leases_of(db, party).await? {
        let unit = Unit::find(db, lease.unit_id).await?;
        let building = Building::find(db, lease.building_id).await?;
        let instalments = lease
            .charges(db)
            .await?
            .iter()
            .filter(|c| c.charge_type == RENT)
            .count();
        let mut label = format!(
            "{} · Unit {}, {} · {} to {} · {} a year · {} cheque{}",
            lease.number_with_version(),
            unit.map(|u| u.number).unwrap_or_default(),
            building.map(|b| b.name_en).unwrap_or_default(),
            lease.start_date,
            lease.end_date,
            money_text(lease.annual_rent),
            instalments,
            if instalments == 1 { "" } else { "s" },
        );
        if let Some(number) = &lease.ejari_number {
            label.push_str(&format!(" · Ejari {number}"));
        }
        rows.push(PortalRow {
            label,
            status: lease_status_text(lease.status).to_owned(),
        });
    }
    Ok(rows)
}

/// The *Cheques* rows of the tenant `party`: the cheques of its leases, by
/// date, with the status wl-payments stored.
///
/// # Errors
///
/// The database's errors and the cheque register's.
pub async fn tenant_cheques(state: &AppState, party: PartyId) -> Result<Vec<PortalRow>> {
    let mut seen: Vec<i64> = Vec::new();
    let mut rows = Vec::new();
    for lease in leases_of(&state.db, party).await? {
        for cheque in crate::cheques::of_version(state, &lease).await? {
            if seen.contains(&cheque.id.0) || cheque.party != party {
                continue;
            }
            seen.push(cheque.id.0);
            rows.push((
                cheque.date,
                PortalRow {
                    label: format!(
                        "Cheque {} · {} · {} · {} · {}",
                        cheque.number,
                        cheque.bank,
                        cheque.date,
                        money_text(cheque.amount),
                        lease.number(),
                    ),
                    status: cheque_status_text(cheque.status).to_owned(),
                },
            ));
        }
    }
    rows.sort_by_key(|(date, _)| *date);
    Ok(rows.into_iter().map(|(_, row)| row).collect())
}

/// Whether the lease version `lease_id` is one of the tenant `party`'s that
/// the portal shows.
///
/// # Errors
///
/// The database's errors.
pub async fn lease_of_tenant(db: &Db, party: PartyId, lease_id: i64) -> Result<bool> {
    Ok(Lease::find(db, lease_id)
        .await?
        .is_some_and(|lease| lease.tenant_party_id == party.0 && lease.status != LeaseStatus::Draft))
}

/// The statements the owner `party` sees: published or settled, of the
/// agreements it owns, newest first.
///
/// # Errors
///
/// The database's errors.
pub async fn statements_of_owner(db: &Db, party: PartyId) -> Result<Vec<OwnerStatement>> {
    let agreements = ManagementAgreement::query()
        .where_(ManagementAgreement::OWNER_PARTY_ID.eq(party.0))
        .get(db)
        .await?;
    let mut found = Vec::new();
    for agreement in agreements {
        for statement in OwnerStatement::of_agreement(db, agreement.id).await? {
            if matches!(
                statement.status,
                StatementStatus::Published | StatementStatus::Settled
            ) && statement.owner_party_id == party.0
            {
                found.push(statement);
            }
        }
    }
    found.sort_by(|a, b| b.period_start.cmp(&a.period_start).then(b.id.cmp(&a.id)));
    Ok(found)
}

/// The *Statements* rows of the owner `party`.
///
/// # Errors
///
/// The database's errors.
pub async fn owner_statements(db: &Db, party: PartyId) -> Result<Vec<PortalRow>> {
    Ok(statements_of_owner(db, party)
        .await?
        .iter()
        .map(|statement| PortalRow {
            label: format!(
                "{} · {} · balance {}",
                statement.number(),
                statement.period_start.format("%B %Y"),
                money_text(statement.balance()),
            ),
            status: if statement.disputed {
                "Disputed".to_owned()
            } else if statement.status == StatementStatus::Settled {
                "Settled".to_owned()
            } else {
                "Published".to_owned()
            },
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lease_waiting_for_signature_says_so_and_cheques_keep_their_status() {
        assert_eq!(
            lease_status_text(LeaseStatus::PendingSignature),
            "Waiting for signature"
        );
        assert_eq!(cheque_status_text(ChequeStatus::Cleared), "Cleared");
        assert_eq!(cheque_status_text(ChequeStatus::Returned), "Returned");
        assert!(TENANT_DOCUMENT_KINDS.contains(&"ejari_certificate"));
        assert!(!TENANT_DOCUMENT_KINDS.contains(&"abandonment_inspection"));
    }
}
