# Workflow Strategy

This fork keeps pushes to `main` quiet. Heavier validation runs stay available through
`workflow_dispatch`, while pull requests can still use targeted review-time checks.

## Pull Requests

- `bazel.yml` is the main pre-merge verification path for Rust code.
  It runs Bazel `test` and Bazel `clippy` on the supported Bazel targets,
  including the generated Rust test binaries needed to lint inline `#[cfg(test)]`
  code.
- `rust-ci.yml` keeps the Cargo-native PR checks intentionally small:
  - `cargo fmt --check`
  - `cargo shear`
  - `argument-comment-lint` on Linux, macOS, and Windows
  - `tools/argument-comment-lint` package tests when the lint or its workflow wiring changes

## Manual Verification

- `bazel.yml` is available as a manual verification path when the fork needs a full
  Bazel pass.
- `rust-ci-full.yml` is the full Cargo-native verification workflow.
  It keeps the heavier checks off the PR path while still providing an on-demand
  validation path:
  - the full Cargo `clippy` matrix
  - the full Cargo `nextest` matrix
  - release-profile Cargo builds
  - cross-platform `argument-comment-lint`
  - Linux remote-env tests

Other repo-level checks that used to run on `push(main)` in upstream, such as
`ci.yml`, `cargo-deny.yml`, `codespell.yml`, `sdk.yml`, and `v8-canary.yml`, are also
manual-only in this fork so routine sync pushes do not fan out into unrelated CI.

## Rule Of Thumb

- If a build/test/clippy check can be expressed in Bazel, prefer putting the PR-time version in `bazel.yml`.
- Keep `rust-ci.yml` fast enough that it usually does not dominate PR latency.
- Reserve `rust-ci-full.yml` for heavyweight Cargo-native coverage that Bazel does not replace yet.
