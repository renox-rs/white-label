//! The Cheque (wl-payments §3, §4 *Cheque*, §5.3): a post-dated cheque,
//! received from a party or issued to one. Its status moves only through
//! the machine in [`crate::cheque_machine`].
//!
//! A cheque knows what it pays ([`ChequeLink`]: charges, invoices, supplier
//! bills, oldest first), how often it was presented, what the bank paid of
//! it when it bounced (the *honoured* amount), what was settled afterwards
//! and what is still *open*, each bank return with its reason and
//! certificate ([`ChequeReturn`]) and its legal case.
//!
//! The kernel's [`Cheques`](wl_kernel::cheques::Cheques) contract is
//! implemented in [`crate::chequer`]; the transitions live in
//! [`crate::cheque_deposit`] (`flag_due`, `deposit`, `clear`),
//! [`crate::cheque_bounce`] (`bounce`), [`crate::cheque_represent`]
//! (`represent`), [`crate::cheque_settle`] (`settle_part`, `replace`) and
//! [`crate::cheque_legal`] (`refer_to_legal`).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::{CHEQUE_SUBJECT_TYPE, ChequeId, ChequeStatus, ChequeSummary};
use wl_kernel::payments::Direction;
use wl_kernel::{PartyId, Subject};

/// How a legal case is brought (`refer_to_legal`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CaseType {
    /// Enforcement of the returned cheque as an executive instrument (the
    /// default).
    #[default]
    CivilExecution,
    /// A criminal complaint: only for the return reasons on the configured
    /// criminal-grounds list.
    CriminalComplaint,
}

/// A cheque. Money is `i64` fils of [`Cheque::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_cheques",
    state_machine,
    index(status),
    index(party_id),
    index(agreement_type, agreement_id)
)]
pub struct Cheque {
    /// The primary key.
    pub id: i64,
    /// Received (`in`) or issued (`out`).
    pub direction: Direction,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// The drawer of a received cheque, the payee of an issued one.
    pub party_id: i64,
    /// The bank account it is deposited in or drawn on, free text.
    pub bank_account: Option<String>,
    /// The bank it is drawn on.
    pub bank: String,
    /// The cheque number.
    pub number: String,
    /// The date on the cheque.
    pub cheque_date: NaiveDate,
    /// The amount, in fils; always > 0.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The type of the agreement it was received with, e.g. `property.lease`.
    pub agreement_type: Option<String>,
    /// The id of the agreement.
    pub agreement_id: Option<i64>,
    /// The staff member who collected it.
    pub collected_by: Option<i64>,
    /// Whether it was collected in the field (`FIN.ACL.20`).
    pub field_collected: bool,
    /// The cash handover that confirmed a field cheque.
    pub custody_handover_id: Option<i64>,
    /// How many times it was deposited.
    pub presentations: i32,
    /// The date of the latest deposit or the new presentation date.
    pub deposit_date: Option<NaiveDate>,
    /// What the bank paid of it when it bounced, in fils.
    pub honoured_amount: i64,
    /// What was settled after the bounce (`settle_part`), in fils.
    pub settled_amount: i64,
    /// What is still owed after a bounce, in fils: the amount less the
    /// honoured and the settled part; the whole amount before a bounce, 0
    /// once cleared.
    pub open_amount: i64,
    /// The latest bank return reason.
    pub return_reason: Option<String>,
    /// The reference of the latest return certificate.
    pub return_certificate: Option<String>,
    /// The day of the latest bounce (Asia/Dubai).
    pub bounced_on: Option<NaiveDate>,
    /// The bounce fees added so far, before VAT, in fils.
    pub bounce_fee_total: i64,
    /// The legal case's type.
    pub case_type: Option<CaseType>,
    /// The legal case's reference, once the lawyer has one.
    pub case_reference: Option<String>,
    /// The case owner: the user who gets the follow-up task.
    pub case_owner_id: Option<i64>,
    /// The day it was referred (Asia/Dubai).
    pub referred_on: Option<NaiveDate>,
    /// The user who registered it; `None` for the system.
    pub recorded_by: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: ChequeStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A document a cheque pays: a charge, an invoice or a supplier bill.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_cheque_links",
    index(cheque_id),
    index(subject_type, subject_id)
)]
pub struct ChequeLink {
    /// The primary key.
    pub id: i64,
    /// The cheque.
    pub cheque_id: i64,
    /// 0, 1, ...: the order the documents are paid in.
    pub position: i32,
    /// The type of the document, e.g. `invoicing.invoice`.
    pub subject_type: String,
    /// The id of the document.
    pub subject_id: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ChequeLink {
    /// The document as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject {
            subject_type: self.subject_type.clone(),
            subject_id: self.subject_id,
        }
    }
}

