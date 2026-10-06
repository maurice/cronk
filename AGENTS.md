# Project workflow

## Local feedback and resource budget

- **Use `just` tasks for local Rust compilation, linting and tests.** The `justfile` serializes heavy commands across this repository's worktrees and defaults to two compiler jobs. Do not bypass it with direct Cargo commands, background validation, or parallel validation commands. CI runs independently. Run `just` to discover tasks and `just setup` when adopting the workflow to activate the checked-in hook (do not overwrite custom hooks).
- **Unit/integration tests must use cargo-nextest**, via `just test`, `just test-lib`, `just test-integration`, or `just test-affected`. Never use `cargo test`, even as a fallback or with `--no-run`; install nextest or report the missing tool. The only exception is doctests: `just doc` (nextest cannot run them).
- Iterate with the smallest relevant target: `just check --lib`, `just test-lib -E 'test(name)'`, or `just test-integration ui -E 'test(name)'`. Name filters alone do not limit which binaries Cargo compiles: always select a target for focused feedback.
- `just test-affected [base-ref]` includes branch, staged, unstaged and untracked changes (default base: `origin/main`). Integration-test-only edits select those targets; shared source/build/dependency changes conservatively select all targets. This is file-based selection, not a complete impact/dependency analysis. Use explicit targets/filters while iterating on shared source, and broad validation before publishing.
- Run relevant tests after a meaningful batch of edits, not after every small edit. Before publishing Rust changes, run `just fmt`, `just lint`, and the relevant nextest targets; use `just test-all` for broad/shared changes and `just doc` for documentation examples. `just validate` runs the full sequence. Report exactly what was validated; do not claim a focused run covered the full suite.
- Do not run release builds, `cargo clean`, alter `RUSTFLAGS`/profiles, disable incremental compilation, increase job counts, or use nextest `--no-capture` without a task-specific need. These cause rebuilds, contention, or serial test execution. Keep worktree target directories separate.
- The pre-commit hook repeats cached formatting/Clippy checks through the same tasks; do not skip hooks. Just workflow regression tests: `just workflow-test`.
- See [development feedback](docs/development.md) for setup, resource overrides and further optimization options.

## Delivery

All features/fixes should be developed on a new dedicated branch, based off latest origin/main.

After completing each requested change, run the relevant validation, commit the task's changes, and push the current branch to the remote.

Do not include unrelated work or secrets, force-push is only allowed if a feature-branch needs rebasing. Report any validation, commit, or push failure.

Create a Pull Request with a summary of the changes, screenshots if relevant, interesting technical implementation details, before and after benchmarks if appropriate.

Commit messages and PR titles should be a concise description of the change, without prefix: do not use commitlint style titles.
