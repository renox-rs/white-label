//! The enrolment: a student in a batch, a session package or a membership
//! (10 §6 *Enrolment*, §7.1 *Enrolment*, `EDU.ENR.01`).
//!
//! The student, the guardian and the payer are parties (wl-parties, named by
//! id; a student under 18 has a guardian, who is the payer unless another
//! payer is named). A course enrolment holds one seat of its batch from its
//! creation ([`crate::seats`]); a package or a membership holds none. The
//! status moves only through the machine in [`crate::enrolment_machine`];
//! [`crate::enrolments`] holds the services and [`crate::fees`] raises the
//! invoices.
//!
//! Money on the row is in fils with VAT: the catalogue `price` of the course,
//! the package or one month of the membership, the `discount` taken off it,
//! the `registration_fee` and the `admin_fee` the refund policy keeps
//! (`EDU.REF.01`).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Json};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject};

use crate::refund::RefundPolicy;

/// The subject type of an enrolment.
pub const ENROLMENT_SUBJECT_TYPE: &str = "edtech.enrolment";

/// What the enrolment is for.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EnrolmentKind {
    /// A seat in a batch, paid as a course fee (upfront or in instalments).
    #[default]
    Course,
    /// A package of sessions (the *Package* of wl-sales).
    Package,
    /// A monthly membership (a subscription of wl-invoicing).
    Membership,
}

impl EnrolmentKind {
    /// The stored name, as the texts use it.
    pub fn as_str(self) -> &'static str {
        match self {
            EnrolmentKind::Course => "course",
            EnrolmentKind::Package => "package",
            EnrolmentKind::Membership => "membership",
        }
    }
}

/// Where an enrolment stands (10 §7.1 *Enrolment*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EnrolmentStatus {
    /// Seat held; first payment or approval outstanding.
    #[default]
    PendingPayment,
    /// Attending.
    Active,
    /// Paused at the student's request, with a membership pause, or while the
    /// payer is blocked.
    Frozen,
    /// Course ended or package used up (terminal).
    Completed,
    /// Left early; refund or amount owed settled (terminal).
    Withdrawn,
    /// Moved to another batch; a new enrolment exists (terminal).
    Transferred,
    /// Seat hold expired without payment (terminal).
    Lapsed,
}

impl EnrolmentStatus {
    /// The stored name, as the machine and the texts use it.
    pub fn as_str(self) -> &'static str {
        match self {
            EnrolmentStatus::PendingPayment => "pending_payment",
            EnrolmentStatus::Active => "active",
            EnrolmentStatus::Frozen => "frozen",
            EnrolmentStatus::Completed => "completed",
            EnrolmentStatus::Withdrawn => "withdrawn",
            EnrolmentStatus::Transferred => "transferred",
            EnrolmentStatus::Lapsed => "lapsed",
        }
    }

    /// Whether the enrolment counts against the seats of its batch: `active`,
    /// `frozen` and `pending_payment` (`EDU.ACL.17`).
    pub fn takes_a_seat(self) -> bool {
        matches!(
            self,
            EnrolmentStatus::PendingPayment | EnrolmentStatus::Active | EnrolmentStatus::Frozen
        )
    }

    /// Whether the enrolment is over.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            EnrolmentStatus::Completed
                | EnrolmentStatus::Withdrawn
                | EnrolmentStatus::Transferred
                | EnrolmentStatus::Lapsed
        )
    }
}

