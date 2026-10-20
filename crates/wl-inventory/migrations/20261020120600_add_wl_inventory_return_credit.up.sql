-- Crediting or cancelling a Return to supplier (wl-inventory §4 *Return to
-- supplier* `credit`, `cancel`).
-- Why and by whom it was cancelled, and what became of the rejected goods
-- kept for collection (`received_again` or `discarded`).
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN cancel_reason TEXT;
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN cancelled_by INTEGER;
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN cancelled_at TEXT;
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN kept_goods TEXT;
-- How it was credited (`debit_note`, `refund_received`, `supplier_refused`),
-- why, by whom and when; the debit note that completed the cover; the value
-- posted as wastage when the supplier refused.
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN credit_basis TEXT;
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN credit_reason TEXT;
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN credited_by INTEGER;
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN credited_at TEXT;
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN debit_note_id INTEGER;
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN wastage_fils INTEGER NOT NULL DEFAULT 0;

-- What a debit note (credit line) or a replacement receipt (replace line)
-- has covered of the line, in thousandths.
ALTER TABLE "wl_inventory_supplier_return_lines" ADD COLUMN covered_milli INTEGER NOT NULL DEFAULT 0;
