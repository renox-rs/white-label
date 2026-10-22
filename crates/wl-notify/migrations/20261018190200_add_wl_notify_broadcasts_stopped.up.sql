-- Who stopped a broadcast and when (NTF.RUL.03, BL-NTF-09). A stopped
-- broadcast keeps its state; its unsent messages are suppressed.
ALTER TABLE "wl_notify_broadcasts" ADD COLUMN stopped_by INTEGER;
ALTER TABLE "wl_notify_broadcasts" ADD COLUMN stopped_at TEXT;
