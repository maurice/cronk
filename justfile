# Local Rust feedback: discover tasks with `just` or `just --list`.
set positional-arguments

# List available development tasks.
default:
    @just --list

# One-command Linux/WSL onboarding: native prerequisites, pinned tools, hooks + fast builds.
setup: _setup-native _bootstrap-mise _setup-tools _setup-hooks
    #!/usr/bin/env bash
    set -euo pipefail
    if just _mise exec rust -- rustc -vV | grep -x 'host: x86_64-unknown-linux-gnu' >/dev/null; then
        just fast-setup
    else
        echo 'Pinned tools and hooks are ready; optional fast configuration is x86-64 only.'
    fi

# Install only the pinned tools needed by a CI job (no hooks/fast configuration).
setup-ci group='checks': _setup-native _bootstrap-mise
    #!/usr/bin/env bash
    set -euo pipefail
    case "$1" in
        rust) tools=(rust) ;;
        checks) tools=(rust nextest) ;;
        release) tools=(rust cross) ;;
        *) echo 'Expected setup-ci group: rust, checks or release.' >&2; exit 2 ;;
    esac
    just _setup-tools "${tools[@]}"
    # Let artifact-cache actions see the same Rust toolchain as the just tasks.
    if [[ -n "${GITHUB_ENV:-}" ]]; then
        version=$(just _mise exec rust -- printenv RUSTUP_TOOLCHAIN)
        printf 'RUSTUP_TOOLCHAIN=%s\n' "$version" >> "$GITHUB_ENV"
    fi

# Inspect repository-managed tool versions.
tools:
    @just _mise ls --current

# Refresh artifact resolutions after intentionally editing mise.toml tool pins.
tools-lock:
    @just _mise lock --platform linux-x64 --platform linux-arm64

