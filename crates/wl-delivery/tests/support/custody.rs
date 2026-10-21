//! A cash custody for the delivery tests: the crate does not depend on
//! `wl-payments`, so the tests provide a `CashCustody` that records what the
//! module collects, releases and ends, keeps one handover per holder and run,
//! and lets a test count the handover (emitting `CashHandoverCounted`, as
//! `wl-payments` does).

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::custody::{
    CashCollection, CashCustody, CashHandoverCounted, CustodyFuture, HandoverId, HandoverKind,
    HandoverStatus, HandoverSummary, PeriodKind,
};

#[derive(Default)]
struct Inner {
    collections: Vec<CashCollection>,
    released: Vec<Subject>,
    ended: Vec<(i64, Subject, DateTime)>,
    /// `(holder, run)` of the handovers counted, with their status and the
    /// amount counted.
    counted: Vec<(i64, Subject, HandoverStatus, i64)>,
}

/// The recording `CashCustody`.
#[derive(Clone, Default)]
pub struct FakeCustody(Arc<Mutex<Inner>>);

fn refused(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

impl FakeCustody {
    /// What was collected and not released, oldest first.
    pub fn collections(&self) -> Vec<CashCollection> {
        self.0.lock().unwrap().collections.clone()
    }

    /// The sources whose cash was released.
    pub fn released(&self) -> Vec<Subject> {
        self.0.lock().unwrap().released.clone()
    }

    /// The periods that ended: holder, run and when.
    pub fn ended(&self) -> Vec<(i64, Subject, DateTime)> {
        self.0.lock().unwrap().ended.clone()
    }

    /// The cash expected in the handover of `holder` for the run `period`.
    pub fn expected(&self, holder: i64, period: &Subject) -> i64 {
        self.0
            .lock()
            .unwrap()
            .collections
            .iter()
            .filter(|c| c.holder_user == holder && c.period.subject.as_ref() == Some(period))
            .map(|c| c.amount)
            .sum()
    }

    /// Counts the handover of `holder` for the run `period` as `counted_by`
    /// found `counted` fils: `counted`, or `short` when it is below what was
    /// expected, and emits `CashHandoverCounted`.
    pub async fn count(
        &self,
        state: &AppState,
        holder: i64,
        period: &Subject,
        counted: i64,
        counted_by: i64,
    ) {
        let expected = self.expected(holder, period);
        let status = if counted >= expected {
            HandoverStatus::Counted
        } else {
            HandoverStatus::Short
        };
        self.0
            .lock()
            .unwrap()
            .counted
            .push((holder, period.clone(), status, counted));
        state
            .emit(CashHandoverCounted {
                handover: HandoverId(1),
                holder_user: holder,
                period: Some(period.clone()),
                expected,
                counted,
                status,
                counted_by,
            })
            .await
            .expect("the count is announced");
    }

    fn summary(&self, holder: i64, period: &Subject) -> Option<HandoverSummary> {
        let inner = self.0.lock().unwrap();
        let first = inner
            .collections
            .iter()
            .find(|c| c.holder_user == holder && c.period.subject.as_ref() == Some(period))?;
        let expected: i64 = inner
            .collections
            .iter()
            .filter(|c| c.holder_user == holder && c.period.subject.as_ref() == Some(period))
            .map(|c| c.amount)
            .sum();
        let counted = inner
            .counted
            .iter()
            .find(|(h, p, _, _)| *h == holder && p == period);
        let (status, amount) = counted.map_or((HandoverStatus::Open, 0), |(_, _, s, a)| (*s, *a));
        Some(HandoverSummary {
            id: HandoverId(1),
            kind: HandoverKind::Period,
            status,
            holder_user: holder,
            responsible_user: holder,
            period_kind: PeriodKind::Run,
            period: Some(period.clone()),
            date: first.period.date,
            expected,
            counted: amount,
            difference: if status == HandoverStatus::Short {
                expected - amount
            } else {
                0
            },
            recovered: 0,
            ended_at: None,
            escalate: true,
        })
    }
}

impl CashCustody for FakeCustody {
    fn collect<'a>(
        &'a self,
        _state: &'a AppState,
        collection: CashCollection,
    ) -> CustodyFuture<'a, Option<HandoverId>> {
        Box::pin(async move {
            let mut inner = self.0.lock().unwrap();
            if inner
                .collections
                .iter()
                .any(|c| c.source == collection.source)
            {
                return Ok(Some(HandoverId(1)));
            }
            let counted = inner.counted.iter().any(|(h, p, _, _)| {
                *h == collection.holder_user && Some(p) == collection.period.subject.as_ref()
            });
            if counted {
                return Err(refused(
                    "The holder's cash of this period was already counted: raise a later shortage.",
                ));
            }
            inner.collections.push(collection);
            Ok(Some(HandoverId(1)))
        })
    }

    fn release<'a>(&'a self, _state: &'a AppState, source: Subject) -> CustodyFuture<'a, bool> {
        Box::pin(async move {
            let mut inner = self.0.lock().unwrap();
            let Some(index) = inner.collections.iter().position(|c| c.source == source) else {
                return Ok(false);
            };
            let collection = inner.collections[index].clone();
            let counted = inner.counted.iter().any(|(h, p, _, _)| {
                *h == collection.holder_user && Some(p) == collection.period.subject.as_ref()
            });
            if counted {
                return Err(refused(
                    "The holder's cash of this period was already counted: raise a later shortage.",
                ));
            }
            inner.collections.remove(index);
            inner.released.push(source);
            Ok(true)
        })
    }

    fn end_period<'a>(
        &'a self,
        _state: &'a AppState,
        holder_user: i64,
        period: Subject,
        ends_at: DateTime,
    ) -> CustodyFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .ended
                .push((holder_user, period, ends_at));
            Ok(())
        })
    }

    fn handover_of<'a>(
        &'a self,
        _db: &'a Db,
        holder_user: i64,
        period: &'a Subject,
    ) -> CustodyFuture<'a, Option<HandoverSummary>> {
        Box::pin(async move { Ok(self.summary(holder_user, period)) })
    }
}
