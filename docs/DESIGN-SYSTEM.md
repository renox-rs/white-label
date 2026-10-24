# White-Label Design System

> Status: **accepted** (2026-10-14, updated 2026-10-24): teal + terracotta as the platform default, Noto Sans Arabic (shipped by Renox 1.2) for Arabic; IBM Plex Sans Arabic and Noto Kufi Arabic were considered and are not adopted. Reference app: the Renox bike shop
> ([bikeshop.renox.rs](https://bikeshop.renox.rs), source in `renox/examples/bikeshop`).

**Build status (2026-10-24):** Built: the branding keys of section 4 (checked against `crates/wl-kernel/src/config.rs`), the status tones of section 6 through `rx-badge`, the standard list grid and page fill (section 11). Not built: the `wl-` components of section 7 (`wl-status`, `wl-money`, `wl-party-card` ...); no `<wl-` tag exists in any crate, app or host template.

## 1. Decision

The platform uses the **Renox "editorial" theme**, the look of the bike shop, as its base. In
Renox 1.1 that look became the kit's default theme (`renox-ui.css`, #344), so we inherit it
instead of building our own.

Three rules follow:

1. **Never fork the kit.** We do not run `rnx make:component --ui`. A forked kit stops
   receiving Renox updates, for every client at once.
2. **Rebrand with tokens only.** A client's look is a small set of `--rx-*` overrides,
   generated from `client.toml`/settings (see [§4](#4-client-branding)).
3. **No hard-coded values in module CSS.** Every colour, radius, space, shadow and font in
   `wl-*` and app CSS is a `var(--rx-*)` token. This is what makes a rebrand, dark mode and
   RTL work for free.

## 2. The look

What we take from the bike shop:

| Element | Editorial theme |
|---|---|
| Page | Warm cream background (`--rx-bg #f6f1e8`), white surfaces with a hairline (`--rx-separator`) and a soft shadow |
| Accent | Teal `--rx-accent #0b6e66` for actions, links and focus |
| Second accent | Terracotta `--rx-accent-2 #b8532e`, used sparingly: one word in a headline, an eyebrow, a badge |
| Supporting tones | Sand `--rx-tint #ece4d5` (bands, photo backgrounds), soft teal `--rx-accent-soft #dcebe7` (icon circles), ink `--rx-ink #1f2421` (dark footer band) |
| Type | Inter for text, Poppins for titles and figures. Large headlines with one coloured phrase (`<em>`) |
| Shape | Pill buttons and badges on public pages; rounded rectangles inside the staff shell. Corners 12/18/24 px |
| Motion | `--rx-duration 0.22s`, `--rx-ease`; nothing bounces |
| Accessibility | Every text colour keeps 4.5:1 (WCAG AA) in light and dark; touch targets are 44 px (`--rx-target`) |

Why it fits UAE SMEs: the sand-and-teal palette reads as calm and premium, close to the
region's architecture and hospitality brands. It also stays clearly different from the cold
blue enterprise look that clients are trying to get away from (SAP, Oracle).

## 3. Two surfaces, one system

The bike shop has two sides. Every vertical app has the same two:

| Surface | Users | Layout | Style |
|---|---|---|---|
| **Portal** (public + external users) | tenants, client companies, patients, students | `rx-navbar` + centred page | Editorial: hero, photos, pill buttons, `--rx-type-hero`/`--rx-type-section` |
| **Back office** (staff) | the client's staff and managers | `rx-shell`: `rx-sidebar` + full-width `rx-navbar` + content (see `bikeshop/resources/views/layouts/staff.html`) | Dense: rounded-rectangle buttons, `--rx-type-title` and smaller, tables and grids, no hero or photos |

Both share the same tokens, so the client's brand shows on both sides.

**Sidebar conventions** (copied from the bike shop's staff layout):

- one `rx-sidebar-link` per area, with a Lucide icon;
- links grouped under `rx-sidebar-section` headings (e.g. *Front desk*, *Work*, *Manage*);
- every link and every section heading guarded by `rx-if="can('…')"`, so people only see
  what they may open;
- each `wl-*` module contributes its own links. This needs a sidebar hook in Renox
  (see [§8](#8-gaps)).

## 4. Client branding

A client overrides **only** these values in `client.toml` (later editable in the admin):

```toml
[branding]
logo         = "branding/logo.svg"   # square mark + wordmark
accent       = "#0F766E"             # optional, defaults to the platform teal
second       = "#B8532E"             # optional, defaults to the theme's terracotta
display_font = "Poppins"             # optional: "Poppins" | "Inter"
theme        = "classic"             # optional: the kit's older look
```

These are the only keys: `[branding]` is parsed with `deny_unknown_fields`, so a key such as
`accent_2` or `font_display` stops the host with a config error.

The client's name (`[client] name`) is the app's name: the sidebar brand, the public and sign-in
pages, the error pages and the mails' sender name show it (the host sets `{{ brand }}` for the
layouts and the framework's `APP_NAME`; `APP_NAME` itself is not read).

At boot, `wl-kernel` turns them into a stylesheet that is loaded **after** `renox_ui()`. An
app's own `:root` tokens win over the theme:

```css
:root {
  --rx-accent: #0f766e;
  --rx-accent-hover:   /* derived: +6% lightness */;
  --rx-accent-pressed: /* derived: −8% lightness */;
  --rx-accent-text:    /* derived: darkened until ≥ 4.5:1 on --rx-bg and --rx-surface */;
  --rx-accent-soft:    /* derived: 12% accent on --rx-surface */;
  --rx-focus:          /* derived: accent at 35% alpha */;
  --rx-accent-2: #b8532e;
  --rx-accent-2-text:  /* derived, same contrast rule */;
  --rx-chart-1: var(--rx-accent);
}
/* plus the matching dark-mode block, derived the same way */
```

**Contrast guard:** the generator refuses a brand colour that cannot reach 4.5:1 for text,
or 3:1 for white text on the accent button. Onboarding shows the nearest colour that passes
instead. A client can never ship an inaccessible portal by choosing a pale brand colour.

Things a client **cannot** change: spacing, radii, type scale, status colours, layout. They
are what keeps every client looking professionally designed.

## 5. UAE adaptation

| Topic | Rule |
|---|---|
| **Arabic + RTL** | `dir="rtl"` when the locale is `ar`. Module CSS uses logical properties only (`margin-inline-start`, `padding-inline`, `inset-inline-end`), never `left`/`right`. Directional icons (arrows, chevrons) flip; logos, charts and phone numbers do not |
| **Arabic fonts** | Text and titles: *Noto Sans Arabic* (OFL), shipped by Renox 1.2 (IBM Plex Sans Arabic and Noto Kufi Arabic are not adopted). Served from `/_renox/fonts` like Inter/Poppins. Font stacks put the Latin face first, so mixed text works |
| **Digits** | Western digits (0–9) in both languages. This is the norm in UAE business documents and avoids confusion in amounts and IDs |
| **Money** | `AED 1,250.00`: the code before the amount, 2 decimals, tabular figures (`font-variant-numeric: tabular-nums`) in tables and totals. VAT always shown as its own line |
| **Dates** | `14 Oct 2026` in text, `14/10/2026` in dense tables; never `10/14`. Gregorian only |
| **Names & IDs** | Fields for both English and Arabic names where documents require it (parties, invoices). Emirates ID displayed as `784-XXXX-XXXXXXX-X` |
| **Weekend** | Calendars and scheduling start the week on Monday and mark Saturday–Sunday as the weekend (the UAE standard since 2022) |

## 6. Shared status language

Many modules show a status. The same meaning always uses the same tone, through the kit's
`rx-badge` kinds:

| Tone (`rx-badge kind`) | Meaning | Examples |
|---|---|---|
| *(none)* neutral | not started / inactive | invoice **draft**, ticket **open**, document **archived** |
| `info` | in progress / waiting on someone | invoice **sent**, ticket **in progress**, booking **confirmed** |
| `success` | done / valid | invoice **paid**, document **valid**, ticket **resolved** |
| `warning` | needs attention soon | invoice **due in ≤ 7 days**, document **expires in ≤ 30 days**, SLA **at risk** |
| `error` | overdue / broken | invoice **overdue**, document **expired**, SLA **breached** |

Target, **not built**: `wl-kernel` provides `<wl-status :status="invoice.status" domain="invoicing"/>`,
which maps a module's status to a tone and a translated label. Today each module picks the
`rx-badge` kind from the table above (a status column in a grid uses the grid's badge column).
Modules never choose a badge colour of their own.

## 7. Component layers and naming

| Layer | Prefix | Where | Examples |
|---|---|---|---|
| Renox kit | `rx-` | `renox` (OSS) | `rx-grid` (the standard for every list, section 11), `rx-table` (inside detail pages only), `rx-form`, `rx-wizard`, `rx-kanban`, `rx-month-calendar` |
| Platform | `wl-` | `white-label/crates/wl-*` | *targets, none built*: `wl-status`, `wl-money`, `wl-party-card`, `wl-document-list`, `wl-expiry-meter`, `wl-invoice-summary`, `wl-portal-hero` |
| Vertical app | app prefix | `apps/<app>` | `prop-unit-card`, `mp-placement-row` |
| Client | `ext-` | `clients/<client>/ext` | only when configuration cannot do it |

The same prefixes apply to CSS classes: `.wl-…`, `.prop-…`. This follows the bike shop's
`bs-` convention. Following the kit, no stylesheet styles bare elements, and layouts are CSS
grids spaced with `--rx-space-*`.

**Before creating a component**, check in this order:

1. Does a kit component do it?
2. Can it be a composition of kit components in a template?
3. Is it generic enough for every Renox user? Then propose it to `renox-blocks`.
4. Only then add a `wl-` component.

## 8. Gaps

These needed work in Renox. **All three are delivered in Renox 1.2.0** (added to [ARCHITECTURE.md §4a](ARCHITECTURE.md#4a-contributions-to-renox-oss-useful-to-everyone)):

| # | Gap | Notes |
|---|---|---|
| R1 | RTL + Arabic fonts | Delivered (#28): `app.dir`, Noto Sans Arabic, logical CSS across the kit. Layouts write `<html dir="{{ app.dir }}">` |
| R9 | **Brand-token generator** in Renox: `Theme::from_brand(accent, accent_2)` → light and dark tokens with the contrast guard | Delivered (#36): `Theme::from_brand(&Brand::new(…).second(…))`, `App::theme_for` per host, `ContrastError` |
| R10 | **Sidebar contributions from modules** | Delivered (#37) as `NavLink` + `App::nav` / `Registry::nav` and `menu="staff"` on `<rx-sidebar>`; there is no `sidebar_link` |

## 9. Imagery and icons

- **Icons:** Lucide only (bundled with Renox), 1 icon per sidebar link. Icon circles use
  `--rx-accent-soft` or `--rx-tint`, as on the bike shop.
- **Photos:** portal only (hero, empty states). Each vertical ships a small set of licensed
  photos of UAE settings (towers, warehouses, clinics), credited in `CREDITS.md` as the
  bike shop does. The back office uses no photos.
- **Empty states:** an icon circle, one sentence and one primary action. No illustrations.

## 10. Review checklist for any UI pull request

- [ ] Only `var(--rx-*)` values; no hex, px font sizes or fixed shadows
- [ ] Logical properties only; screenshot checked with `dir="rtl"`
- [ ] Checked in dark mode
- [ ] Statuses use the `rx-badge` kinds of section 6 (`wl-status` is a target, not built)
- [ ] Amounts: AED, tabular figures (section 5); money columns of a grid carry a summary
- [ ] A list screen is a `renox::grid::Grid` that fills the screen (section 11); `node .claude/skills/standard-grid/check-fill.js <base-url> <path>` passes
- [ ] Every interactive element ≥ 44 px and keyboard-focusable with a visible `--rx-focus` ring
- [ ] Every new navigation link guarded by `can('…')`

## 11. List screens and page fill

Rule (tracking issue #523, written out in [host/AGENTS.md](../host/AGENTS.md) and the project skill
[standard-grid](../.claude/skills/standard-grid/SKILL.md); load the skill before writing or changing
any page that lists records).

- **Every list is the standard grid**: `renox::grid::Grid` rendered as `<rx-grid>`, with the 13-point
  baseline of the skill: search, sort, filters, pagination, dynamic columns, remembered state
  (`.remember()`), `.advanced_filter()`, role-limited export, an empty state and summaries on
  money columns. `rx-table` stays inside detail pages (a few lines of a record), never for a list.
- **On a desktop screen the page fills the space and does not scroll as a document**:
  `<rx-shell fill>`, one `rx-fill` element (or `rx-panes` of `rx-pane` panels) takes the remaining
  height, every panel scrolls on its own. `<rx-grid>` is a direct child of `<rx-page>`, never
  wrapped in `rx-stack` or `rx-card`, with nothing below it. Tablets and phones fall back to
  document scroll by themselves. Public pages scroll as a document.
- **Check**: `node .claude/skills/standard-grid/check-fill.js <base-url> <path>...`.
