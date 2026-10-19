DROP TABLE IF EXISTS "wl_invoicing_budget_warnings";
DROP INDEX IF EXISTS "wl_invoicing_supplier_bill_lines_cost_code";
DROP INDEX IF EXISTS "wl_invoicing_invoice_lines_charge_type";
ALTER TABLE "wl_invoicing_supplier_bill_lines" DROP COLUMN cost_code;
ALTER TABLE "wl_invoicing_invoice_lines" DROP COLUMN charge_type;
