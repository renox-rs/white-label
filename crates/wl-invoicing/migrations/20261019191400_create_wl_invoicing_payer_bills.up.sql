-- The payer bill (#79 base, wl-invoicing §3 and §4): what one payer is asked
-- to pay for one source, with its payer agreement, lines and the errors the
-- submission channel gave per attempt. Money is in fils of the bill's
-- currency.
CREATE TABLE "wl_invoicing_payer_agreements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL,
    -- The payer (a party: insurer, TPA).
    payer_party_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    -- `customer` or `payer`: who gets the tax invoice.
    vat_recipient TEXT NOT NULL DEFAULT 'customer',
    reference_required BOOLEAN NOT NULL DEFAULT FALSE,
    -- Days after the service in which the payer accepts a first claim.
    first_submission_days INTEGER NOT NULL DEFAULT 30,
    -- Days after a remittance in which the payer accepts a resubmission.
    resubmission_days INTEGER NOT NULL DEFAULT 30,
    resubmission_limit INTEGER NOT NULL DEFAULT 2,
    -- A write-off above this needs an approval (FIN.ACL.14).
    write_off_limit INTEGER NOT NULL DEFAULT 0,
    accepts_late_submissions BOOLEAN NOT NULL DEFAULT FALSE,
    -- Whether lines the payer will not pay may be billed to the customer.
    bill_customer_allowed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_payer_agreements_payer"
    ON "wl_invoicing_payer_agreements" (payer_party_id);

CREATE TABLE "wl_invoicing_payer_bills" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- From the `payer_bill` series at the first `issue`; kept after a
    -- technical rejection and when voided.
    number TEXT,
    legal_entity_id INTEGER NOT NULL,
    agreement_id INTEGER NOT NULL REFERENCES "wl_invoicing_payer_agreements" (id),
    payer_party_id INTEGER NOT NULL,
    customer_party_id INTEGER NOT NULL,
    -- What it is for (a clinic claim, an insurance claim).
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- `customer` or `payer`: who the tax invoice is addressed to.
    vat_recipient TEXT NOT NULL DEFAULT 'customer',
    payer_reference TEXT,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- The day of the service (the visit, the job): the windows count from it.
    service_date TEXT NOT NULL,
    -- The customer's tax invoice that carries the payer share.
    customer_invoice_id INTEGER,
    -- The payer's own tax invoice (the payer is the recipient).
    payer_invoice_id INTEGER,
    claimed_total INTEGER NOT NULL DEFAULT 0,
    paid_total INTEGER NOT NULL DEFAULT 0,
    denied_total INTEGER NOT NULL DEFAULT 0,
    written_off_total INTEGER NOT NULL DEFAULT 0,
    transferred_total INTEGER NOT NULL DEFAULT 0,
    -- Claimed less paid, written off and transferred.
    balance INTEGER NOT NULL DEFAULT 0,
    resubmission_count INTEGER NOT NULL DEFAULT 0,
    -- How many times it was issued (a rejected submission is issued again).
    issue_count INTEGER NOT NULL DEFAULT 0,
    has_remittance BOOLEAN NOT NULL DEFAULT FALSE,
    last_remittance_on TEXT,
    -- Whether the receivable counts in the ageing.
    in_ageing BOOLEAN NOT NULL DEFAULT FALSE,
    -- 05: a void is allowed only before the first submission.
    void_before_first_submission BOOLEAN NOT NULL DEFAULT FALSE,
    -- Closed from draft for a missed first-submission window.
    not_submitted_in_time BOOLEAN NOT NULL DEFAULT FALSE,
    -- A tax document (the payer's invoice, a credit note, an extra invoice)
    -- still has to be issued.
    tax_document_pending BOOLEAN NOT NULL DEFAULT FALSE,
    close_reason TEXT,
    closed_at TEXT,
    closed_by INTEGER,
    created_by INTEGER,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_payer_bills_number"
    ON "wl_invoicing_payer_bills" (number);
CREATE INDEX "wl_invoicing_payer_bills_status"
    ON "wl_invoicing_payer_bills" (status);
CREATE INDEX "wl_invoicing_payer_bills_source"
    ON "wl_invoicing_payer_bills" (source_type, source_id);
CREATE INDEX "wl_invoicing_payer_bills_payer"
    ON "wl_invoicing_payer_bills" (payer_party_id);

CREATE TABLE "wl_invoicing_payer_bill_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payer_bill_id INTEGER NOT NULL
        REFERENCES "wl_invoicing_payer_bills" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    description TEXT NOT NULL,
    vat_category TEXT NOT NULL DEFAULT 'zero_rated',
    -- The line of the tax document that carries it: the payer's invoice, or
    -- the customer's invoice.
    invoice_line_position INTEGER,
    claimed INTEGER NOT NULL DEFAULT 0,
    -- What the payer's tax invoice bills for the line so far.
    invoiced INTEGER NOT NULL DEFAULT 0,
    paid INTEGER NOT NULL DEFAULT 0,
    denied INTEGER NOT NULL DEFAULT 0,
    written_off INTEGER NOT NULL DEFAULT 0,
    transferred INTEGER NOT NULL DEFAULT 0,
    denial_code TEXT,
    denial_reason TEXT,
    recoupment_code TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_payer_bill_lines_position"
    ON "wl_invoicing_payer_bill_lines" (payer_bill_id, position);

-- The errors the submission channel gave, one row per error and attempt.
CREATE TABLE "wl_invoicing_payer_submission_errors" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payer_bill_id INTEGER NOT NULL
        REFERENCES "wl_invoicing_payer_bills" (id) ON DELETE CASCADE,
    -- The submission the channel rejected: 1 for the first.
    attempt INTEGER NOT NULL,
    code TEXT NOT NULL,
    message TEXT NOT NULL,
    recorded_on TEXT NOT NULL,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_payer_submission_errors_bill"
    ON "wl_invoicing_payer_submission_errors" (payer_bill_id, attempt);
