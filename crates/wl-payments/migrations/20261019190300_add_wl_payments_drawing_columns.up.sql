-- A payment with purpose `owner_drawings` knows its drawings category, the
-- request it was recorded from and whether the sole owner approved it alone
-- (`FIN.ACL.23`); a cheque knows the approved drawing it pays (issue #119).
ALTER TABLE "wl_payments_payments" ADD COLUMN drawing_category TEXT;
ALTER TABLE "wl_payments_payments" ADD COLUMN drawing_request_id INTEGER;
ALTER TABLE "wl_payments_payments" ADD COLUMN self_approved INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_payments_cheques" ADD COLUMN drawing_request_id INTEGER;
