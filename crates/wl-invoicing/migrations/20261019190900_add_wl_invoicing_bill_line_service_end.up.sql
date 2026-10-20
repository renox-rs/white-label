-- A supplier bill line may charge for a period (wl-invoicing §5.4 step 3,
-- issue #87): `service_date` is the first day of the work and
-- `service_end_date` its last. A period line counts against a contract's end
-- by its last day. Null: a single day, `service_date`.
ALTER TABLE "wl_invoicing_supplier_bill_lines" ADD COLUMN service_end_date TEXT;
