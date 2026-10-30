"""Regression coverage for packaging a source-built custom CLI for the daemon."""

import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import build_auto_continue_package as custom

from codex_package import cargo
from codex_package.targets import PACKAGE_VARIANTS, TARGET_SPECS


class CustomPackageTests(unittest.TestCase):
    def test_fork_display_and_manifest_versions(self) -> None:
        display, metadata = custom.fork_versions()
        self.assertEqual(display.replace("auto_continue", "auto-continue"), metadata)
        self.assertIn("+auto_continue.", display)
        self.assertEqual(custom.parse_package_version(metadata), metadata)

    @patch.object(custom.subprocess, "run")
    def test_native_linux_target_does_not_default_to_musl(self, run) -> None:
        run.return_value = subprocess.CompletedProcess(
            [], 0, "host: x86_64-unknown-linux-gnu\n"
        )
        self.assertEqual(custom.native_target(), "x86_64-unknown-linux-gnu")

    @patch.object(custom.subprocess, "run")
    def test_prebuilt_official_cli_is_rejected(self, run) -> None:
        run.return_value = subprocess.CompletedProcess([], 0, "codex-cli 0.165.0\n")
        with self.assertRaisesRegex(RuntimeError, "rebuild this fork"):
            custom.check_cli_version(Path("codex"), "0.165.0+auto_continue.0.1.0")

    @patch.object(custom.subprocess, "run")
    def test_prebuilt_custom_cli_is_accepted(self, run) -> None:
        version = "0.165.0+auto_continue.0.1.0"
        run.return_value = subprocess.CompletedProcess([], 0, f"codex-cli {version}\n")
        custom.check_cli_version(Path("codex"), version)

    def test_native_build_reuses_cargo_outputs_and_cross_build_keeps_target(
        self,
    ) -> None:
        target = "x86_64-unknown-linux-gnu"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for relative in (
                "codex",
                "codex-code-mode-host",
                "debug/bwrap",
                f"{target}/debug/bwrap",
            ):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.touch()
            for native in (True, False):
                with (
                    self.subTest(native=native),
                    patch.dict(cargo.os.environ, {"CARGO_TARGET_DIR": str(root)}),
                    patch.object(cargo.subprocess, "run") as run,
                ):
                    outputs = cargo.build_source_binaries(
                        TARGET_SPECS[target],
                        PACKAGE_VARIANTS["codex"],
                        cargo="cargo",
                        profile="dev",
                        native_build=native,
                        entrypoint_bin=root / "codex",
                        code_mode_host_bin=root / "codex-code-mode-host",
                        bwrap_bin=None,
                        codex_command_runner_bin=None,
                        codex_windows_sandbox_setup_bin=None,
                    )
                    command = run.call_args.args[0]
                    self.assertIn("--locked", command)
                    self.assertEqual("--target" in command, not native)
                    self.assertEqual(
                        outputs.bwrap_bin,
                        root / ("debug/bwrap" if native else f"{target}/debug/bwrap"),
                    )


if __name__ == "__main__":
    unittest.main()
