//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::fmt::Display;

use renox::prelude::*;

/// Every key the app uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    (
        "manufacturing.bad_input",
        "Check the form: a value is missing or not in the right form.",
    ),
    ("manufacturing.notice.subject", "{{ title }}"),
    ("manufacturing.notice.body", "{{ body }}"),
    // The work orders follow their sales order (`MFG.RUL.09`).
    (
        "manufacturing.order_change.title",
        "Sales order {{ sales }}, line {{ line }}: work orders adjusted",
    ),
    (
        "manufacturing.order_change.quantity",
        "{{ order }}: quantity {{ from }} to {{ to }} for {{ sales }}.",
    ),
    (
        "manufacturing.order_change.cancelled",
        "{{ order }} is cancelled: {{ sales }} no longer needs it.",
    ),
    (
        "manufacturing.order_change.cancel_reason",
        "Sales order {{ sales }} was changed or cancelled",
    ),
    (
        "manufacturing.order_change.unlinked",
        "{{ order }} keeps running for stock: {{ sales }} no longer needs it.",
    ),
    (
        "manufacturing.order_change.unlink_reason",
        "Sales order {{ sales }} no longer needs it: finish to stock",
    ),
    (
        "manufacturing.order_change.for_stock",
        "{{ order }} keeps its quantity; {{ quantity }} of its output is for stock because {{ sales }} needs less.",
    ),
    (
        "manufacturing.order_change.for_stock_down",
        "{{ order }}: the part of its output for stock is now {{ quantity }}.",
    ),
    (
        "manufacturing.order_change.extra",
        "New planned work order {{ order }} for {{ quantity }}: {{ sales }} needs more than {{ started }} makes.",
    ),
    (
        "manufacturing.order_change.abort_asked",
        "{{ order }}: the production manager was asked to request the abort ({{ sales }} was cancelled with the outcome scrap).",
    ),
    (
        "manufacturing.order_change.abort_title",
        "Request the abort of {{ order }}",
    ),
    (
        "manufacturing.order_change.abort_body",
        "Sales order {{ sales }} was cancelled with the outcome scrap. {{ order }} has material issued or output reported, so only an abort can stop it. Open the work order, choose Abort and send the request with this reason: {{ reason }}",
    ),
    (
        "manufacturing.order_change.abort_done",
        "The abort was approved.",
    ),
    (
        "manufacturing.order_change.abort_reason",
        "Sales order {{ sales }} cancelled, outcome scrap: {{ why }}",
    ),
    (
        "manufacturing.screen.changes.title",
        "Changes from the sales order",
    ),
    ("manufacturing.screen.changes.when", "When"),
    ("manufacturing.screen.changes.what", "What"),
    ("manufacturing.screen.changes.detail", "Detail"),
    ("manufacturing.screen.change.quantity", "Quantity changed"),
    ("manufacturing.screen.change.cancel", "Cancelled"),
    ("manufacturing.screen.change.unlink", "Left the sales order"),
    ("manufacturing.screen.change.for_stock", "Output for stock"),
    ("manufacturing.screen.change.extra_order", "New work order"),
    ("manufacturing.screen.change.abort_task", "Abort asked for"),
    (
        "manufacturing.screen.change.held",
        "Added quantity held until paid",
    ),
    (
        "manufacturing.screen.change.released",
        "Held quantity released",
    ),
    (
        "manufacturing.screen.change.detail.quantity",
        "From {{ from }} to {{ to }}. {{ note }}",
    ),
    (
        "manufacturing.screen.change.detail.cancel",
        "The sales order no longer needs the {{ from }}.",
    ),
    (
        "manufacturing.screen.change.detail.unlink",
        "It keeps running for stock ({{ from }}).",
    ),
    (
        "manufacturing.screen.change.detail.for_stock",
        "{{ to }} of its output is for stock (before: {{ from }}).",
    ),
    (
        "manufacturing.screen.change.detail.extra_order",
        "A planned work order for {{ to }}. {{ note }}",
    ),
    (
        "manufacturing.screen.change.detail.abort_task",
        "The production manager was asked to request the abort of {{ from }}; {{ to }} is no longer needed.",
    ),
    (
        "manufacturing.screen.change.detail.held",
        "{{ to }} waits for the customer's payment.",
    ),
    (
        "manufacturing.screen.change.detail.released",
        "{{ to }} is paid and can be made.",
    ),
    (
        "manufacturing.screen.change.abort_note",
        "The sales order was cancelled with the outcome scrap. Request the abort of this work order; the reason is filled in: {{ reason }}",
    ),
    // Returned and refused goods (`MFG.RUL.10`).
    (
        "manufacturing.returns.inspect_title",
        "Inspect returned lot {{ lot }} ({{ dispatch }})",
    ),
    (
        "manufacturing.returns.inspect_body",
        "{{ quantity }} of {{ lot }} came back from {{ dispatch }} into quarantine ({{ reason }}). Inspect it on the inspections page: pass it to release the lot, or fail it to raise an NCR. The lot is not released before.",
    ),
    (
        "manufacturing.returns.complaint_link",
        "Inspection started for {{ lot }}, {{ quantity }} in quarantine.",
    ),
    (
        "manufacturing.returns.decide_title",
        "Order {{ order }}, line {{ line }}: {{ quantity }} came back",
    ),
    (
        "manufacturing.returns.decide_body",
        "{{ quantity }} of {{ item }} on order {{ order }}, line {{ line }}, came back on {{ dispatch }} ({{ reason }}). Deliver the open quantity again (New dispatch) or cancel it (Cancel the line) on the returned goods page. The goods are in quarantine and wait for QC. Nothing is cancelled for you.",
    ),
    (
        "manufacturing.returns.more_came_back",
        "{{ quantity }} more came back.",
    ),
    (
        "manufacturing.returns.open_now",
        "{{ done }} handled; {{ quantity }} is still open.",
    ),
    (
        "manufacturing.returns.closed.delivered_again",
        "All of it is delivered again.",
    ),
    (
        "manufacturing.returns.closed.cancelled",
        "All of it is delivered again or cancelled.",
    ),
    (
        "manufacturing.returns.closed.order_cancelled",
        "The order was cancelled.",
    ),
    (
        "manufacturing.returns.closed",
        "This decision is closed: nothing is open any more.",
    ),
    (
        "manufacturing.returns.quantity",
        "Give a quantity above zero and not above the {{ open }} that is open.",
    ),
    (
        "manufacturing.returns.cannot_dispatch",
        "A dispatch cannot be made from the original: it is not known any more.",
    ),
    (
        "manufacturing.returns.dispatch_made",
        "A draft dispatch was made for {{ quantity }}.",
    ),
    (
        "manufacturing.returns.inspected",
        "This inspection has its result already.",
    ),
    (
        "manufacturing.returns.waiting",
        "The result of this inspection waits for the production manager's approval.",
    ),
    (
        "manufacturing.late_inspection.future",
        "The time of the decision is in the future.",
    ),
    (
        "manufacturing.returns.passed_said",
        "Inspection passed: the lot is released.",
    ),
    (
        "manufacturing.returns.failed_said",
        "Inspection failed: NCR {{ ncr }} is raised.",
    ),
    (
        "manufacturing.returns.keeps_lot",
        "The goods that came back on {{ dispatch }} wait for their inspection: pass it on the inspections page first.",
    ),
    ("manufacturing.screen.returns.title", "Returned goods"),
    (
        "manufacturing.screen.returns.subtitle",
        "What customers refused or sent back, and what sales decides: deliver the open quantity again or cancel it.",
    ),
    ("manufacturing.screen.returns.filter", "Show"),
    ("manufacturing.screen.returns.order", "Order"),
    ("manufacturing.screen.returns.line_name", "line {{ id }}"),
    ("manufacturing.screen.returns.refused", "Came back"),
    ("manufacturing.screen.returns.open", "Still open"),
    ("manufacturing.screen.returns.reason", "Reason"),
    ("manufacturing.screen.returns.dispatch", "Came back on"),
    ("manufacturing.screen.returns.owner", "Sales person"),
    ("manufacturing.screen.returns.new_dispatch", "New dispatch"),
    (
        "manufacturing.screen.returns.new_dispatch_hint",
        "Makes a draft dispatch to the same customer and address for this quantity. The dispatcher picks and sends it; the task closes when it has left.",
    ),
    ("manufacturing.screen.returns.quantity", "Quantity"),
    (
        "manufacturing.screen.returns.quantity_hint",
        "Leave as it is for everything that is open, or give less.",
    ),
    (
        "manufacturing.screen.returns.bad_quantity",
        "Give the quantity as a number, like 10 or 2.5.",
    ),
    (
        "manufacturing.screen.returns.cancel_line",
        "Cancel the line",
    ),
    (
        "manufacturing.screen.returns.closed_reason",
        "Nothing is open: this decision is closed.",
    ),
    (
        "manufacturing.screen.returns.dispatch_done",
        "A draft dispatch was made. Pick it and send it.",
    ),
    ("manufacturing.screen.returns.empty", "Nothing came back"),
    (
        "manufacturing.screen.returns.empty_text",
        "Goods a customer refuses or sends back wait here for sales to decide.",
    ),
    ("manufacturing.screen.return_status.open", "Open"),
    ("manufacturing.screen.return_status.closed", "Closed"),
    (
        "manufacturing.screen.return_closed.delivered_again",
        "Delivered again",
    ),
    ("manufacturing.screen.return_closed.cancelled", "Cancelled"),
    (
        "manufacturing.screen.return_closed.order_cancelled",
        "The order was cancelled",
    ),
    ("manufacturing.screen.inspections.title", "Returned lots"),
    (
        "manufacturing.screen.inspections.subtitle",
        "Each lot that came back into quarantine is inspected before it is released: pass it, or fail it and raise an NCR.",
    ),
    ("manufacturing.screen.inspections.filter", "Show"),
    ("manufacturing.screen.inspections.lot", "Lot"),
    ("manufacturing.screen.inspections.quantity", "Quantity"),
    ("manufacturing.screen.inspections.dispatch", "Came back on"),
    ("manufacturing.screen.inspections.complaint", "Complaint"),
    ("manufacturing.screen.inspections.ncr", "NCR"),
    (
        "manufacturing.screen.inspections.whole",
        "The whole dispatch came back",
    ),
    (
        "manufacturing.screen.inspections.refused",
        "Refused by the customer",
    ),
    ("manufacturing.screen.inspections.pass", "Pass"),
    (
        "manufacturing.screen.inspections.pass_hint",
        "The goods are fit to sell: the lot is released. The storekeeper then moves them from quarantine to a store with a transfer.",
    ),
    ("manufacturing.screen.inspections.note", "Note"),
    ("manufacturing.screen.inspections.fail", "Fail"),
    (
        "manufacturing.screen.inspections.fail_hint",
        "The goods are not fit to sell: an NCR is raised on the lot for the quantity that came back, and QC decides rework, scrap or return.",
    ),
    ("manufacturing.screen.inspections.defect", "What is wrong"),
    (
        "manufacturing.screen.inspections.pass_done",
        "The lot is released. Move the goods from quarantine to a store with a transfer.",
    ),
    (
        "manufacturing.screen.inspections.fail_done",
        "An NCR is raised on the lot.",
    ),
    ("manufacturing.screen.inspections.empty", "No lot came back"),
    (
        "manufacturing.screen.inspections.empty_text",
        "Lots that come back into quarantine wait here for their inspection.",
    ),
    ("manufacturing.screen.inspections.occurred_at", "Decided at"),
    (
        "manufacturing.screen.inspections.occurred_hint",
        "Leave empty for a decision made now. Fill it in only when you looked at the goods earlier and are recording the result later.",
    ),
    (
        "manufacturing.screen.inspections.late_reason",
        "Why it is entered late",
    ),
    (
        "manufacturing.screen.inspections.outage",
        "Outage reference",
    ),
    (
        "manufacturing.screen.inspections.late",
        "Entered late: {{ reason }}",
    ),
    (
        "manufacturing.screen.inspections.waits",
        "Waiting for the production manager to approve the {{ decision }} decided on {{ at }}",
    ),
    (
        "manufacturing.screen.inspections.blocked_waits",
        "The result you entered waits for the production manager's approval.",
    ),
    (
        "manufacturing.screen.inspections.late_requested",
        "The late entry waits for the production manager's approval.",
    ),
    (
        "manufacturing.screen.inspections.pass_late_done",
        "The lot is released with the late-entry flag. Move the goods from quarantine to a store with a transfer.",
    ),
    (
        "manufacturing.screen.inspections.fail_late_done",
        "An NCR is raised on the lot with the late-entry flag.",
    ),
    ("manufacturing.screen.inspection_status.open", "Waiting"),
    ("manufacturing.screen.inspection_status.passed", "Passed"),
    ("manufacturing.screen.inspection_status.failed", "Failed"),
    // The non-conformance report (`MFG.QCL.02`).
    (
        "manufacturing.ncr.choose",
        "Choose what becomes of the goods: rework, scrap, use as is or return to the supplier.",
    ),
    (
        "manufacturing.ncr.reopen_done",
        "The action of the disposition has happened: the NCR cannot be reopened.",
    ),
    (
        "manufacturing.ncr.close_not_done",
        "The action of the disposition is not done yet: the rework work order is not completed, the scrap has not posted or the Return to supplier is not dispatched.",
    ),
    ("manufacturing.ncr.defect", "Describe the defect."),
    ("manufacturing.ncr.quantity", "Give a quantity above zero."),
    (
        "manufacturing.ncr.item",
        "Name the lot, or the item when it has no lot.",
    ),
    ("manufacturing.ncr.lot_unknown", "The lot is not known."),
    (
        "manufacturing.ncr.too_much",
        "The lot holds only {{ available }}.",
    ),
    (
        "manufacturing.ncr.cause_closed",
        "The cause is changed until the NCR is closed.",
    ),
    (
        "manufacturing.ncr.not_open",
        "Only an open NCR is dispositioned.",
    ),
    (
        "manufacturing.ncr.waiting",
        "A decision on this NCR waits for an approval.",
    ),
    (
        "manufacturing.ncr.needs_lot",
        "This NCR is not on a lot: it cannot be scrapped, used as is or returned.",
    ),
    (
        "manufacturing.ncr.use_as_is_status",
        "The lot cannot be released: it is not in quarantine, on hold or released.",
    ),
    (
        "manufacturing.ncr.own_production",
        "The lot was made in-house: it cannot be returned to a supplier.",
    ),
    (
        "manufacturing.ncr.no_supplier_lot",
        "The supplier lot is not known (the lot has no goods receipt): choose another disposition.",
    ),
    (
        "manufacturing.ncr.outcome",
        "Choose credit or replace for the Return to supplier.",
    ),
    (
        "manufacturing.ncr.no_stock",
        "Stock is not enabled: the goods cannot be moved.",
    ),
    (
        "manufacturing.ncr.return_alive",
        "Cancel the Return to supplier first: that reopens the NCR.",
    ),
    (
        "manufacturing.ncr.rework_alive",
        "Cancel the rework work order first: that reopens the NCR.",
    ),
    (
        "manufacturing.ncr.rework_cancelled",
        "The rework work order was cancelled.",
    ),
    (
        "manufacturing.ncr.rework_aborted",
        "The rework work order was aborted.",
    ),
    (
        "manufacturing.ncr.return_cancelled",
        "The Return to supplier was cancelled.",
    ),
    (
        "manufacturing.ncr.keeps_lot",
        "{{ ncr }} is {{ status }} on this lot: it is not released until its action is done.",
    ),
    ("manufacturing.dispute.title", "Quality dispute: {{ ncrs }}"),
    (
        "manufacturing.dispute.body",
        "A customer disputes an invoice for quality; NCR {{ ncrs }} is open on the delivered lot. The customer says: {{ text }}",
    ),
    (
        "manufacturing.ncr.raiser_approves",
        "The inspector who raised the NCR cannot approve using it as it is: ask again for somebody else to approve.",
    ),
    ("manufacturing.ncr.task_done", "Done."),
    ("manufacturing.ncr.close_title", "Close {{ ncr }}"),
    (
        "manufacturing.ncr.close_body",
        "The action of the disposition of {{ ncr }} is done. Check it and close the NCR.",
    ),
    (
        "manufacturing.ncr.redisposition_title",
        "Choose a new disposition for {{ ncr }}",
    ),
    (
        "manufacturing.ncr.redisposition_body",
        "{{ ncr }} is open again: {{ reason }} Choose what becomes of the goods.",
    ),
    ("manufacturing.ncr.rework_title", "Rework for {{ ncr }}"),
    (
        "manufacturing.ncr.rework_body",
        "Work order {{ order }} was created for the rework of {{ ncr }}: set its line and BOM version and release it.",
    ),
    (
        "manufacturing.problems.plant_name",
        "Give the plant a name.",
    ),
    (
        "manufacturing.problems.plant_unknown",
        "The plant is not known.",
    ),
    (
        "manufacturing.problems.line_fields",
        "Give the line a code and a name, and an hourly rate that is not negative.",
    ),
    (
        "manufacturing.problems.line_code",
        "A line with that code exists already.",
    ),
    ("manufacturing.problems.reason", "Give a reason."),
    (
        "manufacturing.create.plant",
        "Choose a plant that is in use.",
    ),
    (
        "manufacturing.create.line",
        "Choose a line of this plant that is in use.",
    ),
    (
        "manufacturing.create.quantity",
        "Give the item and a quantity above zero.",
    ),
    (
        "manufacturing.create.dates",
        "The last planned day must not be before the first.",
    ),
    (
        "manufacturing.release.no_line",
        "Set the line the work order is scheduled on first.",
    ),
    (
        "manufacturing.release.no_bom",
        "Choose the BOM version the work order is made from first.",
    ),
    (
        "manufacturing.release.bom_not_released",
        "The BOM version is not released.",
    ),
    (
        "manufacturing.hold.reason",
        "Choose the reason: material, machine or quality.",
    ),
    (
        "manufacturing.complete.nothing_reported",
        "Report the good quantity before completing.",
    ),
    (
        "manufacturing.complete.inspection",
        "Final inspection of {{ order }}",
    ),
    (
        "manufacturing.close.pending_issue",
        "A material issue still waits for stock: it must post or be cleared first.",
    ),
    (
        "manufacturing.cancel.work_started",
        "Material was issued or output reported: abort the work order instead.",
    ),
    (
        "manufacturing.issue.state",
        "Materials are issued while the work order is released, in progress or on hold.",
    ),
    (
        "manufacturing.issue.quantity",
        "The quantity must be above zero.",
    ),
    ("manufacturing.issue.item", "Give the material to issue."),
    (
        "manufacturing.issue.lot_needed",
        "Name the lot to take it from: this material is tracked by lot and stock of it is at the store. Lots available: {{ lots }}.",
    ),
    (
        "manufacturing.issue.lot_unavailable",
        "Lot {{ lot }} cannot be taken from at this store. Lots available: {{ lots }}.",
    ),
    (
        "manufacturing.issue.no_store",
        "The line of the work order has no store to issue from.",
    ),
    (
        "manufacturing.report.state",
        "Output is reported while the work order is in progress.",
    ),
    (
        "manufacturing.late_report.future",
        "The time of the output is in the future.",
    ),
    (
        "manufacturing.report.quantity",
        "Report a good quantity or a scrap quantity, not below zero.",
    ),
    (
        "manufacturing.report.no_location",
        "The line has no finished goods location for the output.",
    ),
    (
        "manufacturing.unlink.not_linked",
        "The work order is not linked to a sales order.",
    ),
    (
        "manufacturing.unlink.state",
        "A work order is unlinked while it is planned, released, in progress or on hold.",
    ),
    (
        "manufacturing.unlink.title",
        "{{ order }} no longer serves a sales order",
    ),
    (
        "manufacturing.unlink.body",
        "{{ order }} was unlinked from its sales order ({{ reason }}). It keeps running and its output goes to free stock.",
    ),
    (
        "manufacturing.material.hold_title",
        "Hold {{ order }} for material",
    ),
    (
        "manufacturing.material.hold_body",
        "Material for {{ order }} is short and its issue waits. Put the work order on hold with the reason {{ reason }}, so the schedule shows why the line is idle.",
    ),
    (
        "manufacturing.material.resume_title",
        "Resume {{ order }}: the material arrived",
    ),
    (
        "manufacturing.material.resume_body",
        "The material {{ order }} was held for is here. Resume the work order.",
    ),
    (
        "manufacturing.material.arrived",
        "The material arrived or the pending issue was cleared; no hold is needed.",
    ),
    (
        "manufacturing.material.done",
        "The work order was held, resumed or stopped.",
    ),
    ("manufacturing.void.reason", "Say why the report is voided."),
    (
        "manufacturing.void.not_posted",
        "Only a posted report can be voided.",
    ),
    (
        "manufacturing.void.locked",
        "Output already used: the work order is {{ status }} and its reports are locked. The production manager corrects it with a reversing stock movement.",
    ),
    (
        "manufacturing.void.waiting",
        "A void of this report waits for approval.",
    ),
    (
        "manufacturing.void.used",
        "Output already used: the lot holds {{ left }} of the {{ good }} this report made. The production manager corrects it with a reversing stock movement.",
    ),
    (
        "manufacturing.abort.pending",
        "Abort pending: an abort of this work order waits for approval.",
    ),
    (
        "manufacturing.abort.already_pending",
        "An abort of this work order already waits for approval.",
    ),
    (
        "manufacturing.abort.nothing",
        "Nothing was issued or reported: cancel the work order instead.",
    ),
    (
        "manufacturing.abort.state",
        "A work order is aborted while it is released, in progress or on hold.",
    ),
    (
        "manufacturing.abort.return_too_much",
        "Item {{ item }}: a return must be above zero and not more than was issued less what went back.",
    ),
    (
        "manufacturing.abort.quantities_changed",
        "The quantities issued or reported changed after the request: ask again.",
    ),
    (
        "manufacturing.abort.inspection_title",
        "Final inspection of the output of {{ order }}",
    ),
    (
        "manufacturing.abort.inspection_body",
        "{{ order }} was aborted ({{ reason }}). Its good output is in quarantine until QC inspects it.",
    ),
    ("manufacturing.abort.sales_title", "{{ order }} was aborted"),
    (
        "manufacturing.abort.sales_body",
        "{{ order }} was aborted ({{ reason }}). Materials returned: {{ returned }}; scrapped: {{ scrapped }}.",
    ),
    ("manufacturing.screen.not_set", "Not set"),
    ("manufacturing.screen.no_line", "No line yet"),
    ("manufacturing.screen.item_name", "{{ name }} ({{ code }})"),
    ("manufacturing.screen.item_unknown", "Item {{ id }}"),
    ("manufacturing.screen.source.stock", "Stock"),
    (
        "manufacturing.screen.source.sales_order",
        "Sales order {{ order }}",
    ),
    ("manufacturing.screen.filter.all", "All"),
    ("manufacturing.screen.filter.open", "Open"),
    ("manufacturing.screen.activity.system", "System"),
    ("manufacturing.screen.order.title", "Work order"),
    ("manufacturing.screen.orders.title", "Work orders"),
    (
        "manufacturing.screen.orders.subtitle",
        "What the plant makes, on which line and how far it has got.",
    ),
    ("manufacturing.screen.orders.new", "New work order"),
    (
        "manufacturing.screen.orders.new_hint",
        "The work order starts as planned. Set the line before releasing it.",
    ),
    ("manufacturing.screen.orders.new_save", "Create"),
    ("manufacturing.screen.orders.filter", "Filter by status"),
    ("manufacturing.screen.orders.order", "Work order"),
    ("manufacturing.screen.orders.line", "Line"),
    ("manufacturing.screen.orders.made", "Made / planned"),
    ("manufacturing.screen.orders.period", "Planned"),
    ("manufacturing.screen.orders.empty", "No work orders"),
    (
        "manufacturing.screen.orders.empty_text",
        "Work orders you may see appear here.",
    ),
    ("manufacturing.screen.tab.overview", "Overview"),
    ("manufacturing.screen.tab.materials", "Materials"),
    ("manufacturing.screen.tab.output", "Output"),
    ("manufacturing.screen.tab.approvals", "Approvals"),
    ("manufacturing.screen.tab.activity", "Activity"),
    ("manufacturing.screen.fields.status", "Status"),
    ("manufacturing.screen.fact.item", "Item"),
    ("manufacturing.screen.fact.line", "Line"),
    ("manufacturing.screen.fact.quantity", "Quantity"),
    (
        "manufacturing.screen.fact.quantity_of",
        "{{ made }} of {{ quantity }}",
    ),
    ("manufacturing.screen.fact.scrap", "Scrap"),
    ("manufacturing.screen.fact.source", "Source"),
    ("manufacturing.screen.fact.planned", "Planned"),
    ("manufacturing.screen.fact.started", "Started"),
    ("manufacturing.screen.fact.stopped", "Stopped"),
    ("manufacturing.screen.fact.hold_reason", "On hold for"),
    ("manufacturing.screen.fact.for_stock", "Quantity for stock"),
    ("manufacturing.screen.fact.output_lot", "Output lot"),
    ("manufacturing.screen.fact.lot_name", "Lot {{ id }}"),
    (
        "manufacturing.screen.fact.unlinked",
        "Unlinked from the order",
    ),
    ("manufacturing.screen.fact.cancelled", "Cancelled"),
    ("manufacturing.screen.fact.aborted", "Aborted"),
    ("manufacturing.screen.cost.title", "Cost"),
    (
        "manufacturing.screen.cost.standard",
        "Standard cost of a unit",
    ),
    ("manufacturing.screen.cost.actual", "Actual cost"),
    ("manufacturing.screen.cost.scrap", "Scrap value"),
    ("manufacturing.screen.cost.loss", "Loss"),
    ("manufacturing.screen.abort.outcome", "What the abort did"),
    ("manufacturing.screen.abort.what", "Outcome"),
    (
        "manufacturing.screen.abort.return_label",
        "Return {{ item }} to the store",
    ),
    (
        "manufacturing.screen.abort.return_hint",
        "Up to {{ left }}; leave empty to return none.",
    ),
    (
        "manufacturing.screen.outcome.return",
        "Returned to the store",
    ),
    ("manufacturing.screen.outcome.scrap", "Scrapped"),
    (
        "manufacturing.screen.outcome.output",
        "Good output kept in quarantine",
    ),
    ("manufacturing.screen.materials.pick", "Pick list"),
    ("manufacturing.screen.materials.pick_empty", "No pick list"),
    (
        "manufacturing.screen.materials.pick_empty_text",
        "The list is made when the work order is released.",
    ),
    ("manufacturing.screen.materials.issued", "Issued"),
    ("manufacturing.screen.materials.empty", "Nothing issued"),
    (
        "manufacturing.screen.materials.empty_text",
        "The storekeeper issues materials once the work order is released.",
    ),
    ("manufacturing.screen.materials.item", "Material"),
    ("manufacturing.screen.materials.required", "Needed"),
    ("manufacturing.screen.materials.lot", "Lot"),
    ("manufacturing.screen.materials.quantity", "Quantity"),
    ("manufacturing.screen.materials.returned", "Returned"),
    ("manufacturing.screen.materials.value", "Value"),
    (
        "manufacturing.screen.materials.waiting",
        "Waiting for stock",
    ),
    (
        "manufacturing.screen.materials.short_note",
        "Waiting for material: part of an issue is short and waits as a pending issue.",
    ),
    ("manufacturing.screen.output.number", "Report"),
    ("manufacturing.screen.output.status", "Status"),
    ("manufacturing.screen.output.void", "Void"),
    (
        "manufacturing.screen.output.void_hint",
        "The report was entered on the wrong work order. Its output and scrap are taken back once the line's supervisor or the production manager approves; enter a new report on the right work order.",
    ),
    (
        "manufacturing.screen.output.void_reason",
        "Why is it voided?",
    ),
    ("manufacturing.screen.output.void_submit", "Ask to void"),
    (
        "manufacturing.screen.output.late",
        "Entered late: {{ reason }}",
    ),
    ("manufacturing.screen.output.voided", "Voided"),
    (
        "manufacturing.screen.output.void_waits",
        "Void waits for approval",
    ),
    ("manufacturing.screen.output.void_reason_label", "Reason"),
    (
        "manufacturing.screen.output.void_reversal_open",
        "The stock reversal has not posted yet",
    ),
    (
        "manufacturing.screen.done.void_requested",
        "The void waits for approval.",
    ),
    ("manufacturing.screen.done.voided", "The report was voided."),
    ("manufacturing.screen.output.when", "When"),
    ("manufacturing.screen.output.good", "Good"),
    ("manufacturing.screen.output.scrap", "Scrap"),
    ("manufacturing.screen.output.by", "Reported by"),
    ("manufacturing.screen.output.note", "Note"),
    ("manufacturing.screen.output.empty", "No output reported"),
    (
        "manufacturing.screen.output.empty_text",
        "The line reports its output while the work order is in progress.",
    ),
    ("manufacturing.screen.approval.request", "Request"),
    ("manufacturing.screen.approval.requester", "Asked by"),
    ("manufacturing.screen.approval.requested", "Asked on"),
    ("manufacturing.screen.approval.decided", "Decided on"),
    ("manufacturing.screen.approval.note", "Note"),
    ("manufacturing.screen.approval.decision", "Decision"),
    ("manufacturing.screen.approval.approve", "Approve"),
    ("manufacturing.screen.approval.reject", "Reject"),
    ("manufacturing.screen.approval.comment", "Comment"),
    (
        "manufacturing.screen.approval.comment_optional",
        "Optional.",
    ),
    ("manufacturing.screen.approval.comment_required", "Say why."),
    ("manufacturing.screen.approval.empty", "No approvals"),
    (
        "manufacturing.screen.approval.empty_text",
        "Requests about this work order appear here.",
    ),
    ("manufacturing.screen.approval.waiting", "Waiting"),
    (
        "manufacturing.screen.approval.waiting_for",
        "Waiting for approval: {{ action }}.",
    ),
    ("manufacturing.screen.approval.approved", "Approved"),
    ("manufacturing.screen.approval.rejected", "Rejected"),
    ("manufacturing.screen.approval.returned", "Returned"),
    ("manufacturing.screen.approval.withdrawn", "Withdrawn"),
    ("manufacturing.screen.approval.expired", "Expired"),
    ("manufacturing.screen.approval.failed", "Failed"),
    ("manufacturing.screen.approval.pending", "Waiting"),
    ("manufacturing.screen.activity.when", "When"),
    ("manufacturing.screen.activity.what", "What"),
    ("manufacturing.screen.activity.by", "By"),
    ("manufacturing.screen.activity.reason", "Reason"),
    ("manufacturing.screen.activity.empty", "Nothing yet"),
    ("manufacturing.screen.action.release", "Release"),
    (
        "manufacturing.screen.action.release_hint",
        "Put the work order on its line and make the pick list.",
    ),
    ("manufacturing.screen.action.start", "Start"),
    (
        "manufacturing.screen.action.start_hint",
        "Record the start time.",
    ),
    ("manufacturing.screen.action.hold", "Hold"),
    (
        "manufacturing.screen.action.hold_hint",
        "Stop the work order for a reason.",
    ),
    ("manufacturing.screen.action.resume", "Resume"),
    (
        "manufacturing.screen.action.resume_hint",
        "Take the work order back into production.",
    ),
    ("manufacturing.screen.action.issue", "Issue material"),
    (
        "manufacturing.screen.action.issue_hint",
        "Take a material from the store of the line to the work order.",
    ),
    ("manufacturing.screen.action.report", "Report output"),
    (
        "manufacturing.screen.action.report_hint",
        "Report the good quantity and the scrap.",
    ),
    ("manufacturing.screen.action.complete", "Complete"),
    (
        "manufacturing.screen.action.complete_hint",
        "The output lot takes no more output and a final inspection is requested.",
    ),
    ("manufacturing.screen.action.close", "Close"),
    (
        "manufacturing.screen.action.close_hint",
        "Calculate the actual cost and lock the work order.",
    ),
    (
        "manufacturing.screen.action.unlink",
        "Unlink from the order",
    ),
    (
        "manufacturing.screen.action.unlink_hint",
        "The work order keeps running; its output goes to free stock.",
    ),
    ("manufacturing.screen.action.cancel", "Cancel"),
    (
        "manufacturing.screen.action.cancel_hint",
        "Only when nothing was issued or reported.",
    ),
    ("manufacturing.screen.action.abort", "Abort"),
    (
        "manufacturing.screen.action.abort_hint",
        "Stop a work order that has issued material or reported output. The production manager approves; nothing is posted before.",
    ),
    ("manufacturing.screen.form.plant", "Plant"),
    ("manufacturing.screen.form.line", "Line"),
    ("manufacturing.screen.form.line_placeholder", "Not yet"),
    ("manufacturing.screen.form.item", "Item"),
    (
        "manufacturing.screen.form.item_hint",
        "The number of the finished item.",
    ),
    ("manufacturing.screen.form.bom", "BOM version"),
    (
        "manufacturing.screen.form.bom_hint",
        "The number of a released BOM version.",
    ),
    ("manufacturing.screen.form.quantity", "Quantity"),
    (
        "manufacturing.screen.form.quantity_hint",
        "A number, up to three decimals.",
    ),
    (
        "manufacturing.screen.form.planned_start",
        "First planned day",
    ),
    ("manufacturing.screen.form.planned_end", "Last planned day"),
    ("manufacturing.screen.form.hold_reason", "Reason"),
    ("manufacturing.screen.form.material", "Material"),
    ("manufacturing.screen.form.lot", "Lot"),
    (
        "manufacturing.screen.form.lot_hint",
        "The lot number printed on the pack. A lot-tracked material needs one while stock of it is at the store; one issue for each lot.",
    ),
    ("manufacturing.screen.form.good", "Good quantity"),
    ("manufacturing.screen.form.scrap", "Scrap"),
    ("manufacturing.screen.form.note", "Note"),
    (
        "manufacturing.screen.form.occurred_at",
        "Made at (paper job card)",
    ),
    (
        "manufacturing.screen.form.occurred_hint",
        "Leave empty for output made now. Fill it in only when the line tablets were down and you are entering the job card later.",
    ),
    (
        "manufacturing.screen.form.late_reason",
        "Why it is entered late",
    ),
    ("manufacturing.screen.form.outage", "Outage reference"),
    (
        "manufacturing.screen.form.unlink_reason",
        "Why is it no longer for the order?",
    ),
    (
        "manufacturing.screen.form.cancel_reason",
        "Why is it cancelled?",
    ),
    (
        "manufacturing.screen.form.abort_reason",
        "Why is it aborted?",
    ),
    ("manufacturing.screen.done.created", "Work order created."),
    ("manufacturing.screen.done.released", "Work order released."),
    ("manufacturing.screen.done.started", "Work order started."),
    ("manufacturing.screen.done.held", "Work order on hold."),
    ("manufacturing.screen.done.resumed", "Work order resumed."),
    (
        "manufacturing.screen.done.completed",
        "Work order completed.",
    ),
    ("manufacturing.screen.done.closed", "Work order closed."),
    (
        "manufacturing.screen.done.cancelled",
        "Work order cancelled.",
    ),
    (
        "manufacturing.screen.done.unlinked",
        "Work order unlinked from its sales order.",
    ),
    (
        "manufacturing.screen.done.abort_requested",
        "Abort requested: the production manager decides.",
    ),
    ("manufacturing.screen.done.issued", "Material issued."),
    ("manufacturing.screen.done.reported", "Output reported."),
    (
        "manufacturing.screen.done.reported_late",
        "Output reported with the late-entry flag.",
    ),
    (
        "manufacturing.screen.done.late_requested",
        "The late entry waits for the production manager's approval.",
    ),
    ("manufacturing.screen.status.posted", "Posted"),
    ("manufacturing.screen.status.planned", "Planned"),
    ("manufacturing.screen.status.released", "Released"),
    ("manufacturing.screen.status.in_progress", "In progress"),
    ("manufacturing.screen.status.on_hold", "On hold"),
    ("manufacturing.screen.status.completed", "Completed"),
    ("manufacturing.screen.status.closed", "Closed"),
    ("manufacturing.screen.status.cancelled", "Cancelled"),
    ("manufacturing.screen.status.aborted", "Aborted"),
    ("manufacturing.screen.hold_reason.material", "Material"),
    ("manufacturing.screen.hold_reason.machine", "Machine"),
    ("manufacturing.screen.hold_reason.quality", "Quality"),
    ("manufacturing.screen.transition.release", "Released"),
    ("manufacturing.screen.transition.start", "Started"),
    ("manufacturing.screen.transition.hold", "Put on hold"),
    ("manufacturing.screen.transition.resume", "Resumed"),
    ("manufacturing.screen.transition.complete", "Completed"),
    ("manufacturing.screen.transition.close", "Closed"),
    ("manufacturing.screen.transition.auto_close", "Closed"),
    ("manufacturing.screen.transition.cancel", "Cancelled"),
    ("manufacturing.screen.transition.auto_cancel", "Cancelled"),
    ("manufacturing.screen.transition.abort", "Abort requested"),
    ("manufacturing.screen.transition.abort_approved", "Aborted"),
    (
        "manufacturing.screen.transition.unlink",
        "Unlinked from the order",
    ),
    (
        "manufacturing.screen.transition.unlink_released",
        "Unlinked from the order",
    ),
    (
        "manufacturing.screen.transition.unlink_in_progress",
        "Unlinked from the order",
    ),
    (
        "manufacturing.screen.transition.unlink_on_hold",
        "Unlinked from the order",
    ),
    (
        "manufacturing.screen.transition.auto_unlink",
        "Unlinked from the order",
    ),
    (
        "manufacturing.screen.transition.auto_unlink_released",
        "Unlinked from the order",
    ),
    (
        "manufacturing.screen.transition.auto_unlink_in_progress",
        "Unlinked from the order",
    ),
    (
        "manufacturing.screen.transition.auto_unlink_on_hold",
        "Unlinked from the order",
    ),
    // The NCR screens.
    (
        "manufacturing.screen.done.ncr_raised",
        "The NCR was raised.",
    ),
    (
        "manufacturing.screen.done.ncr_dispositioned",
        "The disposition is decided.",
    ),
    (
        "manufacturing.screen.done.ncr_waiting",
        "The decision waits for an approval.",
    ),
    ("manufacturing.screen.done.ncr_closed", "The NCR is closed."),
    (
        "manufacturing.screen.done.ncr_reopened",
        "The NCR is open again.",
    ),
    (
        "manufacturing.screen.done.ncr_cancelled",
        "The NCR was cancelled.",
    ),
    (
        "manufacturing.screen.done.ncr_cause",
        "The cause was changed.",
    ),
    ("manufacturing.screen.ncrs.title", "Non-conformance reports"),
    (
        "manufacturing.screen.ncrs.subtitle",
        "Defects on lots and what QC decided for them.",
    ),
    ("manufacturing.screen.ncrs.new", "Raise an NCR"),
    (
        "manufacturing.screen.ncrs.new_hint",
        "Record a defect on a lot, or on an item when it has no lot.",
    ),
    ("manufacturing.screen.ncrs.new_save", "Raise"),
    ("manufacturing.screen.ncrs.filter", "Filter by status"),
    ("manufacturing.screen.ncrs.ncr", "NCR"),
    ("manufacturing.screen.ncrs.quantity", "Quantity"),
    ("manufacturing.screen.ncrs.cause", "Cause"),
    ("manufacturing.screen.ncrs.disposition", "Disposition"),
    ("manufacturing.screen.ncrs.raised", "Raised on"),
    ("manufacturing.screen.ncrs.empty", "No NCRs"),
    (
        "manufacturing.screen.ncrs.empty_text",
        "Defects found at inspection appear here.",
    ),
    ("manufacturing.screen.ncrs.form.lot", "Lot"),
    (
        "manufacturing.screen.ncrs.form.lot_hint",
        "The number of the lot in the system.",
    ),
    ("manufacturing.screen.ncrs.form.item", "Item"),
    (
        "manufacturing.screen.ncrs.form.item_hint",
        "Only when the goods have no lot.",
    ),
    ("manufacturing.screen.ncrs.form.defect", "Defect"),
    ("manufacturing.screen.ncrs.form.cause", "Cause"),
    ("manufacturing.screen.ncr.cause_default", "As the lot says"),
    ("manufacturing.screen.ncr.title", "NCR"),
    ("manufacturing.screen.ncr.lot_name", "Lot {{ number }}"),
    (
        "manufacturing.screen.ncr.open_rework",
        "Open the rework work order",
    ),
    (
        "manufacturing.screen.ncr.open_return",
        "Open the Return to supplier",
    ),
    ("manufacturing.screen.ncr.tab.decisions", "Dispositions"),
    ("manufacturing.screen.ncr.decision.what", "Disposition"),
    ("manufacturing.screen.ncr.decision.in_force", "In force"),
    ("manufacturing.screen.ncr.decision.ended", "Ended"),
    (
        "manufacturing.screen.ncr.decision.empty",
        "No disposition yet",
    ),
    (
        "manufacturing.screen.ncr.decision.empty_text",
        "QC decides what becomes of the goods.",
    ),
    ("manufacturing.screen.ncr.fact.lot", "Lot"),
    ("manufacturing.screen.ncr.fact.defect", "Defect"),
    ("manufacturing.screen.ncr.fact.cause", "Cause"),
    (
        "manufacturing.screen.ncr.fact.cause_reason",
        "Why the cause changed",
    ),
    ("manufacturing.screen.ncr.fact.origin", "Raised from"),
    ("manufacturing.screen.ncr.fact.raised", "Raised"),
    (
        "manufacturing.screen.ncr.fact.raised_by",
        "{{ date }} by {{ who }}",
    ),
    ("manufacturing.screen.ncr.fact.supplier", "Supplier"),
    ("manufacturing.screen.ncr.fact.disposition", "Disposition"),
    ("manufacturing.screen.ncr.fact.outcome", "For the supplier"),
    ("manufacturing.screen.ncr.fact.note", "QC note"),
    (
        "manufacturing.screen.ncr.fact.scrap_value",
        "Value scrapped",
    ),
    ("manufacturing.screen.ncr.fact.action", "Action"),
    ("manufacturing.screen.ncr.fact.rework", "Rework work order"),
    ("manufacturing.screen.ncr.fact.return", "Return to supplier"),
    (
        "manufacturing.screen.ncr.fact.return_name",
        "Return {{ id }}",
    ),
    ("manufacturing.screen.ncr.action.done", "Done"),
    ("manufacturing.screen.ncr.action.pending", "Not done yet"),
    (
        "manufacturing.screen.ncr.note.waiting",
        "A decision on this NCR waits for an approval.",
    ),
    (
        "manufacturing.screen.ncr.note.partial",
        "The NCR covers part of the lot: the lot is not released until the action of the disposition is done.",
    ),
    (
        "manufacturing.screen.ncr.note.reopened",
        "The NCR is open again: choose a new disposition. The earlier one stays in the list of dispositions.",
    ),
    ("manufacturing.screen.ncr.rework.no_line", "Choose later"),
    ("manufacturing.screen.ncr.form.note", "Note"),
    (
        "manufacturing.screen.ncr.form.outcome",
        "What the supplier gives",
    ),
    ("manufacturing.screen.ncr.form.reason", "Reason"),
    ("manufacturing.screen.ncr.action.rework", "Rework"),
    (
        "manufacturing.screen.ncr.action.rework_hint",
        "A work order is created to make the goods right.",
    ),
    ("manufacturing.screen.ncr.action.scrap", "Scrap"),
    (
        "manufacturing.screen.ncr.action.scrap_hint",
        "The goods are written off. Above AED 2,000 a second person approves first.",
    ),
    ("manufacturing.screen.ncr.action.use_as_is", "Use as is"),
    (
        "manufacturing.screen.ncr.action.use_as_is_hint",
        "The goods are used as they are. Another inspector or the production manager approves first, then the lot is released.",
    ),
    (
        "manufacturing.screen.ncr.action.return",
        "Return to supplier",
    ),
    (
        "manufacturing.screen.ncr.action.return_hint",
        "A draft Return to supplier is made from the quarantine location.",
    ),
    ("manufacturing.screen.ncr.action.close", "Close"),
    (
        "manufacturing.screen.ncr.action.close_hint",
        "The action of the disposition is done. Close the NCR.",
    ),
    ("manufacturing.screen.ncr.action.reopen", "Reopen"),
    (
        "manufacturing.screen.ncr.action.reopen_hint",
        "The action did not happen: choose a new disposition.",
    ),
    ("manufacturing.screen.ncr.action.cause", "Change the cause"),
    (
        "manufacturing.screen.ncr.action.cause_hint",
        "The supplier scorecard counts only NCRs caused by the supplier.",
    ),
    ("manufacturing.screen.ncr.action.cancel", "Cancel"),
    (
        "manufacturing.screen.ncr.action.cancel_hint",
        "The NCR was raised in error.",
    ),
    ("manufacturing.screen.ncr_status.open", "Open"),
    (
        "manufacturing.screen.ncr_status.dispositioned",
        "Dispositioned",
    ),
    ("manufacturing.screen.ncr_status.closed", "Closed"),
    ("manufacturing.screen.ncr_status.cancelled", "Cancelled"),
    ("manufacturing.screen.disposition.rework", "Rework"),
    ("manufacturing.screen.disposition.scrap", "Scrap"),
    ("manufacturing.screen.disposition.use_as_is", "Use as is"),
    (
        "manufacturing.screen.disposition.return_to_supplier",
        "Return to supplier",
    ),
    ("manufacturing.screen.cause.supplier", "Supplier"),
    (
        "manufacturing.screen.cause.internal_process",
        "Internal process",
    ),
    ("manufacturing.screen.cause.unknown", "Unknown"),
    (
        "manufacturing.screen.origin.incoming",
        "Incoming inspection",
    ),
    ("manufacturing.screen.origin.dispute", "Customer dispute"),
    ("manufacturing.screen.origin.production", "Production"),
    ("manufacturing.screen.origin.other", "Other"),
    ("manufacturing.screen.return_outcome.credit", "Credit"),
    ("manufacturing.screen.return_outcome.replace", "Replacement"),
    (
        "manufacturing.screen.transition.disposition",
        "Dispositioned",
    ),
    (
        "manufacturing.screen.transition.disposition_scrap",
        "Dispositioned",
    ),
    (
        "manufacturing.screen.transition.disposition_use_as_is",
        "Use as is asked",
    ),
    (
        "manufacturing.screen.transition.disposition_scrap_approved",
        "Dispositioned",
    ),
    (
        "manufacturing.screen.transition.disposition_use_as_is_approved",
        "Use as is approved",
    ),
    ("manufacturing.screen.transition.reopen", "Reopened"),
    ("manufacturing.screen.transition.auto_reopen", "Reopened"),
    // The buyer's portal.
    (
        "manufacturing.screen.done.disputed",
        "The dispute was sent. We will look into it.",
    ),
    ("manufacturing.screen.portal.title", "Invoices"),
    (
        "manufacturing.screen.portal.subtitle",
        "Your open invoices. Tell us if something on one is wrong.",
    ),
    ("manufacturing.screen.portal.invoice", "Invoice"),
    ("manufacturing.screen.portal.balance", "Balance"),
    ("manufacturing.screen.portal.due", "Due"),
    (
        "manufacturing.screen.portal.dispute",
        "Dispute this invoice",
    ),
    (
        "manufacturing.screen.portal.dispute_hint",
        "Tell us what is wrong with the invoice. Reminders for it pause while we look.",
    ),
    ("manufacturing.screen.portal.dispute_send", "Send"),
    ("manufacturing.screen.portal.kind", "What is wrong"),
    ("manufacturing.screen.portal.text", "Tell us more"),
    (
        "manufacturing.screen.portal.lines_hint",
        "Tick the lines with the problem. With none ticked, every line is meant.",
    ),
    ("manufacturing.screen.portal.empty", "No open invoices"),
    (
        "manufacturing.screen.portal.empty_text",
        "Invoices you still have to pay appear here.",
    ),
    ("manufacturing.screen.portal.under_review", "Under review"),
    ("manufacturing.screen.dispute_kind.quantity", "Quantity"),
    ("manufacturing.screen.dispute_kind.price", "Price"),
    ("manufacturing.screen.dispute_kind.quality", "Quality"),
    ("manufacturing.screen.dispute_kind.other", "Other"),
    ("manufacturing.screen.invoice_status.issued", "Issued"),
    (
        "manufacturing.screen.invoice_status.partially_paid",
        "Partly paid",
    ),
    ("manufacturing.screen.invoice_status.overdue", "Overdue"),
    (
        "manufacturing.portal.kind",
        "Choose what is wrong: quantity, price, quality or other.",
    ),
    ("manufacturing.screen.ncr.fact.dispute", "Customer dispute"),
    (
        "manufacturing.screen.ncr.fact.dispute_on",
        "Dispute {{ dispute }} on invoice {{ invoice }}",
    ),
    (
        "manufacturing.screen.ncr.fact.customer_says",
        "The customer says",
    ),
    // The recall by customer (BL-MFG-10).
    (
        "manufacturing.recall.repick_title",
        "Re-pick {{ dispatch }}: a lot on it is recalled",
    ),
    (
        "manufacturing.recall.repick_body",
        "Dispatch {{ dispatch }} has not left and holds a lot of recall {{ recall }}, which is held. Pick the line again from lots that are not held, or cancel the dispatch.",
    ),
    ("manufacturing.screen.recalls.title", "Recalls"),
    (
        "manufacturing.screen.recalls.subtitle",
        "What each recall reached: the customers, the work orders and the dispatches.",
    ),
    ("manufacturing.screen.recalls.recall", "Recall"),
    ("manufacturing.screen.recalls.opened", "Opened"),
    ("manufacturing.screen.recalls.empty", "No recalls"),
    (
        "manufacturing.screen.recalls.empty_text",
        "A recall opened in the stock pages appears here.",
    ),
    ("manufacturing.screen.recall.title", "Recall"),
    ("manufacturing.screen.recall.open_recall", "Open the recall"),
    ("manufacturing.screen.recall.tab.customers", "Customers"),
    (
        "manufacturing.screen.recall.tab.bring_back",
        "To bring back",
    ),
    ("manufacturing.screen.recall.tab.repick", "To re-pick"),
    ("manufacturing.screen.recall.customer", "Customer"),
    ("manufacturing.screen.recall.quantity", "Quantity"),
    ("manufacturing.screen.recall.day", "Date"),
    ("manufacturing.screen.recall.notice", "Notice"),
    ("manufacturing.screen.recall.note", "Note"),
    (
        "manufacturing.screen.recall.empty",
        "No customer reached yet",
    ),
    (
        "manufacturing.screen.recall.empty_text",
        "No dispatch or work order of a customer got a recalled lot.",
    ),
    (
        "manufacturing.screen.recall.unknown_customer",
        "Customer not known",
    ),
    (
        "manufacturing.screen.recall.summary",
        "{{ customer }}: {{ shipped }} shipped, {{ held }} not shipped",
    ),
    (
        "manufacturing.screen.recall.bring_back_alert",
        "Goods of this recall left the plant and are not delivered yet: bring them back by hand.",
    ),
    (
        "manufacturing.screen.recall.bring_back_note",
        "On its way: bring it back",
    ),
    (
        "manufacturing.screen.recall.repick_asked",
        "Re-pick asked of the storekeeper",
    ),
    (
        "manufacturing.screen.recall.bring_back_empty",
        "Nothing to bring back",
    ),
    (
        "manufacturing.screen.recall.bring_back_empty_text",
        "No dispatch of this recall is on its way to a customer.",
    ),
    (
        "manufacturing.screen.recall.repick_empty",
        "Nothing to re-pick",
    ),
    (
        "manufacturing.screen.recall.repick_empty_text",
        "No dispatch that has not left holds a lot of this recall.",
    ),
    ("manufacturing.screen.recall_status.open", "Open"),
    ("manufacturing.screen.recall_status.traced", "Traced"),
    ("manufacturing.screen.recall_status.closed", "Closed"),
    ("manufacturing.screen.shipping.shipped", "Shipped"),
    ("manufacturing.screen.shipping.not_shipped", "Not shipped"),
    ("manufacturing.screen.notice.pending", "Not told yet"),
    ("manufacturing.screen.notice.sent", "Told"),
    ("manufacturing.screen.notice.waived", "Not needed"),
    // Product conformity (BL-MFG-11).
    (
        "manufacturing.conformity.kind",
        "{{ kind }} is not a conformity certificate kind.",
    ),
    (
        "manufacturing.conformity.code",
        "Give the code of the item.",
    ),
    (
        "manufacturing.conformity.unknown_item",
        "There is no item with the code {{ code }}.",
    ),
    (
        "manufacturing.conformity.planned_line",
        "Order {{ order }}, line {{ line }}",
    ),
    (
        "manufacturing.conformity.export_only",
        "{{ item }} is export only: it cannot be sold for delivery in the UAE.",
    ),
    (
        "manufacturing.screen.conformity.title",
        "Product conformity",
    ),
    (
        "manufacturing.screen.conformity.subtitle",
        "The items that need a conformity certificate to leave the plant, and the export only items.",
    ),
    ("manufacturing.screen.conformity.new", "Flag an item"),
    (
        "manufacturing.screen.conformity.new_hint",
        "A regulated item is dispatched only while one of its accepted certificates is valid; an export only item is not sold for delivery in the UAE.",
    ),
    ("manufacturing.screen.conformity.new_save", "Save the flags"),
    ("manufacturing.screen.conformity.code", "Item code"),
    (
        "manufacturing.screen.conformity.code_hint",
        "The code of the item in the stock pages.",
    ),
    ("manufacturing.screen.conformity.regulated", "Regulated"),
    ("manufacturing.screen.conformity.export_only", "Export only"),
    (
        "manufacturing.screen.conformity.customer_specific",
        "Made for one customer",
    ),
    (
        "manufacturing.screen.conformity.customer_specific_hint",
        "A cancelled order's started goods of this item are proposed for scrap, not for stock.",
    ),
    ("manufacturing.screen.conformity.item", "Item"),
    (
        "manufacturing.screen.conformity.certificates",
        "Certificates on file",
    ),
    (
        "manufacturing.screen.conformity.empty",
        "No item is flagged",
    ),
    (
        "manufacturing.screen.conformity.empty_text",
        "Flag the items that are regulated or export only.",
    ),
    ("manufacturing.screen.conformity.yes", "Yes"),
    ("manufacturing.screen.conformity.no", "No"),
    (
        "manufacturing.screen.conformity.done",
        "The flags are saved.",
    ),
    (
        "manufacturing.screen.conformity_kind.ecas_certificate",
        "ECAS certificate",
    ),
    (
        "manufacturing.screen.conformity_kind.eqm_licence",
        "Emirates Quality Mark licence",
    ),
    (
        "manufacturing.screen.conformity_kind.food_contact_certificate",
        "Food-contact certificate",
    ),
    ("manufacturing.screen.doc_validity.valid", "Valid"),
    ("manufacturing.screen.doc_validity.expiring", "Expires soon"),
    ("manufacturing.screen.doc_validity.expired", "Expired"),
    (
        "manufacturing.screen.doc_validity.pending",
        "Waits for review",
    ),
    ("manufacturing.screen.doc_validity.rejected", "Rejected"),
    // Export evidence and the VAT correction (BL-MFG-12, MFG.RUL.08).
    (
        "manufacturing.export.warn_title",
        "Exit evidence missing for {{ dispatch }}: {{ days }} days",
    ),
    (
        "manufacturing.export.warn_body",
        "Dispatch {{ dispatch }} left on {{ left }} and no customs exit certificate, bill of lading or airway bill is attached. Invoices at the zero rate: {{ invoices }}. After {{ flag }} days the zero rate no longer holds.",
    ),
    (
        "manufacturing.export.task_title",
        "5% VAT due on export {{ dispatch }}: correct the invoice",
    ),
    (
        "manufacturing.export.task_body",
        "Dispatch {{ dispatch }} left more than {{ days }} days ago without exit evidence, so the zero rate no longer holds. Zero-rated invoices: {{ invoices }}. Issue a credit note and a standard-rated invoice. Customer contract note: {{ note }}",
    ),
    (
        "manufacturing.export.no_note",
        "none on file; check the contract on whether the VAT is recharged",
    ),
    (
        "manufacturing.export.late_title",
        "Evidence received after correction for {{ dispatch }}",
    ),
    (
        "manufacturing.export.late_body",
        "Exit evidence for dispatch {{ dispatch }} was attached on {{ day }}, after the 5% VAT was flagged. Nothing is reversed; decide whether the zero rate can be restored.",
    ),
    ("manufacturing.screen.aborted.title", "Aborted work orders"),
    (
        "manufacturing.screen.aborted.subtitle",
        "Work orders stopped after material was issued, with the reason, who asked and who approved.",
    ),
    ("manufacturing.screen.aborted.order", "Work order"),
    ("manufacturing.screen.aborted.loss", "Loss"),
    ("manufacturing.screen.aborted.stopped", "Stopped on"),
    ("manufacturing.screen.aborted.requested_by", "Asked by"),
    ("manufacturing.screen.aborted.approved_by", "Approved by"),
    (
        "manufacturing.screen.aborted.empty",
        "No aborted work orders",
    ),
    (
        "manufacturing.screen.aborted.empty_text",
        "A work order that is aborted after material was issued is listed here.",
    ),
    ("manufacturing.screen.exports.title", "Export evidence"),
    (
        "manufacturing.screen.exports.subtitle",
        "Zero-rated export dispatches and the proof that the goods left the UAE within 90 days.",
    ),
    ("manufacturing.screen.exports.dispatch", "Dispatch"),
    ("manufacturing.screen.exports.left", "Left on"),
    ("manufacturing.screen.exports.days", "Days"),
    ("manufacturing.screen.exports.evidence", "Evidence"),
    ("manufacturing.screen.exports.attach", "Attach evidence"),
    ("manufacturing.screen.exports.note", "Contract note"),
    ("manufacturing.screen.exports.note_edit", "Contract note"),
    (
        "manufacturing.screen.exports.note_hint",
        "Whether the 5% VAT is recharged to this customer when the zero rate is lost.",
    ),
    ("manufacturing.screen.exports.note_save", "Save the note"),
    (
        "manufacturing.screen.exports.note_done",
        "The note is saved.",
    ),
    (
        "manufacturing.screen.exports.empty",
        "No export dispatch yet",
    ),
    (
        "manufacturing.screen.exports.empty_text",
        "A dispatch billed at the zero rate appears here once it has left.",
    ),
    (
        "manufacturing.screen.export_state.waiting",
        "Waiting for evidence",
    ),
    (
        "manufacturing.screen.export_state.warned",
        "Warned at day 60",
    ),
    ("manufacturing.screen.export_state.vat_due", "5% VAT due"),
    (
        "manufacturing.screen.export_state.evidence",
        "Evidence received",
    ),
    (
        "manufacturing.screen.export_state.evidence_late",
        "5% VAT due; evidence received after correction",
    ),
    // The buyer's PO upload (BL-MFG-06).
    ("manufacturing.po.empty", "Give the customer PO number."),
    (
        "manufacturing.po.duplicate",
        "PO {{ po }} is already used by order {{ number }} ({{ status }}). Send a new PO number, or a call-off number if this is a call-off against it.",
    ),
    (
        "manufacturing.po.duplicate_call_off",
        "PO {{ po }} with call-off {{ call_off }} is already used by order {{ number }} ({{ status }}). Send another call-off number.",
    ),
    (
        "manufacturing.po.uploaded",
        "PO {{ po }} was uploaded already.",
    ),
    (
        "manufacturing.po.told_title",
        "Customer PO {{ po }} uploaded",
    ),
    (
        "manufacturing.po.told_body",
        "A buyer uploaded PO {{ po }} (call-off {{ call_off }}) in the portal. Enter the order.",
    ),
    ("manufacturing.screen.po.title", "Purchase orders"),
    (
        "manufacturing.screen.po.subtitle",
        "Send your purchase order; each PO number and call-off number is used once.",
    ),
    ("manufacturing.screen.po.new", "Upload a PO"),
    (
        "manufacturing.screen.po.new_hint",
        "A call-off against a blanket PO has its own call-off number.",
    ),
    ("manufacturing.screen.po.new_save", "Send the PO"),
    ("manufacturing.screen.po.number", "PO number"),
    ("manufacturing.screen.po.call_off", "Call-off number"),
    (
        "manufacturing.screen.po.call_off_hint",
        "Only for a call-off against a blanket PO.",
    ),
    ("manufacturing.screen.po.note", "Note"),
    ("manufacturing.screen.po.day", "Sent on"),
    ("manufacturing.screen.po.empty", "No PO sent yet"),
    (
        "manufacturing.screen.po.empty_text",
        "The purchase orders you send appear here.",
    ),
    ("manufacturing.screen.po.done", "The PO was sent."),
    // The column labels of the list grids (#556).
    ("manufacturing.screen.grid.item", "Item"),
    ("manufacturing.screen.grid.item_number", "Item number"),
    ("manufacturing.screen.grid.status", "Status"),
    ("manufacturing.screen.grid.made", "Made"),
    ("manufacturing.screen.grid.planned_quantity", "Planned quantity"),
    ("manufacturing.screen.grid.planned_start", "Planned start"),
    ("manufacturing.screen.grid.planned_end", "Planned end"),
    ("manufacturing.screen.grid.source", "Source"),
    ("manufacturing.screen.grid.lot", "Lot"),
    ("manufacturing.screen.grid.lot_number", "Lot number"),
    ("manufacturing.screen.grid.customer", "Customer"),
    ("manufacturing.screen.grid.ended", "How it ended"),
    ("manufacturing.screen.grid.invoices", "Invoices"),
    ("manufacturing.screen.grid.accepted", "Certificates that count"),
    ("manufacturing.screen.grid.reason", "Reason"),
    ("manufacturing.screen.grid.scrap", "Scrap value"),
    ("manufacturing.screen.grid.came_back", "Came back"),
    ("manufacturing.screen.grid.decided_at", "Decided at"),
    ("manufacturing.screen.grid.details", "Details"),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key, &[])
}

