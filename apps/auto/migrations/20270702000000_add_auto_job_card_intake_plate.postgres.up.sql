-- The plate the car had when the job card was opened (`dubai A 12345`): the
-- card is still found by it in the search after the plate moves to another car
-- (BL-AGT-13). The cards opened before keep the car's plate of today.
ALTER TABLE "auto_job_cards" ADD COLUMN intake_plate TEXT;
UPDATE "auto_job_cards" SET intake_plate = (
    SELECT v.plate_emirate || ' ' || v.plate_code || ' ' || v.plate_number
    FROM "auto_vehicles" v
    WHERE v.id = "auto_job_cards".vehicle_id
);
