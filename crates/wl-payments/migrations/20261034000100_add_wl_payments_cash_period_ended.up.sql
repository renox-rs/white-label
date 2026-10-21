-- When the record that is a handover's period ended it (a run's return to the
-- depot or its abort): the clock of a vertical rule that replaces `FIN.RUL.22`
-- (03 `LOG.RUL.13`), kept apart from `period_end`, which also holds the
-- period's planned end.
ALTER TABLE "wl_payments_cash_handovers" ADD COLUMN period_ended_at TEXT;