/// The English text for `key` with `params` filled in as `{{ name }}`; the
/// key itself when it has no text.
pub(crate) fn english(key: &str, params: &[(&str, &(dyn Display + Sync))]) -> String {
    let text = TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| *text);
    fill(text, params)
}

fn fill(text: &str, params: &[(&str, &(dyn Display + Sync))]) -> String {
    let mut data = renox::serde_json::Map::new();
    for (name, value) in params {
        data.insert(
            (*name).to_owned(),
            renox::serde_json::Value::String(value.to_string()),
        );
    }
    wl_kernel::notify::fill(text, &renox::serde_json::Value::Object(data))
}

/// The English text of `key`, unfilled: a notification template.
pub(crate) fn template(key: &str) -> &'static str {
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("", |(_, text)| *text)
}

/// Every text of the screens, by the name a template reads
/// (`tx['orders.title']` is the key `manufacturing.screen.orders.title`).
pub(crate) fn screen_texts(lang: &Lang) -> std::collections::BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .filter_map(|(key, _)| Some((key.strip_prefix("manufacturing.screen.")?, t(lang, key))))
        .collect()
}

/// The text for `key` in the viewer's language with `params` filled in as
/// `{{ name }}`: the app's translation when it has one, else the English one.
pub(crate) fn tf(lang: &Lang, key: &str, params: &[(&str, &(dyn Display + Sync))]) -> String {
    let found = lang.t(key, &[]);
    let text = if found == key { template(key) } else { &found };
    if text.is_empty() {
        return key.to_owned();
    }
    fill(text, params)
}

