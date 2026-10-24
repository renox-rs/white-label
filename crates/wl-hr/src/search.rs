//! Employees, leave requests, payroll runs and final settlements in the
//! search index and the *Related* panel (BL-AGT-11, `PLT.SRC.01`,
//! `PLT.REL.01`).
//!
//! An employee is found by their employee number and name, and by their
//! Emirates ID, passport and labour card (work permit) numbers, which are
//! personal: stored only as hashes, matched on the exact value, shown
//! masked in a result and never in the text. Salary, bank and payroll
//! amounts are financial: no amount is ever put in a document, and payroll
//! runs and settlements carry the `financial` class. Payroll lines are not
//! indexed one by one; they are reached through *Related*.
//!
//! **Where the identity numbers come from.** The labour card is the
//! reference of the visa case's work permit step (this module's own rows).
//! The passport and the Emirates ID are documents of the employee kept by
//! the documents module (`hr.employee` documents of kind `passport`,
//! `emirates_id`, `work_permit`, `labour_card`). The kernel offers no
//! contract to read another module's document numbers inside the indexing
//! transaction, so the employee's document reads them from the documents
//! module's tables when that module is installed (the table exists), and
//! skips them otherwise; the numbers reach the index when the employee is
//! next written or at the rebuild the demo seeds end with. The employee's
//! name is the title of the party's search document (the parties module
//! indexes it), the employee number otherwise.
//!
//! The documents are written in the transaction that writes the record:
//! every transition of the four machines ([`reindex`]), and the creation
//! paths of the employee and the leave request.

use renox::db::{Dialect, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition};
use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::Relation;
use wl_kernel::search::{
    SearchDocument, SearchDocumentRow, Searchable, Sensitivity, index_in, kinds,
};
use wl_kernel::workforce::{
    EMPLOYEE_SUBJECT_TYPE, PAYROLL_RUN_SUBJECT_TYPE, SETTLEMENT_SUBJECT_TYPE,
    VISA_CASE_SUBJECT_TYPE,
};

use crate::employee::Employee;
use crate::leave::{LEAVE_SUBJECT_TYPE, LeaveRequest};
use crate::payroll::PayrollRun;
use crate::permissions;
use crate::settlement::FinalSettlement;
use crate::visa::{STEP_WORK_PERMIT, VisaCase, VisaCaseStep};

/// The subject type of a payroll line (reached through its run or its
/// employee, never indexed on its own).
pub const PAYROLL_LINE_SUBJECT_TYPE: &str = "hr.payroll_line";

/// The kinds of the documents module whose number identifies an employee,
/// with the identifier kind each is stored as.
const IDENTITY_KINDS: &[(&str, &str)] = &[
    ("passport", kinds::PASSPORT),
    ("emirates_id", kinds::EMIRATES_ID),
    ("work_permit", kinds::LABOUR_CARD),
    ("labour_card", kinds::LABOUR_CARD),
];

/// Registers the searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<Employee>(EMPLOYEE_SUBJECT_TYPE)
        .permission(permissions::EMPLOYEES_VIEW)
        .label("Employees")
        .link("/hr/employees/{id}")
        .document_in(|tx, employee| Box::pin(employee_document(tx, employee)))
        .register();
    Searchable::of::<LeaveRequest>(LEAVE_SUBJECT_TYPE)
        .permission(permissions::LEAVE_VIEW)
        .label("Leave requests")
        .document_in(|tx, leave| Box::pin(leave_document(tx, leave)))
        .register();
    Searchable::of::<PayrollRun>(PAYROLL_RUN_SUBJECT_TYPE)
        .permission(permissions::PAYROLL_VIEW)
        .label("Payroll runs")
        .link("/hr/payroll/{id}")
        .document(payroll_document)
        .register();
    Searchable::of::<FinalSettlement>(SETTLEMENT_SUBJECT_TYPE)
        .permission(permissions::SETTLEMENTS_PREPARE)
        .label("Final settlements")
        .link("/hr/settlements/{id}")
        .document_in(|tx, settlement| Box::pin(settlement_document(tx, settlement)))
        .register();

    // employee → visa cases, leave, payroll lines, settlements; the
    // timesheets are declared by the attendance module (its table) and the
    // documents by the documents module (the generic relation).
    Relation::has_many(
        EMPLOYEE_SUBJECT_TYPE,
        "visa_cases",
        VISA_CASE_SUBJECT_TYPE,
        "wl_hr_visa_cases",
        "employee_id",
    )
    .label("Visa cases")
    .permission(permissions::EMPLOYEES_SENSITIVE_VIEW)
    .register();
    Relation::has_many(
        EMPLOYEE_SUBJECT_TYPE,
        "leave",
        LEAVE_SUBJECT_TYPE,
        "wl_hr_leave_requests",
        "employee_id",
    )
    .label("Leave")
    .register();
    Relation::has_many(
        EMPLOYEE_SUBJECT_TYPE,
        "payroll_lines",
        PAYROLL_LINE_SUBJECT_TYPE,
        "wl_hr_payroll_lines",
        "employee_id",
    )
    .label("Payroll lines")
    .permission(permissions::PAYROLL_VIEW)
    .register();
    Relation::has_many(
        EMPLOYEE_SUBJECT_TYPE,
        "settlements",
        SETTLEMENT_SUBJECT_TYPE,
        "wl_hr_final_settlements",
        "employee_id",
    )
    .label("Final settlements")
    .register();
    Relation::belongs_to(
        EMPLOYEE_SUBJECT_TYPE,
        "person",
        PARTY_SUBJECT_TYPE,
        "wl_hr_employees",
        "party_id",
    )
    .label("Person")
    .permission("parties.view")
    .link("/parties/{id}")
    .register();

    // leave request, settlement → the employee; payroll run → its lines.
    Relation::belongs_to(
        LEAVE_SUBJECT_TYPE,
        "employee",
        EMPLOYEE_SUBJECT_TYPE,
        "wl_hr_leave_requests",
        "employee_id",
    )
    .label("Employee")
    .register();
    Relation::belongs_to(
        SETTLEMENT_SUBJECT_TYPE,
        "employee",
        EMPLOYEE_SUBJECT_TYPE,
        "wl_hr_final_settlements",
        "employee_id",
    )
    .label("Employee")
    .register();
    Relation::has_many(
        PAYROLL_RUN_SUBJECT_TYPE,
        "lines",
        PAYROLL_LINE_SUBJECT_TYPE,
        "wl_hr_payroll_lines",
        "run_id",
    )
    .label("Payroll lines")
    .permission(permissions::PAYROLL_VIEW)
    .register();
}

