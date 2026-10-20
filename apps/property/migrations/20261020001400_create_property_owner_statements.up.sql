-- Management agreements and owner statements (01 Property §6, §7.1
-- *Management agreement* and *Owner statement*, `PRP.PRT.03`, `PRP.OWN.01`;
-- BL-PRP-19): the contract with a landlord, the units it covers with the days
-- they come in and go out, the statement of a period with its revision, and
-- its lines.
CREATE TABLE "property_management_agreements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The owner, a party of wl-parties.
    owner_party_id INTEGER NOT NULL,
    -- The fee in basis points of the rent collected (500 = 5%), and a fixed
    -- fee per statement in fils; either may be 0.
    fee_bp INTEGER NOT NULL DEFAULT 0,
    fee_fixed INTEGER NOT NULL DEFAULT 0,
    payout_frequency TEXT NOT NULL DEFAULT 'monthly',
    notice_days INTEGER NOT NULL DEFAULT 90,
    -- Whether the property manager may invoice a negative balance to the
    -- owner (`PRP.ACL.21`).
    may_invoice_negative_balance INTEGER NOT NULL DEFAULT 0,
    start_date TEXT NOT NULL,
    end_date TEXT,
    end_reason TEXT,
    handover_date TEXT,
    -- The management company's legal entity, when it is not the default.
    legal_entity_id INTEGER,
    -- draft, active, ending or ended.
    status TEXT NOT NULL DEFAULT 'draft',
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_management_agreements_owner" ON "property_management_agreements" (owner_party_id);

CREATE TABLE "property_agreement_units" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agreement_id INTEGER NOT NULL REFERENCES "property_management_agreements" (id),
    unit_id INTEGER NOT NULL REFERENCES "property_units" (id),
    date_in TEXT NOT NULL,
    date_out TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_agreement_units_agreement" ON "property_agreement_units" (agreement_id);
CREATE INDEX "property_agreement_units_unit" ON "property_agreement_units" (unit_id);

CREATE TABLE "property_owner_statements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agreement_id INTEGER NOT NULL REFERENCES "property_management_agreements" (id),
    owner_party_id INTEGER NOT NULL,
    period_start TEXT NOT NULL,
    period_end TEXT NOT NULL,
    -- 1, 2, ...: a revision of the same period; previous_id is the one it
    -- replaced.
    revision INTEGER NOT NULL DEFAULT 1,
    previous_id INTEGER,
    -- Fils. The balance is brought forward + collected - expenses - fee +
    -- adjustments; collected includes the rent written off (negative) and
    -- recovered; expenses and fee are positive amounts that are subtracted.
    brought_forward INTEGER NOT NULL DEFAULT 0,
    collected INTEGER NOT NULL DEFAULT 0,
    expenses INTEGER NOT NULL DEFAULT 0,
    fee INTEGER NOT NULL DEFAULT 0,
    adjustments INTEGER NOT NULL DEFAULT 0,
    carried_forward INTEGER NOT NULL DEFAULT 0,
    -- The user who generated or last edited the draft (`PRP.ACL.08`); empty
    -- for a statement the rule generated and nobody edited.
    prepared_by INTEGER,
    approved_by INTEGER,
    approved_on TEXT,
    return_reason TEXT,
    published_on TEXT,
    published_by INTEGER,
    revise_reason TEXT,
    -- The payout recorded for the statement (BL-PRP-20): its amount and the
    -- outgoing payment; revise is refused once there is one.
    payout_amount INTEGER,
    payout_payment_id INTEGER,
    -- draft, approved, published, settled or superseded.
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "property_owner_statements_period" ON "property_owner_statements" (agreement_id, period_start, revision);
CREATE INDEX "property_owner_statements_agreement" ON "property_owner_statements" (agreement_id);

CREATE TABLE "property_statement_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agreement_id INTEGER NOT NULL REFERENCES "property_management_agreements" (id),
    -- The statement it is on; empty while it waits for the next draft.
    statement_id INTEGER,
    -- collected, expense, fee, adjustment, written_off or recovered.
    kind TEXT NOT NULL,
    description TEXT NOT NULL,
    -- Fils, with the sign of its effect on the owner's balance.
    amount INTEGER NOT NULL,
    -- What it comes from (`invoicing.invoice:7`), so it is made once.
    source TEXT,
    -- The line of the other owner's statement a reallocation is linked to.
    linked_to INTEGER,
    -- The day it happened; a pending line goes to the statement of the period
    -- it is in or the next one.
    line_on TEXT NOT NULL,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_statement_lines_statement" ON "property_statement_lines" (statement_id);
CREATE INDEX "property_statement_lines_agreement" ON "property_statement_lines" (agreement_id, statement_id);
