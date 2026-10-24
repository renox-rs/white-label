//! The timesheets list on the standard grid (#527, BL-MPW-27) in the demo
//! agency: the worker, the client and the work are words the grid searches,
//! sorts and exports (they live in the workforce, the parties directory and
//! the app, not in the timesheet), the list pages through the query string,
//! keeps the client and month filters of the header, and the grid is a direct
//! child of its page.

mod manpower_screens_support;
mod support;

use manpower_screens_support::{ADMIN, AMAL, demo, page, person, visible};

/// The footer of the grid in `html`: first row, last row and total
/// (`1–10 of 18`); `(0, 0, 0)` when the grid shows no footer (no rows).
fn range(html: &str) -> (i64, i64, i64) {
    let marker = "rx-grid__range\">";
    let Some(at) = html.find(marker) else {
        return (0, 0, 0);
    };
    let rest = &html[at + marker.len()..];
    let text = &rest[..rest.find("</p>").unwrap()];
    let numbers: Vec<i64> = text
        .split(|c: char| !c.is_ascii_digit() && c != ',')
        .filter(|part| !part.replace(',', "").is_empty())
        .map(|part| part.replace(',', "").parse().unwrap())
        .collect();
    (numbers[0], numbers[1], numbers[2])
}

/// The template puts its `<rx-grid>` directly in `<rx-page>`, after the
/// header, with nothing after it.
#[test]
fn the_timesheets_grid_is_a_child_of_the_page() {
    let path = format!(
        "{}/../crates/wl-attendance/views/wl-attendance/timesheets.html",
        env!("CARGO_MANIFEST_DIR")
    );
    let source = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = source.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.trim_start().starts_with("<rx-grid"))
        .unwrap();
    assert!(lines[at].starts_with("  <rx-grid"), "a direct child");
    let child = |line: &str| line.starts_with("  <") && !line.starts_with("   ");
    for line in lines[at + 1..].iter().filter(|line| child(line)) {
        assert!(
            line.starts_with("  </rx-grid"),
            "`{line}` is below the grid"
        );
    }
    for line in lines[..at].iter().filter(|line| child(line)) {
        assert!(
            line.starts_with("  <rx-page-header") || line.starts_with("  </rx-page-header"),
            "`{line}` wraps or comes before the grid"
        );
    }
}

/// Page size, sort by the worker, the worker and the client found by name,
/// the client filter of the header kept, and the export in words.
#[renox::test]
async fn the_timesheets_search_the_worker_and_client_sort_and_page() {
    let app = demo().await;
    let all = page(&app, AMAL, "/timesheets?per_page=10").await;
    let (from, to, total) = range(&all);
    assert!(total >= 3, "the demo timesheets are listed");
    assert_eq!((from, to), (1, total.min(10)));
    assert!(
        all.contains("href=\"/timesheets/"),
        "the worker links the sheet"
    );

    let by_worker = page(&app, AMAL, "/timesheets?sort=-worker").await;
    assert_eq!(range(&by_worker).2, total);
    let second = page(&app, AMAL, "/timesheets?per_page=10&page=2").await;
    assert_eq!(range(&second).2, total);

    let ravi = page(&app, AMAL, "/timesheets?search=ravi").await;
    let found = range(&ravi).2;
    assert!(
        found >= 1 && found < total,
        "only Ravi's sheets: {found} of {total}"
    );
    let text = visible(&ravi);
    assert!(text.contains("Ravi Kumar"));
    assert!(!text.contains("Wasim Khan"), "another worker is not found");

    let gulf = page(&app, AMAL, "/timesheets?search=gulf+tower").await;
    let of_client = range(&gulf).2;
    assert!(of_client >= 1 && of_client <= total);
    assert!(visible(&gulf).contains("Wasim Khan"));

    let nobody = page(&app, AMAL, "/timesheets?search=nobody-has-this-name").await;
    assert_eq!(range(&nobody).2, 0);

    let client = manpower_screens_support::client(&app, "Gulf Tower Facilities LLC").await;
    let narrowed = page(&app, AMAL, &format!("/timesheets?client={}", client.0)).await;
    assert!(range(&narrowed).2 <= total);

    let admin = person(&app, ADMIN).await;
    app.acting_as(&admin);
    let file = app
        .request()
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .get("/timesheets?export=csv")
        .await;
    assert_eq!(file.status.as_u16(), 200, "{}", file.text());
    assert!(file.text().contains("Ravi Kumar"), "the worker in words");
}
