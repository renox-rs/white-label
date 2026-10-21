//! The fakes the loyalty tests need: a notifier that records, an invoicer that
//! knows its invoices, payer bills, the workforce (which user is whose
//! employee) and a recorder of the loyalty events. They are the other
//! modules' contracts, answered by hand; the rules under test are the real
//! `wl-sales` listeners.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRequest};
use wl_kernel::payer_bills::{PayerBillId, PayerBillStatus, PayerBillSummary, PayerBills};
use wl_kernel::sales::{PointsEarned, PointsReversed};
use wl_kernel::workforce::{EmployeeId, StaffProfile, Workforce, WorkforceFuture};
use wl_kernel::{PartyId, Subject};

/// Every notification asked for.
#[derive(Clone, Default)]
pub struct Mail(Arc<Mutex<Vec<NotifyRequest>>>);

impl Mail {
    /// The notifications of the type `key`.
    pub fn of_type(&self, key: &str) -> Vec<NotifyRequest> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.type_key == key)
            .cloned()
            .collect()
    }
}

impl Notifier for Mail {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request);
            Ok(vec![MessageId(1)])
        })
    }
}

/// One invoice the books know.
#[derive(Clone)]
struct Entry {
    party: PartyId,
    net: i64,
    gross: i64,
}

/// The invoices, drafted or entered by hand.
#[derive(Clone, Default)]
pub struct Books(Arc<Mutex<HashMap<i64, Entry>>>);

impl Books {
    /// An invoice `id` for `party`, `net` before VAT and `gross` with it.
    pub fn add(&self, id: i64, party: PartyId, net: i64, gross: i64) {
        self.0.lock().unwrap().insert(id, Entry { party, net, gross });
    }
}

impl Invoicer for Books {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        Box::pin(async move {
            let mut known = self.0.lock().unwrap();
            let id = 100 + known.len() as i64;
            known.insert(
                id,
                Entry {
                    party: draft.party,
                    net: draft.lines.iter().map(|l| l.net()).sum(),
                    gross: draft.lines.iter().map(|l| l.gross()).sum(),
                },
            );
            Ok(Some(InvoiceId(id)))
        })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        id: InvoiceId,
    ) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async move {
            Ok(self.0.lock().unwrap().get(&id.0).map(|e| InvoiceSummary {
                id,
                number: Some(format!("INV-{}", id.0)),
                status: InvoiceStatus::Paid,
                party: e.party,
                legal_entity_id: Some(1),
                currency: "AED".to_owned(),
                gross_total: e.gross,
                balance: 0,
                due_date: None,
                disputed: false,
                chasing_paused_until: None,
            }))
        })
    }

    fn net_total<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<i64>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().get(&id.0).map(|e| e.net)) })
    }
}

/// The payer bills (insurer shares) by source.
#[derive(Clone, Default)]
pub struct Insurer(Arc<Mutex<Vec<(Subject, i64)>>>);

impl Insurer {
    /// An insurer share of `claimed` fils before VAT for `source`.
    pub fn claim(&self, source: Subject, claimed: i64) {
        self.0.lock().unwrap().push((source, claimed));
    }
}

impl PayerBills for Insurer {
    fn of_source<'a>(
        &'a self,
        _db: &'a Db,
        source: &'a Subject,
    ) -> wl_kernel::payer_bills::PayerBillFuture<'a, Vec<PayerBillSummary>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|(s, _)| s == source)
                .enumerate()
                .map(|(n, (s, claimed))| PayerBillSummary {
                    id: PayerBillId(n as i64 + 1),
                    number: None,
                    status: PayerBillStatus::Issued,
                    payer: PartyId(900),
                    customer: PartyId(0),
                    legal_entity_id: 1,
                    source: s.clone(),
                    currency: "AED".to_owned(),
                    claimed: *claimed,
                    paid: 0,
                    denied: 0,
                    open: *claimed,
                    resubmission_count: 0,
                    has_remittance: false,
                    not_submitted_in_time: false,
                })
                .collect())
        })
    }
}

/// Which user is whose employee: the party of the staff member.
#[derive(Clone, Default)]
pub struct Staff(Arc<Mutex<HashMap<i64, i64>>>);

impl Staff {
    /// The user `user` is an employee whose person is the party `party`.
    pub fn set(&self, user: i64, party: i64) {
        self.0.lock().unwrap().insert(user, party);
    }
}

impl Workforce for Staff {
    fn employee_of_user<'a>(
        &'a self,
        _db: &'a Db,
        user_id: i64,
    ) -> WorkforceFuture<'a, Option<EmployeeId>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .contains_key(&user_id)
                .then_some(EmployeeId(user_id)))
        })
    }

    fn staff<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, Option<StaffProfile>> {
        Box::pin(async move {
            Ok(self.0.lock().unwrap().get(&employee.0).map(|party| StaffProfile {
                employee,
                party: PartyId(*party),
                user_id: Some(employee.0),
                role: "cashier".to_owned(),
                scopes: Vec::new(),
                legal_entity_id: None,
                listed: true,
                on_leave: false,
                returns_on: None,
                managers: Vec::new(),
            }))
        })
    }
}

/// The loyalty events a test saw.
#[derive(Clone, Default)]
pub struct Points {
    /// `PointsEarned`.
    pub earned: Arc<Mutex<Vec<PointsEarned>>>,
    /// `PointsReversed`.
    pub reversed: Arc<Mutex<Vec<PointsReversed>>>,
}

/// The fakes of the other modules, handed to the test.
#[derive(Clone, Default)]
pub struct Shop {
    /// The notifier.
    pub mail: Mail,
    /// The invoices.
    pub books: Books,
    /// The insurer shares.
    pub insurer: Insurer,
    /// The staff.
    pub staff: Staff,
    /// The loyalty events.
    pub points: Points,
}

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn register(&self, app: &mut Registry) {
        wl_kernel::notify::provide_notifier(app, self.mail.clone());
        wl_kernel::invoicing::provide_invoicer(app, self.books.clone());
        wl_kernel::payer_bills::provide_payer_bills(app, self.insurer.clone());
        wl_kernel::workforce::provide_workforce(app, self.staff.clone());
        let log = self.points.earned.clone();
        app.listen(move |event: PointsEarned, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
        let log = self.points.reversed.clone();
        app.listen(move |event: PointsReversed, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
    }
}
