//! BL-KRN-11 (#459), Done when 5: with wl-payments (and wl-invoicing) off,
//! the transaction-aware variants of the money contracts answer as the
//! no-op fallbacks answer the plain methods, and queue nothing.

use renox::auth::Permissions;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::after_commit::AfterCommit;
use wl_kernel::cheques::{ChequeId, ReturnCheques, cheques};
use wl_kernel::deposits::deposits;
use wl_kernel::invoicing::{CreditLine, InvoiceId, invoicer};
use wl_kernel::{ClientConfig, Subject};

const CLIENT: &str = r#"
[client]
name = "Acme Properties LLC"
app = "property"

[modules]
enabled = []

[[legal_entities]]
code = "ACME"
name_en = "Acme Properties LLC"
default = true
"#;

async fn app() -> TestApp {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(App::new().module(Auth::new()).module(Permissions), config)
        .expect("the kernel applies");
    TestApp::new(app).await
}

fn unprocessable<T>(result: Result<T>) -> bool {
    matches!(result, Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)))
}

#[renox::test]
async fn with_the_modules_off_the_variants_answer_as_the_fallbacks() {
    let app = app().await;
    let state = app.state();
    let lease = Subject::new("property.lease", 9);
    let request = || ReturnCheques::new(vec![ChequeId(1), ChequeId(2)], "Lease terminated");

    // The plain methods' answers.
    assert!(unprocessable(
        cheques(state).return_undeposited(state, request(), None).await
    ));
    assert!(
        deposits(state)
            .start_settlement(state, &lease)
            .await
            .unwrap()
            .is_none()
    );

    // The same answers from the variants, inside a caller's transaction.
    let mut tx = state.db.begin().await.unwrap();
    let mut after = AfterCommit::new();
    assert!(unprocessable(
        cheques(state)
            .return_undeposited_in(state, &mut tx, &mut after, request(), None)
            .await
    ));
    assert!(
        deposits(state)
            .start_settlement_in(state, &mut tx, &mut after, &lease)
            .await
            .unwrap()
            .is_none()
    );
    assert!(unprocessable(
        invoicer(state)
            .credit_partly_in(
                state,
                &mut tx,
                &mut after,
                InvoiceId(4),
                vec![CreditLine::new(1, 319_355)],
                "Rent after the move-out date".into(),
                None,
            )
            .await
    ));
    assert!(
        !invoicer(state)
            .discard_draft_in(&mut tx, InvoiceId(4))
            .await
            .unwrap()
    );
    assert!(after.is_empty(), "nothing to emit");
    tx.commit().await.unwrap();
    after.run(state).await.unwrap();
}
