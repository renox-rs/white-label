//! BL-MPW-33 (#513): the Manpower screens in Arabic. The demo agency is booted
//! with every request in Arabic (the way a person whose language is Arabic
//! sees it); the Manpower lists and record pages then read in Arabic, mirror
//! (`dir="rtl"`), keep the names as entered and the digits Western, and a
//! refusal of a service is in Arabic too.
//!
//! The host has no `resources/lang/ar.json` yet, so the sidebar, the kit's own
//! words and the texts of the shared modules (wl-hr, wl-attendance,
//! wl-invoicing) stay English here: those are not the vertical's texts and
//! are not checked.

mod manpower_screens_support;
mod support;

use manpower_screens_support::{
    AMAL, HR, client, contract, day, no_raw_words, person, scalar, visible,
};
use renox::axum::extract::Request;
use renox::axum::middleware::{Next, from_fn};
use renox::axum::response::Response;
use renox::testing::{TestApp, TestResponse};
use wl_kernel::ClientConfig;

use crate::support::noon_clock;

/// The demo client of the Manpower vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-manpower/client.toml"
);

/// Makes every request Arabic, as the language middleware does for a person
/// who chose Arabic.
async fn arabic(mut request: Request, next: Next) -> Response {
    renox::i18n::RequestLocale::switch(&mut request, "ar");
    next.run(request).await
}

/// The demo agency, seeded, answering in Arabic.
async fn demo_in_arabic() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Manpower vertical");
    let app = TestApp::new(
        host::app_for(config)
            .expect("the app")
            .provide(noon_clock())
            .layer(from_fn(arabic)),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    host::manpower_demo::seed(app.kernel()).await.unwrap();
    app
}

/// A GET as `email` the way a browser sends it; the page must answer 200.
async fn page(app: &TestApp, email: &str, path: &str) -> String {
    let user = person(app, email).await;
    app.acting_as(&user);
    let response: TestResponse = app
        .request()
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .get(path)
        .await;
    assert_eq!(response.status.as_u16(), 200, "{path}: {}", response.text());
    response.text()
}

/// Asserts that `html` is an Arabic page laid out right to left, says each of
/// `arabic`, and none of the vertical's English `english` texts.
fn reads_in_arabic(html: &str, arabic: &[&str], english: &[&str]) {
    assert!(html.contains("dir=\"rtl\""), "the page does not mirror");
    assert!(
        html.contains("lang=\"ar\""),
        "the page is not marked Arabic"
    );
    let text = visible(html)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for word in arabic {
        assert!(text.contains(word), "the page does not say `{word}`");
    }
    for word in english {
        assert!(
            !text.contains(word),
            "the page still says `{word}` in English"
        );
    }
    no_raw_words(html);
}

/// AC1, AC4: the lists and record pages the account manager and HR open read
/// in Arabic and mirror; references, names and dates stay as entered, in
/// Western digits.
#[renox::test]
async fn the_manpower_pages_read_in_arabic_and_mirror() {
    let app = demo_in_arabic().await;

    let contracts = page(&app, AMAL, "/manpower/contracts").await;
    reads_in_arabic(
        &contracts,
        &["عقود العملاء", "الاتفاقيات مع الشركات العميلة.", "عقد جديد"],
        &["The agreements with the client companies.", "New contract"],
    );
    // The reference and the client's name are shown as entered.
    assert!(contracts.contains("CC-DEMO-001"));
    assert!(contracts.contains("Gulf Tower Facilities LLC"));

    let id = contract(&app, "CC-DEMO-001").await;
    let record = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    reads_in_arabic(
        &record,
        &["نظرة عامة", "جداول الأسعار", "الموافقات", "السجل"],
        &["Overview", "Timesheet deemed approved"],
    );
    assert!(record.contains("CC-DEMO-001"));

    let requests = page(&app, AMAL, "/manpower/requests").await;
    reads_in_arabic(
        &requests,
        &["العمال الذين يطلبهم العملاء.", "طلب جديد"],
        &["The workers the clients ask for.", "New request"],
    );
    let request = scalar(&app, "select min(id) from manpower_requests").await;
    let request_page = page(&app, AMAL, &format!("/manpower/requests/{request}")).await;
    reads_in_arabic(
        &request_page,
        &["مشغولة"],
        &["Why this request exists", " filled, "],
    );

    let deployments = page(&app, AMAL, "/manpower/deployments").await;
    reads_in_arabic(
        &deployments,
        &["العمال المنتشرون في مواقع العملاء."],
        &["The workers placed at the client sites."],
    );
    let deployment = scalar(&app, "select min(id) from manpower_deployments").await;
    let deployment_page = page(&app, AMAL, &format!("/manpower/deployments/{deployment}")).await;
    reads_in_arabic(
        &deployment_page,
        &["المستندات"],
        &["Why it cannot start yet"],
    );

    let absence = page(&app, HR, "/manpower/absence").await;
    reads_in_arabic(
        &absence,
        &["أيام الغياب"],
        &["Workers who have not turned up.", "Days absent"],
    );
    let case = scalar(&app, "select min(id) from manpower_absence_cases").await;
    let case_page = page(&app, HR, &format!("/manpower/absence/{case}")).await;
    reads_in_arabic(
        &case_page,
        &["محاولات التواصل", "بلاغ وزارة الموارد البشرية والتوطين"],
        &["Contact attempts", "Record report"],
    );
}

/// AC2: a refusal of a service (a new contract that ends before it starts) is
/// shown in Arabic, and nothing is made.
#[renox::test]
async fn a_refusal_is_shown_in_arabic() {
    let app = demo_in_arabic().await;
    let tower = client(&app, "Gulf Tower Facilities LLC").await;
    let (start, end) = (day(30).to_string(), day(10).to_string());
    let user = person(&app, AMAL).await;
    app.acting_as(&user);
    let refused = app
        .htmx()
        .post(
            "/manpower/contracts",
            &[
                ("client_party_id", tower.0.to_string().as_str()),
                ("reference", "CC-NEW-AR"),
                ("start_date", start.as_str()),
                ("end_date", end.as_str()),
            ],
        )
        .await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(
        body.contains("يجب أن يكون تاريخ الانتهاء بعد تاريخ البدء."),
        "{body}"
    );
    assert!(
        !body.contains("The end date must be after the start date"),
        "{body}"
    );
    assert_eq!(
        scalar(&app, "select count(*) from manpower_client_contracts").await,
        3,
        "nothing was made"
    );
}
