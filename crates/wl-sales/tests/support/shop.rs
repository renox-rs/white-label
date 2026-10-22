//! The fakes the loyalty tests need: a notifier that records, an invoicer that
//! knows its invoices, payer bills, the workforce (which user is whose
//! employee) and a recorder of the loyalty events. They are the other
//! modules' contracts, answered by hand; the rules under test are the real
//! `wl-sales` listeners.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use renox::chrono::TimeDelta;
use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRequest, OTP_VALIDITY_MINUTES, OTP_WRONG_TRIES,
    OtpRequest, OtpSent, OtpVerdict,
};
use wl_kernel::payer_bills::{PayerBillId, PayerBillStatus, PayerBillSummary, PayerBills};
use wl_kernel::sales::{
    PointsAdjusted, PointsEarned, PointsExpired, PointsGivenBack, PointsRedeemed, PointsReversed,
};
use wl_kernel::workforce::{EmployeeId, StaffProfile, Workforce, WorkforceFuture};
use wl_kernel::{PartyId, Subject};

/// One code the fake notifier sent: the wl-notify defaults (5 minutes, 3
/// wrong tries) are the real module's, tested in its own crate; the fake keeps
/// them so the redemption flow sees every verdict.
#[derive(Clone)]
struct Code {
    code: String,
    expires: DateTime,
    tries: i64,
    locked: bool,
}

/// Every notification asked for, and the one-time codes sent.
#[derive(Clone, Default)]
pub struct Mail {
    sent: Arc<Mutex<Vec<NotifyRequest>>>,
    codes: Arc<Mutex<HashMap<String, Code>>>,
    otp_requests: Arc<Mutex<Vec<OtpRequest>>>,
    serial: Arc<AtomicI64>,
    cannot_send: Arc<AtomicBool>,
    notify_fails: Arc<AtomicBool>,
}

impl Mail {
    /// Makes the next codes fail to send (or work again).
    pub fn cannot_send_codes(&self, on: bool) {
        self.cannot_send.store(on, Ordering::SeqCst);
    }

    /// Makes the next notifications fail (or work again).
    pub fn notifications_fail(&self, on: bool) {
        self.notify_fails.store(on, Ordering::SeqCst);
    }

    /// The codes asked for so far, oldest first.
    pub fn otp_requests(&self) -> Vec<OtpRequest> {
        self.otp_requests.lock().unwrap().clone()
    }

    /// The live code sent for `subject`.
    pub fn code_for(&self, subject: &Subject) -> String {
        self.codes
            .lock()
            .unwrap()
            .get(&subject.to_string())
            .map(|c| c.code.clone())
            .expect("a code was sent for the sale")
    }

    /// A code that is not the one sent for `subject`.
    pub fn wrong_code(&self, subject: &Subject) -> String {
        if self.code_for(subject) == "000000" {
            "111111".to_owned()
        } else {
            "000000".to_owned()
        }
    }

    /// The notifications of the type `key`.
    pub fn of_type(&self, key: &str) -> Vec<NotifyRequest> {
        self.sent
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
            if self.notify_fails.load(Ordering::SeqCst) {
                return Err(Error::Status(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the notifier is down".to_owned(),
                ));
            }
            self.sent.lock().unwrap().push(request);
            Ok(vec![MessageId(1)])
        })
    }

    fn send_otp<'a>(
        &'a self,
        _state: &'a AppState,
        request: OtpRequest,
    ) -> NotifyFuture<'a, OtpSent> {
        Box::pin(async move {
            self.otp_requests.lock().unwrap().push(request.clone());
            if self.cannot_send.load(Ordering::SeqCst) {
                return Ok(OtpSent::NotSent {
                    text: "Code could not be sent".to_owned(),
                });
            }
            let n = self.serial.fetch_add(1, Ordering::SeqCst);
            let expires = now() + TimeDelta::minutes(OTP_VALIDITY_MINUTES);
            self.codes.lock().unwrap().insert(
                request.subject.to_string(),
                Code {
                    code: format!("{:06}", 424_242 + n),
                    expires,
                    tries: 0,
                    locked: false,
                },
            );
            Ok(OtpSent::Sent {
                message: MessageId(1),
                expires_at: expires,
            })
        })
    }

    fn verify_otp<'a>(
        &'a self,
        _state: &'a AppState,
        _purpose: &'a str,
        subject: &'a Subject,
        code: &'a str,
    ) -> NotifyFuture<'a, OtpVerdict> {
        Box::pin(async move {
            let mut codes = self.codes.lock().unwrap();
            let key = subject.to_string();
            let Some(live) = codes.get_mut(&key) else {
                return Ok(OtpVerdict::Expired);
            };
            if live.locked {
                return Ok(OtpVerdict::Locked);
            }
            if live.expires <= now() {
                return Ok(OtpVerdict::Expired);
            }
            if live.code == code {
                codes.remove(&key);
                return Ok(OtpVerdict::Valid);
            }
            live.tries += 1;
            if live.tries >= OTP_WRONG_TRIES {
                live.locked = true;
                return Ok(OtpVerdict::Locked);
            }
            Ok(OtpVerdict::Wrong {
                tries_left: OTP_WRONG_TRIES - live.tries,
            })
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
        self.0
            .lock()
            .unwrap()
            .insert(id, Entry { party, net, gross });
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

    fn find<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
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

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        _id: InvoiceId,
        _source: Subject,
        _amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async { Ok(()) })
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
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&employee.0)
                .map(|party| StaffProfile {
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
    /// `PointsRedeemed`.
    pub redeemed: Arc<Mutex<Vec<PointsRedeemed>>>,
    /// `PointsGivenBack`.
    pub given_back: Arc<Mutex<Vec<PointsGivenBack>>>,
    /// `PointsExpired`.
    pub expired: Arc<Mutex<Vec<PointsExpired>>>,
    /// `PointsAdjusted`.
    pub adjusted: Arc<Mutex<Vec<PointsAdjusted>>>,
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
        let log = self.points.redeemed.clone();
        app.listen(move |event: PointsRedeemed, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
        let log = self.points.expired.clone();
        app.listen(move |event: PointsExpired, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
        let log = self.points.adjusted.clone();
        app.listen(move |event: PointsAdjusted, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
        let log = self.points.given_back.clone();
        app.listen(move |event: PointsGivenBack, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
    }
}
