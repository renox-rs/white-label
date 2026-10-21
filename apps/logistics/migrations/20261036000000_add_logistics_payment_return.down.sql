DROP TABLE "logistics_account_changes";
DROP TABLE "logistics_payment_returns";
ALTER TABLE "logistics_remittances" DROP COLUMN account_confirmed_note;
ALTER TABLE "logistics_remittances" DROP COLUMN account_confirmed_by;
ALTER TABLE "logistics_remittances" DROP COLUMN account_confirmed_at;
