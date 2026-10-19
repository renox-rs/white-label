//! The post-dated cheque schedule of a lease (`FIN.CHQ.01`, 01 Property §4.1
//! step 8, §2: 1 to 12 post-dated cheques a year).
//!
//! The cheque register belongs to wl-payments; the lease reaches it only
//! through the kernel's [`Cheques`] contract. A schedule is registered
//! against the lease as its *agreement*, so the register can list the cheques
//! of the lease and the lease can see what it has received. A cheque is linked
//! to the charge of the payment plan it pays, so its amount goes to the right
//! instalment when it clears.
//!
//! A cheque belongs to one version of the lease: the one whose charges it is
//! linked to, or, with no link, the first version.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::LegalEntity;
use wl_kernel::cheques::{ChequeSchedule, ChequeStatus, ChequeSummary, ScheduledCheque, cheques};

use crate::building::{BUILDING_SUBJECT_TYPE, refused};
use crate::charge::{CHARGE_SUBJECT_TYPE, ChargeStatus, LeaseCharge};
use crate::lease::{Lease, LeaseStatus};
use crate::texts::english;

/// One post-dated cheque the tenant hands over.
#[derive(Debug, Clone)]
pub struct ChequeEntry {
    /// The cheque number.
    pub number: String,
    /// The date on the cheque.
    pub date: NaiveDate,
    /// The amount in fils.
    pub amount: i64,
    /// The charge of the payment plan it pays, if any.
    pub charge_id: Option<i64>,
}

impl ChequeEntry {
    /// A cheque of `amount` fils dated `date` that pays `charge_id`.
    pub fn new(number: &str, date: NaiveDate, amount: i64, charge_id: Option<i64>) -> Self {
        ChequeEntry {
            number: number.to_owned(),
            date,
            amount,
            charge_id,
        }
    }
}

/// Registers the cheques the tenant of `lease` handed over, drawn on `bank`,
/// all or none, by the user `by` (`None` is the system). The lease must not
/// be cancelled or over, and a cheque can only pay a live charge of this
/// version.
///
/// # Errors
///
/// 422 for no cheque, a lease that has ended, a charge that is not this
/// version's or is cancelled, or a cheque the register refuses (a blank
/// bank, an amount of 0, a number registered twice).
pub async fn register_cheques(
    state: &AppState,
    by: Option<i64>,
    lease: &Lease,
    bank: &str,
    entries: Vec<ChequeEntry>,
) -> Result<Vec<ChequeSummary>> {
    if entries.is_empty() {
        return Err(refused(english("property.cheques.none", &[])));
    }
    if matches!(
        lease.status,
        LeaseStatus::Cancelled
            | LeaseStatus::Terminated
            | LeaseStatus::Renewed
            | LeaseStatus::Amended
    ) {
        return Err(refused(english("property.cheques.lease_over", &[])));
    }
    let charges = lease.charges(&state.db).await?;
    let entity = LegalEntity::for_scope(
        &state.db,
        Some((BUILDING_SUBJECT_TYPE, lease.building_id)),
    )
    .await?;
    let mut schedule = ChequeSchedule::new(lease.agreement(), lease.tenant(), entity.id, bank);
    for entry in &entries {
        let mut cheque = ScheduledCheque::new(&entry.number, entry.date, entry.amount);
        if let Some(charge_id) = entry.charge_id {
            let live = charges
                .iter()
                .find(|c| c.id == charge_id && c.status != ChargeStatus::Cancelled);
            let Some(charge) = live else {
                return Err(refused(english("property.cheques.bad_charge", &[])));
            };
            cheque = cheque.link(charge.subject());
        }
        schedule = schedule.cheque(cheque);
    }
    cheques(state).register_schedule(state, schedule, by).await
}

/// The cheques received with `lease` that belong to this version, oldest
/// date first.
pub async fn of_version(state: &AppState, lease: &Lease) -> Result<Vec<ChequeSummary>> {
    let all = cheques(state)
        .of_agreement(&state.db, &lease.agreement())
        .await?;
    let ids: Vec<i64> = lease.charges(&state.db).await?.iter().map(|c| c.id).collect();
    Ok(all
        .into_iter()
        .filter(|cheque| belongs_to_version(cheque, lease, &ids))
        .collect())
}

/// Whether `cheque` belongs to the version whose charges are `charge_ids`:
/// it is linked to one of them; a cheque linked to no charge of the lease
/// belongs to the first version.
fn belongs_to_version(cheque: &ChequeSummary, lease: &Lease, charge_ids: &[i64]) -> bool {
    let charge_links: Vec<i64> = cheque
        .linked
        .iter()
        .filter(|s| s.subject_type == CHARGE_SUBJECT_TYPE)
        .map(|s| s.subject_id)
        .collect();
    if charge_links.is_empty() {
        return lease.version == 1;
    }
    charge_links.iter().any(|id| charge_ids.contains(id))
}

/// The fils of the cheques that are held for the tenant or paid: `received`,
/// `due_for_deposit`, `deposited` and `cleared`.
pub fn counted_amount(cheques: &[ChequeSummary]) -> i64 {
    cheques
        .iter()
        .filter(|c| {
            matches!(
                c.status,
                ChequeStatus::Received
                    | ChequeStatus::DueForDeposit
                    | ChequeStatus::Deposited
                    | ChequeStatus::Cleared
            )
        })
        .map(|c| c.amount)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::cheques::ChequeId;
    use wl_kernel::payments::Direction;
    use wl_kernel::{PartyId, Subject};

    fn cheque(status: ChequeStatus, amount: i64, links: &[Subject]) -> ChequeSummary {
        ChequeSummary {
            id: ChequeId(1),
            direction: Direction::In,
            party: PartyId(7),
            legal_entity_id: 1,
            bank: "ENBD".into(),
            number: "1".into(),
            date: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
            amount,
            honoured_amount: 0,
            open_amount: amount,
            currency: "AED".into(),
            presentations: 0,
            status,
            agreement: None,
            linked: links.to_vec(),
        }
    }

    #[test]
    fn a_cheque_belongs_to_the_version_whose_charge_it_pays() {
        let v1 = Lease {
            version: 1,
            ..Default::default()
        };
        let v2 = Lease {
            version: 2,
            ..Default::default()
        };
        let pays_v2 = cheque(
            ChequeStatus::Received,
            100,
            &[Subject::new(CHARGE_SUBJECT_TYPE, 20)],
        );
        assert!(belongs_to_version(&pays_v2, &v2, &[20, 21]));
        assert!(!belongs_to_version(&pays_v2, &v1, &[10, 11]));
        let loose = cheque(ChequeStatus::Received, 100, &[]);
        assert!(belongs_to_version(&loose, &v1, &[10]));
        assert!(!belongs_to_version(&loose, &v2, &[20]));
    }

    #[test]
    fn only_held_or_paid_cheques_count() {
        let cheques = [
            cheque(ChequeStatus::Received, 100, &[]),
            cheque(ChequeStatus::Deposited, 200, &[]),
            cheque(ChequeStatus::Cleared, 400, &[]),
            cheque(ChequeStatus::Returned, 800, &[]),
            cheque(ChequeStatus::Bounced, 1600, &[]),
        ];
        assert_eq!(counted_amount(&cheques), 700);
    }
}
