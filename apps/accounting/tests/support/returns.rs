//! What the return tests share: returns and the calendar tasks that track
//! them, the people around a return (preparer, reviewer, the client's
//! approver), and the routes that drive it. The real wl-ticketing is on in
//! [`boot_real`](super::boot_real).

use accounting::returns::{self, NewReturn};
use accounting::{Engagement, Return, ReturnStatus, ReturnType};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_kernel::PartyId;
use wl_kernel::deadlines::{
    DeadlineTaskId, DeadlineTaskStatus, DeadlineTaskSummary, NewDeadlineTask, deadline_tasks,
};

use super::{Booted, Client, MANAGER, d};

/// The people of a return: who prepares it, who reviews it, who approves it
/// for the client.
pub struct Crew {
    /// The preparer (an accountant).
    pub preparer: User,
    /// The reviewer (the engagement manager).
    pub reviewer: User,
    /// The partner (files, ends).
    pub partner: User,
    /// A client user with the *owner* grant: a client approver.
    pub approver: User,
}

impl Booted {
    /// A portal user `name` linked to `person`, acting for `company` with the
    /// grant roles `roles`.
    pub async fn portal_user_with(
        &self,
        name: &str,
        person: PartyId,
        company: PartyId,
        roles: &[&str],
    ) -> User {
        let user = self.user_with_grants(name, &[]).await;
        wl_portal::PortalAccess::register(
            self.state(),
            user.id,
            wl_portal::NewGrant::new(person, company, "client", roles),
        )
        .await
        .expect("the portal grant is registered");
        user
    }

    /// The preparer, the reviewer, the partner and the client's approver
    /// (its UBO, with the *owner* grant) for `client`.
    pub async fn crew(&self, client: &Client) -> Crew {
        Crew {
            preparer: self.accountant().await,
            reviewer: self.manager().await,
            partner: self.partner().await,
            approver: self.portal_user("Omar", client.ubo, client.company).await,
        }
    }

    /// An `active` engagement of `client` (a fixture), managed by `manager`.
    pub async fn active_engagement(&self, client: &Client) -> Engagement {
        self.engagement_in(client, accounting::EngagementStatus::Active, None)
            .await
    }

    /// A VAT calendar task of `engagement` for the period ending 2026-10-31,
    /// due `due`, reviewed by the manager role, as the calendar makes it.
    pub async fn vat_task(&self, engagement: &Engagement, due: NaiveDate) -> DeadlineTaskId {
        deadline_tasks(self.state())
            .create(
                self.state(),
                NewDeadlineTask::new(
                    engagement.subject(),
                    "vat_return",
                    "VAT return, period ending 2026-10-31",
                    due,
                )
                .party(engagement.client())
                .lead_days(21)
                .reviewer_role(MANAGER)
                .evidence_required(),
            )
            .await
            .unwrap()
            .expect("ticketing is on")
    }

    /// The deadline task `id` as ticketing stores it now.
    pub async fn task_now(&self, id: DeadlineTaskId) -> DeadlineTaskSummary {
        deadline_tasks(self.state())
            .find(self.db(), id)
            .await
            .unwrap()
            .expect("the task exists")
    }

    /// `user` runs `action` on the deadline task `id` through ticketing's
    /// route.
    pub async fn task_act(
        &self,
        user: &User,
        id: DeadlineTaskId,
        action: &str,
        pairs: &[(&str, &str)],
    ) -> TestResponse {
        self.post(
            user,
            &format!("/deadline-tasks/{}/actions/{action}", id.0),
            pairs,
        )
        .await
    }

    /// Brings the task `id` to `in_review`: started and sent for review by
    /// `user` (who holds `tasks.work`).
    pub async fn task_in_review(&self, user: &User, id: DeadlineTaskId) {
        self.task_act(user, id, "start", &[]).await.assert_ok();
        self.task_act(user, id, "submit_review", &[])
            .await
            .assert_ok();
        assert_eq!(self.task_now(id).await.status, DeadlineTaskStatus::InReview);
    }

    /// A `preparing` VAT return of `engagement` for the period ending
    /// 2026-10-31, due `due`, linked to the engagement's open VAT task of
    /// that date.
    pub async fn open_return(&self, engagement: &Engagement, due: NaiveDate) -> Return {
        returns::open(
            self.state(),
            None,
            NewReturn::new(engagement.id, ReturnType::Vat, d(10, 31), due),
        )
        .await
        .expect("the return opens")
    }

    /// `ret`, as stored now.
    pub async fn return_now(&self, ret: &Return) -> Return {
        Return::find(self.db(), ret.id).await.unwrap().unwrap()
    }

    /// The status of `ret` now.
    pub async fn return_status(&self, ret: &Return) -> ReturnStatus {
        self.return_now(ret).await.status
    }

    /// `user` runs `action` on `ret` through the staff route, with `pairs`.
    pub async fn ret_act(
        &self,
        user: &User,
        ret: &Return,
        action: &str,
        pairs: &[(&str, &str)],
    ) -> TestResponse {
        self.post(
            user,
            &format!("/accounting/returns/{}/{action}", ret.id),
            pairs,
        )
        .await
    }

    /// `user` runs `action` on `ret` through the portal route, with `pairs`.
    pub async fn portal_act(
        &self,
        user: &User,
        ret: &Return,
        action: &str,
        pairs: &[(&str, &str)],
    ) -> TestResponse {
        self.post(
            user,
            &format!("/accounting/portal/returns/{}/{action}", ret.id),
            pairs,
        )
        .await
    }

    /// Takes `ret` through the real routes up to `status`: the figures and
    /// workpapers by the preparer, `submit`, `review` by the reviewer,
    /// `approve` by the client's approver.
    pub async fn return_at(&self, ret: &Return, crew: &Crew, status: ReturnStatus) -> Return {
        if status != ReturnStatus::Preparing {
            self.ret_act(
                &crew.preparer,
                ret,
                "figures",
                &[
                    ("figures", "Output VAT 12,000; input VAT 4,000"),
                    ("workpapers", "on"),
                ],
            )
            .await
            .assert_ok();
            self.ret_act(&crew.preparer, ret, "submit", &[])
                .await
                .assert_ok();
        }
        if matches!(
            status,
            ReturnStatus::ClientApproval | ReturnStatus::Approved
        ) {
            self.ret_act(&crew.reviewer, ret, "review", &[])
                .await
                .assert_ok();
        }
        if status == ReturnStatus::Approved {
            self.portal_act(&crew.approver, ret, "approve", &[])
                .await
                .assert_ok();
        }
        let now = self.return_now(ret).await;
        assert_eq!(now.status, status);
        now
    }

    /// Takes the `preparing` `ret` (reworked after a withdrawal) through
    /// `submit` and `review` again, by the crew.
    pub async fn return_at_again(&self, ret: &Return, crew: &Crew) {
        self.ret_act(&crew.preparer, ret, "submit", &[])
            .await
            .assert_ok();
        self.ret_act(&crew.reviewer, ret, "review", &[])
            .await
            .assert_ok();
        assert_eq!(self.return_status(ret).await, ReturnStatus::ClientApproval);
    }
}
