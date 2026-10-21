-- Pay accounts and their change (wl-hr §3 *Pay account*, §4, §5.7,
-- `HRM.ACL.05`, issues #209 to #211).

-- Where an employee's salary is paid. A change is requested, approved by
-- another user, cooled for some days and only then used by payroll. The
-- account number is sensitive (`SEN`): only the last 4 digits are shown or
-- sent.
CREATE TABLE "wl_hr_pay_accounts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id),
    -- `bank_account` (IBAN) or `salary_card`.
    channel TEXT NOT NULL,
    routing_code TEXT,
    account_number TEXT NOT NULL,
    last4 TEXT NOT NULL,
    holder_name TEXT NOT NULL,
    -- Required when the holder name does not match the employee's name.
    mismatch_explanation TEXT,
    mismatch_document INTEGER,
    -- The user who asked (the employee's own user in the portal).
    requested_by INTEGER,
    requested_by_employee INTEGER NOT NULL DEFAULT 0,
    approved_by INTEGER,
    approved_on TEXT,
    reject_reason TEXT,
    -- The worker's mobile number on record before the request: the change
    -- notice goes only to it.
    notice_to TEXT,
    notice_email TEXT,
    notice_sent_to TEXT,
    notice_message INTEGER,
    -- Set when the notice gave up (`MessageGaveUp`) or there was no number.
    notice_undelivered_at TEXT,
    -- HR reached the worker another way: by whom, how, when.
    reached_by TEXT,
    reached_method TEXT,
    reached_on TEXT,
    reached_recorded_by INTEGER,
    -- The cooling period in days as set when the request was made.
    cooling_days INTEGER NOT NULL DEFAULT 5,
    cooling_ends_on TEXT,
    -- The day the bank reported the account closed.
    closed_reported_on TEXT,
    waive_reason TEXT,
    stopped_by INTEGER,
    stopped_by_worker INTEGER NOT NULL DEFAULT 0,
    -- 'pending_approval', 'cooling', 'active', 'retired', 'rejected'.
    status TEXT NOT NULL DEFAULT 'pending_approval',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_pay_accounts_employee" ON "wl_hr_pay_accounts" (employee_id, status);

-- The account a SIF line was sent to: the file keeps what was used, later
-- changes do not alter it. No foreign key, so the migration can be reverted.
ALTER TABLE "wl_hr_sif_lines" ADD COLUMN pay_account_id INTEGER;
ALTER TABLE "wl_hr_sif_lines" ADD COLUMN account_last4 TEXT;
