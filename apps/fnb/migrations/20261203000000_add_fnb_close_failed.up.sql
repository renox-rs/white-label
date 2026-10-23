-- Closing a failed delivery with the refund decision (06 F&B §7.1
-- `close_failed`, `FNB.ACL.16`, `FNB.ACL.17`; BL-FNB-02): the decision, the
-- refund asked of wl-payments, the wastage reference, and the refund
-- requests a cashier makes of a customer-side failure.
ALTER TABLE "fnb_orders" ADD COLUMN refund_decision TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN refund_amount INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "fnb_orders" ADD COLUMN refund_id INTEGER;
ALTER TABLE "fnb_orders" ADD COLUMN wastage_ref TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN wastage_posted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "fnb_orders" ADD COLUMN closed_at TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN close_reason TEXT;

CREATE TABLE "fnb_refund_requests" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES "fnb_orders" (id),
    amount INTEGER NOT NULL DEFAULT 0,
    reason TEXT NOT NULL DEFAULT '',
    requested_by INTEGER,
    -- The wl-payments refund, approved by someone other than the requester.
    refund_id INTEGER,
    approved_by INTEGER,
    approved_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_refund_requests_order" ON "fnb_refund_requests" (order_id);
