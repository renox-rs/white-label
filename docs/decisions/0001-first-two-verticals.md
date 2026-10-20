# 0001 — Build two verticals first: Property, then Manpower

- **Status:** accepted; build order superseded by [0005](0005-phases-follow-vertical-numbers.md)
- **Date:** 2026-10-14
- **Related:** [ARCHITECTURE.md](../ARCHITECTURE.md) §6 (module × app matrix), §9 (roadmap)

## Context

The platform targets ten UAE SME verticals:

1. Property & building management
2. Construction & fit-out subcontractors
3. Logistics & freight forwarding
4. Accounting firms & fractional CFO services
5. Private clinics
6. Cloud kitchens & F&B chains
7. Light manufacturing & assembly
8. Auto parts & fleet services
9. Manpower supply & outsourcing
10. Training centres & EdTech

The goal is a set of reusable modules, so that each new client is mostly configuration and
branding instead of new code.

## Decision

We build **two** verticals at the start. They are built one after the other, not in parallel:

1. **Property**. This app proves that the core modules can carry a complete product.
2. **Manpower**. This app proves that the same modules can be reused in a different business
   without being rewritten.

The other eight verticals wait until Manpower ships and reuse has been measured.

## Why only two

A generic module designed before it has two real consumers is almost always the wrong shape.
If we build ten apps at once, we have to guess the abstractions. If we build one app, nothing
tests whether its modules are actually reusable.

Two apps that are deliberately different expose which parts of a module are generic and
which parts leaked from the first domain. This is the platform's rule of three: code is
extracted into a shared module once two apps use it and a third is expected to.

**Success criterion:** at least 60% of Manpower's features run on existing modules without
modifying them. If the share is lower, we refactor the modules before starting a third
vertical.

## Why these two

Each vertical was scored on technical fit and delivery risk. This was **not** a market
analysis.

| Vertical | Core modules used | Fits web/PWA | Depends on closed APIs | Regulatory weight | Notes |
|---|---|---|---|---|---|
| **1. Property** | **Very many** (portal, documents, invoicing, payments, ticketing, notify) | Yes | No | Low | Monthly rent maps naturally to monthly subscriptions |
| **9. Manpower** | **Many** (documents + visa expiry reminders, invoicing, attendance) | Yes | No (WPS is a file export) | Medium | Visa/Emirates ID expiry is an urgent, concrete pain |
| 4. Accounting | Many | Yes | No | Low | Easiest to build, but stresses the core the least |
| 10. EdTech | Medium | Yes | No | Low | Needs scheduling and a light LMS first |
| 8. Auto parts | Medium | Yes | No | Low | Needs inventory and a parts catalogue |
| 7. Manufacturing | Medium | Yes | No | Low | ERP scope tends to grow without limit |
| 2. Construction | Medium | Partly (offline, geofencing) | No | Low | Needs strong mobile/offline support |
| 3. Logistics | Medium | No (background GPS) | No | Low | Better served by a native app |
| 5. Clinics | Medium | Yes | Partly (eClaimLink) | **High** (DHA, medical records) | High compliance risk |
| 6. F&B | Low | Yes | **Yes** (Talabat/Careem partner APIs) | Low | The headline feature depends on partner API access |

Property and Manpower complement each other:

- **Property** exercises the money and portal flow: an external user logs in, receives an
  invoice with VAT, pays it, and reports a maintenance issue.
- **Manpower** exercises the documents and workforce flow: visa and Emirates ID expiry
  reminders, attendance, timesheets, and billing the client company.
- Together they use **7 of the 11 planned modules**, so most of the foundation for the
  remaining eight verticals exists once both are done.

## Consequences

- Phase F1 is the Property vertical. (Phase F2 was Manpower here; since [0005](0005-phases-follow-vertical-numbers.md) F2 is Construction.)
- Accounting is the preferred third vertical, as a quick win. Once Property and Manpower are
  done its modules are almost complete, which makes it cheap to build and useful as a sales
  demo.
- Clinics, F&B and Logistics are deferred until their external dependencies are validated:
  regulatory approval, partner API access, and native mobile.

## Override condition

This ranking is based on technical fit only. **A real prospect in another vertical overrides
it.** A first app that actually sells is worth more than the technically ideal order. When
that happens, record the change in a new decision record that supersedes this one.
