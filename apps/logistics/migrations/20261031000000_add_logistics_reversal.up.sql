-- A delivered shipment whose delivery was reversed (03 Logistics §4.1 row
-- *Recipient denies the delivery, or it was marked by mistake*, §7.1 row
-- `reverse_delivery`, `WRK.ACL.10`, `LOG.ACL.12`; BL-LOG-10).
-- Who is liable and what became of the cash, for the latest reversal.
ALTER TABLE "logistics_shipments" ADD COLUMN liability TEXT;
ALTER TABLE "logistics_shipments" ADD COLUMN cash_treatment TEXT;
-- The COD no longer expected (refunded to the payer), in fils.
ALTER TABLE "logistics_shipments" ADD COLUMN cod_released INTEGER NOT NULL DEFAULT 0;
-- The charge to credit on the invoice it was billed on, in fils.
ALTER TABLE "logistics_shipments" ADD COLUMN credit_due INTEGER NOT NULL DEFAULT 0;
-- Reversed and not delivered again: out of billing.
ALTER TABLE "logistics_shipments" ADD COLUMN reversed INTEGER NOT NULL DEFAULT 0;

CREATE TABLE "logistics_reversals" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    shipment_id INTEGER NOT NULL REFERENCES "logistics_shipments" (id),
    delivery_id INTEGER NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    -- `driver`, `business`, `client` or `none`.
    liability TEXT NOT NULL DEFAULT 'none',
    -- `removed`, `refunded`, `kept` or none.
    cash_treatment TEXT,
    -- The COD the shipment expected when it was reversed, in fils.
    cod_amount INTEGER NOT NULL DEFAULT 0,
    -- The cash recorded as collected, in fils.
    cash_amount INTEGER NOT NULL DEFAULT 0,
    driver_user_id INTEGER,
    approver_id INTEGER,
    -- The state the shipment went back to.
    target_state TEXT NOT NULL DEFAULT 'attempted',
    -- Whether the COD was on a paid remittance.
    remitted INTEGER NOT NULL DEFAULT 0,
    adjustment_line_id INTEGER,
    credit_due INTEGER NOT NULL DEFAULT 0,
    -- What the business bears when it is liable, in fils.
    loss_amount INTEGER NOT NULL DEFAULT 0,
    claim_ticket_id INTEGER,
    -- The operations manager's task: `none`, `open`, `shortage_raised` or
    -- `hr_case`.
    task_status TEXT NOT NULL DEFAULT 'none',
    task_ticket_id INTEGER,
    shortage_handover_id INTEGER,
    hr_ticket_id INTEGER,
    decided_by INTEGER,
    decided_at TEXT,
    settled INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "logistics_reversals_shipment" ON "logistics_reversals" (shipment_id);
CREATE INDEX "logistics_reversals_task" ON "logistics_reversals" (task_status);
