#!/usr/bin/env bash
# Run inside rockylinux/rockylinux:8.10; all setup stays on the build machine.
set -euo pipefail

test "$(uname -m)" = x86_64
test "$(getconf GNU_LIBC_VERSION)" = 'glibc 2.28'
dnf install --assumeyes dnf-plugins-core epel-release rpm-build
dnf config-manager --set-enabled powertools
dnf install --assumeyes gcc gcc-c++ make pkgconf-pkg-config curl ca-certificates \
  binutils patchelf file findutils tar gzip python3 fontconfig-devel mesa-libGL-devel gtk3-devel \
  libX11-devel libXrandr-devel libXi-devel libXcursor-devel libxkbcommon-devel libxkbcommon-x11 \
  wayland-devel xorg-x11-server-Xvfb xorg-x11-utils mesa-dri-drivers \
  xorg-x11-server-Xephyr xorg-x11-xauth xorg-x11-xkb-utils xkeyboard-config \
  mesa-libEGL openbox zenity nxagent xdotool xorg-x11-apps

bash scripts/build-xephyr-portable.sh

curl --fail --location --retry 3 https://sh.rustup.rs --output /tmp/versus-rustup-init.sh
sh /tmp/versus-rustup-init.sh -y --profile minimal --default-toolchain 1.95.0
export PATH="${HOME}/.cargo/bin:$PATH"
rustup component add rustfmt

PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts -p 'test_linux_*.py'
cargo fmt --check
cargo test --locked --offline
cargo build --release --locked --offline
bash scripts/package-linux.sh

# Exercise the packaged entry point with the Rocky 8 loader, without FUSE.
APPIMAGE_EXTRACT_AND_RUN=1 dist/Versus.AppImage --version

# Verify both ordinary X11 and the older NX display used by X2Go.
Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp &
display_pid=$!
app_pid=''
nx_pid=''
host_wm_pid=''
cleanup() {
  if test -n "$app_pid"; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  if test -n "$host_wm_pid"; then
    kill "$host_wm_pid" 2>/dev/null || true
    wait "$host_wm_pid" 2>/dev/null || true
  fi
  if test -n "$nx_pid"; then
    kill "$nx_pid" 2>/dev/null || true
    wait "$nx_pid" 2>/dev/null || true
  fi
  kill "$display_pid" 2>/dev/null || true
  wait "$display_pid" 2>/dev/null || true
}
trap cleanup EXIT
for attempt in {1..50}; do
  test -S /tmp/.X11-unix/X99 && break
  kill -0 "$display_pid"
  sleep 0.1
done
test -S /tmp/.X11-unix/X99
DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 APPIMAGE_EXTRACT_AND_RUN=1 \
  timeout 30s dist/Versus.AppImage &
app_pid=$!
window_opened=false
for attempt in {1..60}; do
  if DISPLAY=:99 xwininfo -root -tree | awk '/"Versus/{found=1} END {exit !found}'; then
    echo 'Packaged Versus window opened on Rocky Linux 8.'
    window_opened=true
    break
  fi
  kill -0 "$app_pid"
  sleep 0.25
done
if ! "$window_opened"; then
  echo 'Packaged GUI did not create a Versus window on Rocky Linux 8.' >&2
  exit 1
fi
# Check actual mappings as well as the final RUNPATH invariant: the ordinary
# app may load its keyboard libraries, but graphics/toolkit files stay native.
python3 - <<'PYMAPS'
from pathlib import Path
found = False
for process in Path("/proc").iterdir():
    if not process.name.isdigit():
        continue
    try:
        executable = (process / "exe").resolve(strict=True)
        if executable.name != "versus" or executable.parent.name != "bin":
            continue
        found = True
        root = executable.parents[2]
        private = str(root / "usr/lib") + "/"
        native = str(root / "usr/lib/native") + "/"
        mappings = (process / "maps").read_text().splitlines()
        unexpected = [line.split()[-1] for line in mappings
                      if private in line and native not in line]
        if unexpected:
            raise SystemExit("Ordinary launch mapped private runtime libraries: " +
                             ", ".join(unexpected))
    except (FileNotFoundError, PermissionError):
        continue
if not found:
    raise SystemExit("Cannot inspect the ordinary packaged Versus process")
print("Ordinary launch keeps the private graphics/toolkit runtime isolated.")
PYMAPS
kill "$app_pid"
wait "$app_pid" || true
app_pid=''

# These displays exist only inside the disposable build container. nxagent is
# the legacy server behind standard X2Go, rather than a modern Xvfb substitute.
# nxagent writes its compiled host keymap here on Rocky 8. The minimal
# container does not create this runtime directory through a desktop session.
mkdir -p /usr/share/X11/xkb/compiled
DISPLAY=:99 nxagent :100 -geometry 1280x800 -nolisten tcp &
nx_pid=$!
for attempt in {1..100}; do
  DISPLAY=:100 xdpyinfo >/dev/null 2>&1 && break
  kill -0 "$nx_pid"
  sleep 0.1
done
DISPLAY=:100 xdpyinfo -ext XInputExtension
# An ordinary host window manager gives the outer display the same close
# protocol a user gets from the title-bar button in a remote desktop.
DISPLAY=:100 openbox --sm-disable --config-file /etc/xdg/openbox/rc.xml &
host_wm_pid=$!
for attempt in {1..100}; do
  if DISPLAY=:100 xprop -root _NET_SUPPORTING_WM_CHECK | grep -q 'window id # 0x'; then
    break
  fi
  kill -0 "$host_wm_pid"
  sleep 0.1
done
DISPLAY=:100 xprop -root _NET_SUPPORTING_WM_CHECK | grep -q 'window id # 0x'
DISPLAY=:100 PYTHONDONTWRITEBYTECODE=1 python3 scripts/smoke-linux-x11.py dist/Versus.AppImage
