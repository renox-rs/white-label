//! Words, not keys, on the pages of a manpower agency (the browser test report
//! of the demo): the documents page of a worker names the worker, not
//! `hr.employee #2`.

mod manpower_support;
mod support;

use host::roles::ADMIN;
use manpower_support::{boot, worker};
use support::person;

/// The documents of a worker are titled with the worker's name.
#[renox::test]
async fn the_documents_page_of_a_worker_shows_the_workers_name() {
    let app = boot().await;
    let admin = person(&app, "Rana", ADMIN).await;
    let w = worker(&app, "Wasim Khan").await;
    app.acting_as(&admin);
    let page = app.get(&format!("/documents/of/hr.employee/{}", w.0)).await;
    page.assert_ok();
    page.assert_see("Wasim Khan");
    page.assert_dont_see("hr.employee #");
}
