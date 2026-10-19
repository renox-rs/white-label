-- Leases (01 Property §6, §7.1, PRP.LSE.01): one row per lease version, the
-- charges of its payment plan, and the cheques a cancelled lease waits for.
-- The status moves only through the Lease machine.
CREATE TABLE "property_leases" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The id of version 1: the lease the versions belong to. Deposits and
    -- cheques are held for it.
    root_id INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 1,
    -- new, renewal or amendment.
    version_kind TEXT NOT NULL DEFAULT 'new',
    -- The version this one replaces (renewal and amendment).
    previous_id INTEGER REFERENCES "property_leases" (id),
    -- The day an amendment applies from.
    effective_date TEXT,
    building_id INTEGER NOT NULL REFERENCES "property_buildings" (id),
    unit_id INTEGER NOT NULL REFERENCES "property_units" (id),
    -- The billed tenant (a wl-parties party). One in the MVP (§13 question 2).
    tenant_party_id INTEGER NOT NULL,
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    -- AED fils.
    annual_rent INTEGER NOT NULL DEFAULT 0,
    deposit_amount INTEGER NOT NULL DEFAULT 0,
    -- A property manager waived the deposit.
    deposit_waived INTEGER NOT NULL DEFAULT 0,
    -- exempt (residential) or standard (commercial), copied from the unit's use.
    vat_category TEXT NOT NULL DEFAULT 'exempt',
    -- The contract's own notice period; empty means the 90 days of the law.
    contract_notice_days INTEGER,
    terms TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_by INTEGER,
    -- The day the version entered `pending_signature` (Asia/Dubai).
    submitted_on TEXT,
    -- PRP.RUL.12: the follow-up date the property manager extended to, the
    -- day the reminder at N days was sent, the due date the extend-or-cancel
    -- task was last opened for.
    follow_up_until TEXT,
    chase_reminded_on TEXT,
    chase_tasked_on TEXT,
    activated_on TEXT,
    -- Cancel (PRP.ACL.18).
    cancel_reason TEXT,
    cancelled_on TEXT,
    cancel_forfeit INTEGER NOT NULL DEFAULT 0,
    cancel_forfeit_clause TEXT,
    -- Set once the money effects of a cancellation ran.
    money_settled_at TEXT,
    -- Ejari / Tawtheeq (PRP.LSE.02).
    ejari_number TEXT,
    ejari_registered_on TEXT,
    ejari_expires_on TEXT,
    ejari_cancelled_on TEXT,
    -- Renewal (PRP.RUL.01, PRP.RUL.09, PRP.ACL.17).
    renewal_decision TEXT NOT NULL DEFAULT 'undecided',
    notice_of_changes_sent_on TEXT,
    notice_of_changes_delivered_on TEXT,
    notice_of_changes_proof TEXT,
    current_terms_only INTEGER NOT NULL DEFAULT 0,
    holding_over_until TEXT,
    -- Notice to end the lease.
    notice_date TEXT,
    notice_party TEXT,
    notice_reason TEXT,
    planned_move_out TEXT,
    actual_move_out TEXT,
    rent_due_until TEXT,
    handed_over_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_leases_root" ON "property_leases" (root_id, version);
CREATE INDEX "property_leases_unit" ON "property_leases" (unit_id, status);
CREATE INDEX "property_leases_tenant" ON "property_leases" (tenant_party_id);
CREATE INDEX "property_leases_status_end" ON "property_leases" (status, end_date);

-- The payment plan and, once active, the charge schedule of a version
-- (FIN.BIL.01 until wl-invoicing keeps the schedules itself).
CREATE TABLE "property_lease_charges" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    lease_id INTEGER NOT NULL REFERENCES "property_leases" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL DEFAULT 1,
    -- `rent`, or a code of the client's charge catalogue.
    charge_type TEXT NOT NULL DEFAULT 'rent',
    description TEXT NOT NULL DEFAULT '',
    due_date TEXT NOT NULL,
    -- AED fils before VAT.
    amount INTEGER NOT NULL DEFAULT 0,
    vat_category TEXT NOT NULL DEFAULT 'exempt',
    -- planned, scheduled, invoiced or cancelled.
    status TEXT NOT NULL DEFAULT 'planned',
    invoice_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_lease_charges_lease" ON "property_lease_charges" (lease_id, position);
CREATE INDEX "property_lease_charges_due" ON "property_lease_charges" (status, due_date);

-- A cheque already deposited when a lease was cancelled: its amount joins the
-- refunds when it clears and nothing is refunded when it bounces.
CREATE TABLE "property_cancel_waits" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    lease_id INTEGER NOT NULL REFERENCES "property_leases" (id) ON DELETE CASCADE,
    cheque_id INTEGER NOT NULL,
    party_id INTEGER NOT NULL,
    -- waiting, refunded or bounced.
    outcome TEXT NOT NULL DEFAULT 'waiting',
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (lease_id, cheque_id)
);
