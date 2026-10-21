//! The payment gateway behind the module (wl-payments §8, A7).
//!
//! Gateways stay behind the [`Gateway`] trait so the module never knows a
//! provider. The default, when none is installed with
//! [`WlPayments::gateway`](crate::WlPayments::gateway), is [`NoGateway`],
//! which knows no disputes. [`FakeGateway`] is for tests. The real adapter
//! (Stripe, N-Genius through `renox-billing`) comes in a later issue; no
//! keys or live calls are used here. `charge` is added to the trait by the
//! issue that needs it. [`Gateway::checkout`] starts a hosted payment,
//! [`Gateway::payment_status`] answers the timeout rule (`FIN.RUL.19`) and
//! [`Gateway::accept_dispute`] tells the gateway a chargeback is accepted
//! (`FIN.ACL.18`).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;

/// The future a gateway call returns.
pub type GatewayFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// A dispute the gateway holds open against a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisputeRef {
    /// The gateway's dispute reference.
    pub reference: String,
    /// The day the evidence is due, if the gateway says.
    pub evidence_due: Option<NaiveDate>,
}

/// The gateway's reference for a refund it accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayRefundRef {
    /// The gateway's refund reference.
    pub reference: String,
}

/// What the gateway says later about a refund it accepted (the webhook of
/// the gateway, [`crate::refund::gateway_refund_outcome`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefundOutcome {
    /// The gateway declined the refund.
    Declined {
        /// The gateway's text; may be empty.
        reason: String,
    },
    /// The gateway paid the refund out.
    Confirmed,
}

/// A hosted payment page the gateway made for an online payment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    /// The gateway's transaction (session) id, stored on the payment.
    pub transaction: String,
    /// The hosted page the party is sent to.
    pub url: String,
}

/// What the gateway says when asked about an online payment (`FIN.RUL.19`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayPaymentStatus {
    /// The payer paid.
    Paid,
    /// The gateway declined it.
    Declined {
        /// The gateway's text; may be empty.
        reason: String,
    },
    /// Not a final answer: the gateway still processes it.
    Processing,
}

/// The error of a gateway call nobody can make.
fn unavailable(what: &str) -> Error {
    Error::Internal(renox::anyhow::anyhow!("the gateway cannot {what}"))
}

/// What the module asks of a payment gateway.
pub trait Gateway: Send + Sync + 'static {
    /// The provider's code, e.g. `stripe`.
    fn provider(&self) -> &str;

    /// The key of the merchant account the payments go through: stored on
    /// each online payment, and how a later decision on it finds this
    /// gateway again ([`by_account`]). The provider's code
    /// unless a client has several accounts with the same provider.
    fn account(&self) -> &str {
        self.provider()
    }

    /// Starts a hosted payment of `amount` fils of `currency`, with the
    /// client's own `reference`, and returns the page to send the party to.
    /// The default cannot: no online payment is available.
    fn checkout<'a>(
        &'a self,
        _amount: i64,
        _currency: &'a str,
        _reference: &'a str,
    ) -> GatewayFuture<'a, Checkout> {
        Box::pin(async { Err(unavailable("start an online payment")) })
    }

    /// Asks the final state of the online payment `gateway_txn`. The default
    /// cannot, which `FIN.RUL.19` treats like a gateway that is down.
    fn payment_status<'a>(
        &'a self,
        _gateway_txn: &'a str,
    ) -> GatewayFuture<'a, GatewayPaymentStatus> {
        Box::pin(async { Err(unavailable("report a payment status")) })
    }

    /// Tells the gateway the dispute `dispute` on `gateway_txn` is accepted
    /// (not contested). The default accepts at once: a gateway without a
    /// disputes call (N-Genius) is told in its own portal and confirmed here.
    fn accept_dispute<'a>(
        &'a self,
        _gateway_txn: &'a str,
        _dispute: &'a str,
    ) -> GatewayFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    /// The dispute open against the transaction `gateway_txn`, if any.
    fn open_dispute<'a>(&'a self, gateway_txn: &'a str) -> GatewayFuture<'a, Option<DisputeRef>>;

    /// Asks the gateway to refund `amount` fils of the transaction
    /// `gateway_txn`; the answer comes later as a [`RefundOutcome`].
    fn refund<'a>(
        &'a self,
        gateway_txn: &'a str,
        amount: i64,
    ) -> GatewayFuture<'a, GatewayRefundRef>;
}

