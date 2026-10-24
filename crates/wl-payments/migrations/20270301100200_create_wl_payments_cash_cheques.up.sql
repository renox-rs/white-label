-- Cheques collected in the field and expected in a cash handover (wl-payments
-- §4 *Cash handover* `count`, `recount`, `FIN.ACL.20`, issue #111): the counter
-- ticks each expected cheque present or missing. A present cheque gets its
-- custody confirmed (the cheque row names the handover) and can be deposited;
-- a missing one stays `missing` until a recount finds it (`found`) or it is
-- returned with the reason "lost" (`returned_lost`).
CREATE TABLE "wl_payments_cash_cheques" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    handover_id INTEGER NOT NULL REFERENCES "wl_payments_cash_handovers" (id) ON DELETE CASCADE,
    cheque_id INTEGER NOT NULL REFERENCES "wl_payments_cheques" (id),
    -- `present`, `missing`, `found`, `returned_lost`.
    state TEXT NOT NULL,
    ticked_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_payments_cash_cheques_cheque" ON "wl_payments_cash_cheques" (handover_id, cheque_id);
CREATE INDEX "wl_payments_cash_cheques_handover" ON "wl_payments_cash_cheques" (handover_id);
