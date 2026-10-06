# Development

## Setup

Install [just](https://just.systems/man/en/installation.html), [mise](https://mise.jdx.dev/getting-started.html) and [rustup](https://rustup.rs), plus a C compiler (`build-essential` on Debian/Ubuntu). Then run `just setup`.

| File | Pins |
| --- | --- |
| `rust-toolchain.toml` | Rust version, clippy, rustfmt (rustup reads it for everything, including editors) |
| `mise.toml` / `mise.lock` | nextest and sccache, with checksums |

To update a pin: edit it, run `mise lock --platform linux-x64 --platform linux-arm64` for mise tools, and commit.

`just` recipes call Cargo through `mise exec --`, so no shell activation is needed. `just` lists the tasks; they are thin wrappers that add `--locked` and nextest, and forward all arguments:

```sh
just check --lib                  # type feedback without linking
just test --lib -E 'test(name)'   # one target, filtered
just test --test ui               # one integration binary
just lint                         # clippy, all targets, -D warnings (pre-commit hook)
just validate                     # fmt, lint, all tests, doctests
```

Agents must use these tasks, not raw `cargo test`. `just doc` is the only exception because nextest cannot run doctests. Choose a test **target** before a name filter: a filter alone doesn't stop Cargo building every test binary. Avoid `--no-capture` (nextest then runs serially).

## Concurrency: deliberately no locking

Each worktree has its own `target/`, and nothing takes a cross-worktree lock, so one session can never block another. Sessions share only sccache's disk cache (set via `RUSTC_WRAPPER` for local runs; skipped when `CI` is set), which lets a fresh worktree reuse compiled dependencies. If the machine is oversubscribed, cap an individual run yourself, e.g. `CARGO_BUILD_JOBS=4 just lint`, `just test --test-threads 2`.

Don't share one `CARGO_TARGET_DIR` between worktrees: Cargo's build-directory lock would serialise them.

## Linker

mold was trialled and removed. Rust already links with its bundled lld on x86-64 Linux: on a real `cronk` debug link, mold was ~3% faster at the median (within noise) and produced a ~10% larger binary. sccache dependency reuse (fresh worktree: 193 s cold → ~100 s) was the worthwhile part.

## CI and releases

CI (`.github/workflows/ci.yml`) uses `.github/actions/setup` (just, mise tools, pinned Rust) then the same tasks: `just fmt`, `just lint`, `just test-ci`, `just doc`, `just build-release`. The release workflow needs only Rust and a revision-pinned `cross` (`cargo install --rev`), and runs `cross` directly.

## Keeping the edit loop fast

- Prefer `check --lib` for compile feedback, and batch edits before testing.
- Keep build flags and profiles stable; switching them or running `cargo clean` forces rebuilds.
- `[profile.dev] debug = 1` keeps line-number backtraces with less debug data to generate and link.
