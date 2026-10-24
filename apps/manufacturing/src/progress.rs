//! Which lines of a sales order have started (07 Manufacturing §7.1 *Sales
//! order* `amend`, `cancel_line`, `cancel_remaining`, `SAL.ACL.05`): the
//! vertical's answer to the kernel's `FulfilmentProgress`, which wl-sales asks
//! before it lets an order change.
//!
//! A line has **started** when a work order linked to it has started (§7.1: it
//! is `in_progress` or `on_hold`, or has material issued or output reported,
//! or is `completed` or `closed`), or when goods of a dispatch for it are
//! picked, out, delivered or lost (a `draft` or `cancelled` dispatch has not
//! started anything). Such a line needs an outcome and a manager's approval
//! to be cancelled or lowered; `MFG.RUL.09` ([`crate::order_changes`]) acts
//! after that approval.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::dispatch_directory;
use wl_kernel::sales::{FulfilmentProgress, SALES_ORDER_SUBJECT_TYPE, SalesFuture, SalesOrderId};

use crate::work_order::{WorkOrder, WorkOrderSource, WorkOrderStatus};

/// The kernel's [`FulfilmentProgress`] for 07.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProductionProgress;

impl FulfilmentProgress for ProductionProgress {
    fn started_lines<'a>(
        &'a self,
        state: &'a AppState,
        order: SalesOrderId,
        lines: &'a [i64],
    ) -> SalesFuture<'a, Vec<i64>> {
        Box::pin(async move {
            let mut started: Vec<i64> = Vec::new();
            let linked = WorkOrder::query()
                .where_(WorkOrder::SALES_ORDER_ID.eq(order.0))
                .where_(WorkOrder::SOURCE.eq(WorkOrderSource::SalesOrder))
                .get(&state.db)
                .await?;
            for work in linked {
                let Some(line) = work.sales_order_line_id else {
                    continue;
                };
                let done = matches!(
                    work.status,
                    WorkOrderStatus::Completed | WorkOrderStatus::Closed
                );
                if lines.contains(&line)
                    && !started.contains(&line)
                    && (work.started() || done)
                    && !matches!(
                        work.status,
                        WorkOrderStatus::Cancelled | WorkOrderStatus::Aborted
                    )
                {
                    started.push(line);
                }
            }
            let source = Subject::new(SALES_ORDER_SUBJECT_TYPE, order.0);
            for dispatch in dispatch_directory(state).of_source(state, &source).await? {
                if matches!(dispatch.status.as_str(), "draft" | "cancelled") {
                    continue;
                }
                for line in &dispatch.lines {
                    if let Some(source_line) = &line.source_line
                        && source_line.subject_type == "sales.sale_line"
                        && lines.contains(&source_line.subject_id)
                        && !started.contains(&source_line.subject_id)
                    {
                        started.push(source_line.subject_id);
                    }
                }
            }
            started.sort_unstable();
            Ok(started)
        })
    }
}
