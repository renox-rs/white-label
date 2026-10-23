-- The F&B brands, outlets, menu items with their recipes, the fulfilment
-- order that follows a sales order through the kitchen and out of the door,
-- and the kitchen tickets (06 F&B §6, §7.1 *Order* and *Kitchen ticket*;
-- base for BL-FNB-01 to BL-FNB-03). Money is fils.
CREATE TABLE "fnb_brands" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    -- The brand's delivery promise in minutes (BL-DLV-10).
    promise_minutes INTEGER NOT NULL DEFAULT 45,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_brands_code" ON "fnb_brands" (code);

CREATE TABLE "fnb_outlets" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    -- The wl-inventory location the kitchen takes its ingredients from.
    location_id INTEGER,
    legal_entity_id INTEGER,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_outlets_code" ON "fnb_outlets" (code);

CREATE TABLE "fnb_menu_items" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The item code of the sale lines.
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    brand_id INTEGER,
    -- The price of one unit before VAT.
    price INTEGER NOT NULL DEFAULT 0,
    -- What one unit costs the kitchen (its recipe at cost): reported as the
    -- wastage value of a line whose consumption was posted.
    unit_cost INTEGER NOT NULL DEFAULT 0,
    available INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_menu_items_code" ON "fnb_menu_items" (code);

CREATE TABLE "fnb_recipe_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    menu_item_id INTEGER NOT NULL REFERENCES "fnb_menu_items" (id),
    -- The stock item (wl-inventory) one unit of the menu item consumes.
    item_id INTEGER NOT NULL,
    per_unit_milli INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_recipe_lines_menu_item" ON "fnb_recipe_lines" (menu_item_id);

CREATE TABLE "fnb_orders" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The number of the sales order (`SO-2026-0001`).
    number TEXT NOT NULL,
    sales_order_id INTEGER NOT NULL,
    party_id INTEGER NOT NULL,
    brand_id INTEGER,
    outlet_id INTEGER,
    channel TEXT NOT NULL DEFAULT '',
    -- `delivery` (own rider) or `pickup`.
    kind TEXT NOT NULL DEFAULT 'delivery',
    payment_mode TEXT NOT NULL DEFAULT '',
    -- What the customer paid before the order was placed, in fils.
    paid_amount INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0,
    recipient_name TEXT NOT NULL DEFAULT '',
    recipient_phone TEXT NOT NULL DEFAULT '',
    address TEXT NOT NULL DEFAULT '',
    area TEXT NOT NULL DEFAULT '',
    -- The wl-delivery delivery of an order that goes out.
    delivery_id INTEGER,
    -- The run the delivery is on and the rider driving it (`DeliveryAssigned`).
    run_id INTEGER,
    rider_user_id INTEGER,
    placed_at TEXT,
    accepted_at TEXT,
    completed_at TEXT,
    -- Whether the kitchen's consumption of the recipe was posted.
    consumption_posted INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'placed',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_orders_sales_order" ON "fnb_orders" (sales_order_id);
CREATE INDEX "fnb_orders_status" ON "fnb_orders" (status);
CREATE INDEX "fnb_orders_party" ON "fnb_orders" (party_id);
CREATE INDEX "fnb_orders_delivery" ON "fnb_orders" (delivery_id);

CREATE TABLE "fnb_tickets" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES "fnb_orders" (id),
    -- The sale line (wl-sales) the ticket cooks.
    sale_line_id INTEGER NOT NULL,
    station TEXT NOT NULL DEFAULT 'pass',
    description TEXT NOT NULL DEFAULT '',
    quantity_milli INTEGER NOT NULL DEFAULT 1000,
    started_at TEXT,
    bumped_at TEXT,
    status TEXT NOT NULL DEFAULT 'queued',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_tickets_order" ON "fnb_tickets" (order_id);
CREATE INDEX "fnb_tickets_status" ON "fnb_tickets" (status);
