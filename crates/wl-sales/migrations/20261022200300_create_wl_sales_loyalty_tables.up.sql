-- The loyalty programme, accounts, points movements and earnings (wl-sales
-- §3 *Loyalty programme*, *Loyalty account*, *Points movement*, §4.7, §5.9;
-- BL-SAL-22). Balances move only with a points movement.
CREATE TABLE "wl_sales_loyalty_programmes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    -- One programme per brand at most; empty is the client's own.
    brand TEXT,
    -- `per_aed` (points per AED spent) or `per_visit` (visits per reward).
    earn_kind TEXT NOT NULL DEFAULT 'per_aed',
    earn_rate INTEGER NOT NULL DEFAULT 1,
    -- The least eligible amount of a sale before VAT, in fils.
    minimum_amount INTEGER NOT NULL DEFAULT 0,
    -- Item codes that never earn, separated by commas.
    excluded_items TEXT NOT NULL DEFAULT '',
    -- What one point is worth, in fils.
    point_value INTEGER NOT NULL DEFAULT 1,
    min_redeem_points INTEGER NOT NULL DEFAULT 500,
    max_share_bps INTEGER NOT NULL DEFAULT 10000,
    visit_reward TEXT NOT NULL DEFAULT '',
    validity_months INTEGER NOT NULL DEFAULT 12,
    terms_version TEXT NOT NULL DEFAULT '1',
    terms_text_en TEXT NOT NULL DEFAULT '',
    terms_text_ar TEXT NOT NULL DEFAULT '',
    effective_from TEXT,
    closing_date TEXT,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_loyalty_programmes_active" ON "wl_sales_loyalty_programmes" (active);

CREATE TABLE "wl_sales_loyalty_accounts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    programme_id INTEGER NOT NULL REFERENCES "wl_sales_loyalty_programmes" (id),
    party_id INTEGER NOT NULL,
    -- The member's code, read out or scanned at the counter.
    code TEXT NOT NULL,
    -- The guardian who accepted the terms for a minor.
    accepted_by_party_id INTEGER,
    -- Digits, with a leading `+` when given; the codes go to it.
    mobile TEXT NOT NULL DEFAULT '',
    enrolled_at TEXT,
    -- `counter`, `portal` or `public`.
    enrolled_via TEXT NOT NULL DEFAULT 'counter',
    enrolled_by_user_id INTEGER,
    terms_version TEXT NOT NULL DEFAULT '1',
    points_balance INTEGER NOT NULL DEFAULT 0,
    points_held INTEGER NOT NULL DEFAULT 0,
    visits INTEGER NOT NULL DEFAULT 0,
    reward_available_until TEXT,
    paused BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'active',
    closed_reason TEXT,
    closed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_loyalty_accounts_code" ON "wl_sales_loyalty_accounts" (code);
CREATE INDEX "wl_sales_loyalty_accounts_party" ON "wl_sales_loyalty_accounts" (programme_id, party_id);
CREATE INDEX "wl_sales_loyalty_accounts_mobile" ON "wl_sales_loyalty_accounts" (programme_id, mobile);
-- One open account per programme and party, one per programme and mobile
-- number (a closed account never blocks a new enrolment).
CREATE UNIQUE INDEX "wl_sales_loyalty_accounts_one_party" ON "wl_sales_loyalty_accounts" (programme_id, party_id) WHERE status = 'active';
CREATE UNIQUE INDEX "wl_sales_loyalty_accounts_one_mobile" ON "wl_sales_loyalty_accounts" (programme_id, mobile) WHERE status = 'active' AND mobile <> '';

-- A sale that earned: the key of "each sale earns once".
CREATE TABLE "wl_sales_loyalty_earnings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES "wl_sales_loyalty_accounts" (id),
    -- The counter sale, the sales order, or the invoice.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- What was counted before VAT, and what the member paid or owes with
    -- VAT (the base of a payment reversal), in fils.
    eligible_net INTEGER NOT NULL DEFAULT 0,
    basis_gross INTEGER NOT NULL DEFAULT 0,
    -- Points earned, or 1 for a visit.
    points INTEGER NOT NULL DEFAULT 0,
    earn_kind TEXT NOT NULL DEFAULT 'per_aed',
    -- Taken back so far, in points (or visits).
    reversed_points INTEGER NOT NULL DEFAULT 0,
    -- The earn movement that holds the lot (the newest after an earn again).
    movement_id INTEGER,
    reward_made BOOLEAN NOT NULL DEFAULT FALSE,
    earned_on TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_loyalty_earnings_source" ON "wl_sales_loyalty_earnings" (source_type, source_id);
CREATE INDEX "wl_sales_loyalty_earnings_account" ON "wl_sales_loyalty_earnings" (account_id, earned_on);

-- A change to a loyalty account.
CREATE TABLE "wl_sales_points_movements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES "wl_sales_loyalty_accounts" (id),
    -- `earn`, `redeem`, `reverse_earn`, `reverse_redeem`, `expire`, `adjust`
    -- or `transfer`.
    kind TEXT NOT NULL,
    -- Points (or visits), signed: positive adds to the balance.
    points INTEGER NOT NULL,
    -- Their value in fils.
    value INTEGER NOT NULL DEFAULT 0,
    legal_entity_id INTEGER,
    -- The record that caused it: a sale, a sales return, a credit note, a
    -- payment, a write-off.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    earning_id INTEGER,
    -- Why: `earn`, `return`, `credit_note`, `payment_reversed`,
    -- `written_off`, `fulfilment_reversed`...
    cause TEXT NOT NULL DEFAULT 'earn',
    -- Earn lots: the day they expire, what was used and what was taken back.
    lot_expires_on TEXT,
    consumed INTEGER NOT NULL DEFAULT 0,
    reversed INTEGER NOT NULL DEFAULT 0,
    -- Reversals and expiries: the earn movement they act on.
    lot_id INTEGER,
    -- A reversal of a payment cause that was earned again.
    reearned BOOLEAN NOT NULL DEFAULT FALSE,
    reason TEXT,
    approval_request_id INTEGER,
    user_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_points_movements_account" ON "wl_sales_points_movements" (account_id);
CREATE INDEX "wl_sales_points_movements_source" ON "wl_sales_points_movements" (source_type, source_id, kind);
CREATE INDEX "wl_sales_points_movements_earning" ON "wl_sales_points_movements" (earning_id);

-- An account that earned on more sales in a day than the limit (SAL.ACL.08),
-- listed for the manager.
CREATE TABLE "wl_sales_loyalty_reviews" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES "wl_sales_loyalty_accounts" (id),
    on_date TEXT NOT NULL,
    sales_count INTEGER NOT NULL DEFAULT 0,
    resolved_at TEXT,
    resolved_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_loyalty_reviews_day" ON "wl_sales_loyalty_reviews" (account_id, on_date);
