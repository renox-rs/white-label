-- The cheque register (wl-payments §3, §4 *Cheque*, §5.3; issue #102): the
-- post-dated cheques received and issued, their bounce records and the
-- documents they pay. The status moves only through the machine; nothing is
-- deleted.
CREATE TABLE "wl_payments_cheques" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- `in` (received) or `out` (issued).
    direction TEXT NOT NULL DEFAULT 'in',
    legal_entity_id INTEGER NOT NULL,
    -- The drawer of a received cheque, the payee of an issued one.
    party_id INTEGER NOT NULL,
    bank_account TEXT,
    bank TEXT NOT NULL,
    number TEXT NOT NULL,
    cheque_date TEXT NOT NULL,
    -- Money in fils of the currency.
    amount INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- The agreement it was received with (a lease), as a subject.
    agreement_type TEXT,
    agreement_id INTEGER,
    -- Who collected it; a cheque collected in the field is deposited only
    -- after a cash handover confirms it (`FIN.ACL.20`).
    collected_by INTEGER,
    field_collected INTEGER NOT NULL DEFAULT 0,
    custody_handover_id INTEGER,
    -- How many times it was deposited, and the date of the latest deposit.
    presentations INTEGER NOT NULL DEFAULT 0,
    deposit_date TEXT,
    -- What the bank paid when it bounced, what was settled afterwards
    -- (`settle_part`), and what is still owed.
    honoured_amount INTEGER NOT NULL DEFAULT 0,
    settled_amount INTEGER NOT NULL DEFAULT 0,
    open_amount INTEGER NOT NULL,
    -- The latest bank return: reason, certificate reference and day.
    return_reason TEXT,
    return_certificate TEXT,
    bounced_on TEXT,
    -- The bounce fees added so far, before VAT.
    bounce_fee_total INTEGER NOT NULL DEFAULT 0,
    -- The legal case: `civil_execution` or `criminal_complaint`, its
    -- reference, its owner and the day it was referred.
    case_type TEXT,
    case_reference TEXT,
    case_owner_id INTEGER,
    referred_on TEXT,
    recorded_by INTEGER,
    status TEXT NOT NULL DEFAULT 'received',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_cheques_status" ON "wl_payments_cheques" (status);
CREATE INDEX "wl_payments_cheques_party" ON "wl_payments_cheques" (party_id);
CREATE INDEX "wl_payments_cheques_agreement" ON "wl_payments_cheques" (agreement_type, agreement_id);

-- What a cheque pays (a charge, an invoice, a supplier bill), oldest first.
CREATE TABLE "wl_payments_cheque_links" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    cheque_id INTEGER NOT NULL REFERENCES "wl_payments_cheques" (id),
    position INTEGER NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_cheque_links_cheque" ON "wl_payments_cheque_links" (cheque_id);
CREATE INDEX "wl_payments_cheque_links_subject" ON "wl_payments_cheque_links" (subject_type, subject_id);

-- Each bank return of a cheque, with its reason and certificate: a cheque
-- that is re-presented and bounces again keeps both records.
CREATE TABLE "wl_payments_cheque_returns" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    cheque_id INTEGER NOT NULL REFERENCES "wl_payments_cheques" (id),
    -- Which presentation bounced: 1 for the first deposit.
    presentation INTEGER NOT NULL,
    reason TEXT NOT NULL,
    -- The reference of the bank's return certificate (a document of
    -- wl-documents, or its number); none for an issued cheque.
    certificate TEXT,
    honoured_amount INTEGER NOT NULL DEFAULT 0,
    honoured_payment_id INTEGER REFERENCES "wl_payments_payments" (id),
    -- The bounce fee added: the catalogue charge type, the amount before
    -- VAT and the charge as a subject (none when it is left to the
    -- accountant as a task).
    fee_charge_type TEXT,
    fee_amount INTEGER NOT NULL DEFAULT 0,
    fee_subject_type TEXT,
    fee_subject_id INTEGER,
    returned_on TEXT NOT NULL,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (cheque_id, presentation)
);

-- The cheque a follow-up task is about, the user it is assigned to (the
-- case owner of a legal case) and the evidence pack it carries.
ALTER TABLE "wl_payments_followup_tasks" ADD COLUMN cheque_id INTEGER;
ALTER TABLE "wl_payments_followup_tasks" ADD COLUMN assignee_id INTEGER;
ALTER TABLE "wl_payments_followup_tasks" ADD COLUMN evidence TEXT NOT NULL DEFAULT '{}';
