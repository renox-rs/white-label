-- The text of the uploaded statement file, read by `parse` (wl-payments §4
-- *Bank statement import*; issue #118).
ALTER TABLE "wl_payments_bank_statement_imports" ADD COLUMN content TEXT;
