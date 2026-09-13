# Publishing `simcore` to crates.io — Release Plan

> **Status: fork rebrand.** SimCore is a hard fork of the `simu` project
> (upstream: <https://github.com/chkhm/simu>, crates.io package `simu-des`).
> Upstream cleared publication via a Siemens OSS review on 2026-08-23; that
> clearance covers the inherited codebase, whose SPDX headers and
> MIT OR Apache-2.0 attribution (Siemens / Christoph Kuhmuench) are preserved
> verbatim. The steps below describe publishing the fork under its new name.

## Context

We want to publish this DES kernel to crates.io under the SimCore brand. Two
problems block a straightforward `cargo publish`:

1. **The name `simcore` is already taken on crates.io.** Crate names are
   globally unique, so the library cannot publish under `simcore`. This mirrors
   upstream's situation, where `simu` was taken and the package shipped as
   `simu-des`.
2. **`Cargo.toml` must carry the fork's identity** (`repository`, `homepage`
   pointing at the fork, publish-required metadata such as `description` and
   `license`).

**Decisions:**
- Package name: **`simcore-des`** on crates.io, while keeping `[lib] name = "simcore"`
  so all code, docs, and imports (`use simcore::...`) are consistent. Users add
  `simcore-des = "0.1"` and write `use simcore::...`.
- License: **MIT OR Apache-2.0** (inherited from upstream; SPDX headers and
  copyright attribution unchanged).

Outcome: a publishable, well-presented `0.1.0` crate that installs as
`simcore-des = "0.1"` and imports as `use simcore::...`, with clean docs.rs output.

## Changes

### 1. `Cargo.toml` — package/lib name split + full metadata

- Set `name = "simcore-des"` under `[package]`.
- Set `[lib] name = "simcore"` (keep existing `path = "src/lib.rs"`).
- Metadata fields:
  - `description = "A tiny deterministic discrete-event simulation kernel for building system simulators — single-threaded async executor, resources, and Monte Carlo parallelism."`
  - `license = "MIT OR Apache-2.0"`
  - `repository = "https://github.com/JetSquirrel/SimCore"`
  - `homepage = "https://github.com/JetSquirrel/SimCore"`
  - `readme = "README.md"`
  - `keywords = ["simulation", "discrete-event", "des", "simpy", "monte-carlo"]` (max 5)
  - `categories = ["simulation", "science", "concurrency"]` (valid crates.io slugs)
  - `rust-version = "1.82"` — the code uses `Option::is_none_or` (`src/env.rs`),
    stabilized in Rust 1.82 (Oct 2024), so the MSRV cannot be lower. 1.82 is well
    over a year old, so this is a safe floor. Verify with `cargo +1.82 build` (or
    `cargo msrv find`) before publishing; if a lower MSRV is ever wanted, replace
    the single `is_none_or` call site with `map_or(true, …)`.
  - `exclude = ["compare/", "simcore-python/", ".github/", "target/", ".claude/", "TESTING.md", "CLAUDE.md", "PUBLISHING.md"]`
    — keeps the published tarball lean: dev-only harnesses and docs stay in git
    but must not ship in the crate.

### 2. License files — inherited from upstream (REUSE 3.3)

The repo is [REUSE 3.3](https://reuse.software/spec-3.3)-compliant: canonical texts live in
`LICENSES/MIT.txt` + `LICENSES/Apache-2.0.txt`, every file carries SPDX info (headers or
`REUSE.toml`), `Cargo.toml` has `license = "MIT OR Apache-2.0"`, `README.md` has the dual-license
section with contribution clause, and CI runs `reuse lint`. The root `LICENSE-MIT` /
`LICENSE-APACHE` copies (Rust-ecosystem convention, GitHub's license detector) are kept as-is.
The rebrand must not alter any SPDX header or the upstream copyright attribution.

### 3. `CHANGELOG.md`

- Keep-a-Changelog style; add a `## [0.1.0]` entry for the fork's first release
  noting the rebrand from `simu`/`simu-des` to SimCore/`simcore-des`.

### 4. README touch-ups (`README.md`)

- Install snippet:
  ```toml
  [dependencies]
  simcore-des = "0.1"
  ```
  with a one-line note that the crate is imported as `use simcore::...`.
- Optionally add crates.io + docs.rs badges (go live after first publish).

## Things intentionally NOT changed

- No kernel source code changes — `[lib] name = "simcore"` plus the
  `use simu::` → `use simcore::` rename preserve the API surface exactly as-is.
- All SPDX copyright/license headers stay byte-identical; the Siemens /
  Christoph Kuhmuench attribution and MIT OR Apache-2.0 license are preserved.
- `compare/` Python harness stays in the git repo (useful for contributors) but is
  excluded from the crate tarball.

## Verification

1. `cargo build` and `cargo test` — still green (no behavioural code changed).
2. `cargo clippy -- -D warnings` — stays warning-clean.
3. `cargo package --list` — confirm `compare/`, `simcore-python/`, `target/` are absent and the
   tarball is small (well under crates.io's 10 MB limit).
4. `cargo publish --dry-run` — must pass with no "missing field" errors; confirm it
   reports the package as `simcore-des`.
5. `cargo doc --no-deps --all-features` — confirm docs build (docs.rs will rebuild this).
6. MSRV sanity: build once with the declared `rust-version` toolchain if `rustup` has it.

## Publish steps (manual)

1. On crates.io: log in with GitHub, **verify account email** (required to publish).
2. Create an API token; run `cargo login <token>`.
3. `cargo publish --dry-run`, then `cargo publish` (publishes `simcore-des`).
4. Tag the release: `git tag v0.1.0 && git push --tags`.
5. docs.rs builds automatically after publish; verify the rendered docs.

## Follow-ups (broader open-source goal)

- Announcement/article (Medium, /r/rust, This Week in Rust, Rust users forum): draft once
  the crate is live and has a docs.rs link to point at.

## Alternative names (in case `simcore-des` is ever reconsidered)

`simcore` is taken on crates.io, hence the `-des` suffix mirroring upstream's
`simu-des` pattern. Any alternative must stay distinct from the existing `simu`,
`desim`, `simrs`, `des`, `simkit`, `rustsim`, and `simq` crates.
