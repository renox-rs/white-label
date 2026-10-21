-- The final settlement lifecycle (wl-hr §3 *Final settlement*, §4, §5.4, §7,
-- issues #212 to #215): the amounts, approve and pay, the payments made, the
-- offset within the Art. 25 caps, the dispute and its resolution, the
-- acknowledgement and the closing, and the marks of the 14-day alerts.

-- The last working day of the employee: the 14-day deadline counts from it.
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN last_working_day TEXT;

-- The dues, AED fils. The dues are their sum.
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN unpaid_salary INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN leave_encashment INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN gratuity INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN notice_pay INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN other_dues INTEGER NOT NULL DEFAULT 0;

-- The recoveries by type (`recoveries` is the overpayment recoveries added by
-- supplementary runs): the caps of Art. 25 differ by type.
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN advance_recoveries INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN fine_recoveries INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN court_debt_recoveries INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN damage_recoveries INTEGER NOT NULL DEFAULT 0;

-- What follows from them (`HRM.ACL.06`, L-23): the part of the recoveries
-- offset within the caps, the net (dues minus the offset, never below 0) and
-- the part above the caps, which the employee owes.
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN recoveries_offset INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN net INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN owed_by_employee INTEGER NOT NULL DEFAULT 0;
-- The part of a revised net below what was already paid, made a receivable
-- at `resolve`.
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN overpaid INTEGER NOT NULL DEFAULT 0;

-- Who entered the amounts (the preparer, `HRM.ACL.04`), who approved, who
-- paid, and the version of the statement (`DOC.GEN.01`).
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN prepared_by INTEGER;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN approved_by INTEGER;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN paid_by INTEGER;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN statement_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN paid_on TEXT;

-- The payee *estate*: the representative and the document giving the right.
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN representative TEXT;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN right_document TEXT;

-- The acknowledgement requested at `pay` (`DOC.SIG.01`) and signed.
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN acknowledgement_requested_on TEXT;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN acknowledgement_signed_on TEXT;
-- From this day a paid settlement closes without a dispute (`close`).
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN closes_on TEXT;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN closed_on TEXT;

-- The dispute: source (`worker`, `mohre_complaint`, `court`), reference, the
-- undisputed amount, the hearing or reply dates, the labour-case ticket, the
-- state it came from, the outcome and the document of the decision.
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN dispute_source TEXT;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN dispute_reference TEXT;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN undisputed_amount INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN hearing_on TEXT;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN reply_due_on TEXT;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN dispute_ticket INTEGER;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN disputed_from TEXT;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN disputed_by INTEGER;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN revised_by INTEGER;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN dispute_outcome TEXT;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN resolution_document TEXT;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN resolved_by INTEGER;

-- `HRM.RUL.09`: the days the alert at N days and the overdue alert were sent,
-- once each.
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN alerted_on TEXT;
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN overdue_alerted_on TEXT;

-- The payments made against a settlement (`FIN.PAY.01`): by WPS or transfer.
CREATE TABLE "wl_hr_settlement_payments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    settlement_id INTEGER NOT NULL REFERENCES "wl_hr_final_settlements" (id),
    -- AED fils, positive.
    amount INTEGER NOT NULL,
    -- `wps` or `transfer`.
    method TEXT NOT NULL,
    reference TEXT NOT NULL DEFAULT '',
    paid_on TEXT NOT NULL,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_settlement_payments_settlement" ON "wl_hr_settlement_payments" (settlement_id);

-- A receivable that a settlement caused (recoveries above the caps, or a
-- revised net below what was paid). No foreign key, so it can be reverted.
ALTER TABLE "wl_hr_receivables" ADD COLUMN settlement_id INTEGER;
