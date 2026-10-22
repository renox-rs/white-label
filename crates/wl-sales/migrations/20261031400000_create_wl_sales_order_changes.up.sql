-- Changes to a placed sales order (BL-SAL-05, BL-SAL-06, BL-SAL-08, BL-SAL-09;
-- base for BL-SAL-07): the kept revisions, a log of every change that was
-- made (read after the commit to emit the events and settle the money), the
-- money steps a change led to, and the line fields an outcome needs.

-- An earlier revision of an order, kept read-only: what the order was before
-- the amendment that replaced it.
CREATE TABLE "wl_sales_order_revisions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES "wl_sales_sales_orders" (id) ON DELETE CASCADE,
    -- The revision number this row keeps.
    revision INTEGER NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    net_total INTEGER NOT NULL DEFAULT 0,
    vat_total INTEGER NOT NULL DEFAULT 0,
    gross_total INTEGER NOT NULL DEFAULT 0,
    -- JSON: the lines of the revision.
    lines TEXT NOT NULL DEFAULT '[]',
    amended_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_order_revisions_revision" ON "wl_sales_order_revisions" (order_id, revision);

-- One change of an order: an amendment, a cancelled line, the remaining lines
-- cancelled (and, with BL-SAL-07, the cancellation). The row is written in the
-- transaction of the transition; `announced_at` is set once the events were
-- emitted and the money settled, so an approved change (run by the kernel)
-- and a direct one are announced the same way.
CREATE TABLE "wl_sales_order_changes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES "wl_sales_sales_orders" (id) ON DELETE CASCADE,
    -- `amend`, `cancel_line`, `cancel_remaining` or `cancel`.
    kind TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    changed_by INTEGER,
    -- JSON: what the kind needs to announce and settle it.
    detail TEXT NOT NULL DEFAULT '{}',
    announced_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_order_changes_order" ON "wl_sales_order_changes" (order_id, announced_at);

-- A money step of a change: a payment link for an increase, a refund, a
-- credit note, an extra invoice, a fee document or an updated draft invoice.
CREATE TABLE "wl_sales_order_settlements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES "wl_sales_sales_orders" (id) ON DELETE CASCADE,
    change_id INTEGER,
    -- The revision in force when the step was made.
    revision INTEGER NOT NULL DEFAULT 1,
    -- `link`, `refund`, `credit_note`, `extra_invoice`, `fee_invoice` or `draft_updated`.
    kind TEXT NOT NULL,
    amount INTEGER NOT NULL DEFAULT 0,
    -- `open`, `paid`, `expired`, `resent`, `reverted` (a link); `requested`,
    -- `done` or `failed` for the other kinds.
    status TEXT NOT NULL DEFAULT 'done',
    -- The link, refund, credit note or invoice in the module that owns it.
    reference_id INTEGER,
    -- The public path of a payment link.
    link_path TEXT,
    due_at TEXT,
    note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_order_settlements_order" ON "wl_sales_order_settlements" (order_id, kind, status);
CREATE INDEX "wl_sales_order_settlements_due" ON "wl_sales_order_settlements" (status, due_at);

-- The state an order had before a credit hold (BL-SAL-10 releases to it).
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN held_from TEXT;

-- Quantity added by an amendment that waits for its payment link to be paid:
-- reserved but not handed over (BL-SAL-06).
ALTER TABLE "wl_sales_sale_lines" ADD COLUMN held_milli INTEGER NOT NULL DEFAULT 0;
-- The outcome, fee and reason of the cancelled quantity of a started line
-- (BL-SAL-07, BL-SAL-08, BL-SAL-09).
ALTER TABLE "wl_sales_sale_lines" ADD COLUMN cancel_outcome TEXT;
ALTER TABLE "wl_sales_sale_lines" ADD COLUMN cancel_fee INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_sales_sale_lines" ADD COLUMN fee_waived BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "wl_sales_sale_lines" ADD COLUMN cancel_reason TEXT;
