//! BL-MPW-16 (#430), 09 Manpower §4.1 row *No replacement can be found*, §4.6,
//! §7.2 `MPW.RUL.13`: 24 hours before the cover period starts (the replacement
//! request's start date and shift start time) the client and the account
//! manager are told once that cover is not found yet; nothing when the request
//! is filled; the count of the missing workers for a partly filled one; at
//! once for a request opened late. No penalty line is made.

mod support;

use manpower::cover::{self, Cover, CoverAsk, causes};
use manpower::cover_notice::notify_due;
use manpower::{
    ClientContract, ContractStatus, Deployment, DeploymentStatus, ManpowerRequest, RequestKind,
    RequestStatus, Site,
};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::notify::NotifyRequest;

use support::{Booted, TOWER, boot, date, dubai, users_of};

struct World {
    t: Booted,
    amal: User,
    contract: ClientContract,
    site: Site,
}

async fn world() -> World {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let site = t.site(TOWER, "Tower A").await;
    World {
        t,
        amal,
        contract,
        site,
    }
}

impl World {
    /// A replacement request for `headcount` guards from `start` at 07:00, in
    /// `status` (a fixture).
    async fn replacement(
        &self,
        headcount: i64,
        start: NaiveDate,
        status: RequestStatus,
    ) -> ManpowerRequest {
        let request = self
            .t
            .request(&self.contract, &self.site, headcount, status)
            .await;
        ManpowerRequest::query()
            .where_(ManpowerRequest::ID.eq(request.id))
            .update(
                self.t.db(),
                &[
                    ("kind", &RequestKind::Replacement),
                    ("start_date", &start),
                    ("shift_start", &Some("07:00".to_owned())),
                ],
            )
            .await
            .unwrap();
        self.t.reload_request(request.id).await
    }

    fn client(&self) -> Vec<NotifyRequest> {
        self.missing(self.t.notices.of_type("manpower.client_notice_urgent"))
    }

    fn staff(&self) -> Vec<NotifyRequest> {
        self.missing(self.t.notices.of_type("manpower.notice"))
    }

    fn missing(&self, all: Vec<NotifyRequest>) -> Vec<NotifyRequest> {
        all.into_iter()
            .filter(|request| request.data.to_string().contains("Cover not found yet"))
            .collect()
    }
}

/// AC1: R is `open`, to start on 2027-03-21 at 07:00; at 06:59 on 2027-03-20
/// nothing is sent; at 07:00 the client and the account manager are told once;
/// a later run sends nothing more.
#[renox::test]
async fn the_client_and_the_account_manager_are_told_once_24_hours_before() {
    let w = world().await;
    let r = w
        .replacement(1, date(2027, 3, 21), RequestStatus::Open)
        .await;

    let early = notify_due(w.t.state(), dubai(date(2027, 3, 20), 6, 59))
        .await
        .unwrap();
    assert!(early.is_empty());
    assert!(w.client().is_empty() && w.staff().is_empty());

    let sent = notify_due(w.t.state(), dubai(date(2027, 3, 20), 7, 0))
        .await
        .unwrap();
    assert_eq!(sent, vec![r.id]);
    let client = w.client();
    assert_eq!(client.len(), 1);
    let body = client[0].data.to_string();
    assert!(body.contains("1 of 1 guard still to be found"), "{body}");
    assert!(
        body.contains("Tower A") && body.contains("2027-03-21"),
        "{body}"
    );
    let staff = w.staff();
    assert_eq!(staff.len(), 1);
    assert_eq!(users_of(&staff[0]), vec![w.amal.id]);
    assert!(
        w.t.reload_request(r.id)
            .await
            .cover_notice_sent_at
            .is_some(),
        "the one-notice flag"
    );

    notify_due(w.t.state(), dubai(date(2027, 3, 20), 12, 0))
        .await
        .unwrap();
    notify_due(w.t.state(), dubai(date(2027, 3, 21), 7, 0))
        .await
        .unwrap();
    assert_eq!(w.client().len(), 1, "not repeated");
    assert_eq!(w.staff().len(), 1);
}

/// AC2: a request that is `filled` or `cancelled` by then, or that is no cover
/// request, gets no notice.
#[renox::test]
async fn nothing_is_sent_for_a_filled_or_cancelled_request_or_a_client_order() {
    let w = world().await;
    w.replacement(1, date(2027, 3, 21), RequestStatus::Filled)
        .await;
    w.replacement(1, date(2027, 3, 21), RequestStatus::Cancelled)
        .await;
    let order =
        w.t.request(&w.contract, &w.site, 1, RequestStatus::Open)
            .await;
    ManpowerRequest::query()
        .where_(ManpowerRequest::ID.eq(order.id))
        .update(
            w.t.db(),
            &[
                ("start_date", &date(2027, 3, 21)),
                ("shift_start", &Some("07:00".to_owned())),
            ],
        )
        .await
        .unwrap();
    let sent = notify_due(w.t.state(), dubai(date(2027, 3, 20), 7, 0))
        .await
        .unwrap();
    assert!(sent.is_empty());
    assert!(w.client().is_empty() && w.staff().is_empty());
}

