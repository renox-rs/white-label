-- 08 Auto: a customer who is blocked as a party (BL-AUT-17, wl-parties 5.6):
-- the advisor marks the estimate lines needed to make the vehicle safe to drive
-- (*safety work*); the branch manager's review finishes those and removes the
-- other lines not yet started with the reason *party blocked*, so they are not
-- billed and stay visible in the history of the job card.
ALTER TABLE "auto_job_labour_lines" ADD COLUMN safety_work BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "auto_job_labour_lines" ADD COLUMN removed_reason TEXT;
ALTER TABLE "auto_job_part_lines" ADD COLUMN safety_work BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "auto_job_part_lines" ADD COLUMN removed_reason TEXT;
