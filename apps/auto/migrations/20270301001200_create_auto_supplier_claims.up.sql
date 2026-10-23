-- 08 Auto supplier warranty claim (`AUT.WAR.02`, 08 section 4.6 step 4,
-- section 6 *Supplier warranty claim*, section 7.1 *Supplier warranty claim*):
-- the recovery of a failed part from the supplier, raised from a warranty
-- ticket (category `warranty`) that was closed as accepted. It keeps the
-- warranty terms it was judged against (period, mileage, labour included),
-- the facts of the part (bought from the supplier on `purchase_date`, fitted at
-- `fitted_odometer`, failed on `failure_date` at `failure_odometer`), what is
-- claimed, the supplier's answer, and how it ended. The status moves only
-- through the machine.
CREATE TABLE "auto_supplier_claims" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ticket_id INTEGER NOT NULL,
    ticket_number TEXT NOT NULL,
    about_type TEXT,
    about_id INTEGER,
    supplier_party_id INTEGER NOT NULL,
    part_description TEXT NOT NULL,
    item_code TEXT,
    brand TEXT,
    quantity_milli INTEGER NOT NULL DEFAULT 1000,
    part_cost INTEGER NOT NULL DEFAULT 0,
    labour_included BOOLEAN NOT NULL DEFAULT FALSE,
    labour_amount INTEGER NOT NULL DEFAULT 0,
    claim_amount INTEGER NOT NULL DEFAULT 0,
    purchase_date TEXT NOT NULL,
    warranty_months INTEGER NOT NULL,
    warranty_km INTEGER,
    fitted_job_card_id INTEGER,
    replacement_job_card_id INTEGER,
    fitted_odometer INTEGER,
    failure_date TEXT NOT NULL,
    failure_odometer INTEGER,
    part_kept BOOLEAN NOT NULL DEFAULT TRUE,
    supplier_reference TEXT,
    claim_document TEXT,
    supplier_return_id INTEGER,
    supplier_answer TEXT NOT NULL,
    supplier_answer_note TEXT,
    answered_at TEXT,
    raised_at TEXT,
    recovered_at TEXT,
    recovery_basis TEXT,
    recovery_reference TEXT,
    written_off_at TEXT,
    write_off_reason TEXT,
    write_off_approved_by INTEGER,
    status TEXT NOT NULL,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_supplier_claims_ticket_id" ON "auto_supplier_claims" (ticket_id);
CREATE INDEX "auto_supplier_claims_supplier_party_id" ON "auto_supplier_claims" (supplier_party_id);
CREATE INDEX "auto_supplier_claims_supplier_return_id" ON "auto_supplier_claims" (supplier_return_id);
CREATE INDEX "auto_supplier_claims_status" ON "auto_supplier_claims" (status);
