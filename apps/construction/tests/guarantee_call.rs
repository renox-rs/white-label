//! BL-CON-10 (#289): a call on a guarantee and its settlement. The accountant
//! records the call with the demand notice and amount; the MD, project manager
//! and accountant are told at once and the accountant gets a task to respond.
//! `settle_call` takes the debit advice (or, for a call withdrawn before
//! payment, amount 0 and the withdrawal letter); the paid amount is booked to
//! the called-guarantees cost code, a partial call leaves the guarantee
//! `active` for the rest and a disputed call is a claim of the final account.

mod support;

use construction::call::{self, COST_CODE};
use construction::expiry;
use construction::guarantee::documents_of;
use construction::{Guarantee, GuaranteeKind, GuaranteeStatus, Project, ProjectStatus};
use wl_kernel::tickets::Assignee;

use support::{Booted, boot, date, users_of};

/// AED in fils.
const fn aed(amount: i64) -> i64 {
    amount * 100
}

fn path(guarantee: &Guarantee, step: &str) -> String {
    format!("/construction/guarantees/{}/{step}", guarantee.id)
}

/// An `active` performance bond of AED 500,000 expiring on `expires_on`.
async fn bond(t: &Booted, project: &Project, expires_on: renox::chrono::NaiveDate) -> Guarantee {
    t.guarantee(
        project,
        GuaranteeKind::Performance,
        aed(500_000),
        GuaranteeStatus::Active,
        Some(expires_on),
    )
    .await
}

/// Done when 1: `call` for AED 200,000 with the demand notice makes the bond
/// `called`, tells the MD, the project manager and the accountant at once and
/// gives the accountant a task to respond.
#[renox::test]
async fn a_call_is_recorded_and_everyone_concerned_is_told_at_once() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let md = t.md("Omar").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let held = bond(&t, &project, date(2027, 9, 30)).await;
    let call_path = path(&held, "call");

    // The demand notice and an amount within the guarantee are needed, and
    // only the accountant records it.
    t.post(
        &accountant,
        &call_path,
        &[("amount", &aed(200_000).to_string()), ("document", " ")],
    )
    .await
    .assert_status(422);
    t.post(
        &accountant,
        &call_path,
        &[
            ("amount", &aed(500_001).to_string()),
            ("document", "DEMAND-1.pdf"),
        ],
    )
    .await
    .assert_status(422);
    t.post(
        &accountant,
        &call_path,
        &[("amount", "0"), ("document", "DEMAND-1.pdf")],
    )
    .await
    .assert_status(422);
    t.post(
        &pm,
        &call_path,
        &[
            ("amount", &aed(200_000).to_string()),
            ("document", "DEMAND-1.pdf"),
        ],
    )
    .await
    .assert_status(403);
    assert_eq!(
        t.reload_guarantee(held.id).await.status,
        GuaranteeStatus::Active
    );
    assert!(t.notices.urgent_titled("called").is_empty());

    t.post(
        &accountant,
        &call_path,
        &[
            ("amount", &aed(200_000).to_string()),
            ("document", "DEMAND-1.pdf"),
        ],
    )
    .await
    .assert_ok();
    let called = t.reload_guarantee(held.id).await;
    assert_eq!(called.status, GuaranteeStatus::Called);
    assert_eq!(called.amount, aed(500_000), "nothing is paid yet");
    let calls = call::calls_of(t.db(), held.id).await.unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].amount, aed(200_000));
    assert_eq!(calls[0].demand_notice, "DEMAND-1.pdf");
    assert_eq!(calls[0].called_by, Some(accountant.id));
    assert!(!calls[0].is_settled());
    assert!(
        t.costs.posted().is_empty(),
        "no cost before the debit advice"
    );
    let documents = documents_of(t.db(), held.id).await.unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].transition, "call");

    // At once: one urgent notice to the three, not the digest's notice.
    let told = t.notices.urgent_titled("called");
    assert_eq!(told.len(), 1);
    let mut users = users_of(&told[0]);
    users.sort_unstable();
    let mut wanted = vec![md.id, pm.id, accountant.id];
    wanted.sort_unstable();
    assert_eq!(users, wanted);
    assert!(
        told[0].data["title"]
            .as_str()
            .unwrap()
            .contains("AED 200,000.00")
    );
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("DEMAND-1.pdf")
    );
    assert!(t.notices.notices_titled("called").is_empty());

    // The task to respond is the accountant's.
    let tasks = t.tasks.with_key("construction.guarantee_call:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].ticket.assignee,
        Some(Assignee::user(accountant.id))
    );
    assert!(tasks[0].ticket.title.contains("Respond to the call"));
    assert_eq!(tasks[0].cancelled, None);

    // A guarantee that is called cannot be called again.
    t.post(
        &accountant,
        &call_path,
        &[
            ("amount", &aed(1_000).to_string()),
            ("document", "DEMAND-2.pdf"),
        ],
    )
    .await
    .assert_status(422);
}

