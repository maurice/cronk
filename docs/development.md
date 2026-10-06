# Faster development feedback on a shared machine

## Setup and tasks

Install [just](https://just.systems/man/en/installation.html) 1.16 or newer, Bash 4+, `flock` (util-linux), and cargo-nextest (CI uses 0.9.146; prefer the [pre-built installer](https://nexte.st/docs/installation/) over compiling it). These recipes target Linux/WSL. Other hosts need compatible Bash/flock installations; they never silently drop coordination.

Run `just` to discover the checked-in `justfile` tasks. There are no custom build/test shell scripts.

```sh
just setup                           # activate checked-in Git hooks
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
just build
just run -- --demo
```

Arguments are forwarded without losing spaces/quotes and recipes run from the justfile directory, including when invoked from a subdirectory. Heavy tasks use `--locked`. `test` accepts arbitrary nextest target/filter flags; the dedicated recipes make target selection explicit. `lint` always checks all targets with warnings denied.

**Agents must use just and nextest**, as specified in `AGENTS.md`. Doctests (`just doc`) are the only exception because nextest cannot execute them. Missing nextest is a setup error, not permission to use another runner. Historical runner-comparison commands in `test-performance.md` are benchmark records, not the development workflow.

## Affected-file selection

`just test-affected [base-ref]` compares HEAD against the merge base with the given ref (default `origin/main`) and includes staged, unstaged and untracked files. It does not fetch automatically; keep your base ref current as part of branch setup.

- Changes only to `tests/<name>.rs` select those integration-test targets, deduplicated. Cargo still rebuilds their dependencies as necessary, but unrelated test binaries need not be built/run.
- Shared application/library source, examples, Cargo files, `build.rs`, Cargo configuration, nextest configuration, test helpers/fixtures, and removed/renamed test targets conservatively select `--all-targets`.
- Other changes (for example Markdown) do not schedule Rust tests. Run `just doc` for doctest changes and `just workflow-test` for justfile/workflow changes.
- An invalid/missing base ref fails rather than claiming nothing needs testing. Pass an explicit valid ref when working without `origin/main`.

This is a conservative file-based first step, **not semantic test-impact analysis**. A library module can affect UI/integration behavior far from that file, so source changes must not be guessed to affect only similarly named tests. For rapid iteration on shared source, choose a known relevant target/name manually; before publishing, use the broad check. `just test-all` and CI retain complete coverage regardless of affected selection.

## Avoid oversubscription before chasing faster tools

The private `_cargo` recipe holds a `flock` lock in Git's common metadata directory for the lifetime of a heavy command, including test execution. Linked worktrees share this lock. Each keeps its own `target/` directory: sharing one between divergent branches can cause Cargo lock waits, artifact replacement and repeated rebuilds. Independent clones do not share the validation lock.

Two compiler jobs and two nextest test processes leave headroom for the desktop and other agents. Tests may themselves create threads, so these limits are not a hard CPU/memory quota. Nextest improves execution scheduling, **not compilation**. Multiple agents each running at full CPU concurrency can be slower than queuing them, especially when linking pushes memory into swap.

The lock is advisory: direct Cargo commands, editor background checks, old hooks and agents in other branches do not acquire it. Existing sessions/worktrees must adopt the justfile and updated hook. `just setup` sets the repository-local hooks path to `.cargo-husky/hooks` (and refuses to overwrite a custom path); linked worktrees share that setting but resolve the hook from their own checked-in files. This avoids depending on cargo-husky's already-cached installation step to pick up updates. Do not delete the lock file while commands are running. It is released automatically when the process (and any children retaining its descriptor) exits. Formatting does not need the lock. `just run` holds it until the application exits, so close interactive runs before scheduling validation. CI continues using direct Cargo commands on isolated runners, with full coverage and `test-threads = "num-cpus"`.

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

1. **Faster linker:** try lld or mold in a personal Cargo config after confirming host/target support. Compare warm edit-to-test times and artifact size. Don't commit Linux-only linker flags or change flags repeatedly (they invalidate caches).
2. **Compiler cache:** sccache can reuse eligible dependency compilations across worktrees with consistent flags/toolchains. Incremental crate compilations are not cacheable by sccache; keep incremental builds for the application rather than globally disabling them for cache hit rates. A cache does not replace the resource budget.
3. **Dependency features:** `tui-lipan` includes PNG snapshots, Markdown and diff rendering. Measure their build cost before splitting snapshot-only features from normal development; keep snapshot coverage in CI and avoid feature combinations that repeatedly rebuild dependencies.
4. **Expensive UI tests:** profile the scrollbar/headless runtime tests. Prefer deterministic event completion over long sleeps/polling if supported, and separate narrow state tests from full rendering tests without dropping end-to-end coverage.

Measure on an idle machine with a fixed toolchain, flags, target selection and budget. Record cold builds separately from warm edits/test execution; a busy multi-agent run is not a reliable before/after speedup benchmark. The existing [runner comparison](test-performance.md) measured execution only with different historical defaults, not these resource/compile changes.

Repository instructions and just tasks provide a supported path, not an execution sandbox. Strict enforcement against arbitrary shell commands requires a deny/allow policy in the agent harness; a Cargo alias cannot replace the built-in `cargo test` command. Never globally shadow Cargo just to enforce this repository's policy.
