-- BL-MFG-14: an item made for one customer (printed labels and packaging) is
-- scrapped, not finished to stock, when its sales order is cancelled after
-- production started (07 §7.1).
ALTER TABLE "manufacturing_item_flags" ADD COLUMN customer_specific INTEGER NOT NULL DEFAULT 0;
