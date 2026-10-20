-- The Subscription (wl-invoicing §3, §4 *Subscription*; base of BL-INV-19,
-- BL-INV-20, BL-INV-21): a recurring plan billed every cycle, its pause
-- causes and its cycles with their cycle invoices. The status moves only
-- through the machine.
CREATE TABLE "wl_invoicing_subscriptions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    legal_entity_id INTEGER,
    -- The vertical's agreement it bills, e.g. `accounting.engagement`.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    plan TEXT NOT NULL,
    -- The price of one unit for one cycle, in fils, without VAT.
    unit_price INTEGER NOT NULL,
    vat_category TEXT NOT NULL DEFAULT 'standard',
    quantity_milli INTEGER NOT NULL DEFAULT 1000,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- week, month, quarter or year.
    cycle TEXT NOT NULL DEFAULT 'month',
    anchor_day INTEGER NOT NULL DEFAULT 1,
    start_date TEXT NOT NULL,
    end_date TEXT,
    notice_days INTEGER NOT NULL DEFAULT 0,
    -- manual or automatic.
    payment_mode TEXT NOT NULL DEFAULT 'manual',
    status TEXT NOT NULL DEFAULT 'active',
    next_cycle_date TEXT NOT NULL,
    -- `FIN.RUL.06`: the day it went past due and the retries done since.
    past_due_on TEXT,
    retries_done INTEGER NOT NULL DEFAULT 0,
    paused_from TEXT,
    pause_reason TEXT,
    -- The user pause days taken in `pause_year` (the allowance).
    pause_days_used INTEGER NOT NULL DEFAULT 0,
    pause_year INTEGER,
    -- A user or party cancel takes effect on this day, after its notice.
    cancel_on TEXT,
    cancelled_on TEXT,
    cancel_reason TEXT,
    final_credit_note_id INTEGER,
    final_invoice_id INTEGER,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_subscriptions_status" ON "wl_invoicing_subscriptions" (status);
CREATE INDEX "wl_invoicing_subscriptions_party_id" ON "wl_invoicing_subscriptions" (party_id);
CREATE INDEX "wl_invoicing_subscriptions_source"
    ON "wl_invoicing_subscriptions" (source_type, source_id);

-- Each cause of a pause: a user pause (with the day it ends), the
-- agreement's pause or a party block (`on_hold`). Open while `ended_on` is
-- empty; at most one open cause of each kind per subscription.
CREATE TABLE "wl_invoicing_subscription_pauses" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subscription_id INTEGER NOT NULL REFERENCES "wl_invoicing_subscriptions" (id) ON DELETE CASCADE,
    -- user, agreement or party_block.
    cause TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    started_on TEXT NOT NULL,
    until TEXT,
    ended_on TEXT,
    by_user_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_subscription_pauses_subscription_id"
    ON "wl_invoicing_subscription_pauses" (subscription_id);
CREATE UNIQUE INDEX "wl_invoicing_subscription_pauses_open"
    ON "wl_invoicing_subscription_pauses" (subscription_id, cause) WHERE ended_on IS NULL;

-- One billed cycle and its cycle invoice (one per cycle start).
CREATE TABLE "wl_invoicing_subscription_cycles" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subscription_id INTEGER NOT NULL REFERENCES "wl_invoicing_subscriptions" (id) ON DELETE CASCADE,
    period_start TEXT NOT NULL,
    period_end TEXT NOT NULL,
    invoice_id INTEGER,
    -- A final pro rata invoice of a cancel.
    is_final BOOLEAN NOT NULL DEFAULT FALSE,
    -- Reopened by a reversed payment: outstanding until paid again.
    reopened BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_subscription_cycles_period"
    ON "wl_invoicing_subscription_cycles" (subscription_id, period_start);
CREATE INDEX "wl_invoicing_subscription_cycles_invoice_id"
    ON "wl_invoicing_subscription_cycles" (invoice_id);
