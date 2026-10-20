//! Support for the cash custody tests: a ticket desk that records the tasks
//! asked of it, an HR desk that records the salary deductions passed to it
//! (wl-hr is not a dependency of the crate), and helpers that act through the
//! real cash pages as the staff of a client with cash holders.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_kernel::Subject;
use wl_kernel::custody::{
    CashCollection, HandoverId, HandoverPeriod, ShortageDeduction, ShortageRecovery, custody,
};
use wl_kernel::tickets::{OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef};
use wl_payments::cash_handover::{CashCount, CashEntry, CashPosting};
use wl_payments::{CashHandover, CashRecovery, permissions as perm};

use super::Booted;

/// A task creator that records the tickets it is asked to open and complete.
#[derive(Clone, Default)]
pub struct CashTickets {
    opened: Arc<Mutex<Vec<OpenTicket>>>,
    completed: Arc<Mutex<Vec<TicketId>>>,
}

impl CashTickets {
    /// The tickets opened, oldest first (ticket `n` is the n-th).
    pub fn opened(&self) -> Vec<OpenTicket> {
        self.opened.lock().unwrap().clone()
    }

    /// The tickets completed by their source.
    pub fn completed(&self) -> Vec<TicketId> {
        self.completed.lock().unwrap().clone()
    }
}

impl TaskCreator for CashTickets {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut opened = self.opened.lock().unwrap();
            opened.push(ticket);
            let n = opened.len() as i64;
            Ok(Some(TicketRef {
                id: TicketId(n),
                number: format!("TKT-{n}"),
            }))
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let opened = self.opened.lock().unwrap();
            let completed = self.completed.lock().unwrap();
            let found = opened.iter().enumerate().find(|(i, ticket)| {
                ticket.source_key.as_deref() == Some(source_key)
                    && !completed.contains(&TicketId(*i as i64 + 1))
            });
            Ok(found.map(|(i, _)| TicketRef {
                id: TicketId(i as i64 + 1),
                number: format!("TKT-{}", i + 1),
            }))
        })
    }

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        _note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            self.completed.lock().unwrap().push(ticket);
            Ok(true)
        })
    }
}

/// A wl-hr that records the salary deductions it is asked to make.
#[derive(Clone, Default)]
pub struct HrDesk(Arc<Mutex<Vec<ShortageDeduction>>>);

impl HrDesk {
    /// The deductions passed, oldest first.
    pub fn deductions(&self) -> Vec<ShortageDeduction> {
        self.0.lock().unwrap().clone()
    }
}

impl ShortageRecovery for HrDesk {
    fn deduct<'a>(
        &'a self,
        _state: &'a AppState,
        deduction: ShortageDeduction,
    ) -> wl_kernel::custody::CustodyFuture<'a, String> {
        Box::pin(async move {
            let mut passed = self.0.lock().unwrap();
            passed.push(deduction);
            Ok(format!("ADJ-{}", passed.len()))
        })
    }
}

