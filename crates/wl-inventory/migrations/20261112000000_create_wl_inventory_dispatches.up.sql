-- Dispatches (wl-inventory §3, §4 *Dispatch*, issues #240 to #244): goods
-- picked, packed and sent to a recipient for a source such as a sales order.
CREATE TABLE "wl_inventory_dispatches" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT UNIQUE,
    -- The record the dispatch fulfils (a sales order).
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    recipient_party_id INTEGER,
    recipient_name TEXT NOT NULL DEFAULT '',
    address TEXT NOT NULL DEFAULT '',
    -- Where the goods leave from, and the scope of that location: who may
    -- pick and dispatch.
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    scope_type TEXT,
    scope_id INTEGER,
    -- `draft`, `picked`, `dispatched`, `delivered`, `returned`, `discarded`,
    -- `lost`, `cancelled`.
    status TEXT NOT NULL DEFAULT 'draft',
    -- `own_delivery`, `courier` or `collection`.
    handover TEXT,
    tracking_number TEXT,
    collector_signature TEXT,
    -- The delivery wl-delivery was asked for (own delivery).
    delivery_id INTEGER,
    -- The delivery note, a PDF document kept as text.
    delivery_note TEXT,
    picked_by INTEGER,
    picked_at TEXT,
    dispatched_by INTEGER,
    dispatched_at TEXT,
    delivered_at TEXT,
    -- The proof of delivery that confirmed it (one confirmation per proof),
    -- and whether a delivery reversal marked it reversed.
    proof_id INTEGER,
    proof_reversed INTEGER NOT NULL DEFAULT 0,
    -- The status the dispatch had when it was marked lost.
    lost_from TEXT,
    -- The value held as a loss while the claim runs, how many times it was
    -- held, and whether the loss is open now.
    loss_fils INTEGER NOT NULL DEFAULT 0,
    loss_count INTEGER NOT NULL DEFAULT 0,
    loss_open INTEGER NOT NULL DEFAULT 0,
    cancel_reason TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_dispatches_status" ON "wl_inventory_dispatches" (status);
CREATE INDEX "wl_inventory_dispatches_source"
    ON "wl_inventory_dispatches" (source_type, source_id);

CREATE TABLE "wl_inventory_dispatch_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dispatch_id INTEGER NOT NULL REFERENCES "wl_inventory_dispatches" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    lot_id INTEGER REFERENCES "wl_inventory_lots" (id),
    -- Thousandths of the base unit; always above 0.
    quantity_milli INTEGER NOT NULL,
    -- The cost of one base unit when the goods left stock, in fils.
    unit_cost INTEGER NOT NULL DEFAULT 0,
    -- The line of the source this line fulfils.
    source_line_type TEXT,
    source_line_id INTEGER,
    -- From the proof of delivery.
    delivered_milli INTEGER NOT NULL DEFAULT 0,
    refused_milli INTEGER NOT NULL DEFAULT 0,
    refusal_reason TEXT,
    -- Of the refused quantity: received back into quarantine, and reported
    -- missing on return. The rest is still *in return*.
    returned_milli INTEGER NOT NULL DEFAULT 0,
    missing_milli INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_dispatch_lines_position"
    ON "wl_inventory_dispatch_lines" (dispatch_id, position);