/// The title of `subject`'s search document, read in `tx`, if it has one.
async fn title_of(tx: &mut Transaction, subject: &Subject) -> Result<Option<String>> {
    Ok(SearchDocumentRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .first(&mut *tx)
        .await?
        .map(|row| row.title)
        .filter(|title| !title.trim().is_empty()))
}

/// The employee's name: their party's title, else the employee number.
async fn name_of(tx: &mut Transaction, employee: &Employee) -> Result<String> {
    Ok(
        title_of(tx, &Subject::new(PARTY_SUBJECT_TYPE, employee.party_id))
            .await?
            .unwrap_or_else(|| employee.employee_number.clone()),
    )
}

/// Whether table `name` exists (a module that is not installed has none).
async fn table_exists(tx: &mut Transaction, name: &str) -> Result<bool> {
    let statement = match tx.dialect() {
        Dialect::Sqlite => "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
        Dialect::Postgres => {
            "SELECT COUNT(*) FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = ?"
        }
    };
    let count: i64 = sql(statement).bind(name).scalar(&mut *tx).await?;
    Ok(count > 0)
}

/// The identity numbers of employee `id`, as `(identifier kind, number)`:
/// the work permit references of their visa cases, and the numbers of their
/// identity documents when the documents module is installed.
async fn identity_numbers(tx: &mut Transaction, id: i64) -> Result<Vec<(&'static str, String)>> {
    let mut numbers = Vec::new();
    let cases: Vec<i64> = VisaCase::query()
        .where_eq("employee_id", id)
        .get(&mut *tx)
        .await?
        .iter()
        .map(|case| case.id)
        .collect();
    if !cases.is_empty() {
        let steps = VisaCaseStep::query()
            .where_in("case_id", cases)
            .where_eq("step_type", STEP_WORK_PERMIT)
            .get(&mut *tx)
            .await?;
        for step in steps {
            if let Some(reference) = step.reference.filter(|r| !r.trim().is_empty()) {
                numbers.push((kinds::LABOUR_CARD, reference));
            }
        }
    }
    if table_exists(tx, "wl_documents_documents").await?
        && table_exists(tx, "wl_documents_kinds").await?
    {
        let placeholders = vec!["?"; IDENTITY_KINDS.len()].join(", ");
        let rows = sql(format!(
            "SELECT k.key AS kind_key, d.number AS number \
             FROM wl_documents_documents d JOIN wl_documents_kinds k ON k.id = d.kind_id \
             WHERE d.subject_type = ? AND d.subject_id = ? AND d.number IS NOT NULL \
             AND d.superseded_by IS NULL AND d.archived_at IS NULL \
             AND k.key IN ({placeholders}) ORDER BY d.id"
        ))
        .bind(EMPLOYEE_SUBJECT_TYPE)
        .bind(id)
        .bind_all(
            IDENTITY_KINDS
                .iter()
                .map(|(key, _)| renox::db::DbValue::Text((*key).to_owned())),
        )
        .fetch_all(&mut *tx)
        .await?;
        for row in &rows {
            let key: String = row.try_get("kind_key")?;
            let number: String = row.try_get("number")?;
            if let Some((_, kind)) = IDENTITY_KINDS.iter().find(|(k, _)| *k == key) {
                numbers.push((*kind, number));
            }
        }
    }
    Ok(numbers)
}

