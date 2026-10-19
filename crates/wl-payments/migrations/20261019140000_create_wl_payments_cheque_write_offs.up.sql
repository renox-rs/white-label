-- The invoice write-offs a cheque `write_off` asked wl-invoicing for (wl-payments
-- §4 *Cheque*, issue #106): one row per linked invoice and attempt. The cheque
-- moves to `written_off` once the rows written off add up to its open amount.
CREATE TABLE "wl_payments_cheque_write_offs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    cheque_id INTEGER NOT NULL REFERENCES "wl_payments_cheques" (id),
    -- The invoice whose open balance is written off.
    invoice_id INTEGER NOT NULL,
    -- The invoice balance asked to be written off, in fils.
    amount INTEGER NOT NULL,
    -- The reason the user gave for the cheque.
    reason TEXT NOT NULL,
    requested_by INTEGER,
    -- The approval request when the write-off is above the limit.
    approval_request_id INTEGER,
    -- `pending`, `written_off` or `failed`.
    status TEXT NOT NULL DEFAULT 'pending',
    -- Why a request failed: rejected, expired, withdrawn or failed.
    failure TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_cheque_write_offs_cheque" ON "wl_payments_cheque_write_offs" (cheque_id);
CREATE INDEX "wl_payments_cheque_write_offs_invoice" ON "wl_payments_cheque_write_offs" (invoice_id);
