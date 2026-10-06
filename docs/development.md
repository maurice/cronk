# Faster development feedback on a shared machine

## Setup and tasks

Install [just](https://just.systems/man/en/installation.html) 1.16 or newer and run **`just setup`**. These recipes support Linux/WSL and Linux devcontainers. Debian/Ubuntu setup installs missing native prerequisites through APT (with sudo when not root); on other Linux distributions, provide equivalent native packages first. Bash 4+ is assumed, as on the supported Debian/Ubuntu hosts.

Setup bootstraps a SHA-256-verified pinned mise binary into the user data directory, installs repository-pinned Rust (including rustfmt/Clippy), nextest, sccache, mold and cross, activates checked-in Git hooks, and enables the local fast-build configuration on x86-64. Rust, nextest, sccache and mold use upstream binaries; cross is compiled once from the pinned Git revision to preserve the previous release installer behavior. Repeat runs reuse installed tools. No global Cargo config or shell startup file is modified; just tasks use `mise exec` internally.

- `mise.toml`: tool version pins, Rust components and release-tool revision.
- `mise.lock`: resolved versions, artifact URLs and checksums where supported. Rust uses rustup's verification; the Git-source cross entry is revision-pinned rather than a binary checksum.
- `.config/mise-bootstrap.json`: mise version and trusted Linux binary hashes (x86-64/ARM64).
- `.config/cargo-fast.toml`: compiler/linker settings, copied to ignored worktree-local Cargo configuration.

Native compiler/system packages remain managed by the OS—not by Cargo. Setup checks for a C compiler, Clang, pkg-config, Git, curl, flock, Python and checksum utilities; it installs only missing prerequisites and does not run an OS upgrade. Existing custom Cargo configuration/hooks are never overwritten. An existing Cargo configuration needs a manual merge, because silently replacing it would be unsafe.

Run `just` to discover the checked-in `justfile` tasks. There are no custom build/test shell scripts.

```sh
just setup                           # complete one-command onboarding
just tools                           # inspect managed versions
just demo                            # the usual interactive TUI feedback loop
just demo --snapshot dashboard.png   # deterministic demo capture
just demo-onboarding                 # the demo setup flow
just gallery                         # PNG/Markdown gallery in .snapshots/
just sync-preview                    # animation frames for the syncing preview
just check --lib                      # type feedback without linking
just test-lib -E 'test(filter)'        # only library tests matching a name
just test-integration ui -E 'test(tab_switches)'
just test-affected                    # branch + local changes vs origin/main
just test-affected HEAD               # only staged/unstaged/untracked changes
just fmt                             # formatting check
just format                          # apply formatting
just lint                            # all-target Clippy, warnings denied
just test-all                        # complete nextest target set
just doc                             # doctests only
just validate                        # fmt, lint, test-all, doc, sequentially
just workflow-test                   # task regression tests, no Rust builds
just check-all
just build
just build-release                   # optimized native build, when specifically needed
just run                             # the configured real workspace
```

Arguments are forwarded without losing spaces/quotes and recipes run from the justfile directory, including when invoked from a subdirectory. Heavy tasks use `--locked`. `test` accepts arbitrary nextest target/filter flags; the dedicated recipes make target selection explicit. `lint` always checks all targets with warnings denied.

**Agents must use just and nextest**, as specified in `AGENTS.md`. Doctests (`just doc`) are the only exception because nextest cannot execute them. Missing nextest is a setup error, not permission to use another runner. Historical runner-comparison commands in `test-performance.md` are benchmark records, not the development workflow.

## Tool updates and devcontainers

For a tooling update, edit the appropriate pin in `mise.toml`, run `just tools-lock`, review/commit both files, then run `just setup`. To update mise itself, review the new upstream binary hashes and update `.config/mise-bootstrap.json` as well as the minimum version in `mise.toml`. There is intentionally no Renovate/Dependabot configuration added for tooling at this stage.

A Debian/Ubuntu Linux devcontainer needs just available and can use `just setup` as its `postCreateCommand`. Root containers install missing APT prerequisites directly; non-root users need sudo permissions. The same checked-in pins and lockfile are used—no separate hand-maintained sequence of tool installers is required. Preserve the user data directories across container recreation if you want to reuse tool downloads, and sccache's user cache for dependency reuse. Do not share one Cargo `target/` between divergent worktrees.

## Optional mold and sccache

`just setup` already installs both tools and enables their worktree configuration on x86-64. To inspect the cache or change that choice:

```sh
just fast-setup     # re-enable the generated configuration if previously disabled
just cache-stats    # inspect shared compiler-cache hits and misses
# Only if you want to stop the trial (also causes a rebuild):
just fast-disable
```

The fast-configuration task supports x86-64 Linux/WSL; other supported Linux architectures get pinned tools/hooks without that target-specific configuration. It performs a mold link smoke test and copies `.config/cargo-fast.toml` to ignored `.cargo/config.toml`. Existing Cargo configuration is never overwritten; customized files must be merged manually, and the disable task will not remove them. Global Cargo configuration is untouched. Each worktree runs its own setup but shares installed tools and sccache's user-level disk cache.

mold uses two linker threads to stay within the local budget. Cargo's application incremental compilation remains enabled; non-incremental dependencies can benefit from sccache. Applications, procedural macros and some generated inputs are not cacheable. Don't turn off incremental builds just to inflate cache hit rates.

The [measured trial](build-performance.md) demonstrated dependency-cache hits across worktrees, but found little mold advantage over this Rust toolchain's already-fast bundled lld. Treat mold as an experiment rather than an assumed dramatic speedup. Keep flags stable after choosing your setup. Environment overrides such as `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS` and `RUSTC_WRAPPER` take precedence over Cargo configuration, so avoid conflicting overrides.

## CI and release tasks

GitHub Actions uses the same public tasks:

| Purpose | Task |
| --- | --- |
| Formatting | `just fmt` |
| All-target lint | `just lint` |
| Complete tests, all failures, full runner concurrency | `just test-ci` |
| Documentation tests | `just doc` |
| Native optimized build | `just build-release` |
| Just-task regression tests | `just workflow-test` |
| Release version + workspace lock update | `just release-version v0.1.0` |
| Rust-only bootstrap (format/native build jobs) | `just setup-ci rust` |
| Rust + nextest bootstrap (test/lint job) | `just setup-ci checks` |
| Rust + cross bootstrap (release job) | `just setup-ci release` |
| Cross-target release build | `just release-build aarch64-unknown-linux-gnu` |
| Embedded version verification via target runner | `just release-check aarch64-unknown-linux-gnu v0.1.0 "$GITHUB_SHA"` |

Under GitHub Actions' `CI=true`, heavy recipes bypass the local advisory lock and two-job default so isolated runners retain their normal compiler budget. Local commands retain coordination. Never set `CI=true` just to evade the local resource budget. CI/release runners do **not** activate the ignored optional mold/sccache configuration. The CI jobs remain independently parallelized; full coverage, release version checks and cross-runner/emulation behavior are preserved.

The release-version task changes the manifest and workspace lockfile intentionally; do not run it as normal edit-loop validation. Setup uses strict locked tool installation; CI caches tools by the manifest/lockfile/bootstrap content and exposes the selected Rust version to the artifact-cache action. Separate nextest/cross installer steps are no longer necessary.

## Affected-file selection

`just test-affected [base-ref]` compares HEAD against the merge base with the given ref (default `origin/main`) and includes staged, unstaged and untracked files. It does not fetch automatically; keep your base ref current as part of branch setup.

- Changes only to `tests/<name>.rs` select those integration-test targets, deduplicated. Cargo still rebuilds their dependencies as necessary, but unrelated test binaries need not be built/run.
- Shared application/library source, examples, Cargo files, `build.rs`, Cargo/tooling configuration, nextest configuration, test helpers/fixtures, and removed/renamed test targets conservatively select `--all-targets`.
- Other changes (for example Markdown) do not schedule Rust tests. Run `just doc` for doctest changes and `just workflow-test` for justfile/workflow changes.
- An invalid/missing base ref fails rather than claiming nothing needs testing. Pass an explicit valid ref when working without `origin/main`.

This is a conservative file-based first step, **not semantic test-impact analysis**. A library module can affect UI/integration behavior far from that file, so source changes must not be guessed to affect only similarly named tests. For rapid iteration on shared source, choose a known relevant target/name manually; before publishing, use the broad check. `just test-all` and CI retain complete coverage regardless of affected selection.

## Avoid oversubscription before chasing faster tools

The private `_exec` recipe holds a `flock` lock in Git's common metadata directory for the lifetime of a heavy command, including test execution. Linked worktrees share this lock. Each keeps its own `target/` directory: sharing one between divergent branches can cause Cargo lock waits, artifact replacement and repeated rebuilds. Independent clones do not share the validation lock.

Two compiler jobs and two nextest test processes leave headroom for the desktop and other agents. Tests may themselves create threads, so these limits are not a hard CPU/memory quota. Nextest improves execution scheduling, **not compilation**. Multiple agents each running at full CPU concurrency can be slower than queuing them, especially when linking pushes memory into swap.

The lock is advisory: direct Cargo commands, editor background checks, old hooks and agents in other branches do not acquire it. Existing sessions/worktrees must adopt the justfile and updated hook. `just setup` sets the repository-local hooks path to `.cargo-husky/hooks` (and refuses to overwrite a custom path); linked worktrees share that setting but resolve the hook from their own checked-in files. This avoids depending on cargo-husky's already-cached installation step to pick up updates. Do not delete the lock file while commands are running. The lock is held by flock's parent process and released when the command finishes; its descriptor is closed in children so a long-lived sccache server cannot retain it. Formatting does not need the lock. `just run` / `just demo` release it after building, before launching the application, so using the interactive TUI does not block validation in other worktrees. Cargo's artifact metadata supplies the executable path, including custom target directories. CI uses the same just tasks on isolated runners, with full coverage and `test-threads = "num-cpus"`.

For a deliberately larger budget on an otherwise idle machine:

```sh
CRONK_BUILD_JOBS=4 NEXTEST_TEST_THREADS=4 just test-integration ui
```

Compiler jobs otherwise respect `CARGO_BUILD_JOBS`, falling back to two. Coordinate overrides with other agents; don't increase them automatically. If an editor invokes Cargo outside just, cap its concurrency and avoid duplicate watch/test loops.

## Cut work from the edit loop

- Use `check --lib` for compile/type feedback; it avoids linking. Batch meaningful edits before testing.
- Choose a test **target** and then a name filter. A filter expression alone does not prevent Cargo from building unrelated integration-test binaries.
- Reserve all-target lint/tests for milestone/final validation or broad changes. The pre-commit hook still runs formatting and cached all-target Clippy; it does not skip validation on a timestamp heuristic.
- Keep build flags/profiles stable. Don't alternate release/debug builds, run `cargo clean`, or turn off incremental compilation as routine troubleshooting.
- `[profile.dev] debug = 1` (inherited by tests) retains line-number backtraces with less debug data to generate/link/store than Cargo's full-debug default. This costs one rebuild when adopted. If inspecting variables in a debugger is necessary, explicitly opt into full debug info and expect another rebuild.
- Preserve test output capture. Nextest's `--no-capture` runs tests serially; use captured failure output rather than disabling capture routinely.

## Further improvements to measure, not enable blindly

1. **Dependency features:** `tui-lipan` includes PNG snapshots, Markdown and diff rendering. Measure their build cost before splitting snapshot-only features from normal development; keep snapshot coverage in CI and avoid feature combinations that repeatedly rebuild dependencies.
2. **Expensive UI tests:** profile the scrollbar/headless runtime tests. Prefer deterministic event completion over long sleeps/polling if supported, and separate narrow state tests from full rendering tests without dropping end-to-end coverage.

Measure on an idle machine with a fixed toolchain, flags, target selection and budget. Record cold builds separately from warm edits/test execution; a busy multi-agent run is not a reliable before/after speedup benchmark. The existing [runner comparison](test-performance.md) measured execution only with different historical defaults; [the build-tool trial](build-performance.md) separately measured dependency reuse and linker behavior.

Repository instructions and just tasks provide a supported path, not an execution sandbox. Strict enforcement against arbitrary shell commands requires a deny/allow policy in the agent harness; a Cargo alias cannot replace the built-in `cargo test` command. Never globally shadow Cargo just to enforce this repository's policy.