/// An enrolment.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "edtech_enrolments",
    state_machine,
    index(batch_id),
    index(student_party_id),
    index(first_invoice_id),
    index(status)
)]
pub struct Enrolment {
    /// The primary key.
    pub id: i64,
    /// Batch, package or membership.
    pub kind: EnrolmentKind,
    /// The batch of a course enrolment.
    pub batch_id: Option<i64>,
    /// The student, a party.
    pub student_party_id: i64,
    /// The guardian of a student under 18, a party.
    pub guardian_party_id: Option<i64>,
    /// Who pays, a party: the guardian, else the student, unless named.
    pub payer_party_id: i64,
    /// The type of the campus (copied from the batch).
    pub campus_type: Option<String>,
    /// The id of the campus.
    pub campus_id: Option<i64>,
    /// The user who enrolled the student; reassignable.
    pub counsellor_id: Option<i64>,
    /// The catalogue item of a package or membership.
    pub item_code: Option<String>,
    /// The name of the item as sold.
    pub item_name: Option<String>,
    /// The catalogue price with VAT, in fils (a membership: one month).
    pub price: i64,
    /// The discount taken off the price, in fils.
    pub discount: i64,
    /// A discount above the limit needs the centre manager's approval
    /// (`activate` guard); true when there is none or it was approved.
    pub discount_approved: bool,
    /// The registration fee with VAT, in fils; kept by the refund policy.
    pub registration_fee: i64,
    /// The admin fee with VAT, in fils; kept once the course started.
    pub admin_fee: i64,
    /// The sessions the fee covers (a package: its credits).
    pub sessions_total: i64,
    /// How many instalments the course fee is paid in (1 = upfront).
    pub instalments: i64,
    /// The refund policy accepted at enrolment.
    pub policy: Json<RefundPolicy>,
    /// When the terms and the refund policy were accepted (`DOC.SIG.01`).
    pub terms_accepted_at: Option<DateTime>,
    /// Whether the batch's seat is counted for this enrolment.
    pub seat_held: bool,
    /// The seat is held until then (`pending_payment`; `EDU.RUL.11`).
    pub seat_hold_until: Option<DateTime>,
    /// The first invoice (`invoicing.invoice`); its payment activates it.
    pub first_invoice_id: Option<i64>,
    /// The package sold for it (`sales.package`).
    pub package_id: Option<i64>,
    /// The subscription of a membership (`invoicing.subscription`).
    pub subscription_id: Option<i64>,
    /// The refund `withdraw` calculated, in fils.
    pub refund_amount: i64,
    /// The amount owed `withdraw` calculated, in fils.
    pub amount_owed: i64,
    /// The day the student left.
    pub withdrawn_on: Option<NaiveDate>,
    /// Why the student left.
    pub withdrawal_reason: Option<String>,
    /// The day the enrolment was completed.
    pub completed_on: Option<NaiveDate>,
    /// What moved here from the enrolment this one replaces, in fils: the
    /// value of the sessions the student had paid for and not used. It counts
    /// as paid (`transfer`, `EDU.ACL.17`).
    pub carried_paid: i64,
    /// The enrolment this one replaces (`transfer`).
    pub transferred_from_id: Option<i64>,
    /// The enrolment that replaces this one (`transfer`).
    pub transferred_to_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: EnrolmentStatus,
    /// The user who created it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Enrolment {
    /// The number people read: `ENR-0007`.
    pub fn number(&self) -> String {
        format!("ENR-{:04}", self.id)
    }

    /// The enrolment as a [`Subject`]: `edtech.enrolment:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(ENROLMENT_SUBJECT_TYPE, self.id)
    }

    /// The student as a [`PartyId`].
    pub fn student(&self) -> PartyId {
        PartyId(self.student_party_id)
    }

    /// The payer as a [`PartyId`].
    pub fn payer(&self) -> PartyId {
        PartyId(self.payer_party_id)
    }

    /// What the student owes for the course, package or month, after the
    /// discount, in fils.
    pub fn fee(&self) -> i64 {
        (self.price - self.discount).max(0)
    }

    /// The enrolment of invoice `invoice_id`, the first one raised for it.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_first_invoice(db: &Db, invoice_id: i64) -> Result<Option<Enrolment>> {
        Enrolment::query()
            .where_(Enrolment::FIRST_INVOICE_ID.eq(invoice_id))
            .order_by(Enrolment::ID)
            .first(db)
            .await
    }

    /// The enrolments of batch `batch_id` that hold a seat, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn seated_in(db: &Db, batch_id: i64) -> Result<Vec<Enrolment>> {
        let all = Enrolment::query()
            .where_(Enrolment::BATCH_ID.eq(batch_id))
            .order_by(Enrolment::ID)
            .get(db)
            .await?;
        Ok(all
            .into_iter()
            .filter(|e| e.status.takes_a_seat())
            .collect())
    }
}

/// Why an enrolment is frozen. Each cause that holds it is stored; it is
/// unfrozen when none is left (10 §7.1 `freeze`, `unfreeze`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FreezeCause {
    /// The student asked: counts against the allowance of 30 days a year.
    #[default]
    Student,
    /// Mirrors the membership pause (`EDU.RUL.14`): not counted again.
    Linked,
    /// The payer is blocked (`PartyBlocked`): not counted.
    Block,
}

impl FreezeCause {
    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            FreezeCause::Student => "student",
            FreezeCause::Linked => "linked",
            FreezeCause::Block => "block",
        }
    }

    /// Whether the days count against the allowance.
    pub fn counts_against_allowance(self) -> bool {
        matches!(self, FreezeCause::Student)
    }
}

