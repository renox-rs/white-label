//! The work orders of the demo company, in several states: planned, running
//! with material issued and output reported (one report typed in later from a
//! paper job card), held for a machine, completed and closed with its cost,
//! and one whose abort waits for the production manager.

use manufacturing::WorkOrder;
use renox::chrono::Duration;
use renox::prelude::*;

use super::{
    MANAGER_EMAIL, OPERATOR_EMAIL, PLANNER_EMAIL, STORE_EMAIL, SUPERVISOR_EMAIL, Seed, day,
};

impl Seed {
    /// A `planned` work order for `quantity` of `code` on line `line` (0 or
    /// 1), linked to the sale line of order `order` when given.
    async fn work_order(
        &mut self,
        key: &'static str,
        code: &'static str,
        quantity: &str,
        line: usize,
        order: Option<&'static str>,
    ) -> Result<i64> {
        let made = self
            .driver
            .post(
                PLANNER_EMAIL,
                "/manufacturing/work-orders",
                &[
                    ("plant_id", &self.plant.to_string()),
                    ("line_id", &self.lines[line].to_string()),
                    ("item_id", &self.items[code].to_string()),
                    ("bom_id", &self.boms[code].to_string()),
                    ("quantity", quantity),
                    ("planned_start", &day(-1).to_string()),
                    ("planned_end", &day(6).to_string()),
                ],
            )
            .await?;
        let id = made
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the work order {key}"))?;
        if let Some(order) = order {
            let (sales_order, sale_line) = self.orders[order];
            WorkOrder::query()
                .where_(WorkOrder::ID.eq(id))
                .update(
                    self.db(),
                    &[
                        ("source", &manufacturing::WorkOrderSource::SalesOrder),
                        ("sales_order_id", &Some(sales_order)),
                        ("sales_order_line_id", &Some(sale_line)),
                    ],
                )
                .await?;
        }
        self.work_orders.insert(key, id);
        Ok(id)
    }

    async fn act(&self, who: &str, id: i64, action: &str, form: &[(&str, &str)]) -> Result {
        self.driver
            .post(
                who,
                &format!("/manufacturing/work-orders/{id}/{action}"),
                form,
            )
            .await?;
        Ok(())
    }

    async fn issue(&self, id: i64, code: &str, quantity: &str, lot: &str) -> Result {
        self.act(
            STORE_EMAIL,
            id,
            "issue",
            &[
                ("item_id", &self.items[code].to_string()),
                ("quantity", quantity),
                ("lot", lot),
            ],
        )
        .await
    }

