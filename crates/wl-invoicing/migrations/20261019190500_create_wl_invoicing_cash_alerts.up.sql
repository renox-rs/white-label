-- The last cash-flow alert of each legal entity (wl-invoicing §6 `FIN.RUL.23`,
-- issue #93): the first week below the threshold when it went out, to decide
-- whether to warn again. `first_week_start` is NULL after a run with no week
-- below, so a later breach notifies again. The forecast itself is not stored.
CREATE TABLE "wl_invoicing_cash_alerts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL,
    -- The first day of the first week below the threshold.
    first_week_start TEXT,
    -- The projected balance of that week and the threshold, in AED fils.
    projected_balance INTEGER NOT NULL DEFAULT 0,
    threshold INTEGER NOT NULL DEFAULT 0,
    sent_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_cash_alerts_entity"
    ON "wl_invoicing_cash_alerts" (legal_entity_id);
