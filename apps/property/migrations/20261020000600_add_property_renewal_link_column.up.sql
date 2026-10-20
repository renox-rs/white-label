-- Renewal links both ways (BL-PRP-07): the version that replaced this one
-- when it was renewed. The new version points back with `previous_id`.
ALTER TABLE "property_leases" ADD COLUMN next_id INTEGER;
