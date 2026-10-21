-- How a payer bill ended (#81): written off, moved to the customer or voided,
-- the amount that moved, and the tax documents the close made (so a retry
-- after a failed document makes nothing twice).
ALTER TABLE "wl_invoicing_payer_bills" ADD COLUMN close_outcome TEXT;
ALTER TABLE "wl_invoicing_payer_bills" ADD COLUMN close_amount INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_invoicing_payer_bills" ADD COLUMN close_credit_note_id INTEGER;
ALTER TABLE "wl_invoicing_payer_bills" ADD COLUMN close_invoice_id INTEGER;
-- The lines and amounts that moved at the close (JSON), to document just
-- them when the bill was reopened and closed again.
ALTER TABLE "wl_invoicing_payer_bills" ADD COLUMN close_lines TEXT NOT NULL DEFAULT '[]';
