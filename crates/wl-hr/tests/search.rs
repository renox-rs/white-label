//! Employees, leave requests, payroll runs and final settlements in the
//! search index and the Related panel (BL-AGT-11): each type's document is
//! written by its write path, found by its identifiers, and its relations
//! are produced; an identity number is stored only as a hash, matches only
//! on the exact value and shows masked; no amount reaches a document.

mod support;

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_hr::employees;
use wl_hr::{NewEmployee, PayrollLine, PayrollRunStatus, VisaCase, VisaCaseStatus, VisaCaseStep};
use wl_kernel::related::{self, RelatedGroup};
use wl_kernel::scopes::Reach;
use wl_kernel::search::{self, Access, kinds};
use wl_kernel::workforce::{
    EMPLOYEE_SUBJECT_TYPE, PAYROLL_RUN_SUBJECT_TYPE, SETTLEMENT_SUBJECT_TYPE,
};
use wl_kernel::{PartyId, Subject};

use support::{boot, branch, nov};

/// Someone who may see every HR type, everywhere.
fn everyone() -> Access {
    let permissions = [
        wl_hr::permissions::EMPLOYEES_VIEW,
        wl_hr::permissions::EMPLOYEES_SENSITIVE_VIEW,
        wl_hr::permissions::LEAVE_VIEW,
        wl_hr::permissions::PAYROLL_VIEW,
        wl_hr::permissions::SETTLEMENTS_PREPARE,
    ];
    Access::with(
        1,
        permissions
            .iter()
            .map(|p| ((*p).to_owned(), Reach::everything())),
    )
}

fn count(groups: &[RelatedGroup], name: &str) -> Option<u64> {
    groups.iter().find(|g| g.name == name).map(|g| g.count)
}

async fn found(db: &Db, text: &str, subject: &Subject) -> bool {
    search::lookup_identifier(db, text)
        .await
        .unwrap()
        .iter()
        .any(|m| &m.subject == subject)
}