/// AC3: R for 2 workers with one cover deployment active (`partially_filled`)
/// at 07:00 on 2027-03-20: the client and the account manager are told once and
/// the notice says one of two is still missing.
#[renox::test]
async fn a_partly_filled_request_says_how_many_are_still_missing() {
    let w = world().await;
    let r = w
        .replacement(2, date(2027, 3, 21), RequestStatus::PartiallyFilled)
        .await;
    ManpowerRequest::query()
        .where_(ManpowerRequest::ID.eq(r.id))
        .update(w.t.db(), &[("filled_count", &1_i64)])
        .await
        .unwrap();
    let sent = notify_due(w.t.state(), dubai(date(2027, 3, 20), 7, 0))
        .await
        .unwrap();
    assert_eq!(sent, vec![r.id]);
    assert_eq!(w.client().len(), 1);
    assert_eq!(w.staff().len(), 1);
    let body = w.client()[0].data.to_string();
    assert!(body.contains("1 of 2 guard still to be found"), "{body}");
}

/// AC4: a request opened at midday for the next morning at 07:00 (less than 24
/// hours before it starts) is noticed at once, when it is opened, and only once.
#[renox::test]
async fn a_request_opened_late_is_noticed_at_once() {
    let w = world().await;
    let ravi = w.t.staff.employ(&w.t.parties, 7, "Ravi Kumar", true);
    ClientContract::query()
        .where_(ClientContract::ID.eq(w.contract.id))
        .update(w.t.db(), &[("cover_required", &true)])
        .await
        .unwrap();
    let covered =
        w.t.deployment(
            &w.contract,
            &w.site,
            ravi,
            Some(date(2027, 12, 31)),
            DeploymentStatus::Active,
        )
        .await;
    let today = date(2027, 3, 20);
    let opened =
        w.t.on(today, async {
            cover::open_cover(
                w.t.state(),
                CoverAsk {
                    deployment: &Deployment::find_or_404(w.t.db(), covered.id).await.unwrap(),
                    from: date(2027, 3, 21),
                    to: None,
                    cause: causes::SUSPENSION,
                    cause_ref: Some("test:late".into()),
                },
            )
            .await
            .unwrap()
        })
        .await;
    assert!(matches!(opened, Cover::Opened(_)));
    assert_eq!(w.client().len(), 1, "told at once");
    assert_eq!(w.staff().len(), 1);

    // The run finds it done.
    w.t.on(today, async {
        notify_due(w.t.state(), dubai(today, 12, 15)).await.unwrap();
    })
    .await;
    assert_eq!(w.client().len(), 1);

    // A request three days ahead waits for its day.
    let far =
        w.t.on(today, async {
            cover::open_cover(
                w.t.state(),
                CoverAsk {
                    deployment: &Deployment::find_or_404(w.t.db(), covered.id).await.unwrap(),
                    from: date(2027, 3, 24),
                    to: Some(date(2027, 3, 26)),
                    cause: causes::LEAVE,
                    cause_ref: Some("test:far".into()),
                },
            )
            .await
            .unwrap()
        })
        .await;
    assert!(matches!(far, Cover::Opened(_)));
    assert_eq!(w.client().len(), 1, "not yet");
    notify_due(w.t.state(), dubai(date(2027, 3, 23), 7, 0))
        .await
        .unwrap();
    assert_eq!(w.client().len(), 2);
}

/// A notice that cannot be sent gives the flag back, so the next run sends it.
#[renox::test]
async fn a_failed_notice_is_sent_by_the_next_run() {
    let w = world().await;
    let r = w
        .replacement(1, date(2027, 3, 21), RequestStatus::Open)
        .await;
    w.t.notices.fail(true);
    let at = dubai(date(2027, 3, 20), 7, 0);
    assert!(notify_due(w.t.state(), at).await.is_err());
    assert!(
        w.t.reload_request(r.id)
            .await
            .cover_notice_sent_at
            .is_none()
    );
    w.t.notices.fail(false);
    let sent = notify_due(w.t.state(), at).await.unwrap();
    assert_eq!(sent, vec![r.id]);
    assert_eq!(w.client().len(), 1);
}