    /// The work orders.
    pub(crate) async fn work(&mut self) -> Result {
        // Planned, for Gulf Fresh Foods' boxes: waits for the planner to release it.
        self.work_order("fresh_box", "BOX-FOOD", "2400", 0, Some("fresh"))
            .await?;

        // Running, for Al Noor's order: material issued, output reported,
        // and a paper job card of the night shift typed in this morning.
        let id = self
            .work_order("noor_box", "BOX-FOOD", "1000", 0, Some("noor"))
            .await?;
        self.act(PLANNER_EMAIL, id, "release", &[]).await?;
        self.issue(id, "KRAFT-LINER", "350", "KL-2305").await?;
        self.issue(id, "FLUTING-MED", "150", "FM-118").await?;
        self.act(SUPERVISOR_EMAIL, id, "start", &[]).await?;
        self.act(
            OPERATOR_EMAIL,
            id,
            "report",
            &[("good", "640"), ("scrap", "8"), ("note", "Day shift")],
        )
        .await?;
        self.act(
            SUPERVISOR_EMAIL,
            id,
            "report",
            &[
                ("good", "160"),
                ("note", "Night shift, paper job card"),
                (
                    "occurred_at",
                    &wl_kernel::BusinessCalendar::default()
                        .local(renox::db::now() - Duration::hours(9))
                        .format("%Y-%m-%dT%H:%M")
                        .to_string(),
                ),
                ("late_reason", "Line tablet offline during the night shift"),
                ("outage_reference", "Wi-Fi outage, line 1"),
            ],
        )
        .await?;

        // Started for Emirates Print House's labels (customer specific).
        let id = self
            .work_order("eph_label", "SLV-EPH", "50000", 1, Some("eph"))
            .await?;
        self.act(PLANNER_EMAIL, id, "release", &[]).await?;
        self.issue(id, "SLEEVE-FILM", "200", "SF-9031").await?;
        self.issue(id, "INK-BLUE", "30", "IK-4410").await?;
        self.act(SUPERVISOR_EMAIL, id, "start", &[]).await?;
        self.act(
            SUPERVISOR_EMAIL,
            id,
            "report",
            &[("good", "18000"), ("scrap", "300")],
        )
        .await?;

        // For stock, held for a machine.
        let id = self
            .work_order("stock_rtl", "BOX-RTL", "2000", 0, None)
            .await?;
        self.act(PLANNER_EMAIL, id, "release", &[]).await?;
        self.issue(id, "KRAFT-LINER", "500", "KL-2291").await?;
        self.act(SUPERVISOR_EMAIL, id, "start", &[]).await?;
        self.act(SUPERVISOR_EMAIL, id, "hold", &[("reason", "machine")])
            .await?;

        // Completed and closed, with its cost.
        let id = self
            .work_order("done_rtl", "BOX-RTL", "400", 0, None)
            .await?;
        self.act(PLANNER_EMAIL, id, "release", &[]).await?;
        self.issue(id, "KRAFT-LINER", "100", "KL-2305").await?;
        self.act(SUPERVISOR_EMAIL, id, "start", &[]).await?;
        self.act(
            OPERATOR_EMAIL,
            id,
            "report",
            &[("good", "396"), ("scrap", "4")],
        )
        .await?;
        self.act(SUPERVISOR_EMAIL, id, "complete", &[]).await?;
        self.act(MANAGER_EMAIL, id, "close", &[]).await?;

        // Started on the wrong BOM, the abort waits for the production manager.
        let id = self
            .work_order("abort_rtl", "BOX-RTL", "600", 0, None)
            .await?;
        self.act(PLANNER_EMAIL, id, "release", &[]).await?;
        self.issue(id, "KRAFT-LINER", "150", "KL-2305").await?;
        self.act(SUPERVISOR_EMAIL, id, "start", &[]).await?;
        let key = format!(
            "return_{}_{}",
            self.items["KRAFT-LINER"], self.lots["KL-2305"]
        );
        self.act(
            PLANNER_EMAIL,
            id,
            "abort",
            &[("reason", "Wrong mould fitted"), (&key, "120")],
        )
        .await?;
        Ok(())
    }

    /// An abort the production manager approved (the list of aborted work
    /// orders has a row with its asker and approver), and a long list of
    /// planned work orders for stock (the list pages).
    pub(crate) async fn more_work(&mut self) -> Result {
        let id = self
            .work_order("aborted_rtl", "BOX-RTL", "300", 0, None)
            .await?;
        self.act(PLANNER_EMAIL, id, "release", &[]).await?;
        self.issue(id, "KRAFT-LINER", "80", "KL-2305").await?;
        self.act(SUPERVISOR_EMAIL, id, "start", &[]).await?;
        let key = format!(
            "return_{}_{}",
            self.items["KRAFT-LINER"], self.lots["KL-2305"]
        );
        self.act(
            PLANNER_EMAIL,
            id,
            "abort",
            &[("reason", "Customer withdrew the order"), (&key, "60")],
        )
        .await?;
        let request = wl_kernel::approvals::ApprovalRequest::query()
            .where_(wl_kernel::approvals::ApprovalRequest::SUBJECT_ID.eq(id))
            .order_by_desc(wl_kernel::approvals::ApprovalRequest::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the abort of {id} was not asked"))?;
        self.driver
            .post(
                MANAGER_EMAIL,
                &format!("/approvals/{}/approve", request.id),
                &[],
            )
            .await?;
        // Planned for stock over the next weeks: the list has more than one
        // page.
        for n in 0..26_usize {
            let (code, quantity) = if n % 2 == 0 {
                ("BOX-FOOD", "500")
            } else {
                ("BOX-RTL", "800")
            };
            self.work_order("stock_more", code, quantity, n % 2, None)
                .await?;
        }
        Ok(())
    }
}
