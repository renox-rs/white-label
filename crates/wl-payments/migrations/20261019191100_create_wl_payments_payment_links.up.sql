-- Online payments (wl-payments §3 Payment link, §4 Payment `start_online`,
-- `confirm`, `fail`, `late_confirm`; issues #95, #96): a request to pay
-- online with an expiry, and what an online payment remembers of it. The
-- status of a link moves only through the machine; nothing is deleted.
CREATE TABLE "wl_payments_payment_links" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    legal_entity_id INTEGER NOT NULL,
    -- Money in fils of the currency.
    amount INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- JSON array of the documents it pays: {target_type, target_id, amount}.
    targets TEXT NOT NULL DEFAULT '[]',
    -- The record paid in advance for (a booking, an order), if any.
    prepaid_type TEXT,
    prepaid_id INTEGER,
    -- The subscription it is a charge for, if any.
    subscription_id INTEGER,
    -- The account key of the gateway the party pays through.
    gateway TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    paid_payment_id INTEGER,
    -- The staff user who sent it: the one who took the payment.
    created_by INTEGER,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_payment_links_status" ON "wl_payments_payment_links" (status);
CREATE INDEX "wl_payments_payment_links_prepaid" ON "wl_payments_payment_links" (prepaid_type, prepaid_id);

-- What an online payment remembers: the link it was paid through, the
-- subscription it charged, and why and when it failed.
ALTER TABLE "wl_payments_payments" ADD COLUMN payment_link_id INTEGER;
ALTER TABLE "wl_payments_payments" ADD COLUMN subscription_id INTEGER;
ALTER TABLE "wl_payments_payments" ADD COLUMN failure_kind TEXT;
ALTER TABLE "wl_payments_payments" ADD COLUMN failure_reason TEXT;
ALTER TABLE "wl_payments_payments" ADD COLUMN failed_at TEXT;
ALTER TABLE "wl_payments_payments" ADD COLUMN late_confirmed_at TEXT;
CREATE INDEX "wl_payments_payments_link" ON "wl_payments_payments" (payment_link_id);
CREATE INDEX "wl_payments_payments_gateway_txn" ON "wl_payments_payments" (gateway_transaction);
