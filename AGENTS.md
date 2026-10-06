# Project workflow

## Local feedback

- **Use `just` tasks for local Rust compilation, linting and tests**, not raw Cargo commands. Run `just setup` once per machine (needs just, mise, rustup); run `just` to list tasks. There is deliberately no cross-worktree lock, so sessions never block each other; each worktree keeps its own `target/`.
- **Unit/integration tests must use cargo-nextest** via `just test`. Never use `cargo test`, even as a fallback; the only exception is doctests: `just doc`.
- Iterate with the smallest relevant target: `just check --lib`, `just test --lib -E 'test(name)'`, or `just test --test ui -E 'test(name)'`. Name filters alone do not limit which binaries Cargo compiles: always select a target.
- Run tests after a meaningful batch of edits, not after every small edit. Before publishing, run `just validate` (fmt, lint, all nextest targets, doctests) and report exactly what was validated.
- Do not run release builds, `cargo clean`, change `RUSTFLAGS`/profiles/tool pins, or use nextest `--no-capture` without a task-specific need. Do not share a `CARGO_TARGET_DIR` between worktrees. If the machine is busy, lower your own `CARGO_BUILD_JOBS` rather than waiting on others.
- The pre-commit hook runs `just fmt` and `just lint`; do not skip it.
- For UI feedback use `just demo` (`--snapshot <path>` for a deterministic capture), `just demo-onboarding` or `just gallery`.
- Pins: Rust in `rust-toolchain.toml`, nextest/sccache in `mise.toml`/`mise.lock`. Change them only for a tooling task. See [development notes](docs/development.md).

## Delivery

All features/fixes should be developed on a new dedicated branch, based off latest origin/main.

After completing each requested change, run the relevant validation, commit the task's changes, and push the current branch to the remote.

Do not include unrelated work or secrets, force-push is only allowed if a feature-branch needs rebasing. Report any validation, commit, or push failure.

Create a Pull Request with a summary of the changes, screenshots if relevant, interesting technical implementation details, before and after benchmarks if appropriate.

Commit messages and PR titles should be a concise description of the change, without prefix: do not use commitlint style titles.
