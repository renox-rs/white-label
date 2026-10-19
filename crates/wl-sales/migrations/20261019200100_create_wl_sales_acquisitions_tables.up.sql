-- Party acquisitions (wl-sales §3 *Party acquisition*, §5.8; issue #139): how a
-- party became a customer, one row per party, the first sale linked, and the
-- sales that may replace it. Not stored on the wl-parties record.
CREATE TABLE "wl_sales_party_acquisitions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- One live row per party; after a merge the other row stays as history.
    party_id INTEGER NOT NULL,
    source_id INTEGER NOT NULL REFERENCES "wl_sales_lead_sources" (id),
    campaign_id INTEGER REFERENCES "wl_sales_campaigns" (id),
    -- `lead`, `staff`, `web_form`, `promo_code` or `import`.
    set_from TEXT NOT NULL DEFAULT 'staff',
    -- The converting lead, if the acquisition was set from one.
    lead_id INTEGER REFERENCES "wl_sales_leads" (id),
    -- Set when a first sale arrived with no source: the source is `other` and
    -- the acquisition is on the exceptions report.
    source_missing INTEGER NOT NULL DEFAULT 0,
    -- The first sale (a Subject), its document date and its value excluding
    -- VAT in AED fils, net of credit notes.
    first_sale_type TEXT,
    first_sale_id INTEGER,
    first_sale_date TEXT,
    first_sale_value INTEGER NOT NULL DEFAULT 0,
    -- The branch of the first sale, else of the converting lead, and the entity.
    scope_type TEXT,
    scope_id TEXT,
    legal_entity_id INTEGER,
    acquired_at TEXT NOT NULL,
    -- `CustomerAcquired` was emitted for the party.
    acquired_emitted INTEGER NOT NULL DEFAULT 0,
    -- After a party merge: the acquisition that was kept.
    superseded_by INTEGER REFERENCES "wl_sales_party_acquisitions" (id),
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_party_acquisitions_party" ON "wl_sales_party_acquisitions" (party_id);
CREATE INDEX "wl_sales_party_acquisitions_source" ON "wl_sales_party_acquisitions" (source_id);
CREATE INDEX "wl_sales_party_acquisitions_campaign" ON "wl_sales_party_acquisitions" (campaign_id);
CREATE INDEX "wl_sales_party_acquisitions_first_sale" ON "wl_sales_party_acquisitions" (first_sale_date);

-- Every sale reported for a party, the candidates for its first sale. The
-- nightly run links the next standing one when the first is voided,
-- cancelled or credited in full.
CREATE TABLE "wl_sales_acquisition_sales" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    sale_date TEXT NOT NULL,
    -- AED fils excluding VAT.
    value INTEGER NOT NULL DEFAULT 0,
    -- The net of the credit notes on it.
    credited INTEGER NOT NULL DEFAULT 0,
    -- Voided or cancelled.
    gone INTEGER NOT NULL DEFAULT 0,
    scope_type TEXT,
    scope_id TEXT,
    legal_entity_id INTEGER,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (subject_type, subject_id)
);

CREATE INDEX "wl_sales_acquisition_sales_party" ON "wl_sales_acquisition_sales" (party_id, sale_date);

-- A change of the source or campaign of an acquisition or a won lead
-- (`SAL.ACL.09`), with its reason and approver. The status moves only
-- through the AcquisitionChange machine.
CREATE TABLE "wl_sales_acquisition_changes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- `acquisition` or `lead`.
    kind TEXT NOT NULL DEFAULT 'acquisition',
    acquisition_id INTEGER REFERENCES "wl_sales_party_acquisitions" (id),
    lead_id INTEGER REFERENCES "wl_sales_leads" (id),
    old_source_id INTEGER,
    new_source_id INTEGER NOT NULL,
    old_campaign_id INTEGER,
    new_campaign_id INTEGER,
    reason TEXT NOT NULL,
    requested_by INTEGER NOT NULL,
    decided_by INTEGER,
    decision_reason TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_acquisition_changes_status" ON "wl_sales_acquisition_changes" (status);