/// The names of the screen texts, for the test that every text a template
/// reads exists.
#[cfg(test)]
pub(crate) fn screen_text_keys() -> Vec<&'static str> {
    TEXTS
        .iter()
        .filter_map(|(key, _)| key.strip_prefix("manufacturing.screen."))
        .collect()
}

/// The word for `value` in `group` (`status`, `hold_reason`); the value
/// itself, without underscores, when it has no text.
pub(crate) fn word(lang: &Lang, group: &str, value: &str) -> String {
    let key = format!("manufacturing.screen.{group}.{value}");
    let found = t(lang, &key);
    if found == key {
        value.replace('_', " ")
    } else {
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("manufacturing.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn every_key_the_code_asks_for_has_a_text() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "rs") || path.ends_with("texts.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for call in source.split("english(").skip(1) {
                let call = call.trim_start();
                let Some(rest) = call.strip_prefix('"') else {
                    continue;
                };
                let key = rest.split('"').next().unwrap_or_default();
                if key.starts_with("manufacturing.") && !TEXTS.iter().any(|(k, _)| *k == key) {
                    missing.push(format!("{}: {key}", path.display()));
                }
            }
        }
        assert!(missing.is_empty(), "keys without a text: {missing:?}");
    }

    #[test]
    fn a_text_is_filled_with_its_parameters() {
        assert_eq!(
            english("manufacturing.notice.subject", &[("title", &"Hello")]),
            "Hello"
        );
        assert_eq!(
            english("manufacturing.nothing", &[]),
            "manufacturing.nothing"
        );
    }
}