#[renox::test]
async fn an_employee_is_found_by_number_and_by_the_exact_labour_card_shown_masked() {
    let t = boot().await;
    let account = t.user("Ravi").await;
    let mut new = NewEmployee::new(PartyId(50), "driver", nov(2))
        .scope(branch())
        .user(account.id)
        .job_title("Driver");
    new.employee_number = "EMP-0042".to_owned();
    let employee = employees::create(t.state(), new).await.unwrap();
    let subject = Subject::new(EMPLOYEE_SUBJECT_TYPE, employee.id);

    // Hired, the employee is indexed: found by the number typed loosely.
    let document = search::document_of(t.db(), &subject)
        .await
        .unwrap()
        .expect("the hire indexed the employee");
    assert!(document.secondary.contains("EMP-0042"));
    assert!(found(t.db(), "emp 0042", &subject).await);

    // The work permit of the visa case is the labour card number.
    let mut case = VisaCase {
        employee_id: employee.id,
        case_type: "new".to_owned(),
        variant: "standard".to_owned(),
        status: VisaCaseStatus::Open,
        ..VisaCase::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    case.insert_in_state(&mut tx, VisaCaseStatus::Open, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut step = VisaCaseStep {
        case_id: case.id,
        step_type: "work_permit".to_owned(),
        reference: Some("LC-90817263".to_owned()),
        ..VisaCaseStep::default()
    };
    step.insert(t.db()).await.unwrap();
    search::rebuild_type(t.db(), EMPLOYEE_SUBJECT_TYPE)
        .await
        .unwrap();

    // Stored as a hash, never in the text; found only on the exact value.
    let identifiers = search::identifiers_of(t.db(), &subject).await.unwrap();
    let card = identifiers
        .iter()
        .find(|row| row.kind == kinds::LABOUR_CARD)
        .expect("the labour card is an identifier");
    assert!(card.hashed);
    assert_ne!(card.value, "LC90817263");
    let document = search::document_of(t.db(), &subject)
        .await
        .unwrap()
        .unwrap();
    for text in [&document.title, &document.secondary, &document.keywords] {
        assert!(!text.contains("90817263"));
    }
    assert!(found(t.db(), "LC-90817263", &subject).await);
    assert!(!found(t.db(), "90817263", &subject).await);

    // The result shows it masked.
    let results = search::search(t.db(), &everyone(), "LC-90817263", 5)
        .await
        .unwrap();
    let hit = results
        .groups
        .iter()
        .flat_map(|g| g.hits.iter())
        .find(|hit| hit.subject == subject)
        .expect("the employee is a result");
    assert_eq!(hit.matched.as_deref(), Some("•••• 7263"));
}

#[renox::test]
async fn leave_payroll_and_settlement_documents_and_the_employee_relations_are_produced() {
    let t = boot().await;
    let (employee, account) = t.ravi().await;
    let employee_subject = Subject::new(EMPLOYEE_SUBJECT_TYPE, employee.id);

    // A leave request, as the employee asks for it.
    let leave = wl_hr::leave_service::request(
        t.state(),
        account.id,
        employee.id,
        "annual",
        nov(20),
        nov(24),
    )
    .await
    .unwrap();
    let leave_subject = Subject::new(wl_hr::LEAVE_SUBJECT_TYPE, leave.id);
    let document = search::document_of(t.db(), &leave_subject)
        .await
        .unwrap()
        .expect("the request indexed the leave");
    assert!(document.title.contains("annual leave"));
    assert_eq!(document.status.as_deref(), Some("requested"));

    // A payroll run with a line: the run is found by its period, no amount
    // is in its document, and it carries the financial class.
    let run = t.payroll_run("2026-11", PayrollRunStatus::Draft).await;
    t.payroll_line(&run, &employee, 300_000, 287_500).await;
    search::rebuild_type(t.db(), PAYROLL_RUN_SUBJECT_TYPE)
        .await
        .unwrap();
    let run_subject = Subject::new(PAYROLL_RUN_SUBJECT_TYPE, run.id);
    let document = search::document_of(t.db(), &run_subject)
        .await
        .unwrap()
        .unwrap();
    assert!(document.title.contains("2026-11"));
    assert_eq!(document.sensitivity, "financial");
    for text in [&document.title, &document.secondary, &document.keywords] {
        assert!(!text.contains("2875") && !text.contains("3000"));
    }

    // A final settlement drafted for the employee.
    let mut tx = t.db().begin().await.unwrap();
    let settlement =
        wl_hr::settlement::draft_or_relabel_in(&mut tx, employee.id, "resignation", "employee")
            .await
            .unwrap();
    tx.commit().await.unwrap();
    let settlement_subject = Subject::new(SETTLEMENT_SUBJECT_TYPE, settlement.id);
    let document = search::document_of(t.db(), &settlement_subject)
        .await
        .unwrap()
        .expect("the draft indexed the settlement");
    assert!(document.title.starts_with("Final settlement"));
    assert_eq!(document.sensitivity, "financial");

    // The relations, as the panel reads them.
    let access = everyone();
    let groups = related::related(t.db(), &access, &employee_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "leave"), Some(1));
    assert_eq!(count(&groups, "payroll_lines"), Some(1));
    assert_eq!(count(&groups, "settlements"), Some(1));
    assert_eq!(count(&groups, "visa_cases"), Some(0));
    let groups = related::related(t.db(), &access, &run_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "lines"), Some(1));
    let groups = related::related(t.db(), &access, &leave_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "employee"), Some(1));
    let groups = related::related(t.db(), &access, &settlement_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "employee"), Some(1));

    // The lines are reached through Related, never indexed one by one.
    let line = PayrollLine::query()
        .where_eq("run_id", run.id)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert!(
        search::document_of(
            t.db(),
            &Subject::new(wl_hr::search::PAYROLL_LINE_SUBJECT_TYPE, line.id)
        )
        .await
        .unwrap()
        .is_none()
    );
}
