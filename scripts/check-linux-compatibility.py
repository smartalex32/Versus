#!/usr/bin/env python3
"""Reject ELF binaries/libraries that exceed the selected glibc baseline."""

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys


def required_glibc_versions(version_info):
    # Definitions describe symbols PROVIDED by a library. Only its version-needs
    # section describes symbols it requires from the host or another library.
    needs = re.search(r"Version needs section.*?(?=\nVersion .* section|\Z)",
                      version_info, re.DOTALL)
    if needs is None:
        return set()
    return set(re.findall(r"Name: (GLIBC_[A-Za-z0-9_.]+)", needs.group()))


def incompatible_versions(versions, maximum=(2, 28)):
    incompatible = []
    for version in versions:
        suffix = version[len("GLIBC_"):]
        if not re.fullmatch(r"[0-9]+(?:\.[0-9]+)+", suffix):
            incompatible.append(version)
        elif tuple(map(int, suffix.split("."))) > maximum:
            incompatible.append(version)
    return sorted(incompatible)


def check_paths(paths, maximum=(2, 28)):
    baseline = ".".join(map(str, maximum))
    checked = 0
    failures = []
    for root in paths:
        if not root.exists():
            raise ValueError("Missing compatibility input: {}".format(root))
        for path in sorted(root.rglob("*")) if root.is_dir() else [root]:
            if path.is_symlink() or not path.is_file():
                continue
            with path.open("rb") as source:
                if source.read(4) != b"\x7fELF":
                    continue
            checked += 1
            info = subprocess.check_output(
                ["readelf", "--version-info", str(path)],
                env=dict(os.environ, LC_ALL="C"), universal_newlines=True)
            unsupported = incompatible_versions(required_glibc_versions(info), maximum)
            if unsupported:
                failures.append("{}: {}".format(path, ", ".join(unsupported)))
    if not checked:
        raise ValueError("No ELF files found in compatibility inputs")
    if failures:
        raise ValueError("Compatibility requires glibc <= {}:\n".format(baseline) +
                         "\n".join(failures))
    print("Checked {} ELF files: glibc requirements <= {}".format(checked, baseline))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--maximum", required=True, choices=["2.28", "2.35"])
    parser.add_argument("paths", nargs="+", type=Path)
    arguments = parser.parse_args()
    try:
        check_paths(arguments.paths, tuple(map(int, arguments.maximum.split("."))))
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
