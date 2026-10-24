//! What the grid tests of the verticals share: the page as a browser gets
//! it, the grid's footer (`1–10 of 18`), a query-string value, and the check
//! that a template puts its grid directly in the page.

#![allow(dead_code)]

use renox::testing::TestApp;

/// The page at `path` as a browser gets it; it must answer 200.
pub async fn load(app: &TestApp, path: &str) -> String {
    let page = app
        .request()
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .get(path)
        .await;
    assert_eq!(page.status.as_u16(), 200, "{path}: {}", page.text());
    page.text()
}

/// The footer of the grid in `html`: first row, last row and total
/// (`1–10 of 18`); `(0, 0, 0)` when the grid shows no footer (no rows).
pub fn range(html: &str) -> (i64, i64, i64) {
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

/// The total rows the grid at `path` counts.
pub async fn total(app: &TestApp, path: &str) -> i64 {
    range(&load(app, path).await).2
}

/// `text` for a query string.
pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The template `template` (under `apps/`) puts its `<rx-grid>` directly in
/// `<rx-page>`, after the header, with nothing but dialogs after it.
pub fn grid_is_a_child_of_the_page(template: &str) {
    let path = format!("{}/../apps/{template}", env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{path}"));
    let lines: Vec<&str> = source.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.trim_start().starts_with("<rx-grid"))
        .unwrap_or_else(|| panic!("{template} has no grid"));
    assert!(
        lines[at].starts_with("  <rx-grid"),
        "{template}: the grid is a direct child of <rx-page>"
    );
    let child = |line: &str| line.starts_with("  <") && !line.starts_with("   ");
    for line in lines[..at].iter().filter(|line| child(line)) {
        assert!(
            ["  <rx-page-header", "  <rx-include", "  </rx-page-header"]
                .iter()
                .any(|ok| line.starts_with(ok)),
            "{template}: `{line}` wraps or comes before the grid"
        );
    }
    for line in lines[at + 1..].iter().filter(|line| child(line)) {
        assert!(
            [
                "  </rx-grid",
                "  <rx-sheet",
                "  </rx-sheet",
                "  <rx-confirm",
                "  <rx-action-sheet",
                "  </rx-action-sheet",
                "  <template",
                "  </template",
            ]
            .iter()
            .any(|ok| line.starts_with(ok)),
            "{template}: `{line}` is below the grid"
        );
    }
}