/// One bank return of a cheque: its reason, certificate, the part the bank
/// paid and the bounce fee.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_cheque_returns",
    unique(cheque_id, presentation),
    index(cheque_id)
)]
pub struct ChequeReturn {
    /// The primary key.
    pub id: i64,
    /// The cheque.
    pub cheque_id: i64,
    /// Which presentation bounced: 1 for the first deposit.
    pub presentation: i32,
    /// The bank's return reason.
    pub reason: String,
    /// The reference of the bank's return certificate (received cheques).
    pub certificate: Option<String>,
    /// What the bank paid, in fils.
    pub honoured_amount: i64,
    /// The payment recorded for the honoured part.
    pub honoured_payment_id: Option<i64>,
    /// The catalogue charge type of the bounce fee.
    pub fee_charge_type: Option<String>,
    /// The bounce fee before VAT, in fils; 0 when none.
    pub fee_amount: i64,
    /// The type of the charge that was added.
    pub fee_subject_type: Option<String>,
    /// The id of the charge that was added.
    pub fee_subject_id: Option<i64>,
    /// The day it was returned (Asia/Dubai).
    pub returned_on: NaiveDate,
    /// The user who recorded it.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ChequeReturn {
    /// The charge that was added as the fee, if any.
    pub fn fee_charge(&self) -> Option<Subject> {
        match (&self.fee_subject_type, self.fee_subject_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// The returns of `cheque_id`, oldest first.
    pub async fn of_cheque(db: &Db, cheque_id: i64) -> Result<Vec<ChequeReturn>> {
        ChequeReturn::query()
            .where_(ChequeReturn::CHEQUE_ID.eq(cheque_id))
            .order_by(ChequeReturn::PRESENTATION)
            .get(db)
            .await
    }

    /// The latest return of `cheque_id`.
    pub async fn latest(db: &Db, cheque_id: i64) -> Result<Option<ChequeReturn>> {
        Ok(Self::of_cheque(db, cheque_id).await?.pop())
    }
}

impl Cheque {
    /// The cheque's id in the kernel contract.
    pub fn cheque_id(&self) -> ChequeId {
        ChequeId(self.id)
    }

    /// The cheque as a [`Subject`]: `payments.cheque:42`.
    pub fn subject(&self) -> Subject {
        Subject::new(CHEQUE_SUBJECT_TYPE, self.id)
    }

    /// The drawer or payee.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// Whether it is a received cheque (money in).
    pub fn is_received(&self) -> bool {
        self.direction == Direction::In
    }

    /// The agreement it was received with, if any.
    pub fn agreement(&self) -> Option<Subject> {
        match (&self.agreement_type, self.agreement_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// What is still owed on a bounced cheque: the amount less what the bank
    /// paid and what was settled; never below 0.
    pub fn owed(&self) -> i64 {
        (self.amount - self.honoured_amount - self.settled_amount).max(0)
    }

    /// What other modules see of it, with the documents it pays.
    pub fn summary(&self, linked: Vec<Subject>) -> ChequeSummary {
        ChequeSummary {
            id: self.cheque_id(),
            direction: self.direction,
            party: self.party(),
            legal_entity_id: self.legal_entity_id,
            bank: self.bank.clone(),
            number: self.number.clone(),
            date: self.cheque_date,
            amount: self.amount,
            honoured_amount: self.honoured_amount,
            open_amount: self.open_amount,
            currency: self.currency.clone(),
            presentations: self.presentations,
            status: self.status,
            agreement: self.agreement(),
            linked,
        }
    }

    /// The documents it pays, oldest first.
    pub async fn links(&self, db: &Db) -> Result<Vec<ChequeLink>> {
        ChequeLink::query()
            .where_(ChequeLink::CHEQUE_ID.eq(self.id))
            .order_by(ChequeLink::POSITION)
            .get(db)
            .await
    }

    /// The documents it pays as subjects, oldest first.
    pub async fn linked(&self, db: &Db) -> Result<Vec<Subject>> {
        Ok(self.links(db).await?.iter().map(ChequeLink::subject).collect())
    }

    /// The bank returns, oldest first.
    pub async fn returns(&self, db: &Db) -> Result<Vec<ChequeReturn>> {
        ChequeReturn::of_cheque(db, self.id).await
    }

    /// The cheques received with `agreement`, by date.
    pub async fn of_agreement(db: &Db, agreement: &Subject) -> Result<Vec<Cheque>> {
        Cheque::query()
            .where_(Cheque::AGREEMENT_TYPE.eq(agreement.subject_type.as_str()))
            .where_(Cheque::AGREEMENT_ID.eq(agreement.subject_id))
            .order_by(Cheque::CHEQUE_DATE)
            .order_by(Cheque::ID)
            .get(db)
            .await
    }

    /// The bounced received cheques of `party` still owed (`bounced`,
    /// `legal_case`), other than `except`.
    pub async fn owed_of_party(db: &Db, party: PartyId, except: Option<i64>) -> Result<Vec<Cheque>> {
        let owed = Cheque::query()
            .where_(Cheque::PARTY_ID.eq(party.0))
            .where_(Cheque::DIRECTION.eq(Direction::In))
            .where_(Cheque::STATUS.is_in([ChequeStatus::Bounced, ChequeStatus::LegalCase]))
            .order_by(Cheque::ID)
            .get(db)
            .await?;
        Ok(owed
            .into_iter()
            .filter(|c| Some(c.id) != except)
            .collect())
    }

    /// What `party` still owes on its bounced received cheques in
    /// `currency`, in fils.
    pub async fn owed_amount(db: &Db, party: PartyId, currency: &str) -> Result<i64> {
        Cheque::query()
            .where_(Cheque::PARTY_ID.eq(party.0))
            .where_(Cheque::DIRECTION.eq(Direction::In))
            .where_(Cheque::CURRENCY.eq(currency))
            .where_(Cheque::STATUS.is_in([ChequeStatus::Bounced, ChequeStatus::LegalCase]))
            .sum::<i64, _>(db, "open_amount")
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cheque() -> Cheque {
        Cheque {
            id: 5,
            party_id: 7,
            amount: 2_000_000,
            open_amount: 2_000_000,
            currency: "AED".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_cheque_is_a_subject_of_its_own_type() {
        assert_eq!(cheque().subject().to_string(), "payments.cheque:5");
        assert_eq!(cheque().cheque_id(), ChequeId(5));
        assert_eq!(cheque().party(), PartyId(7));
    }

    #[test]
    fn what_is_owed_is_the_amount_less_the_honoured_and_settled_parts() {
        let mut c = cheque();
        assert_eq!(c.owed(), 2_000_000);
        c.honoured_amount = 600_000;
        assert_eq!(c.owed(), 1_400_000);
        c.settled_amount = 400_000;
        assert_eq!(c.owed(), 1_000_000);
        c.settled_amount = 9_000_000;
        assert_eq!(c.owed(), 0);
    }

    #[test]
    fn the_direction_decides_received_or_issued() {
        let mut c = cheque();
        assert!(c.is_received());
        c.direction = Direction::Out;
        assert!(!c.is_received());
    }

    #[test]
    fn the_agreement_needs_both_columns() {
        let mut c = cheque();
        assert_eq!(c.agreement(), None);
        c.agreement_type = Some("property.lease".into());
        assert_eq!(c.agreement(), None);
        c.agreement_id = Some(9);
        assert_eq!(c.agreement().map(|a| a.to_string()).as_deref(), Some("property.lease:9"));
    }

    #[test]
    fn the_summary_carries_the_amounts_and_the_links() {
        let mut c = cheque();
        c.honoured_amount = 600_000;
        c.open_amount = 1_400_000;
        c.presentations = 1;
        c.status = ChequeStatus::Bounced;
        let summary = c.summary(vec![Subject::new("invoicing.invoice", 3)]);
        assert_eq!(summary.id, ChequeId(5));
        assert_eq!((summary.honoured_amount, summary.open_amount), (600_000, 1_400_000));
        assert_eq!(summary.status, ChequeStatus::Bounced);
        assert_eq!(summary.linked.len(), 1);
    }

    #[test]
    fn a_return_names_its_fee_charge_only_with_both_columns() {
        let mut r = ChequeReturn::default();
        assert_eq!(r.fee_charge(), None);
        r.fee_subject_type = Some("invoicing.charge".into());
        r.fee_subject_id = Some(12);
        assert_eq!(r.fee_charge().map(|c| c.to_string()).as_deref(), Some("invoicing.charge:12"));
    }
}
