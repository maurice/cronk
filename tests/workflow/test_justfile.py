"""Regression tests for just tasks, using fake Cargo rather than building Rust."""

import hashlib
import json
import re
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
        (self.repo / ".config").mkdir()
        shutil.copy2(source.parent / ".config/cargo-fast.toml",
                     self.repo / ".config/cargo-fast.toml")
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        cargo = self.bin / "cargo"
        cargo.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, pathlib, sys, time\n"
            "if sys.argv[1:] == ['nextest', '--version']:\n"
            "    sys.exit(int(os.environ.get('MISSING_NEXTEST', '0')))\n"
            "with open(os.environ['CALL_LOG'], 'a') as log:\n"
            "    log.write(json.dumps({'args': (['cross'] if os.path.basename(sys.argv[0]) == 'cross' else []) + sys.argv[1:], 'cwd': os.getcwd(), "
            "'jobs': os.environ.get('CARGO_BUILD_JOBS')}) + '\\n')\n"
            # Release metadata resolution needs the registry index even with a lockfile.
            # Model a genuinely absent index, not unconditional fake-Cargo success.
            "if os.environ.get('RELEASE_METADATA_GUARD') == '1' and sys.argv[1] == 'update':\n"
            "    metadata = pathlib.Path(os.environ['CARGO_HOME']) / 'registry/index/config.json'\n"
            "    if '--offline' in sys.argv and not metadata.exists():\n"
            "        print('no matching package named `anyhow` found: registry metadata absent in offline mode', file=sys.stderr)\n"
            "        sys.exit(101)\n"
            "    if sys.argv[1:] != ['update', '--workspace']:\n"
            "        sys.exit('release update must be workspace-only and allow registry access')\n"
            "    if os.environ.get('REGISTRY_NETWORK_FAILURE') == '1':\n"
            "        print('failed to download registry config.json', file=sys.stderr)\n"
            "        sys.exit(101)\n"
            "    metadata.parent.mkdir(parents=True, exist_ok=True)\n"
            "    metadata.write_text('{}')\n"
            "if '--message-format=json-render-diagnostics' in sys.argv:\n"
            "    print(json.dumps({'reason': 'compiler-artifact', 'target': {'kind': ['bin']}, 'executable': os.environ['FAKE_APPLICATION']}))\n"
            "time.sleep(float(os.environ.get('CARGO_DELAY', '0')))\n"
            "if os.path.basename(sys.argv[0]) == 'cross' and sys.argv[1] == 'run':\n"
            "    print(os.environ.get('FAKE_CROSS_VERSION', 'wrong version'))\n"
            "sys.exit(int(os.environ.get('CARGO_EXIT', '0')))\n"
        )
        cargo.chmod(0o755)
        shutil.copy2(cargo, self.bin / "cross")
        application = self.bin / "application"
        application.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, sys, time\n"
            "with open(os.environ['CALL_LOG'], 'a') as log:\n"
            "    log.write(json.dumps({'args': ['app', *sys.argv[1:]], 'cwd': os.getcwd(), "
            "'jobs': os.environ.get('CARGO_BUILD_JOBS')}) + '\\n')\n"
            "time.sleep(float(os.environ.get('APP_DELAY', '0')))\n"
        )
        application.chmod(0o755)
        for name in ("mold", "sccache", "clang", "rustc"):
            tool = self.bin / name
            tool.write_text(
                "#!/usr/bin/env python3\n"
                "import os, pathlib, sys\n"
                "if pathlib.Path(sys.argv[0]).name == 'rustc':\n"
                "    print('host: ' + os.environ.get('FAKE_HOST', 'x86_64-unknown-linux-gnu'))\n"
                "    print('compiler details\\n' * 8192)  # Catch early-exit grep/SIGPIPE.\n"
                "if '-o' in sys.argv:\n"
                "    output = pathlib.Path(sys.argv[sys.argv.index('-o') + 1])\n"
                "    output.write_text('#!/bin/sh\\nexit 0\\n')\n"
                "    output.chmod(0o755)\n"
            )
            tool.chmod(0o755)
        for name in ("mise.toml", "mise.lock", ".config/mise-bootstrap.json"):
            shutil.copy2(source.parent / name, self.repo / name)
        bootstrap = json.loads((self.repo / ".config/mise-bootstrap.json").read_text())
        self.data = self.root / "data"
        self.mise = self.data / "cronk/mise" / bootstrap["version"] / "mise"
        self.mise.parent.mkdir(parents=True)
        self.mise.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, pathlib, re, sys\n"
            "args = sys.argv[1:]\n"
            "with open(os.environ['MISE_LOG'], 'a') as log:\n"
            "    log.write(json.dumps(args) + '\\n')\n"
            # Model mise v2026.10.3 src/backend/cargo.rs dependencies: rust,
            # plus cargo-binstall/sccache when configured. Do not trust PATH:
            # the selected managed versions must be installed or in this batch.
            "if args[0] == 'install':\n"
            "    manifest = pathlib.Path('mise.toml').read_text()\n"
            "    configured = set(re.findall(r'(?m)^([\\w-]+)\\s*=', manifest.split('[tools]')[1]))\n"
            "    aliases = dict(re.findall(r'(?m)^([\\w-]+)\\s*=\\s*\"([^\"]+)\"', manifest.split('[tool_alias]')[1].split('[tools]')[0]))\n"
            "    requested = set(arg for arg in args[1:] if not arg.startswith('-')) or configured\n"
            "    state = pathlib.Path(os.environ['MISE_INSTALL_STATE'])\n"
            "    installed = set(json.loads(state.read_text())) if state.exists() else set()\n"
            "    for tool in requested:\n"
            "        if aliases.get(tool, '').startswith('cargo:'):\n"
            "            dependencies = {'rust', 'cargo-binstall', 'sccache'} & configured\n"
            "            missing = dependencies - requested - installed\n"
            "            if missing:\n"
            "                sys.exit('requires configured install dependencies: ' + ', '.join(sorted(missing)))\n"
            "    state.write_text(json.dumps(sorted(installed | requested)))\n"
            "if args[0] == 'exec':\n"
            "    os.environ['RUSTUP_TOOLCHAIN'] = '1.99.0'\n"
            "    command = args[args.index('--') + 1:]\n"
            "    os.execvp(command[0], command)\n"
        )
        self.mise.chmod(0o755)
        checksum = hashlib.sha256(self.mise.read_bytes()).hexdigest()
        bootstrap["sha256"] = {"linux-x64": checksum, "linux-arm64": checksum}
        (self.repo / ".config/mise-bootstrap.json").write_text(json.dumps(bootstrap))
        self.git("add", "justfile", ".config", "mise.toml", "mise.lock")
        self.commit("Initial")
        self.git("branch", "baseline")
        self.log = self.root / "calls.jsonl"
        self.mise_log = self.root / "mise.jsonl"
        self.env = {**os.environ, "PATH": f"{self.bin}:{os.environ['PATH']}",
                    "CALL_LOG": str(self.log), "MISE_LOG": str(self.mise_log),
                    "MISE_INSTALL_STATE": str(self.root / "installed-tools.json"),
                    "XDG_DATA_HOME": str(self.data), "FAKE_APPLICATION": str(application)}
        for name in ("CARGO_BUILD_JOBS", "CRONK_BUILD_JOBS", "CI", "GITHUB_ENV", "GITHUB_PATH"):
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
            (("check-all",), ["check", "--locked", "--all-targets"], "2"),
            (("lint",), ["clippy", "--locked", "--all-targets", "--", "-D", "warnings"], "2"),
            (("test-integration", "ui", "-E", "test(tab switches)"),
             ["nextest", "run", "--locked", "--test", "ui", "-E", "test(tab switches)"], "2"),
            (("test-all",), ["nextest", "run", "--locked", "--all-targets"], "2"),
            (("test-ci",), ["nextest", "run", "--locked", "--all-targets", "--profile", "ci"], "2"),
            (("test-lib", "-E", "test(filter)"),
             ["nextest", "run", "--locked", "--lib", "-E", "test(filter)"], "2"),
            (("doc",), ["test", "--locked", "--doc"], "2"),
            (("build",), ["build", "--locked"], "2"),
            (("build-release",), ["build", "--locked", "--release"], "2"),
            (("release-build", "aarch64-unknown-linux-musl"),
             ["cross", "build", "--locked", "--release", "--target", "aarch64-unknown-linux-musl", "--bin", "cronk"], "2"),
            (("run", "--", "--demo"), ["app", "--demo"], None),
            (("run", "--target-dir", "path with spaces", "--", "--demo"), ["app", "--demo"], None),
            (("demo",), ["app", "--demo"], None),
            (("demo-onboarding",), ["app", "--demo", "--onboarding"], None),
            (("gallery",), ["app"], None),
            (("sync-preview",), ["app"], None),
            (("demo", "--snapshot", "image with spaces.png"),
             ["app", "--demo", "--snapshot", "image with spaces.png"], None),
        ]
        for args, expected, jobs in cases:
            with self.subTest(args=args):
                result = self.run_just(*args)
                self.assertEqual(result.returncode, 0, result.stderr)
                if expected[0] == "app":
                    build_args = {"gallery": ["--example", "gallery"],
                                  "sync-preview": ["--example", "sync_preview"]}.get(args[0], [])
                    if args[0] == "run":
                        build_args = list(args[1:args.index("--")])
                    self.assertEqual(self.calls()[-2]["args"],
                                     ["build", "--locked", "--message-format=json-render-diagnostics", *build_args])
                    self.assertEqual(self.calls()[-2]["jobs"], "2")
                self.assertEqual(self.calls()[-1], {
                    "args": expected, "cwd": str(self.repo), "jobs": jobs,
                })

    def test_missing_nextest_fails_without_fallback(self):
        result = self.run_just("test", env={"MISSING_NEXTEST": "1"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cargo-nextest is required", result.stderr)
        self.assertEqual(self.calls(), [])

    def test_demo_releases_build_lock_before_interactive_application_exits(self):
        process = subprocess.Popen(
            ["just", "--justfile", str(self.repo / "justfile"), "demo"],
            env={**self.env, "APP_DELAY": "2"},
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        try:
            deadline = time.monotonic() + 10
            while len(self.calls()) < 2 and time.monotonic() < deadline:
                time.sleep(0.02)
            self.assertEqual(self.calls()[-1]["args"], ["app", "--demo"])
            self.assertEqual(self.run_just("check").returncode, 0)
            self.assertIsNone(process.poll(), "check should not wait for demo to exit")
            self.assertEqual(process.wait(timeout=10), 0)
        finally:
            if process.poll() is None:
                process.kill()
            process.wait()
        result = self.run_just("demo", env={"CARGO_EXIT": "7"})
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls()[-1]["args"][0], "build")

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

    def test_fast_setup_is_local_idempotent_and_reversible(self):
        result = self.run_just("fast-setup")
        self.assertEqual(result.returncode, 0, result.stderr)
        config = self.repo / ".cargo/config.toml"
        self.assertEqual(config.read_text(), (self.repo / ".config/cargo-fast.toml").read_text())
        self.assertEqual(self.run_just("fast-setup").returncode, 0)
        self.assertEqual(self.run_just("fast-disable").returncode, 0)
        self.assertFalse(config.exists())
        self.assertEqual(self.run_just("fast-disable").returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_fast_setup_and_disable_protect_custom_config(self):
        self.change(".cargo/config.toml", "[build]\njobs = 1\n")
        self.assertNotEqual(self.run_just("fast-setup").returncode, 0)
        self.assertNotEqual(self.run_just("fast-disable").returncode, 0)
        self.assertEqual((self.repo / ".cargo/config.toml").read_text(), "[build]\njobs = 1\n")
        (self.repo / ".cargo/config.toml").unlink()
        self.change(".cargo/config", "[build]\njobs = 1\n")
        self.assertNotEqual(self.run_just("fast-setup").returncode, 0)
        self.assertFalse((self.repo / ".cargo/config.toml").exists())

    def test_fast_setup_rejects_unsupported_hosts(self):
        result = self.run_just("fast-setup", env={"FAKE_HOST": "aarch64-apple-darwin"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("x86_64 Linux/WSL", result.stderr)
        self.assertFalse((self.repo / ".cargo/config.toml").exists())

    def test_setup_uses_locked_tools_and_ci_selects_only_required_subset(self):
        manifest = (self.repo / "mise.toml").read_text()
        self.assertRegex(manifest, r'(?m)^exec_auto_install = false$')
        self.assertRegex(manifest, r'(?m)^not_found_auto_install = false$')
        result = self.run_just("setup")
        self.assertEqual(result.returncode, 0, result.stderr)
        entries = [json.loads(line) for line in self.mise_log.read_text().splitlines()]
        self.assertIn(["install", "--locked"], entries)
        for group, tools in (("rust", ["rust"]), ("checks", ["rust", "nextest"]),
                             ("release", ["rust", "sccache", "cross"])):
            result = self.run_just("setup-ci", group, env={"CI": "true"})
            self.assertEqual(result.returncode, 0, result.stderr)
            entries = [json.loads(line) for line in self.mise_log.read_text().splitlines()]
            self.assertEqual(entries[-1], ["install", "--locked", *tools])
        self.assertNotEqual(self.run_just("setup-ci", "unknown").returncode, 0)
        github_env = self.root / "github-env"
        result = self.run_just("setup-ci", "rust", env={"CI": "true", "GITHUB_ENV": str(github_env)})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(github_env.read_text(), "RUSTUP_TOOLCHAIN=1.99.0\n")

    def test_cross_installers_include_dependencies_on_fresh_runner(self):
        state = Path(self.env["MISE_INSTALL_STATE"])
        for task, env in ((("setup-ci", "release"), {"CI": "true"}),
                          (("install-cross",), {})):
            with self.subTest(task=task):
                state.unlink(missing_ok=True)
                # All fake tools are on PATH, but no managed version is installed.
                # The old subset must fail, independent of the argument assertion.
                result = self.run_just("_setup-tools", "rust", "cross", env=env)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("requires configured install dependencies: sccache", result.stderr)
                self.assertFalse(state.exists())
                result = self.run_just(*task, env=env)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(set(json.loads(state.read_text())), {"rust", "sccache", "cross"})
                self.assertFalse((self.repo / ".cargo/config.toml").exists())
                entries = [json.loads(line) for line in self.mise_log.read_text().splitlines()]
                self.assertEqual(entries[-1], ["install", "--locked", "rust", "sccache", "cross"])

    def test_missing_managed_environment_reports_setup(self):
        self.mise.unlink()
        result = self.run_just("check")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Run just setup", result.stderr)
        self.assertEqual(self.calls(), [])

    def test_bootstrap_checks_download_before_installing_or_executing(self):
        payload = self.mise.read_bytes()
        self.mise.unlink()
        mock = self.root / "mock_python"
        mock.mkdir()
        (mock / "sitecustomize.py").write_text(
            "import io, os, urllib.request\n"
            "urllib.request.urlopen = lambda *a, **k: io.BytesIO(bytes.fromhex(os.environ['TEST_PAYLOAD']))\n"
        )
        env = {"PYTHONPATH": str(mock), "TEST_PAYLOAD": b"untrusted bytes".hex()}
        result = self.run_just("_bootstrap-mise", env=env)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertFalse(self.mise.exists())
        env["TEST_PAYLOAD"] = payload.hex()
        result = self.run_just("_bootstrap-mise", env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.mise.read_bytes(), payload)
        self.assertEqual(self.run_just("_bootstrap-mise").returncode, 0)

    def test_workflow_calls_and_documentation_use_public_just_tasks(self):
        root = Path(__file__).resolve().parents[2]
        tasks = set(subprocess.check_output(["just", "--justfile", str(root / "justfile"),
                                             "--summary"], text=True).split())
        for workflow in (root / ".github/workflows").glob("*.yml"):
            text = workflow.read_text()
            self.assertNotRegex(text, r'(?m)^\s*(run: )?(cargo|cross) (build|test|run|clippy|fmt|install|update)\b')
            for task in re.findall(r'\bjust ([a-z][a-z-]*)', text):
                self.assertIn(task, tasks, f"Unknown task {task} in {workflow}")
        for document in [root / "README.md", *(root / "docs").glob("*.md")]:
            self.assertNotRegex(document.read_text(), r'(?m)^cargo (run|build|test|install|clippy|fmt)\b')

    def test_job_overrides(self):
        self.run_just("check", env={"CARGO_BUILD_JOBS": "3"})
        self.assertEqual(self.calls()[-1]["jobs"], "3")
        self.run_just("check", env={"CARGO_BUILD_JOBS": "3", "CRONK_BUILD_JOBS": "4"})
        self.assertEqual(self.calls()[-1]["jobs"], "4")

    def test_ci_keeps_runner_budget_and_does_not_acquire_local_lock(self):
        result = self.run_just("test-ci", env={"CI": "true"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIsNone(self.calls()[-1]["jobs"])
        self.assertFalse((self.repo / ".git/cronk-dev.lock").exists())
        self.run_just("check", env={"CI": "true", "CARGO_BUILD_JOBS": "6"})
        self.assertEqual(self.calls()[-1]["jobs"], "6")

    def test_release_version_updates_manifest_and_requests_workspace_lock_update(self):
        self.change("Cargo.toml", '[package]\nname = "cronk"\nversion = "0.0.0"\n')
        result = self.run_just("release-version", "v1.2.3-beta.1+build")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('version = "1.2.3-beta.1+build"', (self.repo / "Cargo.toml").read_text())
        self.assertEqual(self.calls()[-1]["args"], ["update", "--workspace"])
        self.assertNotEqual(self.run_just("release-version", "v1.2.4").returncode, 0)
        self.assertNotEqual(self.run_just("release-version", 'invalid"tag').returncode, 0)

    def test_release_version_resolves_missing_registry_metadata_without_updating_dependencies(self):
        home = self.root / "empty-cargo-home"
        home.mkdir()
        env = {"CARGO_HOME": str(home), "RELEASE_METADATA_GUARD": "1"}
        self.assertEqual(list(home.iterdir()), [])
        # Prove that this fake rejects the previously broken command on a cold runner.
        result = self.run_just("_cargo", "update", "--workspace", "--offline", env=env)
        self.assertEqual(result.returncode, 101, result.stderr)
        self.assertIn("registry metadata absent in offline mode", result.stderr)
        self.assertEqual(list(home.iterdir()), [])
        # A broad dependency update must not satisfy the release-preparation guard.
        result = self.run_just("_cargo", "update", env=env)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("release update must be workspace-only", result.stderr)
        self.assertEqual(list(home.iterdir()), [])
        self.change("Cargo.toml", '[package]\nname = "cronk"\nversion = "0.0.0"\n')
        result = self.run_just("release-version", "v0.0.5", env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('version = "0.0.5"', (self.repo / "Cargo.toml").read_text())
        self.assertEqual(self.calls()[-1]["args"], ["update", "--workspace"])
        self.assertEqual(self.calls()[-1]["jobs"], "2")
        self.assertTrue((home / "registry/index/config.json").exists())

    def test_release_version_registry_failure_propagates_and_leaves_manifest_changed(self):
        home = self.root / "empty-cargo-home"
        home.mkdir()
        env = {"CARGO_HOME": str(home), "RELEASE_METADATA_GUARD": "1",
               "REGISTRY_NETWORK_FAILURE": "1"}
        self.change("Cargo.toml", '[package]\nname = "cronk"\nversion = "0.0.0"\n')
        lock = 'version = 4\n[[package]]\nname = "cronk"\nversion = "0.0.0"\n'
        self.change("Cargo.lock", lock)
        result = self.run_just("release-version", "v0.0.5", env=env)
        self.assertEqual(result.returncode, 101, result.stderr)
        self.assertIn("failed to download registry config.json", result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertEqual(self.calls()[-1]["args"], ["update", "--workspace"])
        self.assertEqual(list(home.iterdir()), [])
        self.assertIn('version = "0.0.5"', (self.repo / "Cargo.toml").read_text())
        self.assertEqual((self.repo / "Cargo.lock").read_text(), lock)
        # Failure must release the coordination lock, not block later feedback.
        self.assertEqual(self.run_just("check").returncode, 0)

    def test_release_check_verifies_target_binary_output(self):
        sha = "0123456789abcdef"
        result = self.run_just("release-check", "aarch64-unknown-linux-gnu", "v1.2.3", sha,
                               env={"FAKE_CROSS_VERSION": "cronk v1.2.3@0123456789ab"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cronk v1.2.3@0123456789ab", result.stdout)
        self.assertEqual(self.calls()[-1]["args"], [
            "cross", "run", "--locked", "--release", "--target",
            "aarch64-unknown-linux-gnu", "--bin", "cronk", "--", "--version",
        ])
        result = self.run_just("release-check", "aarch64-unknown-linux-gnu", "v1.2.3", sha)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Release version mismatch", result.stderr)

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
