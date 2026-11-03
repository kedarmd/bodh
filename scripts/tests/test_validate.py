"""Exercise the runner at its subprocess boundary, without compiling Rust."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


class ValidationRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "checkout with spaces"
        (self.root / "scripts").mkdir(parents=True)
        self.runner = self.root / "scripts/validate"
        shutil.copy(Path(__file__).resolve().parents[1] / "validate", self.runner)
        self.log = Path(self.temp.name) / "commands.jsonl"
        fake_bin = Path(self.temp.name) / "bin"
        fake_bin.mkdir()
        for tool in ["cargo", "rustc", "python3"]:
            executable = fake_bin / tool
            executable.write_text(
                f"#!{sys.executable}\n"
                "import json, os, sys\n"
                "from pathlib import Path\n"
                "with open(os.environ['VALIDATION_TEST_LOG'], 'a') as log:\n"
                "    log.write(json.dumps([Path(sys.argv[0]).name, sys.argv[1:], os.getcwd()]) + '\\n')\n"
                "if Path(sys.argv[0]).name == 'cargo' and sys.argv[1] == os.environ.get('VALIDATION_TEST_FAIL'):\n"
                "    sys.exit(23)\n"
            )
            executable.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
            "VALIDATION_TEST_LOG": str(self.log),
        }
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(
            ["git", "-C", str(self.root), "-c", "user.name=Runner test", "-c",
             "user.email=runner@example.invalid", "-c", "commit.gpgsign=false", "-c",
             "core.hooksPath=/dev/null", "commit", "-q", "--allow-empty", "-m", "fixture"],
            check=True,
        )

    def run_mode(self, *args, fail=None):
        env = {**self.env}
        if fail:
            env["VALIDATION_TEST_FAIL"] = fail
        return subprocess.run(
            [str(self.runner), *args], cwd=self.temp.name, env=env,
            capture_output=True, text=True,
        )

    def cargo_commands(self):
        if not self.log.exists():
            return []
        return [entry for line in self.log.read_text().splitlines()
                if (entry := json.loads(line))[0] == "cargo" and entry[1] != ["--version"]]

    def test_core_runs_from_another_directory_with_quoted_manifest(self):
        result = self.run_mode()
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = self.cargo_commands()
        self.assertEqual([args[0] for _, args, _ in commands], ["fmt", "check", "clippy", "test"])
        for _, args, cwd in commands:
            self.assertEqual(cwd, str(self.root))
            self.assertEqual(args[args.index("--manifest-path") + 1],
                             str(self.root / "spikes/native-sequence/Cargo.toml"))
        self.assertIn("Working tree: dirty", result.stdout)

    def test_failed_lint_stops_before_tests(self):
        result = self.run_mode("desktop", fail="clippy")
        self.assertEqual(result.returncode, 23)
        self.assertEqual([args[0] for _, args, _ in self.cargo_commands()], ["fmt", "check", "clippy"])
        self.assertNotIn("Validation passed", result.stdout)
        self.assertIn("Validation failed", result.stderr)

    def test_desktop_includes_native_checks_without_launching_gui(self):
        result = self.run_mode("desktop")
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = self.cargo_commands()
        self.assertEqual([args[0] for _, args, _ in commands],
                         ["fmt", "check", "clippy", "test", "check", "clippy", "test"])
        self.assertNotIn("--no-default-features", commands[-1][1])

    def test_gui_and_profile_use_locked_release_cargo_run_even_on_failure(self):
        for mode in ["gui", "profile"]:
            with self.subTest(mode=mode):
                self.log.unlink(missing_ok=True)
                result = self.run_mode(mode, fail="run")
                self.assertEqual(result.returncode, 23)
                commands = self.cargo_commands()
                self.assertEqual(len(commands), 1)
                args = commands[0][1]
                self.assertEqual(args[:3], ["run", "--release", "--locked"])
                self.assertEqual("--profile" in args, mode == "profile")

    def test_invalid_mode_runs_no_tools(self):
        for args in [("unknown",), ("core", "extra")]:
            result = self.run_mode(*args)
            self.assertEqual(result.returncode, 2)
            self.assertFalse(self.log.exists())


if __name__ == "__main__":
    unittest.main()
