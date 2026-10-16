# wl-<name>

> Status: draft | reviewed | accepted · Last updated: YYYY-MM-DD

## 1. Purpose

One paragraph: the business problem the module solves, independent of any vertical.

## 2. Capabilities covered

| ID | Capability | Used by verticals | Scope |
|---|---|---|---|

## 3. Concepts and entities

| Entity | Description | Important fields |
|---|---|---|

## 4. Lifecycles

Statuses per entity, in order, with the allowed transitions and the UI tone.

## 5. Workflows

Numbered steps for the main flows: who does what, and what the system does in response.

## 6. Interfaces to other modules

- **Provides:** traits or services that other modules call.
- **Consumes:** traits from `wl-kernel` that it calls.
- **Events emitted:** name, when, payload.
- **Attaches to:** which `Subject`s it can attach to (polymorphic references).

## 7. Configuration per client

What a client can change through `client.toml` or the settings, and the defaults.

## 8. Extension points

Strategy traits, events and overridable views that verticals or client extensions use.

## 9. Renox fit

| Need | Renox provides | Gap (→ ARCHITECTURE §4a ID) |
|---|---|---|

## 10. Compliance

Tax, data protection, retention, audit requirements that apply to this module.

## 11. Out of scope

What this module deliberately does not do, and where that lives instead.

## 12. Open questions
