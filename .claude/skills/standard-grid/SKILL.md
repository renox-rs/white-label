---
name: standard-grid
description: Build or change any list screen, report, queue or table page so it uses the standard Renox grid (renox::grid::Grid, `<rx-grid>`) with the full baseline (search, sort, filters, pagination, dynamic columns, remembered state, export, summaries) and the desktop "fill the space" rule (`<rx-shell fill>`, `rx-fill`, `rx-panes`). Use whenever a page lists records, adds a screen to a module or vertical, converts an `<rx-table>` list, or when asked to check or review list screens. Also use before writing any new `apps/*` or `crates/wl-*` page that shows rows.
---

# standard-grid

Every screen that lists records is a `renox::grid::Grid` that fills the screen on a desktop. This is the rule of
tracking issue #523 and of the Renox dashboard standard. Read it before writing a list page; check against it before
saying a page is done.

Sources (read them, do not guess the API):
- Renox guide `docs/grid.md` and `docs/ui.md` "The dashboard standard: `<rx-shell fill>`", of the pinned Renox version
  (`host/AGENTS.md` links them; offline: `~/.cargo/registry/src/*/renox-core-*/src/grid/mod.rs`).
- Benchmark, the most complete grids: Renox `examples/bikeshop/src/app/reports/grids.rs` (`report_grid()`), `stock/levels.rs`,
  `rentals/fleet.rs`. Clone with `git clone --depth 1 --branch v1.2.0 https://github.com/renox-rs/renox`.
- In this repo: `/parties` (`crates/wl-parties/src/pages.rs`, `crates/wl-parties/views/wl-parties/index.html`) for a plain
  entity list with role-limited export; `/payments` and `/invoices` for computed columns and badges.

## When it applies

A page whose main content is rows of records: lists, queues, inboxes of tasks, logs, reports with a table, reference lists
(categories, kinds, settings tables). Not in scope: a table of lines inside one record's detail page (it stays `rx-table`),
phone-only screens (the driver's stops), and matrix cells (cash-flow forecast). A detail page still follows the fill rule.

## 1. The handler

```rust
let page = grid(&lang, can_export).page(Model::query(), &request).await?;   // request: GridRequest
Ok(view("wl-x/index.html", context! { rows => page }))
```

Build the `Grid` in one function `grid(..)` so tests and export reuse it. Scope the query exactly as the page did before
(legal entity, branch, own records, portal); the grid adds search, sort and paging on top of it, never replaces the scope.
No `LIMIT` that hides older rows.

## 2. The baseline: all 13 or a one-line reason in the PR

| # | Mechanism | Call |
|---|---|---|
| 1 | Stable unique id | `Grid::new("<module>-<screen>")` (the per-user column choice is stored under it) |
| 2 | Page size | `.per_page(25)` |
| 3 | Remembered state | `.remember()` |
| 4 | Advanced filter | `.advanced_filter()` |
| 5 | Cards on phones | `.cards_on_mobile()` and `.mobile()` on 3 to 4 key columns |
| 6 | Export | `.exports()` only for roles that may export (`can_export`); no sensitive column in it |
| 7 | Empty state | `.empty_state(&t("empty"), Some(&t("empty_hint")))`, translated, says what creates the first row |
| 8 | Default sort | `.sort_by("-created_at")` or by number, explicit |
| 9 | Search | `.searchable()` on every text column people look a record up by |
| 10 | Column choice | identifier `.frozen().link(..)`; secondary columns `.hidden()`, not dropped |
| 11 | Typed columns | `Column::money / date / datetime / select / bool / quantity`; statuses `.badges(&[..])` |
| 12 | Row link | `.row_url("/x/{id}")` |
| 13 | Fill the space | `<rx-grid>` direct child of `<rx-page>` (section 3) |

On top, where the data allows: `.summary(Summary::Sum | Count)` on money and quantity columns of financial, stock and report
screens; `.groups(..)` / `.under([..])` on reports and long lists with a natural grouping; `.poll(..)` on live boards
(kitchen, dispatch, call lists). Existing page filters (entity, status, location) become grid filters, or feed the same query
and still work in the URL. Row actions that existed (approve, call, mark) stay (custom column or row action).

## 3. Fill the space on desktop

The layout `host/resources/views/layouts/app.html` is `<rx-shell fill>`. On a desktop (>= 1024 px wide, >= 576 px tall) the
document never scrolls, every panel scrolls by itself, and one element takes the height left.

```html
<rx-page layout="layouts/app.html" :title="tx['title']">
  <rx-page-header :title="tx['title']">…header actions…</rx-page-header>
  <rx-grid :page="rows"/>
</rx-page>
```

Rules:
- The `<rx-grid>` is a **direct child of `<rx-page>`**, after the header. **Never wrap it** in `<rx-stack>`, `<rx-card>`, a `div`
  with a height, or `card`; a wrapper stops it filling. The old pattern `<rx-page><rx-stack><rx-page-header/><rx-table card>`
  is exactly what to remove.
- Filters, tabs and figures above the grid are small and live in the header or a toolbar. Nothing goes below the grid; put
  secondary blocks in a side panel or a tab.
- Two blocks side by side: `<div class="rx-panes rx-fill" style=…--rx-panes: 2fr 1fr>` with `<section class="rx-pane">`
  children, each scrolling by itself. One panel: `<section class="rx-pane rx-fill">`. A panel without data holds `<rx-empty>`.
- Detail pages (a record with sections) are not grids but use the same: `rx-panes rx-fill`, not a free-flowing stack.
- No fixed heights, no viewport CSS per page, no inline `<script>` or `style=` (they break under `CSP=strict`).
- Tablets and phones scroll as a document by themselves; do not special-case them.

## 4. Verify (do not say done without this)

1. `rnx view:check` (or the project's view test) passes; `rnx db:check` if a model changed.
2. Start the client that shows the screen (`host/AGENTS.md` / README: `CLIENT_CONFIG=clients/<demo>/client.toml`,
   `migrate:fresh --seed`, `serve`) and run `.claude/skills/standard-grid/check-fill.js <base-url> <path>...`
   (Playwright is preinstalled; `node check-fill.js http://localhost:3001 /property/leases`). It logs in as
   `admin@example.com`, opens each path at 1440x900 and 390x844 and fails if: the document scrolls on desktop, the grid is not
   a direct child of the page content, the grid does not reach the bottom of the window, search, a page-size choice or column
   menu are missing, or the page scrolls sideways on the phone.
3. Look at the screenshot the script writes (`/tmp/standard-grid/<name>.png`) at desktop and phone width, light and dark, and
   in Arabic if the client has it.
4. Add or extend a test that opens the screen, searches, sorts, changes page and page size, and checks the row count and
   that the grid is a direct child of the page content.
5. List the mechanisms from section 2 you left out, each with its reason, in the PR or the issue.

## Common mistakes

- Keeping `<rx-table>` and calling it a grid because it has a filter form.
- Wrapping the grid in `rx-card` "for the look": the grid already has its own surface.
- Putting a second table, a chart or a "roles" block under the grid (this is what `/users` does today: move it).
- Dropping a column to make room instead of `.hidden()`.
- Exporting columns the role may not see on screen.
- Reading `GridRequest` but building the query without it, so search or paging is ignored.
- A `LIMIT 200` in the query (the notification log).
