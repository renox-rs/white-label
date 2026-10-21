-- A delivery under review (wl-delivery §4.1 `open_review`, `close_review`,
-- #160): the frozen flag holds its cash on delivery out of remittances and
-- settlements and its billing; the review ticket holds the conversation.
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN frozen INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN review_ticket_id INTEGER;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN review_reason TEXT;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN review_note TEXT;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN reviewed_at TEXT;

-- What the dispatcher and the driver did and saw while the goods were
-- traced (scans, notes, depot receipts); the untraced clock of `WRK.RUL.17`
-- reads the latest of them (#162). `open_review` and `dispatch` leave one.
CREATE TABLE "wl_delivery_traces" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    delivery_id INTEGER NOT NULL REFERENCES "wl_delivery_deliveries" (id) ON DELETE CASCADE,
    -- `dispatch`, `review`, `scan`, `note`, `depot_receipt`, `found`.
    kind TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    depot TEXT NOT NULL DEFAULT '',
    occurred_at TEXT NOT NULL,
    actor_user_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_delivery_traces_delivery" ON "wl_delivery_traces" (delivery_id, occurred_at);
