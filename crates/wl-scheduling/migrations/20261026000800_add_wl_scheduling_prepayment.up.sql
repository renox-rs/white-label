-- BL-SCH-10 (SCH.RUL.04, wl-scheduling §3 and §5.2): the page where the party
-- pays the prepayment of a self-booking, and the refunds asked of the payments
-- that arrived after the hold ended (one per payment).
ALTER TABLE "wl_scheduling_bookings" ADD COLUMN prepayment_path TEXT;

CREATE TABLE "wl_scheduling_prepayment_refunds" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    booking_id INTEGER NOT NULL REFERENCES "wl_scheduling_bookings" (id),
    -- The payment (wl-payments) that arrived after the booking was released.
    payment_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    -- The refund asked of wl-payments; empty while the request is on its way.
    refund_id INTEGER,
    created_at TEXT
);

CREATE INDEX "wl_scheduling_prepayment_refunds_booking" ON "wl_scheduling_prepayment_refunds" (booking_id);
CREATE UNIQUE INDEX "wl_scheduling_prepayment_refunds_payment" ON "wl_scheduling_prepayment_refunds" (payment_id);
