"""Exercise the local install and desktop-launch boundaries without compiling."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class LocalInstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(__file__).resolve().parents[2]
        self.prefix = Path(self.temp.name) / 'installed Bodh "test" %'
        self.fake_bin = Path(self.temp.name) / "fake-bin"
        self.fake_bin.mkdir()
        cargo = self.fake_bin / "cargo"
        cargo.write_text(
            f"#!{sys.executable}\n"
            "import os, sys\n"
            "from pathlib import Path\n"
            "args = sys.argv[1:]\n"
            "manifest = Path(args[args.index('--manifest-path') + 1])\n"
            "repo = manifest.parent\n"
            "binary_name = 'bodh' if repo == Path(os.environ['INSTALL_TEST_REPO']) else 'bodh-sequence-spike'\n"
            "binary = Path(os.environ['CARGO_TARGET_DIR']) / 'release' / binary_name\n"
            "binary.parent.mkdir(parents=True, exist_ok=True)\n"
            "command = 'cli' if binary_name == 'bodh' else 'gui'\n"
            "binary.write_text(\"#!/bin/sh\" + chr(10) + \"printf '%s:%s' '\" + command + \"' \\\"$*\\\"\" + chr(10))\n"
            "binary.chmod(0o755)\n"
        )
        cargo.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{self.fake_bin}{os.pathsep}{os.environ['PATH']}",
            "INSTALL_TEST_REPO": str(self.root),
            "CARGO_TARGET_DIR": str(Path(self.temp.name) / "cargo-target"),
            "XDG_DATA_HOME": str(Path(self.temp.name) / "xdg-data"),
        }

    def test_install_exposes_headless_cli_and_bodh_desktop_launcher(self):
        invalid_xdg = subprocess.run(
            [str(self.root / "scripts/install-local"), "--prefix", str(self.prefix)],
            cwd=self.temp.name,
            env={**self.env, "XDG_DATA_HOME": "relative-data"},
            capture_output=True, text=True,
        )
        self.assertEqual(invalid_xdg.returncode, 2)
        self.assertIn("XDG_DATA_HOME must be an absolute path", invalid_xdg.stderr)

        result = subprocess.run(
            [str(self.root / "scripts/install-local"), "--prefix", str(self.prefix)],
            cwd=self.temp.name, env=self.env, capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.prefix / "bin/bodh").is_file())
        self.assertTrue((self.prefix / "bin/bodh-gui").is_file())
        self.assertTrue((self.prefix / "bin/bodh-gui-launcher").is_file())
        desktop = (Path(self.env["XDG_DATA_HOME"]) / "applications/bodh.desktop").read_text()
        self.assertIn("Name=Bodh\n", desktop)
        self.assertIn('Exec="', desktop)
        self.assertIn("%%/bin/bodh-gui-launcher\"\n", desktop)

        outside_checkout = Path(self.temp.name) / "elsewhere"
        outside_checkout.mkdir()
        cli = subprocess.run(
            [str(self.prefix / "bin/bodh"), "contract", "describe"],
            cwd=outside_checkout, capture_output=True, text=True,
        )
        self.assertEqual(cli.returncode, 0, cli.stderr)
        self.assertIn("cli:contract describe", cli.stdout)

        launcher = str(self.prefix / "bin/bodh-gui-launcher")
        no_display = subprocess.run(
            [launcher], cwd=outside_checkout,
            env={k: v for k, v in self.env.items() if k not in {"DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET"}},
            capture_output=True, text=True,
        )
        self.assertNotEqual(no_display.returncode, 0)
        self.assertIn("active Wayland or X11 graphical session", no_display.stderr)

        ldd = self.fake_bin / "ldd"
        ldd.write_text("#!/bin/sh\nprintf 'libbodh-test.so => not found\\n'\n")
        ldd.chmod(0o755)
        missing_library = subprocess.run(
            [launcher], cwd=outside_checkout,
            env={**self.env, "DISPLAY": ":1"}, capture_output=True, text=True,
        )
        self.assertNotEqual(missing_library.returncode, 0)
        self.assertIn("libbodh-test.so", missing_library.stderr)
        self.assertIn("native libraries are missing", missing_library.stderr)

        ldd.unlink()
        gui = subprocess.run(
            [launcher, "--open", "--repo", "/tmp/checkout", "--pr", "12"],
            cwd=outside_checkout,
            env={**self.env, "DISPLAY": ":1"}, capture_output=True, text=True,
        )
        self.assertEqual(gui.returncode, 0, gui.stdout + gui.stderr)
        self.assertIn("gui:--open --repo /tmp/checkout --pr 12", gui.stdout)


if __name__ == "__main__":
    unittest.main()
