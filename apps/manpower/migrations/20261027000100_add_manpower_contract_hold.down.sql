DROP TABLE "manpower_contract_holds";
ALTER TABLE "manpower_client_contracts" DROP COLUMN held_by;
ALTER TABLE "manpower_client_contracts" DROP COLUMN held_at;
ALTER TABLE "manpower_client_contracts" DROP COLUMN status_before_hold;
