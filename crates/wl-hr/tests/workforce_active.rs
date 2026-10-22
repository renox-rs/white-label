//! Base 09 (`Workforce::is_active`): the kernel asks wl-hr whether an
//! employee is `active` now, which a deployment `activate` needs (09
//! Manpower §7.1 *Deployment*). Onboarding, notice, an unknown employee and
//! one who is gone are not active.

mod support;

use wl_hr::EmployeeStatus;
use wl_kernel::workforce::{EmployeeId, workforce};

use support::boot;

/// Only an `active` employee is active; every other status and an unknown
/// employee answer no.
#[renox::test]
async fn only_an_active_employee_is_active() {
    let t = boot().await;
    let book = workforce(t.state());
    for (party, status, expected) in [
        (1, EmployeeStatus::Active, true),
        (2, EmployeeStatus::Onboarding, false),
        (3, EmployeeStatus::Notice, false),
        (4, EmployeeStatus::OnLeave, false),
        (5, EmployeeStatus::Exited, false),
    ] {
        let employee = t.employee_in(party, status).await;
        assert_eq!(
            book.is_active(t.state(), EmployeeId(employee.id))
                .await
                .unwrap(),
            expected,
            "{status:?}"
        );
    }
    assert!(!book.is_active(t.state(), EmployeeId(9_999)).await.unwrap());
}