# Native compiler prerequisites belong to the OS, not Cargo or mise.
_setup-native:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "$(uname -s)" == Linux ]] || { echo 'Setup currently supports Linux/WSL.' >&2; exit 1; }
    packages=()
    for pair in 'cc:build-essential' 'clang:clang' 'pkg-config:pkg-config' 'git:git' 'curl:curl' 'flock:util-linux' 'python3:python3' 'sha256sum:coreutils'; do
        tool=${pair%%:*}
        command -v "$tool" >/dev/null || packages+=("${pair#*:}")
    done
    if (( ${#packages[@]} )); then
        command -v apt-get >/dev/null || { echo "Install missing native prerequisites: ${packages[*]}" >&2; exit 1; }
        privilege=()
        if (( EUID != 0 )); then privilege=(sudo); fi
        echo "Installing missing native packages (may request sudo): ${packages[*]}"
        "${privilege[@]}" apt-get update
        "${privilege[@]}" apt-get install --yes --no-install-recommends "${packages[@]}" ca-certificates
    fi

# Bootstrap a pinned mise binary, checking the repository-pinned SHA-256.
_bootstrap-mise:
    #!/usr/bin/env python3
    import hashlib
    import json
    import os
    from pathlib import Path
    import platform
    import tempfile
    import urllib.request

    config = json.loads(Path('.config/mise-bootstrap.json').read_text())
    architecture = {'x86_64': 'x64', 'aarch64': 'arm64'}.get(platform.machine())
    if platform.system() != 'Linux' or architecture is None:
        raise SystemExit('Bootstrap supports x86-64/ARM64 Linux only.')
    name = f"mise-v{config['version']}-linux-{architecture}"
    expected = config['sha256'][f'linux-{architecture}']
    data = Path(os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share')))
    binary = data / 'cronk/mise' / config['version'] / 'mise'
    if binary.is_file() and hashlib.sha256(binary.read_bytes()).hexdigest() == expected:
        raise SystemExit(0)
    binary.parent.mkdir(parents=True, exist_ok=True)
    url = f"https://github.com/jdx/mise/releases/download/v{config['version']}/{name}"
    print(f"Downloading pinned mise {config['version']}...", flush=True)
    with urllib.request.urlopen(url, timeout=120) as response:
        content = response.read()
    if hashlib.sha256(content).hexdigest() != expected:
        raise SystemExit('mise checksum mismatch; refusing to install or execute download.')
    with tempfile.NamedTemporaryFile(dir=binary.parent, delete=False) as output:
        temporary = Path(output.name)
        output.write(content)
    try:
        temporary.chmod(0o755)
        temporary.replace(binary)
    finally:
        temporary.unlink(missing_ok=True)

# Use managed tools without modifying shell startup files or global PATH.
_mise +args:
    #!/usr/bin/env python3
    import json
    import os
    from pathlib import Path
    import sys

    version = json.loads(Path('.config/mise-bootstrap.json').read_text())['version']
    data = Path(os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share')))
    binary = data / 'cronk/mise' / version / 'mise'
    if not binary.is_file():
        raise SystemExit('Development tools are not set up. Run just setup first.')
    os.execv(binary, [str(binary), *sys.argv[1:]])

# Tool installation is bounded locally; CI installs only its job's subset.
_setup-tools *tools:
    #!/usr/bin/env bash
    set -euo pipefail
    just _mise trust mise.toml
    if [[ "${CI:-}" == true ]]; then exec just _mise install --locked "$@"; fi
    export CARGO_BUILD_JOBS="${CRONK_BUILD_JOBS:-${CARGO_BUILD_JOBS:-2}}"
    # Keep the lock in flock's parent, not inherited by long-lived tool daemons.
    exec flock --close "$(git rev-parse --git-common-dir)/cronk-dev.lock" just _mise install --locked "$@"

# Activate checked-in hooks without rebuilding cargo-husky.
_setup-hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    current=$(git config --get core.hooksPath || true)
    if [[ -n "$current" && "$current" != .cargo-husky/hooks ]]; then
        echo "Refusing to replace custom core.hooksPath: $current" >&2
        exit 1
    fi
    git config --local core.hooksPath .cargo-husky/hooks

# Opt into mold + sccache for this worktree, without changing global Cargo config.
fast-setup:
    @just _mise exec rust mold sccache -- just _fast-setup

_fast-setup:
    #!/usr/bin/env bash
    set -euo pipefail
    for tool in mold sccache clang flock; do
        command -v "$tool" >/dev/null || { echo "Install $tool first." >&2; exit 1; }
    done
    if ! rustc -vV | grep -x 'host: x86_64-unknown-linux-gnu' >/dev/null; then
        echo 'This opt-in configuration supports x86_64 Linux/WSL only.' >&2
        exit 1
    fi
    rustc -C linker-features=-lld --print target-libdir >/dev/null
    exec 9>"$(git rev-parse --git-common-dir)/cronk-dev.lock"
    flock 9
    if [[ -e .cargo/config ]]; then
        echo 'Refusing to override existing .cargo/config.' >&2
        exit 1
    fi
    if [[ -e .cargo/config.toml ]]; then
        if cmp -s .config/cargo-fast.toml .cargo/config.toml; then
            echo 'Fast-build configuration is already enabled.'
            exit 0
        fi
        echo 'Refusing to overwrite existing .cargo/config.toml; merge settings manually.' >&2
        exit 1
    fi
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    printf 'int main(void) { return 0; }\n' | clang -fuse-ld=mold -Wl,--threads=2 -x c - -o "$tmp/smoke"
    "$tmp/smoke"
    sccache --version
    mkdir -p .cargo
    cp .config/cargo-fast.toml .cargo/config.toml
    echo 'Enabled mold + sccache in this worktree. Expect a one-time rebuild.'

# Remove only an unmodified generated configuration (changes cause a rebuild).
fast-disable:
    #!/usr/bin/env bash
    set -euo pipefail
    exec 9>"$(git rev-parse --git-common-dir)/cronk-dev.lock"
    flock 9
    if [[ ! -e .cargo/config.toml ]]; then exit 0; fi
    if ! cmp -s .config/cargo-fast.toml .cargo/config.toml; then
        echo 'Refusing to remove customized .cargo/config.toml.' >&2
        exit 1
    fi
    rm .cargo/config.toml
    echo 'Removed the generated worktree configuration; global settings are unchanged.'

# Inspect compiler-cache hits/misses without clearing the shared cache/statistics.
cache-stats:
    @just _mise exec sccache -- sccache --show-stats

# Check formatting without waiting for compilation/tests.
fmt:
    @just _mise exec rust -- cargo fmt --all -- --check

# Apply Rust formatting.
format:
    @just _mise exec rust -- cargo fmt --all

# Restore manifest-pinned nextest (normally handled by setup).
install-nextest:
    @just _setup-tools nextest

# Type-check without linking. Prefer `just check --lib` during iteration.
check *args:
    @just _cargo check --locked "$@"

# Type-check every target without linking.
check-all:
    @just check --all-targets

# Check all targets with warnings denied (also used by the commit hook).
lint:
    @just _cargo clippy --locked --all-targets -- -D warnings

# Run nextest with explicit Cargo targets/filter arguments as needed.
test *args:
    @just _cargo nextest run --locked "$@"

# Run the complete unit/integration/example target set.
test-all:
    @just test --all-targets

# Run every target on an isolated CI runner, collecting all failures.
test-ci:
    @just test --all-targets --profile ci

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
            src/*|examples/*|tests/*|Cargo.toml|Cargo.lock|build.rs|.cargo/*|.config/nextest.toml|.config/cargo-fast.toml|mise.toml|mise.lock|.config/mise-bootstrap.json)
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

# Build an optimized native binary; not part of routine edit/test feedback.
build-release *args:
    @just _cargo build --locked --release "$@"

# Build under the shared budget, then release the lock while the application runs.
# Pass build options before -- and application arguments after it.
run *args:
    #!/usr/bin/env python3
    import json
    import os
    import subprocess
    import sys

    arguments = sys.argv[1:]
    separator = arguments.index('--') if '--' in arguments else len(arguments)
    build = arguments[:separator]
    application = arguments[separator + 1:]
    result = subprocess.run(
        ['just', 'build', '--message-format=json-render-diagnostics', *build],
        stdout=subprocess.PIPE, text=True,
    )
    if result.returncode:
        raise SystemExit(result.returncode)
    executables = set()
    for line in result.stdout.splitlines():
        artifact = json.loads(line)
        if (artifact.get('reason') == 'compiler-artifact' and artifact.get('executable')
                and artifact.get('target', {}).get('kind') in (['bin'], ['example'])):
            executables.add(artifact['executable'])
    if len(executables) != 1:
        raise SystemExit('Select exactly one executable using --bin or --example.')
    executable = executables.pop()
    os.execv(executable, [executable, *application])

# Open the demo TUI; extra arguments can request onboarding or snapshots.
demo *args:
    @just run -- --demo "$@"

# Open the demo onboarding flow, optionally capturing a snapshot.
demo-onboarding *args:
    @just demo --onboarding "$@"

# Generate the deterministic UI gallery in .snapshots/.
gallery:
    @just run --example gallery

# Generate syncing animation frames from fictional demo fixtures.
sync-preview:
    @just run --example sync_preview

# Restore manifest-pinned cross (normally handled by setup).
install-cross:
    @just _setup-tools rust cross

# Set the package version for a release tag and update only the workspace lock entry.
release-version tag:
    #!/usr/bin/env python3
    from pathlib import Path
    import re
    import subprocess
    import sys

    tag = sys.argv[1]
    if not re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?', tag):
        raise SystemExit('Expected a semantic version with a v prefix, for example v0.1.0')
    manifest = Path('Cargo.toml')
    text = manifest.read_text()
    placeholder = 'version = "0.0.0"'
    if text.count(placeholder) != 1:
        raise SystemExit('Cargo.toml package version placeholder is missing or ambiguous')
    manifest.write_text(text.replace(placeholder, f'version = "{tag[1:]}"', 1))
    subprocess.run(['just', '_cargo', 'update', '--workspace', '--offline'], check=True)

# Build the release binary for one cross target.
release-build target:
    @just _exec cross build --locked --release --target "$1" --bin cronk

# Verify the embedded release version through cross's target runner/emulation.
release-check target tag sha:
    #!/usr/bin/env bash
    set -euo pipefail
    expected="cronk ${2}@${3:0:12}"
    actual=$(just _exec cross run --locked --release --target "$1" --bin cronk -- --version)
    if [[ "$actual" != "$expected" ]]; then
        echo "Release version mismatch: expected '$expected', got '$actual'" >&2
        exit 1
    fi
    echo "$actual"

# Full pre-publication Rust validation, sequentially (not on every edit).
validate: fmt lint test-all doc

# Regression-test task routing and affected-file selection without building Rust.
workflow-test:
    python3 -m unittest discover -s tests/workflow -v

# Keep Cargo implementation details private; forward arguments without re-parsing.
_cargo +args:
    @just _exec cargo "$@"

# Local: shared worktree lock + budget. CI: isolated runners use their full budget.
_exec tool +args:
    #!/usr/bin/env bash
    set -euo pipefail
    tools=(rust)
    if [[ "$1" == cross ]]; then tools+=(cross); fi
    if [[ "$1" == cargo && "$2" == nextest ]]; then
        tools+=(nextest)
        if ! just _mise exec rust nextest -- cargo nextest --version >/dev/null 2>&1; then
            echo 'cargo-nextest is required; run just setup to install the pinned tools.' >&2
            echo 'Do not fall back to cargo test; only just doc uses that runner.' >&2
            exit 1
        fi
    fi
    if [[ "${CI:-}" == true ]]; then
        exec just _mise exec "${tools[@]}" -- "$@"
    fi
    tools+=(mold sccache)
    if ! command -v flock >/dev/null 2>&1; then
        echo 'flock (util-linux) is required for cross-worktree coordination.' >&2
        exit 1
    fi
    common_dir=$(git rev-parse --git-common-dir)
    echo 'Waiting for the repository local-validation lock...' >&2
    export CARGO_BUILD_JOBS="${CRONK_BUILD_JOBS:-${CARGO_BUILD_JOBS:-2}}"
    # Close the descriptor in children so sccache cannot retain the lock forever.
    exec flock --close "$common_dir/cronk-dev.lock" just _mise exec "${tools[@]}" -- "$@"
