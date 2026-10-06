"""Regression tests for just tasks, using fake Cargo rather than building Rust."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest


@unittest.skipUnless(shutil.which("flock") and shutil.which("just"), "requires just and flock")
class JustWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cronk workflow ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        source = Path(__file__).resolve().parents[2] / "justfile"
        shutil.copy2(source, self.repo / "justfile")
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        self.git("add", "justfile")
        self.commit("Initial")
        self.git("branch", "baseline")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        cargo = self.bin / "cargo"
        cargo.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, sys, time\n"
            "if sys.argv[1:] == ['nextest', '--version']:\n"
            "    sys.exit(int(os.environ.get('MISSING_NEXTEST', '0')))\n"
            "with open(os.environ['CALL_LOG'], 'a') as log:\n"
            "    log.write(json.dumps({'args': sys.argv[1:], 'cwd': os.getcwd(), "
            "'jobs': os.environ.get('CARGO_BUILD_JOBS')}) + '\\n')\n"
            "time.sleep(float(os.environ.get('CARGO_DELAY', '0')))\n"
            "sys.exit(int(os.environ.get('CARGO_EXIT', '0')))\n"
        )
        cargo.chmod(0o755)
        self.log = self.root / "calls.jsonl"
        self.env = {**os.environ, "PATH": f"{self.bin}:{os.environ['PATH']}",
                    "CALL_LOG": str(self.log)}
        for name in ("CARGO_BUILD_JOBS", "CRONK_BUILD_JOBS"):
            self.env.pop(name, None)

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.repo), *args], check=True,
                              capture_output=True, text=True)

    def commit(self, message):
        self.git("-c", "user.name=Test", "-c", "user.email=test@example.com",
                 "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", message)

    def change(self, path, content="// changed\n"):
        file = self.repo / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(content)

    def run_just(self, *args, env=None, repo=None):
        return subprocess.run(
            ["just", "--justfile", str((repo or self.repo) / "justfile"), *args],
            cwd=self.root, env={**self.env, **(env or {})},
            capture_output=True, text=True, timeout=15,
        )

    def calls(self):
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_command_routing_and_argument_quoting(self):
        cases = [
            (("fmt",), ["fmt", "--all", "--", "--check"], None),
            (("format",), ["fmt", "--all"], None),
            (("check", "--lib"), ["check", "--locked", "--lib"], "2"),
            (("lint",), ["clippy", "--locked", "--all-targets", "--", "-D", "warnings"], "2"),
            (("test-integration", "ui", "-E", "test(tab switches)"),
             ["nextest", "run", "--locked", "--test", "ui", "-E", "test(tab switches)"], "2"),
            (("test-all",), ["nextest", "run", "--locked", "--all-targets"], "2"),
            (("test-lib", "-E", "test(filter)"),
             ["nextest", "run", "--locked", "--lib", "-E", "test(filter)"], "2"),
            (("doc",), ["test", "--locked", "--doc"], "2"),
            (("build",), ["build", "--locked"], "2"),
            (("run", "--", "--demo"), ["run", "--locked", "--", "--demo"], "2"),
        ]
        for args, expected, jobs in cases:
            with self.subTest(args=args):
                result = self.run_just(*args)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.calls()[-1], {
                    "args": expected, "cwd": str(self.repo), "jobs": jobs,
                })

    def test_missing_nextest_fails_without_fallback(self):
        result = self.run_just("test", env={"MISSING_NEXTEST": "1"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cargo-nextest is required", result.stderr)
        self.assertEqual(self.calls(), [])

    def test_listing_and_unknown_task_do_not_run_cargo(self):
        self.assertEqual(self.run_just().returncode, 0)
        self.assertNotEqual(self.run_just("unknown").returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_setup_activates_checked_in_hooks_without_overwriting_custom_path(self):
        self.assertEqual(self.run_just("setup").returncode, 0)
        self.assertEqual(self.git("config", "--get", "core.hooksPath").stdout.strip(),
                         ".cargo-husky/hooks")
        self.assertEqual(self.run_just("setup").returncode, 0)
        self.git("config", "core.hooksPath", "/custom/hooks")
        result = self.run_just("setup")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Refusing to replace custom", result.stderr)
        self.assertEqual(self.git("config", "--get", "core.hooksPath").stdout.strip(),
                         "/custom/hooks")

    def test_job_overrides(self):
        self.run_just("check", env={"CARGO_BUILD_JOBS": "3"})
        self.assertEqual(self.calls()[-1]["jobs"], "3")
        self.run_just("check", env={"CARGO_BUILD_JOBS": "3", "CRONK_BUILD_JOBS": "4"})
        self.assertEqual(self.calls()[-1]["jobs"], "4")

    def test_failure_is_propagated_and_releases_lock(self):
        self.assertNotEqual(self.run_just("lint", env={"CARGO_EXIT": "7"}).returncode, 0)
        self.assertEqual(self.run_just("check").returncode, 0)

    def test_validate_runs_complete_checks_in_order(self):
        result = self.run_just("validate")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call["args"] for call in self.calls()], [
            ["fmt", "--all", "--", "--check"],
            ["clippy", "--locked", "--all-targets", "--", "-D", "warnings"],
            ["nextest", "run", "--locked", "--all-targets"],
            ["test", "--locked", "--doc"],
        ])

    def test_affected_selects_committed_staged_unstaged_and_untracked_tests(self):
        self.change("tests/committed.rs")
        self.change("tests/unstaged.rs", "// original\n")
        self.git("add", "tests")
        self.commit("Add tests")
        self.change("tests/unstaged.rs")
        self.change("tests/staged.rs")
        self.git("add", "tests/staged.rs")
        self.change("tests/untracked.rs")
        result = self.run_just("test-affected", "baseline")
        self.assertEqual(result.returncode, 0, result.stderr)
        args = self.calls()[-1]["args"]
        self.assertEqual(args[:3], ["nextest", "run", "--locked"])
        self.assertEqual(len(args), 11, "each affected target should be selected once")
        self.assertEqual(set(args[3::2]), {"--test"})
        self.assertEqual(set(args[4::2]), {"committed", "unstaged", "staged", "untracked"})

    def test_affected_shared_changes_run_all_targets(self):
        for path in ("src/model.rs", "src/ui/view.rs", "Cargo.toml", "Cargo.lock",
                     "build.rs", "examples/gallery.rs", ".cargo/config.toml",
                     ".config/nextest.toml", "tests/common/mod.rs", "tests/fixtures/data.json"):
            with self.subTest(path=path):
                self.change(path)
                result = self.run_just("test-affected", "baseline")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.calls()[-1]["args"],
                                 ["nextest", "run", "--locked", "--all-targets"])
                (self.repo / path).unlink()

    def test_affected_deleted_or_renamed_tests_run_all_targets(self):
        self.change("tests/original.rs")
        self.git("add", "tests")
        self.commit("Add original")
        self.git("branch", "before-rename")
        self.git("mv", "tests/original.rs", "tests/renamed.rs")
        result = self.run_just("test-affected", "before-rename")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls()[-1]["args"],
                         ["nextest", "run", "--locked", "--all-targets"])

    def test_affected_docs_only_skip_rust_and_invalid_base_fails(self):
        self.change("README.md", "Documentation\n")
        result = self.run_just("test-affected", "baseline")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("No affected Rust targets", result.stdout)
        self.assertEqual(self.calls(), [])
        self.assertNotEqual(self.run_just("test-affected", "invalid-ref").returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_linked_worktrees_share_lock_but_fmt_does_not_wait(self):
        linked = self.root / "linked worktree"
        self.git("worktree", "add", "-q", "-b", "linked", str(linked))
        processes = []
        try:
            first = subprocess.Popen(
                ["just", "--justfile", str(self.repo / "justfile"), "check"],
                env={**self.env, "CARGO_DELAY": "2"},
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            processes.append(first)
            deadline = time.monotonic() + 10
            while not self.calls() and time.monotonic() < deadline:
                time.sleep(0.02)
            self.assertEqual(len(self.calls()), 1)
            second = subprocess.Popen(
                ["just", "--justfile", str(linked / "justfile"), "test-lib"],
                env=self.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            processes.append(second)
            time.sleep(0.2)
            self.assertIsNone(second.poll(), "second worktree should wait")
            self.assertEqual(len(self.calls()), 1)
            self.assertEqual(self.run_just("fmt", repo=linked).returncode, 0)
            self.assertEqual(self.calls()[-1]["args"][0], "fmt")
            self.assertEqual(first.wait(timeout=10), 0)
            self.assertEqual(second.wait(timeout=10), 0)
            self.assertEqual(self.calls()[-1]["cwd"], str(linked))
            self.assertEqual(self.calls()[-1]["args"][0:2], ["nextest", "run"])
        finally:
            for process in processes:
                if process.poll() is None:
                    process.kill()
                process.wait()


if __name__ == "__main__":
    unittest.main()
