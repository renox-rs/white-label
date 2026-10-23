//! BL-MPW-34 (#518), `HRM.ACL.05`: the approvals inbox lists the pay accounts
//! that wait in `pending_approval` for a second user. The demo agency has one
//! (Anil Thapa, asked for by HR). Each test opens the inbox as a role of the
//! demo agency and checks the words, the masked number, the separation of
//! duties, the dashboard count and that a decision takes the row away.

mod manpower_screens_support;
mod support;

use manpower_screens_support::{
    ADMIN, HR, MD, SUPERVISOR, demo, employee, fits_a_phone, no_raw_words, page, post, says, scalar,
};
use renox::testing::TestApp;

/// Anil's pay account, waiting for a second user.
async fn anil(app: &TestApp) -> i64 {
    let id = employee(app, "Anil Thapa").await;
    scalar(
        app,
        &format!("select id from wl_hr_pay_accounts where employee_id = {id}"),
    )
    .await
}

/// The status of an account.
async fn status_is(app: &TestApp, id: i64, status: &str) -> bool {
    scalar(
        app,
        &format!("select count(*) from wl_hr_pay_accounts where id = {id} and status = '{status}'"),
    )
    .await
        == 1
}

/// AC1: the MD's inbox says *Approve a new pay account*, the worker, the end
/// of the number (never the whole number), who asked and when, with *Approve*
/// and *Reject*.
#[renox::test]
async fn the_md_sees_the_pending_pay_account_in_words_with_the_number_masked() {
    let app = demo().await;
    let id = anil(&app).await;
    let html = page(&app, MD, "/approvals").await;
    says(
        &html,
        &[
            "Approve a new pay account",
            "Anil Thapa",
            "Account ending",
            "Requested by",
            "Approve",
            "Reject",
            &format!("/hr/pay-accounts/{id}/transitions/approve"),
            &format!("/hr/pay-accounts/{id}/transitions/reject"),
        ],
    );
    let number: String = renox::db::sql(format!(
        "select account_number from wl_hr_pay_accounts where id = {id}"
    ))
    .scalar::<String>(app.db())
    .await
    .unwrap();
    assert!(!html.contains(&number), "the full account number is shown");
    assert!(!html.contains(&number[..number.len() - 4]));
    no_raw_words(&html);
    fits_a_phone(&html);
}

/// AC2: when the holder's name differs from the worker's, the explanation shows.
#[renox::test]
async fn the_explanation_of_a_name_mismatch_is_on_the_row() {
    let app = demo().await;
    let id = anil(&app).await;
    renox::db::sql(format!(
        "update wl_hr_pay_accounts set holder_name = 'Sunita Thapa', \
         mismatch_explanation = 'Salary goes to his wife' where id = {id}"
    ))
    .execute(app.db())
    .await
    .unwrap();
    let html = page(&app, MD, "/approvals").await;
    says(
        &html,
        &[
            "Sunita Thapa",
            "The name differs",
            "Salary goes to his wife",
        ],
    );
}

/// AC3: HR, who asked, sees *Waiting for another approver* and no button; HR
/// does not hold `pay_accounts.approve` in this agency, so a direct approve is 403.
#[renox::test]
async fn the_requester_waits_for_another_approver_and_has_no_button() {
    let app = demo().await;
    let id = anil(&app).await;
    let html = page(&app, HR, "/approvals").await;
    says(
        &html,
        &["Waiting for another approver", "Approve a new pay account"],
    );
    assert!(
        !html.contains("transitions/approve"),
        "a button for the requester"
    );
    post(
        &app,
        HR,
        &format!("/hr/pay-accounts/{id}/transitions/approve"),
        &[],
    )
    .await
    .assert_status(403);
    assert!(status_is(&app, id, "pending_approval").await);
}

/// AC3: separation of duties: an approver who asked for the account himself
/// sees it as waiting for another approver and his own approve is refused with
/// the 422 text, while the admin can decide it.
#[renox::test]
async fn an_approver_who_asked_for_the_account_cannot_approve_it() {
    let app = demo().await;
    let id = anil(&app).await;
    renox::db::sql(format!(
        "update wl_hr_pay_accounts set requested_by = \
         (select id from users where email = '{MD}') where id = {id}"
    ))
    .execute(app.db())
    .await
    .unwrap();
    let html = page(&app, MD, "/approvals").await;
    says(&html, &["Waiting for another approver"]);
    assert!(
        !html.contains("transitions/approve"),
        "a button for the requester"
    );
    let refusal = post(
        &app,
        MD,
        &format!("/hr/pay-accounts/{id}/transitions/approve"),
        &[],
    )
    .await;
    refusal.assert_status(422);
    assert!(
        refusal
            .text()
            .contains("The user who asked for the account cannot approve it."),
        "{}",
        refusal.text()
    );
    assert!(status_is(&app, id, "pending_approval").await);
    let other = page(&app, ADMIN, "/approvals").await;
    says(&other, &["transitions/approve"]);
}

/// AC4 and AC6: the MD approves; the row leaves every inbox, the account is
/// cooling and the dashboard no longer counts it. Before, the dashboard counted it.
#[renox::test]
async fn approving_takes_the_row_out_of_every_inbox_and_the_dashboard() {
    let app = demo().await;
    let id = anil(&app).await;
    let before = page(&app, MD, "/dashboard").await;
    says(
        &before,
        &[
            "1 approval waiting for you",
            "Approve a new pay account: Anil Thapa",
            "href=\"/approvals\"",
        ],
    );
    post(
        &app,
        MD,
        &format!("/hr/pay-accounts/{id}/transitions/approve"),
        &[],
    )
    .await
    .assert_status(303);
    assert!(status_is(&app, id, "cooling").await);
    for who in [MD, ADMIN, HR] {
        let html = page(&app, who, "/approvals").await;
        assert!(
            !html.contains("Approve a new pay account"),
            "still listed for {who}"
        );
    }
    let after = page(&app, MD, "/dashboard").await;
    assert!(!after.contains("waiting for you"), "still counted");
}

/// AC5: reject needs a reason; with one the account is rejected and gone.
#[renox::test]
async fn reject_needs_a_reason_and_removes_the_row() {
    let app = demo().await;
    let id = anil(&app).await;
    let path = format!("/hr/pay-accounts/{id}/transitions/reject");
    post(&app, MD, &path, &[("reason", "")])
        .await
        .assert_status(422);
    assert!(status_is(&app, id, "pending_approval").await);
    post(&app, MD, &path, &[("reason", "The IBAN is not his")])
        .await
        .assert_status(303);
    assert!(status_is(&app, id, "rejected").await);
    let html = page(&app, ADMIN, "/approvals").await;
    assert!(!html.contains("Approve a new pay account"));
}

/// AC6: the admin may decide the same row; AC8: the supervisor sees none.
#[renox::test]
async fn the_admin_may_decide_and_the_supervisor_sees_no_pay_account() {
    let app = demo().await;
    let id = anil(&app).await;
    let html = page(&app, ADMIN, "/approvals").await;
    says(&html, &["Approve a new pay account", "Anil Thapa"]);
    let supervisor = page(&app, SUPERVISOR, "/approvals").await;
    assert!(!supervisor.contains("Approve a new pay account"));
    says(&supervisor, &["Nothing needs your action"]);
    post(
        &app,
        ADMIN,
        &format!("/hr/pay-accounts/{id}/transitions/approve"),
        &[],
    )
    .await
    .assert_status(303);
    assert!(status_is(&app, id, "cooling").await);
}
