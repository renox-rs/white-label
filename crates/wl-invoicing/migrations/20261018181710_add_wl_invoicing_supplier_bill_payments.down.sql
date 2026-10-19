DROP TABLE IF EXISTS "wl_invoicing_bill_unallocations";
DROP TABLE IF EXISTS "wl_invoicing_supplier_bill_allocations";
ALTER TABLE "wl_invoicing_supplier_bills" DROP COLUMN balance;
ALTER TABLE "wl_invoicing_supplier_bills" DROP COLUMN allocated_total;
