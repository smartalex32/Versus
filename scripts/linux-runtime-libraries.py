#!/usr/bin/env python3
"""Copy a trusted Linux ELF runtime closure into an AppDir without symlinks."""

import argparse
from pathlib import Path
import re
import shutil
import subprocess
import sys


# These are supplied by the host's glibc loader.  Everything else discovered
# from trusted build-host executables is bundled, including X11, XCB, GLVND,
# Mesa, libdrm, and C++ runtimes.
GLIBC_MODULES = {
    "ld-linux-x86-64.so.2", "libBrokenLocale.so.1", "libanl.so.1",
    "libc.so.6", "libcidn.so.1", "libdl.so.2", "libm.so.6",
    "libmvec.so.1", "libnss_compat.so.2", "libnss_dns.so.2",
    "libnss_files.so.2", "libnss_hesiod.so.2", "libnss_nis.so.2",
    "libnss_nisplus.so.2", "libpthread.so.0", "libresolv.so.2",
    "librt.so.1", "libthread_db.so.1", "libutil.so.1",
}


def dependencies(path):
    result = subprocess.run(
        ["ldd", str(path)], universal_newlines=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        raise ValueError("ldd failed for {}:\n{}".format(path, result.stderr))
    found = []
    for line in result.stdout.splitlines():
        if "=> not found" in line:
            raise ValueError("Unresolved ELF dependency for {}: {}".format(path, line))
        match = re.search(r"=>\s+(/\S+)", line)
        if match is None:
            match = re.match(r"\s*(/\S+)", line)
        if match is not None:
            found.append(Path(match.group(1)))
    return found


def copy_resolved(source, destination, copied):
    source = source.resolve(strict=True)
    name = destination.name
    previous = copied.get(name)
    if previous is not None:
        if previous != source:
            raise ValueError("Two runtime libraries use {}: {} and {}".format(
                name, previous, source))
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    # linuxdeploy can leave a source-targeted symlink in place.  Remove it
    # before copying so a later closure pass cannot overwrite the build host.
    if destination.is_symlink() or destination.exists():
        if destination.is_dir():
            raise ValueError("Runtime library destination is a directory: {}".format(
                destination))
        destination.unlink()
    shutil.copyfile(source, destination)
    shutil.copystat(source, destination)
    copied[name] = source


def collect(libraries, executables, destination):
    copied = {}
    seen = set()
    pending = []
    for library in libraries:
        name = library.name
        resolved = library.resolve(strict=True)
        copy_resolved(resolved, destination / name, copied)
        pending.append(resolved)
    pending.extend(path.resolve(strict=True) for path in executables)

    while pending:
        path = pending.pop()
        if path in seen:
            continue
        seen.add(path)
        for dependency in dependencies(path):
            if dependency.name in GLIBC_MODULES:
                continue
            resolved = dependency.resolve(strict=True)
            copy_resolved(resolved, destination / dependency.name, copied)
            pending.append(resolved)
    return copied


def copy_native(libraries, destination):
    copied = {}
    for library in libraries:
        name = library.name
        copy_resolved(library.resolve(strict=True), destination / name, copied)
    return copied


def write_manifest(path, runtime, native):
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = ["# Runtime libraries copied as resolved ELF files."]
    for name, source in sorted(runtime.items()):
        lines.append("usr/lib/{} <- {}".format(name, source))
    lines.append("# Native-mode keyboard libraries copied as resolved ELF files.")
    for name, source in sorted(native.items()):
        lines.append("usr/lib/native/{} <- {}".format(name, source))
    path.write_text("\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--native-destination", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--library", action="append", default=[], type=Path)
    parser.add_argument("--executable", action="append", default=[], type=Path)
    parser.add_argument("--native-library", action="append", default=[], type=Path)
    arguments = parser.parse_args()
    try:
        runtime = collect(arguments.library, arguments.executable, arguments.destination)
        native = copy_native(arguments.native_library, arguments.native_destination)
        write_manifest(arguments.manifest, runtime, native)
        missing = [arguments.destination / name for name in runtime
                   if not (arguments.destination / name).is_file()]
        missing.extend(arguments.native_destination / name for name in native
                       if not (arguments.native_destination / name).is_file())
        if missing:
            raise ValueError("Runtime library staging produced non-files: {}".format(
                ", ".join(map(str, missing))))
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(error, file=sys.stderr)
        return 1
    print("Bundled {} full-runtime and {} native-mode libraries".format(
        len(runtime), len(native)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
