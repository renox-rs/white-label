-- `transfer` (issue #109): the deposit is handed to another holder against the
-- holder's receipt, and the money passed on is an outgoing payment to it.
ALTER TABLE "wl_payments_deposits" ADD COLUMN transferred_to INTEGER;
ALTER TABLE "wl_payments_deposits" ADD COLUMN transfer_receipt TEXT;
ALTER TABLE "wl_payments_deposits" ADD COLUMN transfer_payment_id INTEGER;
