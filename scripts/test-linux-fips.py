#!/usr/bin/env python3
"""Exercise Rocky vendor FIPS checks only inside the disposable CI container."""
import os
from pathlib import Path
import resource
import subprocess
import sys
import tempfile


def main():
    image = Path(sys.argv[1]).resolve(strict=True)
    marker = Path('/etc/gcrypt/fips_enabled')
    if marker.exists():
        raise RuntimeError('Expected a disposable container without a FIPS test marker')
    # Negative controls deliberately abort in vendor constructors; avoid core files.
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    environment = os.environ.copy()
    environment['OPENSSL_FORCE_FIPS_MODE'] = '1'
    with tempfile.TemporaryDirectory(prefix='versus-fips-') as directory:
        staging = Path(directory)
        subprocess.run([str(image), '--appimage-extract'], cwd=staging,
                       stdout=subprocess.DEVNULL, check=True)
        libraries = staging / 'squashfs-root/usr/lib'
        probes = {}
        for name, flags in [('libgcrypt.so.20', ['-DTEST_GCRYPT', '-lgcrypt']),
                            ('libcrypto.so.1.1', ['-lcrypto'])]:
            probe = staging / (name + '-probe')
            subprocess.run(['gcc', '-std=c99', '-Wall', '-Wextra', '-Werror',
                            'scripts/check-linux-fips.c', '-o', str(probe)] +
                           flags + ['-ldl'], check=True)
            probes[name] = probe
        environment['LD_LIBRARY_PATH'] = str(libraries)

        def run(name, success):
            result = subprocess.run([str(probes[name]), str(libraries / name)],
                                    env=environment, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, universal_newlines=True,
                                    timeout=30)
            if (result.returncode == 0) != success:
                raise RuntimeError('Unexpected FIPS probe result for {}: {}'.format(
                    name, result.stdout))
            if success:
                print(result.stdout, end='', flush=True)

        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.touch(exist_ok=False)
        try:
            for name in probes:
                run(name, True)
            for name in ['libgcrypt.so.20', 'libcrypto.so.1.1', 'libssl.so.1.1']:
                probe_name = name if name in probes else 'libcrypto.so.1.1'
                sidecar = libraries / ('.' + name + '.hmac')
                original = sidecar.read_bytes()
                try:
                    sidecar.unlink()
                    run(probe_name, False)
                    sidecar.write_bytes(b'0' * 64 + b'\n')
                    run(probe_name, False)
                finally:
                    sidecar.write_bytes(original)
                library = libraries / name
                size = library.stat().st_size
                try:
                    with library.open('ab') as stream:
                        stream.write(b'integrity-negative-control')
                    run(probe_name, False)
                finally:
                    with library.open('r+b') as stream:
                        stream.truncate(size)
                run(probe_name, True)
                print('FIPS rejected missing/altered HMAC and altered {}.'.format(name),
                      flush=True)
            # Repeat the existing GUI/browse/resize/cleanup checks with vendor
            # FIPS activation hooks. Neither hook is included in the AppImage.
            environment.pop('LD_LIBRARY_PATH')
            print('Running packaged X11/nxagent GUI tests with vendor FIPS checks.', flush=True)
            subprocess.run(['bash', 'scripts/test-linux-x11.sh', str(image)],
                           env=environment, check=True)
        finally:
            marker.unlink()
    print('Vendor FIPS library and GUI checks passed.', flush=True)


if __name__ == '__main__':
    main()
