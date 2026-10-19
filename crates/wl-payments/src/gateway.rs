//! The payment gateway behind the module (wl-payments §8, A7).
//!
//! Gateways stay behind the [`Gateway`] trait so the module never knows a
//! provider. The default, when none is installed with
//! [`WlPayments::gateway`](crate::WlPayments::gateway), is [`NoGateway`],
//! which knows no disputes. [`FakeGateway`] is for tests. The real adapter
//! (Stripe, N-Genius through `renox-billing`) comes in a later issue; no
//! keys or live calls are used here. `charge` and `accept_dispute` are
//! added to the trait by the issues that need them.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
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

/// What the module asks of a payment gateway.
pub trait Gateway: Send + Sync + 'static {
    /// The provider's code, e.g. `stripe`.
    fn provider(&self) -> &str;

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
#[derive(Debug, Clone, Default)]
pub struct FakeGateway {
    disputes: Arc<Mutex<HashMap<String, DisputeRef>>>,
    asked: Arc<Mutex<Vec<String>>>,
    refusals: Arc<Mutex<HashMap<String, String>>>,
    refunded: Arc<Mutex<Vec<(String, i64)>>>,
}

impl FakeGateway {
    /// A gateway with no disputes.
    pub fn new() -> Self {
        Self::default()
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

/// The gateway in `state`: the one installed, else [`NoGateway`].
pub fn gateway_of(state: &AppState) -> Arc<dyn Gateway> {
    state.provided::<Installed>().map_or_else(
        || Arc::new(NoGateway) as Arc<dyn Gateway>,
        |installed| installed.0.clone(),
    )
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
}