/// How an employee reads in a search result.
async fn employee_document(tx: &mut Transaction, employee: &Employee) -> Result<SearchDocument> {
    let name = name_of(tx, employee).await?;
    let mut document = SearchDocument::new(name)
        .secondary(format!(
            "{} · {}",
            employee.employee_number, employee.job_title
        ))
        .identifier(kinds::NUMBER, &employee.employee_number)
        .keyword(&employee.job_role)
        .status(employee.status.as_str())
        .legal_entity(employee.legal_entity_id);
    if let Some(department) = &employee.department {
        document = document.keyword(department);
    }
    for (kind, number) in identity_numbers(tx, employee.id).await? {
        document = document.identifier_of_class(kind, number, Sensitivity::Personal);
    }
    Ok(document)
}

/// How a leave request reads in a search result.
async fn leave_document(tx: &mut Transaction, leave: &LeaveRequest) -> Result<SearchDocument> {
    let employee = Employee::query()
        .where_eq("id", leave.employee_id)
        .first(&mut *tx)
        .await?;
    let (name, entity) = match &employee {
        Some(employee) => (name_of(tx, employee).await?, employee.legal_entity_id),
        None => (String::new(), None),
    };
    let kind = leave.leave_type.replace('_', " ");
    Ok(SearchDocument::new(format!(
        "{kind} leave · {} to {}",
        leave.from_date, leave.to_date
    ))
    .secondary(name)
    .status(leave.status.as_str())
    .legal_entity(entity))
}

/// How a payroll run reads in a search result: its period and type, never
/// an amount.
fn payroll_document(run: &PayrollRun) -> SearchDocument {
    SearchDocument::new(format!("Payroll {} ({})", run.period, run.run_type))
        .keyword(&run.period)
        .status(run.status.as_str())
        .legal_entity(run.legal_entity_id)
        .sensitivity(Sensitivity::Financial)
}

/// How a final settlement reads in a search result: whose it is and why,
/// never an amount.
async fn settlement_document(
    tx: &mut Transaction,
    settlement: &FinalSettlement,
) -> Result<SearchDocument> {
    let employee = Employee::query()
        .where_eq("id", settlement.employee_id)
        .first(&mut *tx)
        .await?;
    let (name, number, entity) = match &employee {
        Some(employee) => (
            name_of(tx, employee).await?,
            employee.employee_number.clone(),
            employee.legal_entity_id,
        ),
        None => (String::new(), String::new(), None),
    };
    Ok(SearchDocument::new(format!("Final settlement · {name}"))
        .secondary(format!(
            "{number} · {}",
            settlement.reason.replace('_', " ")
        ))
        .status(settlement.status.as_str())
        .legal_entity(entity)
        .sensitivity(Sensitivity::Financial))
}

/// Writes the document of `subject_type` record `id` in `tx`.
pub(crate) async fn index(tx: &mut Transaction, subject_type: &str, id: i64) -> Result {
    index_in(tx, &Subject::new(subject_type, id)).await?;
    Ok(())
}

/// `transition`, writing the record's document after its own effects, in
/// its transaction.
pub(crate) fn reindex<M>(transition: Transition<M>, subject_type: &'static str) -> Transition<M>
where
    M: StateMachine + Model<Key = i64> + Send + Sync + 'static,
{
    transition.effect(move |record: &M, _ctx, tx| {
        let id = record.id();
        Box::pin(async move { index(tx, subject_type, id).await })
    })
}

/// An employee transition that keeps the employee's document current.
pub(crate) fn reindex_employee(transition: Transition<Employee>) -> Transition<Employee> {
    reindex(transition, EMPLOYEE_SUBJECT_TYPE)
}

/// A leave transition that keeps the request's document current.
pub(crate) fn reindex_leave(transition: Transition<LeaveRequest>) -> Transition<LeaveRequest> {
    reindex(transition, LEAVE_SUBJECT_TYPE)
}

/// A payroll run transition that keeps the run's document current.
pub(crate) fn reindex_payroll(transition: Transition<PayrollRun>) -> Transition<PayrollRun> {
    reindex(transition, PAYROLL_RUN_SUBJECT_TYPE)
}

/// A settlement transition that keeps the settlement's document current.
pub(crate) fn reindex_settlement(
    transition: Transition<FinalSettlement>,
) -> Transition<FinalSettlement> {
    reindex(transition, SETTLEMENT_SUBJECT_TYPE)
}
