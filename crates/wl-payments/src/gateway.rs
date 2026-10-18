//! The payment gateway behind the module (wl-payments §8, A7).
//!
//! Gateways stay behind the [`Gateway`] trait so the module never knows a
//! provider. The default, when none is installed with
//! [`WlPayments::gateway`](crate::WlPayments::gateway), is [`NoGateway`],
//! which knows no disputes. [`FakeGateway`] is for tests. The real adapter
//! (Stripe, N-Genius through `renox-billing`) comes in a later issue; no
//! keys or live calls are used here. `charge`, `refund` and
//! `accept_dispute` are added to the trait by the issues that need them.

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

/// What the module asks of a payment gateway.
pub trait Gateway: Send + Sync + 'static {
    /// The provider's code, e.g. `stripe`.
    fn provider(&self) -> &str;

    /// The dispute open against the transaction `gateway_txn`, if any.
    fn open_dispute<'a>(&'a self, gateway_txn: &'a str) -> GatewayFuture<'a, Option<DisputeRef>>;
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
}

/// A gateway for tests: disputes are set by hand and the questions are
/// recorded.
#[derive(Debug, Clone, Default)]
pub struct FakeGateway {
    disputes: Arc<Mutex<HashMap<String, DisputeRef>>>,
    asked: Arc<Mutex<Vec<String>>>,
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
}
