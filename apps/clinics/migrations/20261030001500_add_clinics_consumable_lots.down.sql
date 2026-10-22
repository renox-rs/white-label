DROP INDEX IF EXISTS "clinics_consumable_issues_lot_id";
DROP INDEX IF EXISTS "clinics_consumable_issues_charge_line_id";
DROP INDEX IF EXISTS "clinics_consumable_issues_visit_id";
DROP TABLE IF EXISTS "clinics_consumable_issues";
ALTER TABLE "clinics_charge_lines" DROP COLUMN issued_at;
ALTER TABLE "clinics_charge_lines" DROP COLUMN lot_number;
ALTER TABLE "clinics_charge_lines" DROP COLUMN lot_id;
ALTER TABLE "clinics_charge_lines" DROP COLUMN lot_tracked;
