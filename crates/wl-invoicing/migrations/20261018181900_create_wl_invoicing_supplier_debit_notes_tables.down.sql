DROP TABLE IF EXISTS "wl_invoicing_supplier_debit_note_lines";
DROP TABLE IF EXISTS "wl_invoicing_supplier_debit_notes";
DROP INDEX IF EXISTS "wl_invoicing_supplier_bill_lines_po_line";
ALTER TABLE "wl_invoicing_supplier_bill_lines" DROP COLUMN po_line_id;
ALTER TABLE "wl_invoicing_supplier_bill_lines" DROP COLUMN po_line_type;