/// `2027-04-01` as a date.
pub fn date(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

impl Booted {
    /// A member of staff who may count cash and record recoveries.
    pub async fn counter(&self, name: &str) -> User {
        self.user(name, &[perm::CASH_VIEW, perm::CASH_COUNT]).await
    }

    /// A member of staff who may write shortages off and approve write-offs.
    pub async fn cash_manager(&self, name: &str) -> User {
        self.user(
            name,
            &[perm::CASH_VIEW, perm::CASH_COUNT, perm::CASH_WRITE_OFF],
        )
        .await
    }

    /// A holder: a member of staff who holds cash and counts nothing.
    pub async fn holder(&self, name: &str) -> User {
        self.user(name, &[perm::CASH_VIEW]).await
    }

    /// `holder` collected `amount` fils on `day` for the source `n` of the
    /// delivery module; the handover.
    pub async fn collect(&self, holder: &User, day: &str, amount: i64, n: i64) -> CashHandover {
        let collection = CashCollection::new(
            holder.id,
            HandoverPeriod::day(date(day)),
            amount,
            Subject::new("delivery.delivery", n),
        )
        .location("Dubai branch");
        let id = custody(self.state())
            .collect(self.state(), collection)
            .await
            .expect("the cash is collected")
            .expect("payments is on");
        self.handover(id).await
    }

    /// The handover `id`, as stored now.
    pub async fn handover(&self, id: HandoverId) -> CashHandover {
        CashHandover::find(self.db(), id.0)
            .await
            .unwrap()
            .expect("the handover exists")
    }

    /// `handover`, as stored now.
    pub async fn reloaded(&self, handover: &CashHandover) -> CashHandover {
        self.handover(HandoverId(handover.id)).await
    }

    /// `by` posts `pairs` to `path`, as the page sends them.
    pub async fn cash_post(&self, by: &User, path: &str, pairs: &[(&str, &str)]) -> TestResponse {
        self.app.acting_as(by);
        self.app.htmx().post(path, pairs).await
    }

    /// `by` counts `handover` at `amount` (dirhams as typed).
    pub async fn count_cash(&self, by: &User, handover: &CashHandover, amount: &str) -> TestResponse {
        self.cash_post(by, &format!("/cash/{}/count", handover.id), &[("counted", amount)])
            .await
    }

    /// `by` recounts `handover` at `amount`, the holder being present.
    pub async fn recount_cash(
        &self,
        by: &User,
        handover: &CashHandover,
        amount: &str,
    ) -> TestResponse {
        self.cash_post(
            by,
            &format!("/cash/{}/recount", handover.id),
            &[("counted", amount), ("holder_present", "yes")],
        )
        .await
    }

    /// `by` records a part recovery of `amount` on `handover`.
    pub async fn recover_part(
        &self,
        by: &User,
        handover: &CashHandover,
        amount: &str,
    ) -> TestResponse {
        self.cash_post(
            by,
            &format!("/cash/{}/recoveries", handover.id),
            &[("amount", amount)],
        )
        .await
    }

    /// `by` writes off the rest of `handover`.
    pub async fn write_off_cash(&self, by: &User, handover: &CashHandover) -> TestResponse {
        self.cash_post(
            by,
            &format!("/cash/{}/write-off", handover.id),
            &[("reason", "Till error")],
        )
        .await
    }

    /// A `short` handover of `holder` on `day`: `expected` fils expected,
    /// `counted` fils counted by `counter` (a fresh counter named Chandra).
    pub async fn short_handover(
        &self,
        holder: &User,
        day: &str,
        expected: i64,
        counted: &str,
        counter: &User,
    ) -> CashHandover {
        let handover = self.collect(holder, day, expected, 1).await;
        self.count_cash(counter, &handover, counted).await.assert_ok();
        self.reloaded(&handover).await
    }

    /// The pending approval request of the subject, if one waits.
    pub async fn pending_request(
        &self,
        subject: &Subject,
    ) -> Option<wl_kernel::approvals::ApprovalRequest> {
        wl_kernel::approvals::ApprovalRequest::pending_about(self.db(), subject)
            .await
            .unwrap()
            .pop()
    }

    /// Changes the write-off limit of cash shortages in the stored policy (the
    /// module's builder sets it for the process, which parallel tests share).
    pub async fn set_cash_write_off_limit(&self, fils: i64) {
        wl_kernel::approvals::ApprovalPolicy::query()
            .where_(
                wl_kernel::approvals::ApprovalPolicy::ACTION
                    .eq(wl_payments::cash_settle::ACTION),
            )
            .update(self.db(), &[("threshold", &fils)])
            .await
            .unwrap();
    }

    /// The counts of `handover`.
    pub async fn counts(&self, handover: &CashHandover) -> Vec<CashCount> {
        CashCount::of_handover(self.db(), handover.id).await.unwrap()
    }

    /// The cash entries of `handover`.
    pub async fn entries(&self, handover: &CashHandover) -> Vec<CashEntry> {
        CashEntry::of_handover(self.db(), handover.id).await.unwrap()
    }

    /// The postings of `handover`.
    pub async fn postings(&self, handover: &CashHandover) -> Vec<CashPosting> {
        CashPosting::of_handover(self.db(), handover.id).await.unwrap()
    }

    /// The recoveries of `handover`.
    pub async fn recoveries(&self, handover: &CashHandover) -> Vec<CashRecovery> {
        CashRecovery::of_handover(self.db(), handover.id).await.unwrap()
    }
}

/// `at` as an instant.
pub fn at(text: &str) -> renox::db::DateTime {
    text.parse().expect("an instant")
}
