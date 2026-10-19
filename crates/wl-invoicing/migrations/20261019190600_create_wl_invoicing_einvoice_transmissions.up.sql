-- One sending of an invoice or credit note to the accredited service
-- provider (wl-invoicing §4 *E-invoice transmission*, §5.12, `FIN.INV.01`;
-- issues #72 and #73). The transmission never changes the issued document.
CREATE TABLE "wl_invoicing_einvoice_transmissions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL,
    -- `invoicing.invoice` or `invoicing.credit_note`, and its id.
    document_type TEXT NOT NULL,
    document_id INTEGER NOT NULL,
    -- The name of the ASP adapter of the entity when it was queued.
    adapter TEXT,
    -- The recipient identifier read from the party; stored here, never on
    -- the document.
    recipient_id TEXT,
    -- The structured payload built from the issued document, and its
    -- SHA-256 over the stored text.
    payload TEXT NOT NULL DEFAULT '{}',
    payload_hash TEXT NOT NULL DEFAULT '',
    -- 1 for the first sending, plus one for each `resubmit`.
    attempt INTEGER NOT NULL DEFAULT 1,
    -- How many tries of this attempt failed because the ASP could not be
    -- reached (or no adapter was configured); decides the next try.
    tries INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    -- When the attempt began: the retry times are measured from it.
    queued_at TEXT NOT NULL,
    next_try_at TEXT,
    submitted_at TEXT,
    response_at TEXT,
    asp_reference TEXT,
    -- The error codes and messages of a rejection, as JSON lists.
    error_codes TEXT NOT NULL DEFAULT '[]',
    error_messages TEXT NOT NULL DEFAULT '[]',
    -- The credit note and the new invoice that corrected the document.
    replaced_by_credit_note_id INTEGER,
    replaced_by_invoice_id INTEGER,
    -- `queued`, `submitted`, `accepted`, `rejected` or `superseded`.
    status TEXT NOT NULL DEFAULT 'queued',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_einvoice_transmissions_document"
    ON "wl_invoicing_einvoice_transmissions" (document_type, document_id);
CREATE INDEX "wl_invoicing_einvoice_transmissions_status"
    ON "wl_invoicing_einvoice_transmissions" (status, next_try_at);
CREATE INDEX "wl_invoicing_einvoice_transmissions_entity"
    ON "wl_invoicing_einvoice_transmissions" (legal_entity_id, status);