/// The gateway when none is installed: it knows no disputes.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoGateway;

impl Gateway for NoGateway {
    fn provider(&self) -> &str {
        "none"
    }

    fn open_dispute<'a>(&'a self, _gateway_txn: &'a str) -> GatewayFuture<'a, Option<DisputeRef>> {
        Box::pin(async { Ok(None) })
    }

    fn refund<'a>(
        &'a self,
        _gateway_txn: &'a str,
        _amount: i64,
    ) -> GatewayFuture<'a, GatewayRefundRef> {
        Box::pin(async {
            Err(Error::Internal(renox::anyhow::anyhow!(
                "no payment gateway is installed"
            )))
        })
    }
}

/// A gateway for tests: disputes and refund refusals are set by hand and
/// the questions are recorded. Refunds are accepted with the references
/// `re_fake_1`, `re_fake_2`, ...; the outcome is delivered by the test
/// through [`crate::refund::gateway_refund_outcome`].
#[derive(Debug, Clone)]
pub struct FakeGateway {
    account: String,
    disputes: Arc<Mutex<HashMap<String, DisputeRef>>>,
    asked: Arc<Mutex<Vec<String>>>,
    refusals: Arc<Mutex<HashMap<String, String>>>,
    refunded: Arc<Mutex<Vec<(String, i64)>>>,
    statuses: Arc<Mutex<HashMap<String, GatewayPaymentStatus>>>,
    status_down: Arc<Mutex<HashSet<String>>>,
    checkouts: Arc<Mutex<Vec<(i64, String)>>>,
    accepted: Arc<Mutex<Vec<(String, String)>>>,
    accept_refusals: Arc<Mutex<HashMap<String, String>>>,
}

/// Counts the fakes, so each has an account key of its own: the tests of one
/// process boot many apps, and a decision finds its gateway by that key.
static FAKES: AtomicUsize = AtomicUsize::new(1);

impl Default for FakeGateway {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeGateway {
    /// A gateway with no disputes, under an account key of its own.
    pub fn new() -> Self {
        FakeGateway {
            account: format!("fake-{}", FAKES.fetch_add(1, Ordering::Relaxed)),
            disputes: Arc::default(),
            asked: Arc::default(),
            refusals: Arc::default(),
            refunded: Arc::default(),
            statuses: Arc::default(),
            status_down: Arc::default(),
            checkouts: Arc::default(),
            accepted: Arc::default(),
            accept_refusals: Arc::default(),
        }
    }

    /// What the gateway answers when asked about the online payment
    /// `gateway_txn`; an unscripted payment is still `Processing`.
    pub fn set_status(&self, gateway_txn: &str, status: GatewayPaymentStatus) {
        self.statuses
            .lock()
            .expect("lock")
            .insert(gateway_txn.to_owned(), status);
    }

    /// Makes the status query of `gateway_txn` fail, like a gateway that is
    /// down.
    pub fn fail_status_query(&self, gateway_txn: &str) {
        self.status_down
            .lock()
            .expect("lock")
            .insert(gateway_txn.to_owned());
    }

    /// The hosted payments started, as (amount, reference), in order; the
    /// n-th one has the transaction `cs_fake_<n>`.
    pub fn checkouts(&self) -> Vec<(i64, String)> {
        self.checkouts.lock().expect("lock").clone()
    }

    /// Makes `accept_dispute` of `gateway_txn` fail with `message`.
    pub fn refuse_accept(&self, gateway_txn: &str, message: &str) {
        self.accept_refusals
            .lock()
            .expect("lock")
            .insert(gateway_txn.to_owned(), message.to_owned());
    }

    /// The disputes accepted, as (transaction, dispute), in order.
    pub fn accepted(&self) -> Vec<(String, String)> {
        self.accepted.lock().expect("lock").clone()
    }

    /// Opens a dispute against `gateway_txn`.
    pub fn dispute(&self, gateway_txn: &str, reference: &str) {
        self.disputes.lock().expect("lock").insert(
            gateway_txn.to_owned(),
            DisputeRef {
                reference: reference.to_owned(),
                evidence_due: None,
            },
        );
    }

    /// Makes `refund` of `gateway_txn` fail with `message`.
    pub fn refuse_refund(&self, gateway_txn: &str, message: &str) {
        self.refusals
            .lock()
            .expect("lock")
            .insert(gateway_txn.to_owned(), message.to_owned());
    }

