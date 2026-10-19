-- Leads (wl-sales §3, §4.1; issue #139) and their notes. The status moves only
-- through the Lead machine.
CREATE TABLE "wl_sales_leads" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The party, once there is one (it may be a new, unverified party).
    party_id INTEGER,
    -- Who the lead is, while there is no party.
    contact_name TEXT NOT NULL DEFAULT '',
    source_id INTEGER NOT NULL REFERENCES "wl_sales_lead_sources" (id),
    campaign_id INTEGER REFERENCES "wl_sales_campaigns" (id),
    -- The campaign's branch, else the web form's, else the owner's; the
    -- entity is the lead's entity in reports.
    legal_entity_id INTEGER,
    scope_type TEXT,
    scope_id TEXT,
    interest TEXT NOT NULL DEFAULT '',
    owner_id INTEGER,
    next_action_date TEXT,
    -- A code of the lost reasons: `duplicate` and `spam_or_invalid` are left
    -- out of every lead count.
    lost_reason TEXT,
    -- The vertical's conversion record, once won.
    converted_type TEXT,
    converted_id INTEGER,
    won_at TEXT,
    status TEXT NOT NULL DEFAULT 'new',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_leads_party" ON "wl_sales_leads" (party_id, status);
CREATE INDEX "wl_sales_leads_campaign" ON "wl_sales_leads" (campaign_id, status);
CREATE INDEX "wl_sales_leads_source" ON "wl_sales_leads" (source_id);
CREATE INDEX "wl_sales_leads_converted" ON "wl_sales_leads" (converted_type, converted_id);

-- What was said or entered on a lead: the contact note, and a web form
-- submission for a party that already had an open lead on the campaign.
CREATE TABLE "wl_sales_lead_notes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    lead_id INTEGER NOT NULL REFERENCES "wl_sales_leads" (id) ON DELETE CASCADE,
    -- `contact`, `web_form` or `staff`.
    kind TEXT NOT NULL DEFAULT 'staff',
    note TEXT NOT NULL DEFAULT '',
    user_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_lead_notes_lead" ON "wl_sales_lead_notes" (lead_id);
