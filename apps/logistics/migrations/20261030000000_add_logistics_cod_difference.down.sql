DROP TABLE "logistics_cod_losses";
ALTER TABLE "logistics_shipments" DROP COLUMN cod_approved_by;
ALTER TABLE "logistics_shipments" DROP COLUMN cod_instruction_by;
ALTER TABLE "logistics_shipments" DROP COLUMN cod_instruction;
ALTER TABLE "logistics_shipments" DROP COLUMN cod_collected;
