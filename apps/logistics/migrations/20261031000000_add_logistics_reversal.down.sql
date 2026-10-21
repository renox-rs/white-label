DROP TABLE "logistics_reversals";
ALTER TABLE "logistics_shipments" DROP COLUMN reversed;
ALTER TABLE "logistics_shipments" DROP COLUMN credit_due;
ALTER TABLE "logistics_shipments" DROP COLUMN cod_released;
ALTER TABLE "logistics_shipments" DROP COLUMN cash_treatment;
ALTER TABLE "logistics_shipments" DROP COLUMN liability;
