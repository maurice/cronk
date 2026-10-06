# Test runner comparison

This is a historical benchmark record, not the current agent workflow. Use [the just tasks](development.md) for normal validation. Reproducing the runner comparison is an explicit benchmarking task only: it intentionally invokes the old runner and should never be part of routine agent checks.

## Results

Measured on 2026-10-05 in an isolated worktree based on `7f9afc1`, with unchanged test sources and dependencies:

| Paired run | Cargo's built-in test runner | nextest | Speedup |
| --- | ---: | ---: | ---: |
| 1 | 324.207 s | 192.612 s | 1.68× |
| 2 | 182.663 s | 140.361 s | 1.30× |
| 3 | 200.411 s | 147.367 s | 1.36× |
| **Median** | **200.411 s** | **147.367 s** | **1.36×** |

The median saves **53.044 seconds (26.5%)** per full warm run. All six runs passed **272 tests**, with **zero ignored/skipped tests**, across **10 test binaries** (two contain no tests). No tests were rewritten, removed, filtered, or retried to achieve this result.

This is a useful improvement, not a claim of 3× speedup for this project. Cargo's existing runner executes binaries sequentially while nextest schedules individual tests across binaries. The expensive headless UI/scrollbar tests still dominate runtime, and per-test process startup can add overhead for very short tests.

## Environment and methodology

- Linux x86_64, WSL2 (`6.18.33.2-microsoft-standard-WSL2`).
- Intel Core i7-1185G7, 4 physical cores / 8 logical CPUs available.
- `rustc 1.97.1 (8bab26f4f 2026-07-14)`; `cargo-nextest 0.9.146`.
- Default debug/test build profile; same isolated worktree and local `target/` directory for both runners. No release-profile or dependency changes.
- Both runners used their then-default concurrency (8 logical CPUs); the nextest configuration at the measured revision made that default explicit.
- Test binaries were built first with `cargo test --locked --all-targets --no-run`; an unmeasured nextest run warmed execution/discovery. The initial build took 122.93 s and is **excluded** from the comparison.
- Three measured pairs, ordered Cargo → nextest, nextest → Cargo, Cargo → nextest. Commands ran sequentially, never concurrently with each other. Whole-command wall time was measured with Python's monotonic clock; stdout/stderr went to log files for both runners. Warm Cargo checks, discovery and runner startup are included.
- This was a shared development host, not an exclusive benchmark machine. Other agent activity and short tooling checks can affect timings, especially the first pair. The broad range is why all samples are shown rather than just the fastest run. These are local measurements, **not GitHub Actions CI timings** or guaranteed speedups on other machines.

Nextest does not run doctests. The old all-targets command also did not run them. CI now runs `just doc` separately to cover documentation tests; its time is **not included** in this like-for-like runner comparison. Runner installation, linting, release builds and cold compilation are also excluded. Nextest does not accelerate compilation.

## Current validation and measurements

For normal validation, use the [development tasks](development.md), not the old runner comparison:

```sh
just test-all
just doc
# CI collects every failure:
just test-ci
```

Reproducing the historical Cargo-versus-nextest comparison requires explicitly checking out the measured revision and following its recorded methodology on an idle host; it is not a supported routine agent task. The old raw-Cargo reproduction script has intentionally been removed from current instructions.

Keep the Rust toolchain, nextest version, test targets, build profile, CPU allocation and output capture consistent when measuring current tasks. Avoid `--no-capture`, which serializes nextest execution. At the measured revision, CI differed only in disabling fail-fast; `just test-ci` does the same via `--no-fail-fast`.
