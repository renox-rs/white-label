-- The invoice a new invoice replaces after an e-invoice correction (wl-invoicing
-- §4 *E-invoice transmission* `supersede`; issue #73): the user names it on
-- the new invoice, and its superseded transmission is linked to the new
-- invoice when that is issued.
ALTER TABLE "wl_invoicing_invoices" ADD COLUMN replaces_invoice_id INTEGER;
