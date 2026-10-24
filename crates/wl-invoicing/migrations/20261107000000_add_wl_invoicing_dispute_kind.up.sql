-- The kind of a dispute a vertical's form asks for (quantity, price, quality, other); a vertical reads it from the InvoiceDisputed event (07 Manufacturing links a quality dispute to an NCR).
ALTER TABLE "wl_invoicing_disputes" ADD COLUMN kind TEXT;
