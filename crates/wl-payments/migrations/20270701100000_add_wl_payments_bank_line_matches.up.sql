-- The match engine of the bank reconciliation (wl-payments §5.7, BL-PAY-29):
-- a line's suggestion (what, for which party, with which targets, why and how
-- sure) and its confirmed match (the party and the targets the payment is
-- recorded for at `complete`, who confirmed it). Targets are JSON lists of
-- [id, amount] pairs: invoices with the amount to allocate, or the payment or
-- cheque the line is.
ALTER TABLE "wl_payments_bank_statement_lines" ADD COLUMN suggestion_kind TEXT;
ALTER TABLE "wl_payments_bank_statement_lines" ADD COLUMN suggestion_party_id INTEGER;
ALTER TABLE "wl_payments_bank_statement_lines" ADD COLUMN suggestion_targets TEXT;
ALTER TABLE "wl_payments_bank_statement_lines" ADD COLUMN suggestion_reason TEXT;
ALTER TABLE "wl_payments_bank_statement_lines" ADD COLUMN suggestion_score INTEGER;
ALTER TABLE "wl_payments_bank_statement_lines" ADD COLUMN match_party_id INTEGER;
ALTER TABLE "wl_payments_bank_statement_lines" ADD COLUMN match_targets TEXT;
ALTER TABLE "wl_payments_bank_statement_lines" ADD COLUMN confirmed_by INTEGER;
