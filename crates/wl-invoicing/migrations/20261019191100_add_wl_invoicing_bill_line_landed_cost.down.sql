DROP INDEX IF EXISTS "wl_invoicing_supplier_bill_lines_landed_cost";
ALTER TABLE "wl_invoicing_supplier_bill_lines" DROP COLUMN landed_cost_id;
ALTER TABLE "wl_invoicing_supplier_bill_lines" DROP COLUMN landed_cost_type;