/// Done when 2, 3 and 4: `settle_call` with the debit advice for AED 200,000
/// books AED 200,000 to the called-guarantees cost code and leaves the bond
/// `active` at AED 300,000; a full call settled for AED 500,000 makes it
/// `released`; marked disputed, the AED 200,000 is a claim of the final
/// account.
#[renox::test]
async fn a_partial_call_is_booked_and_leaves_the_rest_a_full_call_releases_it() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let held = bond(&t, &project, date(2027, 9, 30)).await;

    t.post(
        &accountant,
        &path(&held, "call"),
        &[
            ("amount", &aed(200_000).to_string()),
            ("document", "DEMAND-1.pdf"),
        ],
    )
    .await
    .assert_ok();
    let settle = path(&held, "settle-call");

    // The debit advice is needed, and no more than the call is paid.
    t.post(
        &accountant,
        &settle,
        &[("amount", &aed(200_000).to_string()), ("document", " ")],
    )
    .await
    .assert_status(422);
    t.post(
        &accountant,
        &settle,
        &[
            ("amount", &aed(200_001).to_string()),
            ("document", "DEBIT-ADVICE-1.pdf"),
        ],
    )
    .await
    .assert_status(422);
    t.post(
        &pm,
        &settle,
        &[
            ("amount", &aed(200_000).to_string()),
            ("document", "DEBIT-ADVICE-1.pdf"),
        ],
    )
    .await
    .assert_status(403);
    assert!(t.costs.posted().is_empty());
    assert_eq!(
        t.reload_guarantee(held.id).await.status,
        GuaranteeStatus::Called
    );

    t.post(
        &accountant,
        &settle,
        &[
            ("amount", &aed(200_000).to_string()),
            ("document", "DEBIT-ADVICE-1.pdf"),
        ],
    )
    .await
    .assert_ok();
    let rest = t.reload_guarantee(held.id).await;
    assert_eq!(rest.status, GuaranteeStatus::Active);
    assert_eq!(rest.amount, aed(300_000));
    assert_eq!(rest.expires_on, Some(date(2027, 9, 30)), "the expiry stays");
    let posted = t.costs.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].amount_aed, aed(200_000));
    assert_eq!(posted[0].cost_code.as_deref(), Some(COST_CODE));
    assert_eq!(posted[0].cost_object, project.subject());
    let calls = call::calls_of(t.db(), held.id).await.unwrap();
    assert_eq!(calls[0].amount_paid, Some(aed(200_000)));
    assert_eq!(
        calls[0].settlement_document.as_deref(),
        Some("DEBIT-ADVICE-1.pdf")
    );
    assert!(calls[0].is_settled());
    assert!(calls[0].booked_at.is_some());
    // The task to respond is closed.
    let tasks = t.tasks.with_key("construction.guarantee_call:");
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].cancelled.is_some());
    let documents = documents_of(t.db(), held.id).await.unwrap();
    assert_eq!(
        documents
            .iter()
            .map(|document| document.transition.as_str())
            .collect::<Vec<_>>(),
        ["call", "settle_call"]
    );

    // The project manager disputes it: a claim of the final account. The
    // accountant may not.
    assert!(
        call::claims_of(t.db(), project.id)
            .await
            .unwrap()
            .is_empty()
    );
    let dispute = format!("/construction/guarantee-calls/{}/dispute", calls[0].id);
    t.post(&accountant, &dispute, &[]).await.assert_status(403);
    t.post(&pm, &dispute, &[]).await.assert_ok();
    let claims = call::claims_of(t.db(), project.id).await.unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].amount_paid, Some(aed(200_000)));
    let closing = t
        .get(
            &pm,
            &format!("/construction/projects/{}/closing", project.id),
        )
        .await;
    closing.assert_ok();
    let view: renox::serde_json::Value = renox::serde_json::from_str(&closing.text()).unwrap();
    assert_eq!(
        view["guarantee_call_claims"][0]["amount_paid"],
        aed(200_000)
    );
    assert_eq!(
        view["guarantee_call_claims"][0]["demand_notice"],
        "DEMAND-1.pdf"
    );

    // A second call for the rest, paid in full, releases the bond.
    t.post(
        &accountant,
        &path(&held, "call"),
        &[
            ("amount", &aed(300_000).to_string()),
            ("document", "DEMAND-2.pdf"),
        ],
    )
    .await
    .assert_ok();
    t.post(
        &accountant,
        &settle,
        &[
            ("amount", &aed(300_000).to_string()),
            ("document", "DEBIT-ADVICE-2.pdf"),
        ],
    )
    .await
    .assert_ok();
    let released = t.reload_guarantee(held.id).await;
    assert_eq!(released.status, GuaranteeStatus::Released);
    assert_eq!(t.costs.posted().len(), 2);
    assert_eq!(t.costs.posted()[1].amount_aed, aed(300_000));
    // Not disputed: only the first call is a claim.
    assert_eq!(call::claims_of(t.db(), project.id).await.unwrap().len(), 1);

    // A call of the whole AED 500,000 paid in full on another bond.
    let whole = bond(&t, &project, date(2027, 9, 30)).await;
    t.post(
        &accountant,
        &path(&whole, "call"),
        &[
            ("amount", &aed(500_000).to_string()),
            ("document", "DEMAND-3.pdf"),
        ],
    )
    .await
    .assert_ok();
    // Paid in full, the bond is released with its amount kept.
    t.post(
        &accountant,
        &path(&whole, "settle-call"),
        &[
            ("amount", &aed(500_000).to_string()),
            ("document", "DEBIT-ADVICE-3.pdf"),
        ],
    )
    .await
    .assert_ok();
    let done = t.reload_guarantee(whole.id).await;
    assert_eq!(done.status, GuaranteeStatus::Released);
    assert_eq!(done.amount, aed(500_000));
    assert_eq!(t.costs.posted().len(), 3);
}