    /// The refunds accepted, as (transaction, amount), in order.
    pub fn refunded(&self) -> Vec<(String, i64)> {
        self.refunded.lock().expect("lock").clone()
    }

    /// The transactions `open_dispute` was asked about, in order.
    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("lock").clone()
    }
}

impl Gateway for FakeGateway {
    fn provider(&self) -> &str {
        "fake"
    }

    fn account(&self) -> &str {
        &self.account
    }

    fn checkout<'a>(
        &'a self,
        amount: i64,
        _currency: &'a str,
        reference: &'a str,
    ) -> GatewayFuture<'a, Checkout> {
        let mut checkouts = self.checkouts.lock().expect("lock");
        checkouts.push((amount, reference.to_owned()));
        let transaction = format!("cs_fake_{}_{}", self.account, checkouts.len());
        let url = format!("https://pay.example/{transaction}");
        Box::pin(async move { Ok(Checkout { transaction, url }) })
    }

    fn payment_status<'a>(
        &'a self,
        gateway_txn: &'a str,
    ) -> GatewayFuture<'a, GatewayPaymentStatus> {
        let down = self
            .status_down
            .lock()
            .expect("lock")
            .contains(gateway_txn);
        let status = self
            .statuses
            .lock()
            .expect("lock")
            .get(gateway_txn)
            .cloned()
            .unwrap_or(GatewayPaymentStatus::Processing);
        Box::pin(async move {
            if down {
                Err(unavailable("answer, it is down"))
            } else {
                Ok(status)
            }
        })
    }

    fn accept_dispute<'a>(
        &'a self,
        gateway_txn: &'a str,
        dispute: &'a str,
    ) -> GatewayFuture<'a, ()> {
        let refused = self
            .accept_refusals
            .lock()
            .expect("lock")
            .get(gateway_txn)
            .cloned();
        if refused.is_none() {
            self.accepted
                .lock()
                .expect("lock")
                .push((gateway_txn.to_owned(), dispute.to_owned()));
        }
        Box::pin(async move {
            match refused {
                Some(message) => Err(Error::Internal(renox::anyhow::anyhow!(message))),
                None => Ok(()),
            }
        })
    }

    fn open_dispute<'a>(&'a self, gateway_txn: &'a str) -> GatewayFuture<'a, Option<DisputeRef>> {
        self.asked
            .lock()
            .expect("lock")
            .push(gateway_txn.to_owned());
        let found = self
            .disputes
            .lock()
            .expect("lock")
            .get(gateway_txn)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn refund<'a>(
        &'a self,
        gateway_txn: &'a str,
        amount: i64,
    ) -> GatewayFuture<'a, GatewayRefundRef> {
        let refused = self
            .refusals
            .lock()
            .expect("lock")
            .get(gateway_txn)
            .cloned();
        let result = match refused {
            Some(message) => Err(Error::Internal(renox::anyhow::anyhow!(message))),
            None => {
                let mut refunded = self.refunded.lock().expect("lock");
                refunded.push((gateway_txn.to_owned(), amount));
                Ok(GatewayRefundRef {
                    reference: format!("re_fake_{}", refunded.len()),
                })
            }
        };
        Box::pin(async move { result })
    }
}

/// The installed gateway, as `register` provides it.
#[derive(Clone)]
pub(crate) struct Installed(pub Arc<dyn Gateway>);

/// The gateway installed in `state`, if any.
pub(crate) fn installed_gateway(state: &AppState) -> Option<Arc<dyn Gateway>> {
    state
        .provided::<Installed>()
        .map(|installed| installed.0.clone())
}

/// The gateway in `state`: the one installed, else [`NoGateway`].
pub fn gateway_of(state: &AppState) -> Arc<dyn Gateway> {
    state.provided::<Installed>().map_or_else(
        || Arc::new(NoGateway) as Arc<dyn Gateway>,
        |installed| installed.0.clone(),
    )
}

/// The installed gateways by account key. A decision that is taken where no
/// [`AppState`] reaches (the final check of an approval) finds the gateway of
/// a payment here.
static REGISTRY: std::sync::LazyLock<std::sync::RwLock<HashMap<String, Arc<dyn Gateway>>>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(HashMap::new()));

/// Remembers `gateway` under its account key.
pub(crate) fn remember(gateway: &Arc<dyn Gateway>) {
    REGISTRY
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(gateway.account().to_owned(), Arc::clone(gateway));
}

