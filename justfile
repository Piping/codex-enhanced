set working-directory := "codex-rs"
set positional-arguments
sccache_prefix := "if command -v sccache >/dev/null 2>&1; then export RUSTC_WRAPPER=sccache; export SCCACHE_CACHE_SIZE=${SCCACHE_CACHE_SIZE:-10G}; fi;"

# Display help
help:
    just -l

# `codex`
alias c := codex
codex *args:
    {{sccache_prefix}} cargo run --bin codex -- "$@"

# `codex exec`
exec *args:
    {{sccache_prefix}} cargo run --bin codex -- exec "$@"

# Start codex-exec-server, enable the app-server TUI, and run codex-tui.
[no-cd]
tui-with-exec-server *args:
    ./scripts/run_tui_with_exec_server.sh "$@"

# Run the CLI version of the file-search crate.
file-search *args:
    {{sccache_prefix}} cargo run --bin codex-file-search -- "$@"

# Build the CLI and run the app-server test client
app-server-test-client *args:
    {{sccache_prefix}} cargo build -p codex-cli
    {{sccache_prefix}} cargo run -p codex-app-server-test-client -- --codex-bin ./target/debug/codex "$@"

# format code
fmt:
    cargo fmt -- --config imports_granularity=Item 2>/dev/null

fix *args:
    {{sccache_prefix}} cargo clippy --fix --tests --allow-dirty "$@"

clippy *args:
    {{sccache_prefix}} cargo clippy --tests "$@"

install:
    rustup show active-toolchain
    cargo fetch

# Recursively clean Rust build artifacts older than 1 day.
[no-cd]
sweep:
    cargo sweep --recursive --time 1 ./codex-rs

# Run post-edit validation tests with cargo-nextest.
test:
    {{sccache_prefix}} cargo nextest run --no-fail-fast

# Run the default fast local iteration pass for tui, ext, and cli.
verify-fast:
    {{sccache_prefix}} cargo check -p codex-tui
    {{sccache_prefix}} cargo build -p codex-tui
    {{sccache_prefix}} cargo check -p codex-ext
    {{sccache_prefix}} cargo build -p codex-ext
    {{sccache_prefix}} cargo check -p codex-cli
    {{sccache_prefix}} cargo build -p codex-cli

# Run a fast local iteration pass for explicitly selected crates/targets.
verify-fast-crate *args:
    {{sccache_prefix}} cargo check "$@"
    {{sccache_prefix}} cargo build "$@"

# Run the narrow `/loop` edit-run verification path for codex-tui.
verify-tui-loop:
    {{sccache_prefix}} cargo check -p codex-tui
    {{sccache_prefix}} cargo build -p codex-tui

# Build and run Codex from source using Bazel.
# Note we have to use the combination of `[no-cd]` and `--run_under="cd $PWD &&"`
# to ensure that Bazel runs the command in the current working directory.
[no-cd]
bazel-codex *args:
    bazel run //codex-rs/cli:codex --run_under="cd $PWD &&" -- "$@"

[no-cd]
bazel-lock-update:
    bazel mod deps --lockfile_mode=update

[no-cd]
bazel-lock-check:
    ./scripts/check-module-bazel-lock.sh

bazel-test:
    bazel test //... --keep_going

bazel-remote-test:
    bazel test //... --config=remote --platforms=//:rbe --keep_going

build-for-release:
    bazel build //codex-rs/cli:release_binaries --config=remote

# Run the MCP server
mcp-server-run *args:
    {{sccache_prefix}} cargo run -p codex-mcp-server -- "$@"

# Regenerate the json schema for config.toml from the current config types.
write-config-schema:
    {{sccache_prefix}} cargo run -p codex-core --bin codex-write-config-schema

# Regenerate vendored app-server protocol schema artifacts.
write-app-server-schema *args:
    {{sccache_prefix}} cargo run -p codex-app-server-protocol --bin write_schema_fixtures -- "$@"

[no-cd]
write-hooks-schema:
    {{sccache_prefix}} cargo run --manifest-path ./codex-rs/Cargo.toml -p codex-hooks --bin write_hooks_schema_fixtures

# Run the argument-comment Dylint checks across codex-rs.
[no-cd]
argument-comment-lint *args:
    ./tools/argument-comment-lint/run-prebuilt-linter.sh "$@"

[no-cd]
argument-comment-lint-from-source *args:
    ./tools/argument-comment-lint/run.sh "$@"

# Tail logs from the state SQLite database
log *args:
    {{sccache_prefix}} if [ "${1:-}" = "--" ]; then shift; fi; cargo run -p codex-state --bin logs_client -- "$@"
