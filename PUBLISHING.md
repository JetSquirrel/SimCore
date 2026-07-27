# Publishing `simu` to crates.io — Release Plan

> **Status: NOT YET EXECUTED.** Pending employer publication permission (private
> spare-time activity). Execute the steps below once cleared.

## Context

We want to open-source this DES library and publish it to crates.io (and later write a
promotional article). Two problems block a straightforward `cargo publish`:

1. **The name `simu` is already taken on crates.io** — it's an unrelated "CLI tool for
   managing iOS simulators" (v0.1.0). Crate names are globally unique, so the library
   cannot publish under `simu`. The nearest prior-art competitor `desim` (SimPy-inspired
   DES) also exists, so the new name should be distinct from it too.
2. **`Cargo.toml` is missing all publish-required metadata** (`description`, `license`)
   and there are no `LICENSE` files in the repo.

**Decisions:**
- Package name: **`simu-des`** on crates.io, while keeping `[lib] name = "simu"` so all
  existing code, docs, and imports (`use simu::...`) remain unchanged. Users add
  `simu-des = "0.1"` and still write `use simu::...`.
- License: **MIT OR Apache-2.0** (Rust ecosystem standard dual license).

Outcome: a publishable, well-presented `0.1.0` crate that installs as
`simu-des = "0.1"` and imports as `use simu::...`, with clean docs.rs output.

## Changes

### 1. `Cargo.toml` — package/lib name split + full metadata

- Set `name = "simu-des"` under `[package]`.
- Add `[lib] name = "simu"` (keep existing `path = "src/lib.rs"`).
- Add metadata fields:
  - `description = "Discrete-event simulation for Rust, inspired by SimPy — single-threaded async executor, resources, and Monte Carlo parallelism."`
  - `license = "MIT OR Apache-2.0"`
  - `repository = "https://github.com/chkhm/simu"`
  - `homepage = "https://github.com/chkhm/simu"`
  - `readme = "README.md"`
  - `keywords = ["simulation", "discrete-event", "des", "simpy", "monte-carlo"]` (max 5)
  - `categories = ["simulation", "science", "concurrency"]` (valid crates.io slugs)
  - `authors = ["Christoph Kuhmuench <christoph.kuhmuench@gmail.com>"]`
  - `rust-version = "1.82"` — the code uses `Option::is_none_or` (`src/env.rs`),
    stabilized in Rust 1.82 (Oct 2024), so the MSRV cannot be lower. 1.82 is well
    over a year old, so this is a safe floor. Verify with `cargo +1.82 build` (or
    `cargo msrv find`) before publishing; if a lower MSRV is ever wanted, replace
    the single `is_none_or` call site with `map_or(true, …)`.
  - `exclude = ["compare/", "reviews/", ".github/", "target/", ".claude/", "PLAN.md", "TESTING.md", "CLAUDE.md", "PUBLISHING.md"]`
    — keeps the published tarball lean. **Critical: `compare/` alone is ~147 MB**
    (Python parity harness) and must not ship in the crate.

### 2. License files — ✅ DONE (superseded by REUSE 3.3 compliance, 2026-07-27)

The repo is now [REUSE 3.3](https://reuse.software/spec-3.3)-compliant: canonical texts live in
`LICENSES/MIT.txt` + `LICENSES/Apache-2.0.txt`, every file carries SPDX info (headers or
`REUSE.toml`), `Cargo.toml` has `license = "MIT OR Apache-2.0"`, `README.md` has the dual-license
section with contribution clause, and CI runs `reuse lint`.

Remaining at publish time (optional): add root `LICENSE-MIT` / `LICENSE-APACHE` copies for the
Rust-ecosystem convention and GitHub's license detector — REUSE ignores standard root license
files, so this cannot break compliance.

### 3. `CHANGELOG.md` (new, optional but recommended)

- Keep-a-Changelog style; a single `## [0.1.0]` entry summarizing the MVP feature set
  (pulled from `CLAUDE.md` Status section).

### 4. README touch-ups (`README.md`)

- Add an install snippet:
  ```toml
  [dependencies]
  simu-des = "0.1"
  ```
  with a one-line note that the crate is imported as `use simu::...`.
- Optionally add crates.io + docs.rs badges (go live after first publish).

## Things intentionally NOT changed

- No source code changes — `[lib] name = "simu"` preserves every `use simu::...` path,
  examples, tests, and the API surface exactly as-is.
- `compare/` Python harness stays in the git repo (useful for contributors) but is
  excluded from the crate tarball.

## Verification

1. `cargo build` and `cargo test` — still green (no code changed).
2. `cargo clippy -- -D warnings` — stays warning-clean.
3. `cargo package --list` — confirm `compare/`, `reviews/`, `target/` are absent and the
   tarball is small (well under crates.io's 10 MB limit).
4. `cargo publish --dry-run` — must pass with no "missing field" errors.
5. `cargo doc --no-deps --all-features` — confirm docs build (docs.rs will rebuild this).
6. MSRV sanity: build once with the declared `rust-version` toolchain if `rustup` has it.

## Publish steps (manual, after employer permission)

1. On crates.io: log in with GitHub, **verify account email** (required to publish).
2. Create an API token; run `cargo login <token>`.
3. `cargo publish --dry-run`, then `cargo publish`.
4. Tag the release: `git tag v0.1.0 && git push --tags`.
5. docs.rs builds automatically after publish; verify the rendered docs.

## Follow-ups (broader open-source goal)

- Announcement/article (Medium, /r/rust, This Week in Rust, Rust users forum): draft once
  the crate is live and has a docs.rs link to point at.

## Alternative names checked (all free on crates.io, in case `simu-des` is reconsidered)

`simu-rs`, `simu-core`, `simu-engine`, `dessim`, `simulay`, `disevent`. Taken/unavailable:
`simu`, `desim`, `simrs`, `des`, `simkit`, `rustsim`, `simq`.
