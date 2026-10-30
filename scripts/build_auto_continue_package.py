#!/usr/bin/env python3
"""Assemble a complete, pinned daemon package for this source-built fork."""

import os
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
os.environ["CODEX_REPO_ROOT"] = str(Path(__file__).resolve().parent.parent)

from codex_package.cli import (
    archive_package,
    assemble_package,
    create_parser,
    parse_package_version,
)
from codex_package.targets import REPO_ROOT, TARGET_SPECS
from codex_package.version import read_workspace_version


def fork_versions() -> tuple[str, str]:
    source = (REPO_ROOT / "codex-rs/tui/src/version.rs").read_text()
    suffix = re.search(r'"(\+auto_continue\.\d+\.\d+\.\d+)"', source)
    if suffix is None:
        raise RuntimeError("Cannot read the custom version from tui/src/version.rs")
    display = read_workspace_version() + suffix.group(1)
    # Underscores are part of our display name, but are forbidden in SemVer.
    metadata = parse_package_version(display.replace("auto_continue", "auto-continue"))
    return display, metadata


def native_target() -> str:
    result = subprocess.run(
        ["rustc", "-vV"], check=True, capture_output=True, text=True, timeout=30
    )
    for line in result.stdout.splitlines():
        if line.startswith("host: "):
            target = line.removeprefix("host: ").strip()
            if target in TARGET_SPECS:
                return target
    raise RuntimeError("Unsupported Rust host; specify --target explicitly")


def check_cli_version(binary: Path, expected: str) -> None:
    result = subprocess.run(
        [str(binary.resolve()), "--version"],
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
    )
    actual = result.stdout.strip()
    if actual != f"codex-cli {expected}":
        raise RuntimeError(
            f"Expected codex-cli {expected}, got {actual!r}; rebuild this fork's CLI"
        )


def main() -> int:
    display, metadata = fork_versions()
    parser = create_parser()
    parser.description = __doc__
    parser.set_defaults(package_version=metadata)
    args = parser.parse_args()
    if args.variant != "codex":
        parser.error("daemon --from-cli requires the codex variant")
    if args.package_version != metadata:
        parser.error(f"this fork requires package version {metadata}")
    if not hasattr(args, "target"):
        args.target = native_target()
        args.native_build = True
    if args.entrypoint_bin is not None:
        check_cli_version(args.entrypoint_bin, display)
    package = assemble_package(args)
    binary_name = "codex.exe" if "windows" in args.target else "codex"
    check_cli_version(package / "bin" / binary_name, display)
    archive_package(package, args)
    print(f"CLI version: {display}; package metadata version: {metadata}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