/// One cause that holds an enrolment frozen.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "edtech_enrolment_freezes", index(enrolment_id))]
pub struct EnrolmentFreeze {
    /// The primary key.
    pub id: i64,
    /// The enrolment.
    pub enrolment_id: i64,
    /// Why.
    pub cause: FreezeCause,
    /// The first day frozen.
    pub started_on: NaiveDate,
    /// The planned last day (the freeze ends the day after), if known.
    pub until_on: Option<NaiveDate>,
    /// The days counted against the allowance.
    pub counted_days: i64,
    /// The day the cause ended; `None` while it holds the freeze.
    pub ended_on: Option<NaiveDate>,
    /// The user who froze it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl EnrolmentFreeze {
    /// The causes that hold `enrolment_id` frozen now.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open_of(db: &Db, enrolment_id: i64) -> Result<Vec<EnrolmentFreeze>> {
        EnrolmentFreeze::query()
            .where_(EnrolmentFreeze::ENROLMENT_ID.eq(enrolment_id))
            .where_(EnrolmentFreeze::ENDED_ON.is_null())
            .order_by(EnrolmentFreeze::ID)
            .get(db)
            .await
    }

    /// The days of the student `student_party_id` already counted against the
    /// allowance in the year of `on`, over all their enrolments.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn counted_in_year(db: &Db, student_party_id: i64, on: NaiveDate) -> Result<i64> {
        use renox::chrono::Datelike;
        let enrolments = Enrolment::query()
            .where_(Enrolment::STUDENT_PARTY_ID.eq(student_party_id))
            .get(db)
            .await?;
        let mut days = 0;
        for enrolment in enrolments {
            let rows = EnrolmentFreeze::query()
                .where_(EnrolmentFreeze::ENROLMENT_ID.eq(enrolment.id))
                .get(db)
                .await?;
            days += rows
                .iter()
                .filter(|row| row.started_on.year() == on.year())
                .map(|row| row.counted_days)
                .sum::<i64>();
        }
        Ok(days)
    }
}

/// Where an instalment of the course fee stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InstalmentStatus {
    /// Not invoiced yet.
    #[default]
    Scheduled,
    /// Its invoice was drafted.
    Invoiced,
    /// Cancelled with the enrolment (`withdraw`).
    Cancelled,
}

/// One instalment of an enrolment's course fee.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "edtech_instalments",
    unique(enrolment_id, number),
    index(due_date)
)]
pub struct Instalment {
    /// The primary key.
    pub id: i64,
    /// The enrolment.
    pub enrolment_id: i64,
    /// 1, 2, 3 ...
    pub number: i64,
    /// The day it is invoiced.
    pub due_date: NaiveDate,
    /// The amount with VAT, in fils.
    pub amount: i64,
    /// Its invoice, once drafted.
    pub invoice_id: Option<i64>,
    /// Where it stands.
    pub status: InstalmentStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Instalment {
    /// The instalments of `enrolment_id`, in order.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_enrolment(db: &Db, enrolment_id: i64) -> Result<Vec<Instalment>> {
        Instalment::query()
            .where_(Instalment::ENROLMENT_ID.eq(enrolment_id))
            .order_by(Instalment::NUMBER)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_enrolment_is_numbered_and_is_a_subject() {
        let e = Enrolment {
            id: 7,
            ..Enrolment::default()
        };
        assert_eq!(e.number(), "ENR-0007");
        assert_eq!(e.subject().to_string(), "edtech.enrolment:7");
    }

    #[test]
    fn the_fee_is_the_price_less_the_discount_never_below_nothing() {
        let e = Enrolment {
            price: 252_000,
            discount: 25_200,
            ..Enrolment::default()
        };
        assert_eq!(e.fee(), 226_800);
        let free = Enrolment {
            price: 100,
            discount: 500,
            ..Enrolment::default()
        };
        assert_eq!(free.fee(), 0);
    }

    #[test]
    fn only_the_open_states_take_a_seat() {
        assert!(EnrolmentStatus::PendingPayment.takes_a_seat());
        assert!(EnrolmentStatus::Active.takes_a_seat());
        assert!(EnrolmentStatus::Frozen.takes_a_seat());
        for ended in [
            EnrolmentStatus::Completed,
            EnrolmentStatus::Withdrawn,
            EnrolmentStatus::Transferred,
            EnrolmentStatus::Lapsed,
        ] {
            assert!(!ended.takes_a_seat());
            assert!(ended.is_terminal());
        }
    }

    #[test]
    fn only_the_students_own_freeze_counts_against_the_allowance() {
        assert!(FreezeCause::Student.counts_against_allowance());
        assert!(!FreezeCause::Linked.counts_against_allowance());
        assert!(!FreezeCause::Block.counts_against_allowance());
    }
}
