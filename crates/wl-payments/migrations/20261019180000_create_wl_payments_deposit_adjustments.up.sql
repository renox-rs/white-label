-- The adjustments of a held deposit when its source agreement changes the
-- amount (wl-payments §4 *Deposit* `adjust`, issue #115): an increase raises a
-- top-up charge, a decrease a part refund. The deposit stays `held`.
CREATE TABLE "wl_payments_deposit_adjustments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    deposit_id INTEGER NOT NULL REFERENCES "wl_payments_deposits" (id),
    -- The version of the source (lease amendment or renewal) that changed it.
    source_version INTEGER NOT NULL,
    old_amount INTEGER NOT NULL,
    new_amount INTEGER NOT NULL,
    -- The day the change takes effect: the due date of a top-up charge.
    effective_on TEXT NOT NULL,
    -- `top_up` or `part_refund`.
    kind TEXT NOT NULL,
    -- The increase, or the part refunded, in fils; 0 when nothing was paid in
    -- above the new amount, so nothing is refunded.
    difference INTEGER NOT NULL,
    -- The top-up charge, once raised.
    charge_subject_type TEXT,
    charge_subject_id INTEGER,
    -- The part refund of a decrease.
    refund_id INTEGER REFERENCES "wl_payments_refunds" (id),
    -- Whether the held amount was lowered by the part refund now.
    held_applied INTEGER NOT NULL DEFAULT 0,
    -- `open`, `settled` (top-up paid, part refund paid) or `closed` (part
    -- refund rejected or cancelled).
    status TEXT NOT NULL DEFAULT 'open',
    closed_reason TEXT,
    adjusted_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_deposit_adjustments_deposit" ON "wl_payments_deposit_adjustments" (deposit_id, status);
CREATE INDEX "wl_payments_deposit_adjustments_refund" ON "wl_payments_deposit_adjustments" (refund_id);
