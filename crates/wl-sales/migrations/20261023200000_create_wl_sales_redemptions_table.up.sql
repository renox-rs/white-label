-- A member's redemption of points on a sale (wl-sales §5.9 step 3; BL-SAL-23).
-- It is pending until the member confirms with the code, held while the sale
-- is open, applied when the sale completes (a counter sale) or is confirmed
-- (an order), released when the hold lapses or the sale is abandoned, and
-- refused when the code is wrong three times or could not be sent.
CREATE TABLE "wl_sales_redemptions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES "wl_sales_loyalty_accounts" (id),
    -- The counter sale or sales order the points pay part of.
    sale_type TEXT NOT NULL,
    sale_id INTEGER NOT NULL,
    -- The redemption line on the sale, once confirmed.
    line_id INTEGER,
    points INTEGER NOT NULL,
    -- What the points are worth, in fils (points x the point value).
    value INTEGER NOT NULL DEFAULT 0,
    -- The sale's value with VAT after the other discounts, in fils: the base
    -- of the maximum share and of what a cancelled line or a return gives back.
    basis INTEGER NOT NULL DEFAULT 0,
    -- What was given back so far, in points and in fils of the sale's value.
    returned_points INTEGER NOT NULL DEFAULT 0,
    returned_value INTEGER NOT NULL DEFAULT 0,
    requested_by INTEGER,
    -- `code` or `portal`.
    confirmed_via TEXT,
    -- Held until: points held for an open sale are not redeemable elsewhere
    -- and do not expire. Empty: held until the sale resolves (an order).
    hold_until TEXT,
    -- Why it was refused: `too_many_tries` or `code_not_sent`.
    refusal TEXT,
    applied_at TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_redemptions_account" ON "wl_sales_redemptions" (account_id);
CREATE INDEX "wl_sales_redemptions_sale" ON "wl_sales_redemptions" (sale_type, sale_id);
CREATE INDEX "wl_sales_redemptions_status" ON "wl_sales_redemptions" (status);
-- One live redemption per sale.
CREATE UNIQUE INDEX "wl_sales_redemptions_one_live" ON "wl_sales_redemptions" (sale_type, sale_id) WHERE status IN ('pending', 'held', 'applied');
