-- How an owner statement was settled and the invoice to the owner for a
-- negative balance (01 Property §7.1 *Owner statement* `pay_out`,
-- `carry_forward`, `invoice_owner`; BL-PRP-20).
ALTER TABLE "property_owner_statements" ADD COLUMN settled_how TEXT;
ALTER TABLE "property_owner_statements" ADD COLUMN owner_invoice_id BIGINT;
