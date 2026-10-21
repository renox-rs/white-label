-- What a delivered delivery did to the invoice that bills it (wl-invoicing
-- §5.2 step 4, #85): one row per delivery with a cash-on-delivery
-- difference, so an event received twice changes nothing a second time.
CREATE TABLE "wl_invoicing_cod_differences" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The delivery, as `delivery.delivery:42`.
    delivery_type TEXT NOT NULL,
    delivery_id INTEGER NOT NULL,
    -- The record the delivery was requested for.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- The invoice that bills the delivery; NULL when none was found.
    invoice_id INTEGER,
    -- Who bears the difference: customer, client, business or driver.
    absorbed_by TEXT NOT NULL,
    -- The difference in fils, above 0.
    amount INTEGER NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    -- customer_owes, client_remittance, draft_reduced, credited, no_invoice
    -- or refused.
    outcome TEXT NOT NULL,
    -- The credit note of an issued invoice.
    credit_note_id INTEGER,
    -- The position of the negative line on a draft.
    line_position INTEGER,
    -- Why the credit was refused.
    note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_cod_differences_delivery"
    ON "wl_invoicing_cod_differences" (delivery_type, delivery_id);
CREATE INDEX "wl_invoicing_cod_differences_invoice"
    ON "wl_invoicing_cod_differences" (invoice_id);
