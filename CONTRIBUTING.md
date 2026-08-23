# Contributing to simu

Contributions to simu are always welcome. This document explains the general
requirements on contributions and the recommended preparation steps. It also
sketches the typical integration process of pull requests.

## Contribution Checklist

- use git to manage your changes [*recommended*]

- follow the Rust coding style enforced by `rustfmt` and `clippy` [**required**]
  - run `cargo fmt` before committing
  - `cargo clippy --all-targets -- -D warnings` must stay warning-clean, both
    with and without the `monte-carlo` feature

- add the required SPDX licensing header to each new file introduced
  [**required**]
  - the repository is [REUSE 3.3](https://reuse.software/spec-3.3)-compliant;
    copy the header from any neighbouring file (see [REUSE.toml](REUSE.toml)
    for files covered in bulk, e.g. Markdown)
  - CI runs `reuse lint` and fails pull requests that add files without it

- structure patches logically, in small steps [**required**]
  - one separable functionality/fix/refactoring = one commit
  - do not mix those in a single commit
  - after each commit, the tree still has to build and pass tests, i.e. do not
    add even temporary breakages inside a series (helps when bisecting)
  - use `git rebase -i` to restructure a series

- base patches on top of the latest `master`

- test patches sufficiently (obvious, but...) [**required**]
  - `cargo test` (and `cargo test --features monte-carlo`) pass
  - new behaviour comes with unit, integration, or doc-tests
  - no regressions are caused in affected code
  - changes to scheduling semantics must keep the SimPy parity harness
    (`compare/`, run in CI) green

- keep the documentation in sync [**required**]
  - `SPEC.md` is the design source of truth and `API.md` the signature
    cheat-sheet; update both when behaviour or the public API changes
  - code patterns in `llms.txt` and `docs/simu-for-agents.md` must be verbatim
    copies of doc-tests — never hand-write parallel snippets
  - add a line to `CHANGELOG.md` under *Unreleased* for user-visible changes

- add Signed-off-by to all commits [**required**]
  - to certify the "Developer's Certificate of Origin", see below
  - use `git commit -s`
  - check with your employer when not working on your own!

- open a pull request on GitHub [**required**]
  - describe the motivation and the approach; reference related issues
  - CC people who you think should look at the change, e.g. someone who wrote
    the code you are fixing or refactoring

- post follow-up version(s) if feedback requires this

- send a reminder if nothing happened after about a week

## Developer's Certificate of Origin 1.1

When signing-off a patch for this project like this

    Signed-off-by: Random J Developer <random@developer.example.org>

using your real name (no pseudonyms or anonymous contributions), you declare the
following:

    By making a contribution to this project, I certify that:

        (a) The contribution was created in whole or in part by me and I
            have the right to submit it under the open source license
            indicated in the file; or

        (b) The contribution is based upon previous work that, to the best
            of my knowledge, is covered under an appropriate open source
            license and I have the right under that license to submit that
            work with modifications, whether created in whole or in part
            by me, under the same open source license (unless I am
            permitted to submit under a different license), as indicated
            in the file; or

        (c) The contribution was provided directly to me by some other
            person who certified (a), (b) or (c) and I have not modified
            it.

        (d) I understand and agree that this project and the contribution
            are public and that a record of the contribution (including all
            personal information I submit with it, including my sign-off) is
            maintained indefinitely and may be redistributed consistent with
            this project or the open source license(s) involved.

## License

simu is dual-licensed under **MIT OR Apache-2.0**. Unless you explicitly state
otherwise, any contribution intentionally submitted for inclusion in the work by
you, as defined in the Apache-2.0 license, shall be dual licensed as above,
without any additional terms or conditions.

## Contribution Integration Process

1. pull request review performed on GitHub
   - at least by the maintainers, but everyone is invited
   - feedback has to consider design, functionality and style
   - simpler and clearer code preferred, even if the original code works fine

2. CI must be green: build, clippy, tests (both Monte Carlo backends),
   `reuse lint`, and the SimPy parity harness

3. accepted pull requests are merged into `master`

4. releases are tagged (`vX.Y.Z`) and published to crates.io as
   [`simu-des`](https://crates.io/crates/simu-des) by the maintainers

Questions and ideas are welcome as GitHub issues or discussions at
<https://github.com/chkhm/simu>.
