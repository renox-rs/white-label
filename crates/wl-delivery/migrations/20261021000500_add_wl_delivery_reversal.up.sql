-- A delivery reversed (wl-delivery §4.1 `reverse_delivery`, `WRK.ACL.06`,
-- `WRK.ACL.10`, #161): the original proof is kept and marked reversed with
-- why, who bears it and who approved it. It is never edited otherwise.
ALTER TABLE "wl_delivery_proofs" ADD COLUMN reversed_at TEXT;
ALTER TABLE "wl_delivery_proofs" ADD COLUMN reversal_reason TEXT;
ALTER TABLE "wl_delivery_proofs" ADD COLUMN reversal_liability TEXT;
ALTER TABLE "wl_delivery_proofs" ADD COLUMN reversed_by INTEGER;
