# Development tasks. Run `just` to list them. Requires: just, mise, rustup.
set positional-arguments

# Run commands with the mise-pinned CLIs (nextest, sccache) on PATH.
mise := "mise exec --"

# Share compiled dependencies between worktrees (CI has its own cache).
export RUSTC_WRAPPER := if env("CI", "") == "" { "sccache" } else { "" }

default:
    @just --list

# One-time setup: pinned Rust + CLIs and the git hooks.
setup:
    rustup show active-toolchain || rustup toolchain install
    mise trust --quiet
    mise install
    git config --local core.hooksPath .cargo-husky/hooks

# Check formatting (`format` applies it).
fmt:
    cargo fmt --all -- --check

format:
    cargo fmt --all

# Type-check without linking, e.g. `just check --lib`.
check *args:
    {{mise}} cargo check --locked "$@"

# Clippy over all targets, warnings denied (also the pre-commit hook).
lint:
    {{mise}} cargo clippy --locked --all-targets -- -D warnings

# Nextest with any cargo/nextest args, e.g. `just test --lib -E 'test(name)'`.
test *args:
    {{mise}} cargo nextest run --locked "$@"

test-all:
    just test --all-targets

# CI: report every failure instead of stopping at the first.
test-ci:
    just test --all-targets --no-fail-fast

# Doctests: the one thing nextest cannot run.
doc:
    {{mise}} cargo test --locked --doc

build *args:
    {{mise}} cargo build --locked "$@"

build-release:
    {{mise}} cargo build --locked --release

# Run the app: `just run -- --check`.
run *args:
    {{mise}} cargo run --locked "$@"

# Open the demo TUI; extra args e.g. `--snapshot dashboard.png`.
demo *args:
    just run -- --demo "$@"

demo-onboarding *args:
    just demo --onboarding "$@"

# Generate the UI gallery in .snapshots/.
gallery:
    just run --example gallery

# Generate syncing animation frames.
sync-preview:
    just run --example sync_preview

# Everything to run before publishing.
validate: fmt lint test-all doc
