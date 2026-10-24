-- 08 Auto: suspending, ending and terminating a fleet contract (BL-AUT-13,
-- BL-AUT-12). A termination keeps whether the fleet asked for the early
-- termination fee to be waived (the owner's approval waives it), who approved
-- it and when, the invoice of the fee, when the fleet contact was told, and
-- when the effects of the end were all done (the daily run tries an end that
-- stopped half way again). A booking the plan asked for keeps the day the
-- advisor confirmed it while the contract is suspended.
ALTER TABLE "auto_fleet_contracts" ADD COLUMN termination_fee_waived BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "auto_fleet_contracts" ADD COLUMN termination_approved_by INTEGER;
ALTER TABLE "auto_fleet_contracts" ADD COLUMN termination_approved_at TEXT;
ALTER TABLE "auto_fleet_contracts" ADD COLUMN termination_fee_invoice_id INTEGER;
ALTER TABLE "auto_fleet_contracts" ADD COLUMN contact_notified_at TEXT;
ALTER TABLE "auto_fleet_contracts" ADD COLUMN wound_down_at TEXT;
ALTER TABLE "auto_fleet_services" ADD COLUMN booking_confirmed_at TEXT;
