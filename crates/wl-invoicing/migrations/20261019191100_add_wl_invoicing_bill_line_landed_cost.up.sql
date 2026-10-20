-- A freight, duty or clearing line coded to a wl-inventory landed cost
-- (#78): the line posts no cost to a cost object on approval; the amount goes
-- to the landed cost with `SupplierBillApproved`.
ALTER TABLE "wl_invoicing_supplier_bill_lines" ADD COLUMN landed_cost_type TEXT;
ALTER TABLE "wl_invoicing_supplier_bill_lines" ADD COLUMN landed_cost_id INTEGER;

CREATE INDEX "wl_invoicing_supplier_bill_lines_landed_cost"
    ON "wl_invoicing_supplier_bill_lines" (landed_cost_type, landed_cost_id);
