-- Handing open stops over to another driver's run and aborting a run
-- (wl-delivery §3 *Run*, §4.2 `hand_over`, `abort`, #165, #164): the
-- hand-over record the dispatcher proposes and the receiving driver
-- confirms, the stops it names, and who aborted a run and why.
ALTER TABLE "wl_delivery_runs" ADD COLUMN abort_note TEXT;
ALTER TABLE "wl_delivery_runs" ADD COLUMN aborted_by INTEGER;
ALTER TABLE "wl_delivery_runs" ADD COLUMN aborted_at TEXT;

-- One hand-over from the giving run to the receiving run. It waits
-- (`confirmed_at` empty) until the receiving driver has confirmed each parcel
-- and counted the cash; only then does the giving run's `hand_over` run.
CREATE TABLE "wl_delivery_hand_overs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES "wl_delivery_runs" (id) ON DELETE CASCADE,
    to_run_id INTEGER NOT NULL REFERENCES "wl_delivery_runs" (id) ON DELETE CASCADE,
    -- `breakdown`, `accident`, `driver_unwell` or `load_balancing`.
    reason TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    requested_by INTEGER NOT NULL,
    requested_at TEXT NOT NULL,
    -- The receiving driver's confirmation, the cash counted (fils), the
    -- cheques confirmed (references, comma separated) and where.
    confirmed_by INTEGER,
    confirmed_at TEXT,
    cash_counted INTEGER,
    cheques TEXT NOT NULL DEFAULT '',
    latitude_micro INTEGER,
    longitude_micro INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_delivery_hand_overs_run" ON "wl_delivery_hand_overs" (run_id);
CREATE INDEX "wl_delivery_hand_overs_to_run" ON "wl_delivery_hand_overs" (to_run_id);

-- The stops a hand-over names. `moved` is set at the confirmation for the
-- stops that were still open then; a stop delivered meanwhile is skipped.
CREATE TABLE "wl_delivery_hand_over_stops" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    hand_over_id INTEGER NOT NULL REFERENCES "wl_delivery_hand_overs" (id) ON DELETE CASCADE,
    delivery_id INTEGER NOT NULL REFERENCES "wl_delivery_deliveries" (id) ON DELETE CASCADE,
    moved INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_delivery_hand_over_stops_stop"
    ON "wl_delivery_hand_over_stops" (hand_over_id, delivery_id);
