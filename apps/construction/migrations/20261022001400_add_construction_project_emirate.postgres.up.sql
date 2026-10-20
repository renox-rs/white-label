-- `CON.ACL.13` (02 Construction §3.3; BL-CON-16): the emirate of the project's
-- site. The subcontractor registration check applies to Dubai projects.
ALTER TABLE "construction_projects" ADD COLUMN emirate TEXT NOT NULL DEFAULT 'dubai';
