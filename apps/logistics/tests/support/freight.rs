//! Freight job fixtures: a job is opened and moved along its lifecycle
//! through the real routes, as the documentation officer would.

#![allow(dead_code)]

use logistics::{FreightJob, FreightStatus};
use renox::prelude::*;

use super::World;

impl World {
    /// The documentation officer opens a sea job for Souq Express.
    pub async fn open_job(&self) -> FreightJob {
        let client = *self.job_client.lock().unwrap();
        self.post(
            &self.docs,
            "/freight",
            &[
                ("client_party_id", client.0.to_string().as_str()),
                ("description", "1 x 40HC of kitchenware"),
                ("mode", "sea"),
                ("origin", "Shanghai"),
                ("destination", "Jebel Ali"),
                ("eta", "2026-11-20"),
            ],
        )
        .await
        .assert_status(303);
        let job = FreightJob::query()
            .order_by_desc(FreightJob::ID)
            .first(self.db())
            .await
            .unwrap()
            .expect("the job is opened");
        assert_eq!(job.status, FreightStatus::Booked);
        job
    }

    /// The job `id`, as stored now.
    pub async fn job(&self, id: i64) -> FreightJob {
        FreightJob::find_or_404(self.db(), id).await.unwrap()
    }

    /// `user` runs the action `action` of the job with `fields`.
    pub async fn job_action(
        &self,
        user: &User,
        job: &FreightJob,
        action: &str,
        fields: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.post(user, &format!("/freight/{}/{action}", job.id), fields)
            .await
    }

    /// The job has departed (`in_transit`).
    pub async fn in_transit(&self) -> FreightJob {
        let job = self.open_job().await;
        self.job_action(&self.docs, &job, "depart", &[("bl_awb", "MAEU123456789")])
            .await
            .assert_status(303);
        let after = self.job(job.id).await;
        assert_eq!(after.status, FreightStatus::InTransit);
        after
    }

    /// The job is in customs clearance.
    pub async fn in_customs(&self) -> FreightJob {
        let job = self.in_transit().await;
        self.job_action(&self.docs, &job, "arrive", &[])
            .await
            .assert_status(303);
        let after = self.job(job.id).await;
        assert_eq!(after.status, FreightStatus::CustomsClearance);
        after
    }

    /// The job is cleared (`ready_for_delivery`).
    pub async fn ready_for_delivery(&self) -> FreightJob {
        let job = self.in_customs().await;
        self.job_action(
            &self.docs,
            &job,
            "clear",
            &[
                ("declaration_no", "DEC-2026-0001"),
                ("duties", "250.00"),
                ("delivery_order", "DO-0001"),
            ],
        )
        .await
        .assert_status(303);
        let after = self.job(job.id).await;
        assert_eq!(after.status, FreightStatus::ReadyForDelivery);
        after
    }

    /// The job is `delivered`.
    pub async fn job_delivered(&self) -> FreightJob {
        let job = self.ready_for_delivery().await;
        self.job_action(
            &self.docs,
            &job,
            "deliver",
            &[("pod_reference", "epod-9001")],
        )
        .await
        .assert_status(303);
        let after = self.job(job.id).await;
        assert_eq!(after.status, FreightStatus::Delivered);
        after
    }

    /// A revenue line of `amount` dirhams, issued, on the job.
    pub async fn job_revenue(&self, job: &FreightJob, amount: &str) {
        self.post(
            &self.docs,
            &format!("/freight/{}/revenue", job.id),
            &[
                ("description", "Freight and clearance"),
                ("amount", amount),
                ("issued", "1"),
            ],
        )
        .await
        .assert_status(303);
    }

    /// A cost of `amount` dirhams on the job, allocated to the company.
    pub async fn job_cost(&self, job: &FreightJob, amount: &str) -> i64 {
        self.post(
            &self.docs,
            &format!("/freight/{}/costs", job.id),
            &[
                ("description", "Shipping line bill"),
                ("amount", amount),
                ("source_ref", "SB-1001"),
            ],
        )
        .await
        .assert_status(303);
        let line = logistics::JobLine::query()
            .order_by_desc(logistics::JobLine::ID)
            .first(self.db())
            .await
            .unwrap()
            .expect("the cost is recorded");
        self.post(
            &self.docs,
            &format!("/freight/lines/{}/allocate", line.id),
            &[("liability", "company")],
        )
        .await
        .assert_status(303);
        line.id
    }

    /// The job is `closed` with revenue AED 10,000 and a cost of AED 7,000:
    /// a margin of AED 3,000.
    pub async fn job_closed(&self) -> FreightJob {
        let job = self.job_delivered().await;
        self.job_revenue(&job, "10000.00").await;
        self.job_cost(&job, "7000.00").await;
        self.job_action(&self.docs, &job, "close", &[])
            .await
            .assert_status(303);
        let after = self.job(job.id).await;
        assert_eq!(after.status, FreightStatus::Closed);
        after
    }
}
