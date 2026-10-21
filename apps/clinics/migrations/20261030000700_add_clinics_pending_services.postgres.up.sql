-- 05 Clinics prior authorisation at check-in (BL-CLN-02): the services the
-- visit will use and the units of each, entered by the receptionist before
-- the check-in (the booked service with one unit when nothing was entered).
ALTER TABLE "clinics_pending_check_ins" ADD COLUMN services TEXT NOT NULL DEFAULT '[]';