/// Done when 5: a bond expiring 2027-08-31 and `called` on 2027-08-20 is still
/// `called` on 2027-09-01; after a partial `settle_call` on 2027-09-10 it is
/// `active` and becomes `expired` on 2027-09-11, with the accountant and the
/// project manager notified. No extension task is made while it is `called`.
#[renox::test]
async fn a_called_guarantee_does_not_expire_until_it_is_settled() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let held = bond(&t, &project, date(2027, 8, 31)).await;

    t.on(date(2027, 8, 20), async {
        t.post(
            &accountant,
            &path(&held, "call"),
            &[
                ("amount", &aed(200_000).to_string()),
                ("document", "DEMAND-1.pdf"),
            ],
        )
        .await
        .assert_ok();
    })
    .await;
    assert!(
        t.tasks
            .with_key("construction.guarantee_extension:")
            .is_empty()
    );

    t.on(date(2027, 9, 1), async {
        let run = expiry::run_on(t.state(), date(2027, 9, 1)).await.unwrap();
        assert!(run.expired.is_empty());
        assert!(run.reminded.is_empty());
    })
    .await;
    assert_eq!(
        t.reload_guarantee(held.id).await.status,
        GuaranteeStatus::Called
    );
    assert!(t.notices.notices_titled("expired").is_empty());

    t.on(date(2027, 9, 10), async {
        t.post(
            &accountant,
            &path(&held, "settle-call"),
            &[
                ("amount", &aed(200_000).to_string()),
                ("document", "DEBIT-ADVICE-1.pdf"),
            ],
        )
        .await
        .assert_ok();
    })
    .await;
    let back = t.reload_guarantee(held.id).await;
    assert_eq!(back.status, GuaranteeStatus::Active);
    assert_eq!(back.amount, aed(300_000));

    t.on(date(2027, 9, 11), async {
        expiry::daily(t.state()).await.unwrap();
    })
    .await;
    assert_eq!(
        t.reload_guarantee(held.id).await.status,
        GuaranteeStatus::Expired
    );
    let told = t.notices.notices_titled("expired");
    assert_eq!(told.len(), 2, "the accountant and the project manager");
    let mut users: Vec<i64> = told.iter().flat_map(users_of).collect();
    users.sort_unstable();
    let mut wanted = vec![accountant.id, pm.id];
    wanted.sort_unstable();
    assert_eq!(users, wanted);
}

