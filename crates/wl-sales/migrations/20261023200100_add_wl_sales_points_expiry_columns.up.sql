-- Points expiry (SAL.RUL.08; BL-SAL-24): the day a lot's member was reminded
-- (one reminder per lot), and when the reminder of an account gave up (the
-- account shows "reminder not delivered"; the lot still expires).
ALTER TABLE "wl_sales_points_movements" ADD COLUMN reminded_on TEXT;
ALTER TABLE "wl_sales_loyalty_accounts" ADD COLUMN expiry_reminder_failed_at TEXT;
