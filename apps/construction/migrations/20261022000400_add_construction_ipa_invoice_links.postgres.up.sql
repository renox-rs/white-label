-- What `invoice` made for an IPA (02 Construction §7.1 *IPA*): the credit note
-- for the accepted liquidated damages, the dispute of the contested part and
-- the supplier bill from the client for the accepted back- and
-- contra-charges. They are kept as they are made, so a run that stopped
-- half-way resumes without making a document twice.
ALTER TABLE "construction_ipas" ADD COLUMN credit_note_id BIGINT;
ALTER TABLE "construction_ipas" ADD COLUMN dispute_id BIGINT;
ALTER TABLE "construction_ipas" ADD COLUMN supplier_bill_id BIGINT;
