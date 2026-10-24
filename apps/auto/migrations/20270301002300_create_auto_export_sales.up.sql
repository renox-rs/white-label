-- 08 Auto export sales (`AUT.EXP.01`, `AUT.RUL.15`, BL-AUT-14): a zero-rated
-- sale the accountant registered by its invoice with the supply date (the
-- delivery or collection date), the steps of the rule that watches the exit
-- evidence (told at 60 days, flagged *5% VAT due* at 90 days, told that
-- evidence came late), and the exit evidence attached to it: a customs exit
-- certificate, a bill of lading or an airway bill.
CREATE TABLE "auto_export_sales" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_id INTEGER NOT NULL,
    invoice_number TEXT,
    party_id INTEGER NOT NULL,
    currency TEXT NOT NULL,
    gross_total INTEGER NOT NULL DEFAULT 0,
    supply_date TEXT NOT NULL,
    notified_at TEXT,
    vat_flagged_at TEXT,
    late_evidence_notified_at TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_export_sales_invoice_id" ON "auto_export_sales" (invoice_id);
CREATE INDEX "auto_export_sales_party_id" ON "auto_export_sales" (party_id);

CREATE TABLE "auto_export_evidence" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    export_sale_id INTEGER NOT NULL REFERENCES "auto_export_sales" (id),
    kind TEXT NOT NULL,
    reference TEXT NOT NULL,
    attached_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "auto_export_evidence_export_sale_id" ON "auto_export_evidence" (export_sale_id);
