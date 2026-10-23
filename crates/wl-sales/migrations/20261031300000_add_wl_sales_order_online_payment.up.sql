-- The external order number of an aggregator or website order, unique per
-- channel (BL-SAL-01), and the online payment of an order: its status, the
-- time the gateway has, its payment link and the promo code it took a use of
-- (BL-SAL-02, BL-SAL-03), and one row per payment the gateway confirmed for an
-- order so each payment is handled once (BL-SAL-02, BL-SAL-04, SAL.RUL.06).
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN external_number TEXT;
-- The number trimmed and lower-cased; empty (NULL) when the number is not
-- checked (no number, or the setting is off). NULLs never collide.
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN external_key TEXT;
-- `not_required`, `pending`, `received` or `failed`.
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN payment_status TEXT NOT NULL DEFAULT 'not_required';
-- When the payment window ends (set on `submit_online`).
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN payment_due_at TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN payment_link_id INTEGER;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN payment_link TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN promo_code_id INTEGER;

CREATE UNIQUE INDEX "wl_sales_sales_orders_external" ON "wl_sales_sales_orders" (channel, external_key);
CREATE INDEX "wl_sales_sales_orders_payment_due" ON "wl_sales_sales_orders" (payment_due_at);

CREATE TABLE "wl_sales_order_payments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES "wl_sales_sales_orders" (id) ON DELETE CASCADE,
    -- The payment (wl-payments); a payment is handled once.
    payment_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    -- `claimed` while it is handled, then `confirmed` (it confirmed the
    -- order), `refund_requested` (SAL.RUL.06) or `refund_refused`.
    outcome TEXT NOT NULL DEFAULT 'claimed',
    -- The refund wl-payments was asked for, if any.
    refund_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_order_payments_payment" ON "wl_sales_order_payments" (payment_id);
CREATE INDEX "wl_sales_order_payments_order" ON "wl_sales_order_payments" (order_id);
