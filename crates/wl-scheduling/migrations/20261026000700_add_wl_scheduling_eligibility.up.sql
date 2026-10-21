-- BL-SCH-07, BL-SCH-08 (SCH.RUL.05, SCH.ACL.04): the qualifications and
-- documents a service type needs of the resource that delivers it, and the
-- role a person works in (for a role-bound licence).
ALTER TABLE "wl_scheduling_service_types" ADD COLUMN required_qualifications TEXT NOT NULL DEFAULT '';
ALTER TABLE "wl_scheduling_service_types" ADD COLUMN required_documents TEXT NOT NULL DEFAULT '';
ALTER TABLE "wl_scheduling_resources" ADD COLUMN role TEXT;
