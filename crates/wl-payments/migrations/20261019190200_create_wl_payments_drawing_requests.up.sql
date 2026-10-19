-- An owner drawing paid out only after its approval (wl-payments §3 *Payment*,
-- §4 `record`, `FIN.ACL.21`; issue #119). Saving a drawing keeps its details
-- here and asks wl-invoicing for the approval; the final approval records the
-- payment with these details, a rejection, a withdrawal or an expiry records
-- nothing. A drawing paid by cheque waits, `approved`, for its cheque.
CREATE TABLE "wl_payments_drawing_requests" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL,
    -- The code of the wl-invoicing budget category of kind `owner_drawings`.
    category TEXT NOT NULL,
    -- The category's owner party, the payee.
    party_id INTEGER NOT NULL,
    method TEXT NOT NULL,
    method_details TEXT NOT NULL DEFAULT '{}',
    -- In AED fils.
    amount INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    payment_date TEXT NOT NULL,
    reference TEXT,
    bank_account TEXT,
    requested_by INTEGER,
    approved_by INTEGER,
    -- Recorded without a second approver (`FIN.ACL.23`).
    self_approved INTEGER NOT NULL DEFAULT 0,
    -- Why it was not approved, or why it could not be recorded.
    reason TEXT,
    payment_id INTEGER,
    cheque_id INTEGER,
    -- `requested`, `approved`, `recorded` or `rejected`.
    status TEXT NOT NULL DEFAULT 'requested',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_drawing_requests_status"
    ON "wl_payments_drawing_requests" (status);
