# Silas-Blue — Architectural Decisions

## 2026-11-03: Dependency Modernization (v6.0.19)
- Bumped `parking_lot` from 0.12 to 0.13 (closes Dependabot PR #32).
- Bumped `rusqlite` from 0.40 to 0.41 with bundled SQLite 3.39 (closes Dependabot PR #31).
- Bumped `base64` from 0.22 to 0.23 (closes Dependabot PR #30).
- Validated with clean test suite (100% pass) and strict Clippy zero-warning gate.

## 2026-11-02: Standardized Context & 1Password CLI Architecture
- Adopted two-tier context hierarchy anchored in C:\Users\Myke\Documents\GitHub\rbnd.cc.
- Enforced zero-plaintext credential rule using 1Password CLI (op:// secret references).
