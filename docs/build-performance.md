# Local mold and sccache trial

Measured on 2026-10-06 on this Linux/WSL x86-64 development host (8 logical CPUs), with Rust 1.99.0, mold 3.0.0 and sccache 0.18.0. Compiler jobs and linker threads were limited to two. The host was not exclusively reserved, so these are observations, not guaranteed speedups.

## Dependency reuse with sccache

All builds used `just build --lib` with the same dependency graph, development profile (`debug = 1`) and optional mold/sccache configuration. Each had a **fresh Cargo target directory**; the latter two reused the compiler cache, not existing Cargo artifacts. Application incremental compilation stayed enabled.

| Scenario | Cargo-reported build time | Added Rust cache hits | Added Rust cache misses |
| --- | ---: | ---: | ---: |
| Initial cache fill, fresh target directory | 193 s | 0 | 250 |
| Another fresh target directory, same worktree | 113 s | 230 | 20 |
| Temporary linked worktree with its own fresh target directory | 97 s | 230 | 20 |

The two reuse runs each also added 31 native/assembler hits. This demonstrates actual cross-worktree dependency reuse. It is a single sequence, not a paired no-sccache comparison; filesystem caches and host load also affect elapsed time. Do not infer that every warm edit will be twice as fast.

Executables, procedural macros, incremental application compilations, compiler feature probes and dependencies whose generated inputs change can still miss or bypass the cache. Build-script probe failures can appear in sccache's counters even when the Cargo build succeeds. All three Cargo builds succeeded; the cache reported no read/write errors or timeouts.

To inspect ongoing reuse, run `just cache-stats`. A no-op Cargo build may not invoke the compiler at all, so it is not a meaningful cache benchmark. Do not clear a shared cache or disable application incremental compilation merely to improve hit percentages.

## mold versus Rust's existing lld

This Rust toolchain already uses bundled lld on x86-64 Linux; comparing mold only against GNU bfd would exaggerate its benefit.

One actual `cronk` debug link was captured with lld's reproduction archive. The same captured inputs were replayed directly through bundled lld 23.1.1 and mold 3.0.0. Both used two threads. After warmups, six paired runs alternated linker order. These timings measure **linking only**, excluding Rust compilation, Cargo checks and test execution.

| Linker | Warm median | Range | Output size |
| --- | ---: | ---: | ---: |
| Bundled lld | 0.662 s | 0.625–0.715 s | 172,355,000 bytes |
| mold | 0.642 s | 0.439–0.821 s | 189,666,088 bytes |

Both linked binaries ran and reported the expected CLI build version. The approximately 3% median difference is small relative to run-to-run noise; mold's output was approximately 10% larger. **There is no convincing large linker speedup here.** Keep mold an opt-in experiment, not a universal CI/release default. sccache's dependency reuse is the more promising improvement for this multi-worktree setup.

## Scope and setup

The normal `just setup` task now installs the pinned tooling through mise and activates the fast configuration on x86-64. `just fast-setup` can re-enable it: it copies `.config/cargo-fast.toml` to ignored `.cargo/config.toml` in the current worktree, validates x86-64 Linux support and performs a linker smoke test. It refuses to overwrite existing Cargo configuration. Global Cargo settings and CI/release configuration remain unchanged.

`just fast-disable` removes only an unmodified generated configuration. Both enabling and disabling change compiler/linker settings and can cause a rebuild, so pick stable settings instead of toggling them on every task. Each worktree runs setup separately and shares sccache's user-level disk cache while retaining its own Cargo target directory. The initial measurements used Homebrew-installed tool binaries; normal onboarding now uses the same version pins with managed upstream artifacts, avoiding separate Homebrew installation steps.

See [development tasks](development.md) for the normal workflow and [sccache's Rust limitations](https://github.com/mozilla/sccache/blob/main/docs/Rust.md) for which compilations are cacheable.
