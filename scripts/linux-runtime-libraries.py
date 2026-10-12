#!/usr/bin/env python3
"""Copy a trusted Linux ELF runtime closure into an AppDir without symlinks."""

import argparse
from pathlib import Path
from pathlib import PurePosixPath
import re
import shutil
import subprocess
import sys


# These are supplied by the host's glibc loader. Everything else discovered
# from trusted build-host executables is bundled with the compatibility mode.
GLIBC_MODULES = {
    "ld-linux-x86-64.so.2", "libBrokenLocale.so.1", "libanl.so.1",
    "libc.so.6", "libcidn.so.1", "libdl.so.2", "libm.so.6",
    "libmvec.so.1", "libnss_compat.so.2", "libnss_dns.so.2",
    "libnss_files.so.2", "libnss_hesiod.so.2", "libnss_nis.so.2",
    "libnss_nisplus.so.2", "libpthread.so.0", "libresolv.so.2",
    "librt.so.1", "libthread_db.so.1", "libutil.so.1",
}

# These Rocky libraries validate vendor-provided HMAC files when FIPS checks
# are enabled. Preserve the vendor integrity metadata rather than generating
# replacement tokens for potentially modified binaries.
REQUIRED_INTEGRITY_SONAMES = {
    "libcrypto.so.1.1", "libssl.so.1.1", "libgcrypt.so.20",
    "libgnutls.so.30", "libnettle.so.6", "libhogweed.so.4", "libgmp.so.10",
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


def copy_resolved(source, destination, copied, record_name=None):
    source = source.resolve(strict=True)
    name = destination.name if record_name is None else record_name
    previous = copied.get(name)
    if previous is not None:
        if previous != source:
            raise ValueError("Two runtime libraries use {}: {} and {}".format(
                name, previous, source))
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.is_symlink() or destination.exists():
        if destination.is_dir():
            raise ValueError("Runtime library destination is a directory: {}".format(
                destination))
        destination.unlink()
    shutil.copyfile(source, destination)
    shutil.copystat(source, destination)
    copied[name] = source
    return source


def hmac_name(soname):
    return ".{}.hmac".format(soname)


def integrity_sidecar(source, soname):
    """Return the original vendor integrity file and its AppDir-relative path."""
    candidates = (
        (source.parent / hmac_name(soname), Path(hmac_name(soname))),
        (source.parent / "fipscheck" / "{}.hmac".format(soname),
         Path("fipscheck") / "{}.hmac".format(soname)),
    )
    for sidecar, relative_destination in candidates:
        if sidecar.exists():
            if not sidecar.is_file():
                raise ValueError("Runtime HMAC is not a regular file: {}".format(sidecar))
            return sidecar, relative_destination
    if soname in REQUIRED_INTEGRITY_SONAMES:
        locations = ", ".join(str(candidate[0]) for candidate in candidates)
        raise ValueError("Missing required runtime HMAC for {}: {}".format(
            soname, locations))
    return None


def copy_hmac_sidecar(source, destination, soname, copied):
    """Copy an adjacent or fipscheck vendor HMAC without changing its layout."""
    integrity = integrity_sidecar(source, soname)
    if integrity is None:
        return
    sidecar, relative_destination = integrity
    copy_resolved(sidecar, destination / relative_destination, copied,
                  relative_destination.as_posix())


def copy_library_with_sidecar(source, destination, copied, sidecars):
    resolved = source.resolve(strict=True)
    copy_resolved(resolved, destination, copied)
    copy_hmac_sidecar(resolved, destination.parent, destination.name, sidecars)


def collect(libraries, executables, destination, sidecars=None):
    copied = {}
    sidecars = {} if sidecars is None else sidecars
    seen = set()
    pending = []
    for library in libraries:
        resolved = library.resolve(strict=True)
        copy_library_with_sidecar(resolved, destination / library.name, copied, sidecars)
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
            copy_library_with_sidecar(
                resolved, destination / dependency.name, copied, sidecars)
            pending.append(resolved)
    return copied


def copy_native(libraries, destination, sidecars=None):
    copied = {}
    sidecars = {} if sidecars is None else sidecars
    for library in libraries:
        copy_library_with_sidecar(
            library.resolve(strict=True), destination / library.name, copied, sidecars)
    return copied


def write_manifest(path, runtime, native, runtime_sidecars, native_sidecars):
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = ["# Runtime libraries copied as resolved ELF files."]
    lines.extend("usr/lib/{} <- {}".format(name, source)
                 for name, source in sorted(runtime.items()))
    lines.append("# Vendor HMAC sidecars copied unchanged in their RPM-provided layout.")
    lines.extend("usr/lib/{} <- {}".format(name, source)
                 for name, source in sorted(runtime_sidecars.items()))
    lines.append("# Native-mode libraries copied as resolved ELF files.")
    lines.extend("usr/lib/native/{} <- {}".format(name, source)
                 for name, source in sorted(native.items()))
    lines.append("# Vendor HMAC sidecars copied unchanged in their RPM-provided layout.")
    lines.extend("usr/lib/native/{} <- {}".format(name, source)
                 for name, source in sorted(native_sidecars.items()))
    path.write_text("\n".join(lines) + "\n")


def manifest_entries(path):
    entries = []
    for line in path.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        destination, marker, source = line.partition(" <- ")
        if not marker or not destination or not source:
            raise ValueError("Malformed runtime manifest entry: {}".format(line))
        relative = PurePosixPath(destination)
        if (relative.is_absolute() or ".." in relative.parts or
                relative.parts[:2] != ("usr", "lib")):
            raise ValueError("Unsafe runtime manifest destination: {}".format(destination))
        entries.append((relative, Path(source)))
    if not entries:
        raise ValueError("Runtime manifest has no copied files: {}".format(path))
    return entries


def files_match(first, second):
    with first.open("rb") as first_stream, second.open("rb") as second_stream:
        while True:
            first_chunk = first_stream.read(1024 * 1024)
            second_chunk = second_stream.read(1024 * 1024)
            if first_chunk != second_chunk:
                return False
            if not first_chunk:
                return True


def verify_manifest(manifest, payload_root):
    """Ensure final payload files remain byte-for-byte build-host RPM inputs."""
    for relative, source in manifest_entries(manifest):
        source = source.resolve(strict=True)
        destination = payload_root.joinpath(*relative.parts)
        if destination.is_symlink() or not destination.is_file():
            raise ValueError("Missing or symlinked packaged runtime file: {}".format(
                destination))
        if not files_match(source, destination):
            raise ValueError("Packaged runtime file differs from build-host input: {}".format(
                destination))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", type=Path)
    parser.add_argument("--native-destination", type=Path)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--verify-manifest", metavar="PAYLOAD_ROOT", type=Path)
    parser.add_argument("--library", action="append", default=[], type=Path)
    parser.add_argument("--executable", action="append", default=[], type=Path)
    parser.add_argument("--native-library", action="append", default=[], type=Path)
    arguments = parser.parse_args()
    try:
        if arguments.verify_manifest is not None:
            if arguments.manifest is None:
                parser.error("--verify-manifest requires --manifest")
            verify_manifest(arguments.manifest, arguments.verify_manifest)
            print("Verified packaged runtime files against build-host inputs")
            return 0
        if (arguments.destination is None or arguments.native_destination is None or
                arguments.manifest is None):
            parser.error("--destination, --native-destination, and --manifest are required")
        runtime_sidecars = {}
        runtime = collect(arguments.library, arguments.executable, arguments.destination,
                          runtime_sidecars)
        # Native mode must remain self-contained too: the main executable uses
        # these dynamically loaded roots while deliberately avoiding the
        # private Xephyr/GTK/Mesa namespace.
        native_sidecars = {}
        native = collect(arguments.native_library, [], arguments.native_destination,
                         native_sidecars)
        write_manifest(arguments.manifest, runtime, native, runtime_sidecars, native_sidecars)
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
