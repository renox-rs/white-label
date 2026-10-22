//! The texts of the production batches (BL-FNB-11): keys `fnb.batch.*`
//! (toasts, refusals, history, notices) and `fnb.screen.batch.*` (what a
//! template reads as `tx['batch...']`).

/// Every text of the production batches.
pub(crate) const BATCH_TEXTS: &[(&str, &str)] = &[
    // Toasts.
    ("fnb.batch.planned", "Batch planned."),
    ("fnb.batch.started", "Batch started: the ingredient lots are recorded."),
    ("fnb.batch.finished", "Batch finished: its lot is in stock."),
    ("fnb.batch.discarded", "Batch discarded: the wastage is posted."),
    (
        "fnb.batch.discard_waiting",
        "The discard waits for a second person to approve the wastage.",
    ),
    ("fnb.batch.cancelled", "Batch cancelled."),
    // Refusals.
    (
        "fnb.batch.problem.no_lots",
        "Pick an ingredient lot for every ingredient before starting the batch.",
    ),
    ("fnb.batch.problem.yield", "Enter the yield that came out, above zero."),
    ("fnb.batch.problem.bad_yield", "That is not a quantity."),
    ("fnb.batch.problem.expiry_needed", "Enter the expiry day of the lot the batch made."),
    (
        "fnb.batch.problem.expiry_later",
        "The expiry {{ expiry }} is later than {{ earliest }}, the earliest expiry of the ingredient lots used.",
    ),
    (
        "fnb.batch.problem.lot_unknown",
        "No usable lot of {{ item }} was found.",
    ),
    (
        "fnb.batch.problem.lot_wrong",
        "Lot {{ lot }} is not a lot of {{ item }}.",
    ),
    (
        "fnb.batch.problem.lot_on_hold",
        "Lot {{ lot }} is on hold and cannot go into a batch.",
    ),
    (
        "fnb.batch.problem.lot_not_usable",
        "Lot {{ lot }} cannot be used ({{ status }}).",
    ),
    (
        "fnb.batch.problem.lot_expired",
        "Lot {{ lot }} is expired and cannot go into a batch.",
    ),
    (
        "fnb.batch.problem.lot_short",
        "Lot {{ lot }} of {{ item }} has {{ left }}; the batch needs {{ needed }}.",
    ),
    (
        "fnb.batch.problem.lot_missing",
        "There is no lot of {{ item }} in stock that can be used.",
    ),
    ("fnb.batch.problem.no_recipe", "Choose a recipe."),
    (
        "fnb.batch.problem.no_location",
        "The outlet has no stock location to take the ingredients from.",
    ),
    ("fnb.batch.problem.not_planned", "Only a planned batch is started or cancelled."),
    ("fnb.batch.problem.not_in_production", "Only a batch in production is finished."),
    ("fnb.batch.problem.discard_reason", "Give the reason for discarding the batch."),
    (
        "fnb.batch.problem.discard_on_hold",
        "A batch on hold is discarded by deciding its stock hold line as discard, so the hold's approvals apply.",
    ),
    ("fnb.batch.problem.not_produced", "Only a produced batch is discarded here."),
    (
        "fnb.batch.problem.stock_off",
        "The stock module is off: the batch cannot be finished.",
    ),
    // What the history says.
    ("fnb.batch.history.planned", "Planned: {{ recipe }}"),
    ("fnb.batch.history.started", "Started {{ number }}"),
    ("fnb.batch.history.finished", "Finished {{ number }}"),
    ("fnb.batch.history.held", "Lot held: {{ reason }}"),
    ("fnb.batch.history.released", "Lot released: {{ number }}"),
    ("fnb.batch.discard_default", "Discarded"),
    // Notices to the chefs.
    ("fnb.batch.hold_title", "Batch {{ number }} is on hold"),
    (
        "fnb.batch.hold_body",
        "The lot of batch {{ number }} is on hold ({{ reason }}). The batch is blocked at every outlet: do not use, transfer or serve it until the hold is decided.",
    ),
    ("fnb.batch.release_title", "Batch {{ number }} is released"),
    (
        "fnb.batch.release_body",
        "The hold on the lot of batch {{ number }} is released: the batch can be used again.",
    ),
    ("fnb.batch.discard_title", "Batch {{ number }} is discarded"),
    (
        "fnb.batch.discard_body",
        "The lot of batch {{ number }} was discarded ({{ reason }}). The wastage is posted.",
    ),
    // The screens.
    ("fnb.screen.batch.title", "Production batches"),
    (
        "fnb.screen.batch.subtitle",
        "Sauces, marinades and doughs made in bulk, with the ingredient lots they used.",
    ),
    ("fnb.screen.batch.plan", "Plan a batch"),
    ("fnb.screen.batch.plan_title", "Plan a production batch"),
    (
        "fnb.screen.batch.plan_description",
        "Choose the recipe and how much to make. The ingredient lots are picked when the batch starts.",
    ),
    ("fnb.screen.batch.recipe", "Recipe"),
    ("fnb.screen.batch.planned", "Planned yield"),
    (
        "fnb.screen.batch.planned_hint",
        "In the item's unit, up to three decimals. Empty: the recipe's standard yield.",
    ),
    ("fnb.screen.batch.outlet", "Kitchen"),
    ("fnb.screen.batch.number", "Batch"),
    ("fnb.screen.batch.actual", "Actual yield"),
    ("fnb.screen.batch.expiry", "Expiry"),
    (
        "fnb.screen.batch.expiry_hint",
        "Not later than the earliest expiry of the ingredient lots used.",
    ),
    ("fnb.screen.batch.empty", "No batches yet"),
    (
        "fnb.screen.batch.empty_text",
        "Plan a batch of a recipe to make a sauce or a marinade for the outlets.",
    ),
    ("fnb.screen.batch.start", "Start"),
    ("fnb.screen.batch.start_title", "Start the batch"),
    (
        "fnb.screen.batch.start_description",
        "Pick the lot of every ingredient. The earliest expiry is proposed; a lot on hold or expired is refused.",
    ),
    ("fnb.screen.batch.no_lot", "No lot can be used"),
    ("fnb.screen.batch.finish", "Finish"),
    ("fnb.screen.batch.finish_title", "Finish the batch"),
    (
        "fnb.screen.batch.finish_description",
        "Enter the yield and the expiry of the lot the batch made. The ingredient lots are consumed.",
    ),
    ("fnb.screen.batch.discard", "Discard"),
    ("fnb.screen.batch.discard_title", "Discard the batch"),
    (
        "fnb.screen.batch.discard_description",
        "What is left of the batch is logged as wastage. Above AED 200 a second person approves it.",
    ),
    ("fnb.screen.batch.cancel", "Cancel the batch"),
    ("fnb.screen.batch.cancel_title", "Cancel this planned batch?"),
    ("fnb.screen.batch.cancel_message", "A planned batch that never started is deleted."),
    ("fnb.screen.batch.details", "Batch"),
    ("fnb.screen.batch.lot", "Lot"),
    ("fnb.screen.batch.lot_status", "Lot state"),
    ("fnb.screen.batch.lot_left", "Left in stock"),
    ("fnb.screen.batch.hold_reason", "On hold because"),
    ("fnb.screen.batch.discard_reason", "Discarded because"),
    ("fnb.screen.batch.inputs", "Ingredient lots used"),
    ("fnb.screen.batch.ingredient", "Ingredient"),
    ("fnb.screen.batch.quantity", "Quantity"),
    ("fnb.screen.batch.no_inputs", "No ingredient lots recorded"),
    (
        "fnb.screen.batch.no_inputs_text",
        "The lots are recorded when the batch starts.",
    ),
    ("fnb.screen.batch.wastage", "Wastage"),
    ("fnb.screen.batch.value", "Value"),
    ("fnb.screen.batch.posted_by", "Posted by"),
    ("fnb.screen.batch.posted_by.batch", "The batch"),
    ("fnb.screen.batch.posted_by.lot", "The discard of the lot"),
    ("fnb.screen.batch.status.planned", "Planned"),
    ("fnb.screen.batch.status.in_production", "In production"),
    ("fnb.screen.batch.status.produced", "Produced"),
    ("fnb.screen.batch.status.on_hold", "On hold"),
    ("fnb.screen.batch.status.consumed", "Consumed"),
    ("fnb.screen.batch.status.discarded", "Discarded"),
    ("fnb.screen.batch.lot_status.quarantine", "Quarantine"),
    ("fnb.screen.batch.lot_status.released", "Released"),
    ("fnb.screen.batch.lot_status.on_hold", "On hold"),
    ("fnb.screen.batch.lot_status.expired", "Expired"),
    ("fnb.screen.batch.lot_status.rejected", "Rejected"),
    ("fnb.screen.batch.lot_status.depleted", "Used up"),
    (
        "fnb.screen.batch.note_on_hold",
        "The lot of this batch is on hold, so the batch cannot be used, transferred or served anywhere. It is discarded by deciding its stock hold line as discard, and released when the hold is released.",
    ),
    (
        "fnb.screen.batch.note_no_lot",
        "Start is not available: an ingredient has no lot in stock that can be used.",
    ),
    (
        "fnb.screen.batch.note_produced",
        "The batch is in stock. Discard it from here when it is spoiled; a hold on its lot puts it on hold by itself.",
    ),
];
