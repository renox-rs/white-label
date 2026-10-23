//! The stand-ins of the payer bills and the rate cards of wl-invoicing: they
//! keep what they were asked in memory so a test can read it back. The host
//! tests (`host/tests/auto_claims_real.rs`) use the real module.

use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::payer_bills::{
    NewPayerBill, PayerBillFuture, PayerBillId, PayerBillStatus, PayerBillSummary, PayerBills,
};
use wl_kernel::rate_cards::{RateCardFuture, RateCardVersion, RateCards};
use wl_kernel::{PartyId, Subject};

/// One payer bill that was made.
#[derive(Debug, Clone)]
pub struct MadeBill {
    /// The id it answered with.
    pub id: i64,
    /// What it was asked for.
    pub request: NewPayerBill,
    /// Where it stands.
    pub status: PayerBillStatus,
}

impl MadeBill {
    /// What the bill claims, before VAT.
    pub fn claimed(&self) -> i64 {
        self.request.lines.iter().map(|line| line.claimed).sum()
    }
}

/// The payer bills made.
#[derive(Clone, Default)]
pub struct PayerBooks {
    bills: Arc<Mutex<Vec<MadeBill>>>,
}

impl PayerBooks {
    /// Every bill, oldest first.
    pub fn all(&self) -> Vec<MadeBill> {
        self.bills.lock().unwrap().clone()
    }

    /// The bills about `source`.
    pub fn about(&self, source: &Subject) -> Vec<MadeBill> {
        self.all()
            .into_iter()
            .filter(|bill| bill.request.source == *source)
            .collect()
    }

    /// The insurer paid bill `id`.
    pub fn set_status(&self, id: i64, status: PayerBillStatus) {
        for bill in self.bills.lock().unwrap().iter_mut() {
            if bill.id == id {
                bill.status = status;
            }
        }
    }
}

impl PayerBills for PayerBooks {
    fn find<'a>(
        &'a self,
        _db: &'a Db,
        id: PayerBillId,
    ) -> PayerBillFuture<'a, Option<PayerBillSummary>> {
        let found = self
            .bills
            .lock()
            .unwrap()
            .iter()
            .find(|bill| bill.id == id.0)
            .map(|bill| {
                let claimed = bill.claimed();
                let paid = if matches!(bill.status, PayerBillStatus::Paid | PayerBillStatus::Closed)
                {
                    claimed
                } else {
                    0
                };
                PayerBillSummary {
                    id,
                    number: (bill.status != PayerBillStatus::Draft)
                        .then(|| format!("PB-{}", bill.id)),
                    status: bill.status,
                    payer: PartyId(900),
                    customer: bill.request.customer,
                    legal_entity_id: 1,
                    source: bill.request.source.clone(),
                    currency: bill.request.currency.clone(),
                    claimed,
                    paid,
                    denied: 0,
                    open: claimed - paid,
                    resubmission_count: 0,
                    has_remittance: paid > 0,
                    not_submitted_in_time: false,
                }
            });
        Box::pin(async move { Ok(found) })
    }

    fn create<'a>(
        &'a self,
        _state: &'a AppState,
        request: NewPayerBill,
        _by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, PayerBillId> {
        let mut all = self.bills.lock().unwrap();
        let id = 500 + i64::try_from(all.len()).unwrap();
        all.push(MadeBill {
            id,
            request,
            status: PayerBillStatus::Draft,
        });
        Box::pin(async move { Ok(PayerBillId(id)) })
    }

    fn issue<'a>(
        &'a self,
        _state: &'a AppState,
        id: PayerBillId,
        _by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, ()> {
        self.set_status(id.0, PayerBillStatus::Issued);
        Box::pin(async { Ok(()) })
    }
}

/// The sources that have an accepted rate card version in force.
#[derive(Clone, Default)]
pub struct Cards {
    versions: Arc<Mutex<Vec<(Subject, RateCardVersion)>>>,
}

impl Cards {
    /// `source` has rate card `card`, version `version`, in force from `from`.
    pub fn accept(&self, source: Subject, card: i64, version: i64, from: NaiveDate) {
        self.versions.lock().unwrap().push((
            source,
            RateCardVersion {
                rate_card: card,
                version,
                effective_from: from,
            },
        ));
    }
}

impl RateCards for Cards {
    fn accepted_version<'a>(
        &'a self,
        _state: &'a AppState,
        source: &'a Subject,
        on: NaiveDate,
    ) -> RateCardFuture<'a, Option<RateCardVersion>> {
        let found = self
            .versions
            .lock()
            .unwrap()
            .iter()
            .filter(|(subject, version)| subject == source && version.effective_from <= on)
            .max_by_key(|(_, version)| version.version)
            .map(|(_, version)| version.clone());
        Box::pin(async move { Ok(found) })
    }
}