/// Done when 6: a call withdrawn before payment, settled with amount 0 and the
/// withdrawal letter, returns the bond to `active` at AED 500,000 and books
/// nothing; without the letter it is refused.
#[renox::test]
async fn a_call_withdrawn_before_payment_is_settled_with_the_withdrawal_letter() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let accountant = t.accountant("Aisha").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let held = bond(&t, &project, date(2027, 9, 30)).await;
    t.post(
        &accountant,
        &path(&held, "call"),
        &[
            ("amount", &aed(200_000).to_string()),
            ("document", "DEMAND-1.pdf"),
        ],
    )
    .await
    .assert_ok();
    let settle = path(&held, "settle-call");

    let refused = t
        .post(&accountant, &settle, &[("amount", "0"), ("document", "")])
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("withdrawal letter"),
        "{}",
        refused.text()
    );
    assert_eq!(
        t.reload_guarantee(held.id).await.status,
        GuaranteeStatus::Called
    );

    t.post(
        &accountant,
        &settle,
        &[("amount", "0"), ("document", "WITHDRAWAL-LETTER-1.pdf")],
    )
    .await
    .assert_ok();
    let back = t.reload_guarantee(held.id).await;
    assert_eq!(back.status, GuaranteeStatus::Active);
    assert_eq!(back.amount, aed(500_000));
    assert!(t.costs.posted().is_empty(), "nothing is booked");
    let calls = call::calls_of(t.db(), held.id).await.unwrap();
    assert!(calls[0].is_withdrawn());
    assert_eq!(calls[0].booked_at, None);
    // Not a claim, even when marked disputed: nothing was paid.
    t.post(
        &pm,
        &format!("/construction/guarantee-calls/{}/dispute", calls[0].id),
        &[],
    )
    .await
    .assert_ok();
    assert!(
        call::claims_of(t.db(), project.id)
            .await
            .unwrap()
            .is_empty()
    );
    // It can be called again.
    t.post(
        &accountant,
        &path(&held, "call"),
        &[
            ("amount", &aed(100_000).to_string()),
            ("document", "DEMAND-2.pdf"),
        ],
    )
    .await
    .assert_ok();
    assert_eq!(call::calls_of(t.db(), held.id).await.unwrap().len(), 2);
}