/// The gateway installed under the account key `account`, if any.
pub(crate) fn by_account(account: &str) -> Option<Arc<dyn Gateway>> {
    REGISTRY
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(account)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[renox::test]
    async fn no_gateway_knows_no_disputes() {
        let gateway = NoGateway;
        assert_eq!(gateway.provider(), "none");
        assert_eq!(gateway.open_dispute("txn_1").await.expect("ok"), None);
    }

    #[renox::test]
    async fn the_fake_reports_the_disputes_set_and_records_the_questions() {
        let gateway = FakeGateway::new();
        gateway.dispute("txn_1", "dp_9");
        let found = gateway.open_dispute("txn_1").await.expect("ok");
        assert_eq!(found.map(|d| d.reference), Some("dp_9".to_owned()));
        assert_eq!(gateway.open_dispute("txn_2").await.expect("ok"), None);
        assert_eq!(gateway.asked(), ["txn_1", "txn_2"]);
        assert_eq!(gateway.provider(), "fake");
    }

    #[renox::test]
    async fn no_gateway_refuses_a_refund() {
        assert!(NoGateway.refund("txn_1", 100).await.is_err());
    }

    #[renox::test]
    async fn the_fake_accepts_refunds_in_order_unless_scripted_to_refuse() {
        let gateway = FakeGateway::new();
        gateway.refuse_refund("txn_bad", "card closed");
        let first = gateway.refund("txn_1", 50_000).await.expect("ok");
        let second = gateway.refund("txn_2", 700).await.expect("ok");
        assert_eq!(first.reference, "re_fake_1");
        assert_eq!(second.reference, "re_fake_2");
        assert!(gateway.refund("txn_bad", 1).await.is_err());
        assert_eq!(
            gateway.refunded(),
            [("txn_1".to_owned(), 50_000), ("txn_2".to_owned(), 700)]
        );
    }

    #[renox::test]
    async fn the_fake_starts_hosted_payments_and_answers_status_queries() {
        let gateway = FakeGateway::new();
        let first = gateway.checkout(25_000, "AED", "link-1").await.expect("ok");
        let second = gateway.checkout(1_000, "AED", "link-2").await.expect("ok");
        assert!(first.transaction.ends_with("_1"), "{}", first.transaction);
        assert!(second.transaction.ends_with("_2"), "{}", second.transaction);
        assert!(first.url.contains(&first.transaction));
        assert_eq!(
            gateway.checkouts(),
            [(25_000, "link-1".to_owned()), (1_000, "link-2".to_owned())]
        );
        // Unscripted: still processing; scripted: the answer; down: an error.
        assert_eq!(
            gateway.payment_status("t").await.expect("ok"),
            GatewayPaymentStatus::Processing
        );
        gateway.set_status("t", GatewayPaymentStatus::Paid);
        assert_eq!(
            gateway.payment_status("t").await.expect("ok"),
            GatewayPaymentStatus::Paid
        );
        gateway.fail_status_query("t");
        assert!(gateway.payment_status("t").await.is_err());
    }

    #[renox::test]
    async fn no_gateway_cannot_start_a_payment_or_report_a_status() {
        assert!(NoGateway.checkout(1, "AED", "x").await.is_err());
        assert!(NoGateway.payment_status("t").await.is_err());
        // Nothing to tell a gateway without a disputes call.
        assert!(NoGateway.accept_dispute("t", "d").await.is_ok());
    }

    #[renox::test]
    async fn the_fake_records_accepted_disputes_unless_scripted_to_refuse() {
        let gateway = FakeGateway::new();
        gateway.refuse_accept("bad", "dispute closed");
        gateway.accept_dispute("t", "dp_1").await.expect("ok");
        assert!(gateway.accept_dispute("bad", "dp_2").await.is_err());
        assert_eq!(gateway.accepted(), [("t".to_owned(), "dp_1".to_owned())]);
    }

    #[test]
    fn every_fake_has_an_account_key_of_its_own() {
        let a = FakeGateway::new();
        let b = FakeGateway::new();
        assert_ne!(a.account(), b.account());
        assert_eq!(a.provider(), "fake");
        let remembered: Arc<dyn Gateway> = Arc::new(a.clone());
        remember(&remembered);
        assert_eq!(by_account(a.account()).map(|g| g.provider().to_owned()), Some("fake".to_owned()));
        assert!(by_account("nobody").is_none());
    }
}
