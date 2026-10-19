-- The approval of an owner drawing (wl-invoicing §5.16, `FIN.ACL.21`,
-- `FIN.ACL.23`; issue #90): a supplier bill with a line coded to a budget
-- category of kind `owner_drawings` is approved only after one of these is
-- `approved`. The status moves only through the machine.
CREATE TABLE "wl_invoicing_drawing_approvals" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL,
    -- The record the drawing is on: a supplier bill today.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- The drawing in AED fils, with the VAT that is not recovered.
    amount INTEGER NOT NULL,
    requested_by INTEGER,
    approved_by INTEGER,
    approved_at TEXT,
    approval_request_id INTEGER,
    -- Recorded without a second approver (the sole owner, `FIN.ACL.23`).
    self_approved INTEGER NOT NULL DEFAULT 0,
    rejected_reason TEXT,
    -- draft, requested, approved or rejected.
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_drawing_approvals_source"
    ON "wl_invoicing_drawing_approvals" (source_type, source_id);
CREATE INDEX "wl_invoicing_drawing_approvals_status"
    ON "wl_invoicing_drawing_approvals" (status);

-- A supplier bill line coded again after the drawing was rejected, or for
-- any other reason, while the bill is not approved: shown to whoever
-- approves the bill.
CREATE TABLE "wl_invoicing_bill_line_recodings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bill_id INTEGER NOT NULL,
    line_id INTEGER NOT NULL,
    from_code TEXT,
    to_code TEXT,
    recoded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_bill_line_recodings_bill"
    ON "wl_invoicing_bill_line_recodings" (bill_id);
