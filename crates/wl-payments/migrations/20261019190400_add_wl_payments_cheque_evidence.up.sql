-- A cheque held as evidence after a forced termination of the lease it was
-- received with (01 Property `PRP.ACL.19`, issue #262): it stays in the
-- register, is never put on the deposit list and is never deposited.
ALTER TABLE "wl_payments_cheques" ADD COLUMN evidence_note TEXT;
