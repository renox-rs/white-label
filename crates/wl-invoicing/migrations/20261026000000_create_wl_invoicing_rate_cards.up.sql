-- Rate cards (wl-invoicing §3 *Rate card*, `FIN.RAT.01`; 09 Manpower
-- `MPW.ACL.16`): the prices of one source contract. A change is never an
-- edit of an accepted line: it is a new version with an effective date that
-- the client accepts; a version dated in the past needs an approval after
-- the client's acceptance. The status of a version moves only through its
-- machine.
CREATE TABLE "wl_invoicing_rate_cards" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Who is billed.
    party_id INTEGER NOT NULL,
    -- The contract the card prices, e.g. `manpower.client_contract:4`.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (source_type, source_id)
);

CREATE TABLE "wl_invoicing_rate_card_versions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rate_card_id INTEGER NOT NULL REFERENCES "wl_invoicing_rate_cards" (id),
    -- 1, 2, 3 ... per card.
    version INTEGER NOT NULL,
    -- The first day the version prices.
    effective_from TEXT NOT NULL,
    -- The day (Asia/Dubai) the version was made; a version effective before
    -- it is *dated in the past*.
    submitted_on TEXT NOT NULL,
    past_dated INTEGER NOT NULL DEFAULT 0,
    -- pending, awaiting_approval, accepted or rejected.
    status TEXT NOT NULL DEFAULT 'pending',
    accepted_by INTEGER,
    accepted_on TEXT,
    -- The signed acceptance a member of staff recorded for the client.
    accepted_document TEXT,
    approval_request_id INTEGER,
    approved_by INTEGER,
    rejected_reason TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_rate_card_versions_card"
    ON "wl_invoicing_rate_card_versions" (rate_card_id);
CREATE INDEX "wl_invoicing_rate_card_versions_status"
    ON "wl_invoicing_rate_card_versions" (status);

CREATE TABLE "wl_invoicing_rate_card_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    version_id INTEGER NOT NULL REFERENCES "wl_invoicing_rate_card_versions" (id),
    role TEXT NOT NULL,
    shift_type TEXT NOT NULL,
    -- hour, day or month.
    unit TEXT NOT NULL DEFAULT 'hour',
    -- The bill rate of one unit, in fils.
    rate INTEGER NOT NULL,
    -- Multipliers in hundredths: 125 is 1.25; 100 is none.
    ot_multiplier INTEGER NOT NULL DEFAULT 100,
    holiday_multiplier INTEGER NOT NULL DEFAULT 100,
    -- The minimum hours billed per shift, in thousandths of an hour.
    min_hours_milli INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_rate_card_lines_version"
    ON "wl_invoicing_rate_card_lines" (version_id);

-- What a usage record was priced from, so that a version dated in the past
-- can reprice the hours that are neither invoiced nor in a locked timesheet
-- (`MPW.ACL.16`).
CREATE TABLE "wl_invoicing_usage_pricings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    usage_record_id INTEGER NOT NULL UNIQUE REFERENCES "wl_invoicing_usage_records" (id),
    rate_card_id INTEGER NOT NULL,
    -- The version that priced it.
    version_id INTEGER NOT NULL,
    work_date TEXT NOT NULL,
    role TEXT NOT NULL,
    shift_type TEXT NOT NULL,
    -- Normal, overtime, public_holiday ...
    category TEXT NOT NULL,
    -- The hours worked, in thousandths, before the minimum.
    worked_milli INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_usage_pricings_card"
    ON "wl_invoicing_usage_pricings" (rate_card_id, work_date);
