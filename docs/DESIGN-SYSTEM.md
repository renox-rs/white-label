# White-Label Design System

> Status: **accepted** (2026-10-14): teal + terracotta as the platform default, IBM Plex Sans Arabic + Noto Kufi Arabic for Arabic. Reference app: the Renox bike shop
> ([bikeshop.renox.rs](https://bikeshop.renox.rs), source in `renox/examples/bikeshop`).

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
accent       = "#0F766E"             # required
accent_2     = "#B8532E"             # optional, defaults to the theme's terracotta
font_display = "Poppins"             # optional: "Poppins" | "Inter"
```

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
| **Arabic fonts** | Text: *IBM Plex Sans Arabic* (OFL). Titles: *Noto Kufi Arabic* (OFL), as Poppins' counterpart. Served from `/_renox/fonts` like Inter/Poppins. Font stacks put the Latin face first, so mixed text works |
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

`wl-kernel` provides `<wl-status :status="invoice.status" domain="invoicing"/>`, which maps a
module's status to a tone and a translated label. Modules never choose a badge colour
themselves.

## 7. Component layers and naming

| Layer | Prefix | Where | Examples |
|---|---|---|---|
| Renox kit | `rx-` | `renox` (OSS) | `rx-table`, `rx-grid`, `rx-form`, `rx-wizard`, `rx-kanban`, `rx-month-calendar` |
| Platform | `wl-` | `white-label/crates/wl-*` | `wl-status`, `wl-money`, `wl-party-card`, `wl-document-list`, `wl-expiry-meter`, `wl-invoice-summary`, `wl-portal-hero` |
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

These need work in Renox (added to [ARCHITECTURE.md §4a](ARCHITECTURE.md#4a-contributions-to-renox-oss-useful-to-everyone)):

| # | Gap | Notes |
|---|---|---|
| R1 | RTL + Arabic fonts | Today the kit has one `[dir="rtl"]` rule and Latin-only font subsets |
| R9 | **Brand-token generator** in Renox: `Theme::from_brand(accent, accent_2)` → light and dark tokens with the contrast guard | Generic: any Renox app benefits, not only ours |
| R10 | **Sidebar contributions from modules** (`Registry::sidebar_link(section, link, permission)`) | So `wl-*` modules add their own navigation, like R4 does for admin resources |

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
- [ ] Statuses rendered through `wl-status`
- [ ] Amounts through `wl-money` (tabular figures, AED)
- [ ] Every interactive element ≥ 44 px and keyboard-focusable with a visible `--rx-focus` ring
- [ ] Every new navigation link guarded by `can('…')`
