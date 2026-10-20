-- A contract bill with a line for work after the contract's end date is held
-- in `exception` (wl-invoicing §4 `submit`, `FIN.ACL.03`; issue #87): the flag
-- says so, so the creator cannot approve it, and the exception row names the
-- late lines in `detail`.
ALTER TABLE "wl_invoicing_supplier_bills" ADD COLUMN after_end_hold INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_invoicing_supplier_bill_exceptions" ADD COLUMN detail TEXT;
