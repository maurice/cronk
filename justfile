# Local Rust feedback: discover tasks with `just` or `just --list`.
set positional-arguments

# List available development tasks.
default:
    @just --list

# Use the checked-in hook so task updates apply without rebuilding cargo-husky.
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    current=$(git config --get core.hooksPath || true)
    if [[ -n "$current" && "$current" != .cargo-husky/hooks ]]; then
        echo "Refusing to replace custom core.hooksPath: $current" >&2
        exit 1
    fi
    git config --local core.hooksPath .cargo-husky/hooks

# Check formatting without waiting for compilation/tests.
fmt:
    cargo fmt --all -- --check

# Apply Rust formatting.
format:
    cargo fmt --all

# Type-check without linking. Prefer `just check --lib` during iteration.
check *args:
    @just _cargo check --locked "$@"

# Check all targets with warnings denied (also used by the commit hook).
lint:
    @just _cargo clippy --locked --all-targets -- -D warnings

# Run nextest with explicit Cargo targets/filter arguments as needed.
test *args:
    @just _cargo nextest run --locked "$@"

# Run the complete unit/integration/example target set.
test-all:
    @just test --all-targets

# Run only library tests; optional arguments can include a nextest -E filter.
test-lib *args:
    @just test --lib "$@"

# Run one integration binary, optionally filtered with -E 'test(name)'.
test-integration target *args:
    @just test --test "$@"

# Select tests from branch, staged, unstaged and untracked changes vs a Git ref.
# Shared source/dependency changes conservatively run all targets.
test-affected base='origin/main':
    #!/usr/bin/env bash
    set -euo pipefail
    base=$(git merge-base "$1" HEAD)
    mapfile -d '' paths < <(
        git diff --no-renames --name-only -z "$base" HEAD
        git diff --no-renames --name-only -z HEAD
        git ls-files --others --exclude-standard -z
    )
    targets=()
    declare -A selected=()
    broad=false
    for path in "${paths[@]}"; do
        case "$path" in
            tests/*.rs)
                name=${path#tests/}
                # Nested helpers and removed/renamed targets can affect other tests.
                if [[ "$name" != */* && -f "$path" ]]; then
                    if [[ -z "${selected[$name]:-}" ]]; then
                        targets+=(--test "${name%.rs}")
                        selected[$name]=true
                    fi
                else
                    broad=true
                fi
                ;;
            tests/workflow/*) ;; # Covered by just workflow-test, not Rust tests.
            src/*|examples/*|tests/*|Cargo.toml|Cargo.lock|build.rs|.cargo/*|.config/nextest.toml)
                broad=true ;;
        esac
    done
    if "$broad"; then
        echo 'Shared Rust/build changes: running all nextest targets.'
        exec just test-all
    elif (( ${#targets[@]} )); then
        echo "Affected integration targets: ${targets[*]}"
        exec just test "${targets[@]}"
    else
        echo 'No affected Rust targets. Run just doc for doctest changes.'
    fi

# Nextest cannot run doctests: this is the sole Cargo test runner exception.
doc *args:
    @just _cargo test --locked --doc "$@"

# Build locally with the shared compiler budget.
build *args:
    @just _cargo build --locked "$@"

# Run the application or an example (pass app arguments after --).
run *args:
    @just _cargo run --locked "$@"

# Full pre-publication Rust validation, sequentially (not on every edit).
validate: fmt lint test-all doc

# Regression-test task routing and affected-file selection without building Rust.
workflow-test:
    python3 -m unittest discover -s tests/workflow -v

# Shared advisory lock across linked worktrees; keep target/ separate per worktree.
# Private recipe: all public heavy tasks flow through here.
_cargo +args:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "$1" == nextest ]] && ! cargo nextest --version >/dev/null 2>&1; then
        echo 'cargo-nextest is required: https://nexte.st/docs/installation/' >&2
        echo 'Do not fall back to cargo test; only just doc uses that runner.' >&2
        exit 1
    fi
    if ! command -v flock >/dev/null 2>&1; then
        echo 'flock (util-linux) is required for cross-worktree coordination.' >&2
        exit 1
    fi
    common_dir=$(git rev-parse --git-common-dir)
    exec 9>"$common_dir/cronk-dev.lock"
    echo 'Waiting for the repository local-validation lock...' >&2
    flock 9
    export CARGO_BUILD_JOBS="${CRONK_BUILD_JOBS:-${CARGO_BUILD_JOBS:-2}}"
    exec cargo "$@"
