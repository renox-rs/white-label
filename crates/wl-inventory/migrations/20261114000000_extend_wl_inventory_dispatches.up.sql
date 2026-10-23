-- The Dispatch lifecycle (issues #240 to #244) on the tables the sales
-- cancellation base (20261036000200, BL-STK-26) made: the number, the
-- recipient's name and address, the pick, dispatch and delivery facts, the
-- loss, and on a line the position, the cost, the refusal and the return.
--
-- The base's `sale_line_id` is replaced by the general source line
-- (`source_line_type`, `source_line_id`): the sale lines already there
-- become `sales.sale_line`.
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN number TEXT;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN recipient_name TEXT NOT NULL DEFAULT '';
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN address TEXT NOT NULL DEFAULT '';
-- `own_delivery`, `courier` or `collection`.
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN handover TEXT;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN tracking_number TEXT;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN collector_signature TEXT;
-- The delivery wl-delivery was asked for (own delivery).
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN delivery_id INTEGER;
-- The delivery note, a PDF document kept as text.
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN delivery_note TEXT;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN picked_by INTEGER;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN picked_at TEXT;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN dispatched_by INTEGER;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN dispatched_at TEXT;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN delivered_at TEXT;
-- The proof of delivery that confirmed it (one confirmation per proof), and
-- whether a delivery reversal marked it reversed.
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN proof_id INTEGER;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN proof_reversed INTEGER NOT NULL DEFAULT 0;
-- The status the dispatch had when it was marked lost.
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN lost_from TEXT;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN loss_fils INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN loss_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN loss_open INTEGER NOT NULL DEFAULT 0;

UPDATE "wl_inventory_dispatches" SET number = 'DSP-' || printf('%04d', id) WHERE number IS NULL;
CREATE UNIQUE INDEX "wl_inventory_dispatches_number" ON "wl_inventory_dispatches" (number);

ALTER TABLE "wl_inventory_dispatch_lines" ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
-- The cost of one base unit when the goods left stock, in fils.
ALTER TABLE "wl_inventory_dispatch_lines" ADD COLUMN unit_cost INTEGER NOT NULL DEFAULT 0;
-- The line of the source this line fulfils.
ALTER TABLE "wl_inventory_dispatch_lines" ADD COLUMN source_line_type TEXT;
ALTER TABLE "wl_inventory_dispatch_lines" ADD COLUMN source_line_id INTEGER;
-- From the proof of delivery.
ALTER TABLE "wl_inventory_dispatch_lines" ADD COLUMN delivered_milli INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_inventory_dispatch_lines" ADD COLUMN refused_milli INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_inventory_dispatch_lines" ADD COLUMN refusal_reason TEXT;
-- Of the refused quantity: received back into quarantine, and reported
-- missing on return.
ALTER TABLE "wl_inventory_dispatch_lines" ADD COLUMN returned_milli INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_inventory_dispatch_lines" ADD COLUMN missing_milli INTEGER NOT NULL DEFAULT 0;

UPDATE "wl_inventory_dispatch_lines" SET position = (
    SELECT COUNT(*) FROM "wl_inventory_dispatch_lines" other
    WHERE other.dispatch_id = "wl_inventory_dispatch_lines".dispatch_id
      AND other.id <= "wl_inventory_dispatch_lines".id
);
UPDATE "wl_inventory_dispatch_lines"
    SET source_line_type = 'sales.sale_line', source_line_id = sale_line_id
    WHERE sale_line_id IS NOT NULL;
DROP INDEX "wl_inventory_dispatch_lines_sale_line";
ALTER TABLE "wl_inventory_dispatch_lines" DROP COLUMN sale_line_id;
CREATE UNIQUE INDEX "wl_inventory_dispatch_lines_position"
    ON "wl_inventory_dispatch_lines" (dispatch_id, position);
CREATE INDEX "wl_inventory_dispatch_lines_source_line"
    ON "wl_inventory_dispatch_lines" (source_line_type, source_line_id);
