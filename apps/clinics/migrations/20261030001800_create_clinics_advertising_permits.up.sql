-- 05 Clinics, advertising permit (BL-CLN-15, #363, CLN.ACL.13): the regulator
-- permit number and validity recorded for a campaign that promotes services,
-- approved by the medical director. One row per campaign.
CREATE TABLE "clinics_advertising_permits" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    campaign_id INTEGER NOT NULL,
    promotes_services BOOLEAN NOT NULL DEFAULT TRUE,
    permit_number TEXT NOT NULL DEFAULT '',
    valid_to TEXT,
    approved_by INTEGER,
    approved_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "clinics_advertising_permits_campaign_id" ON "clinics_advertising_permits" (campaign_id);
