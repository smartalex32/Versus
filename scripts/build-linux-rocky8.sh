#!/usr/bin/env bash
# Run inside rockylinux/rockylinux:8.10; all setup stays on the build machine.
set -euo pipefail

test "$(uname -m)" = x86_64
test "$(getconf GNU_LIBC_VERSION)" = 'glibc 2.28'
dnf install --assumeyes dnf-plugins-core epel-release rpm-build
dnf config-manager --set-enabled powertools
dnf install --assumeyes gcc gcc-c++ make pkgconf-pkg-config curl ca-certificates \
  binutils file findutils tar gzip python3 fontconfig-devel mesa-libGL-devel gtk3-devel \
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
cleanup() {
  if test -n "$app_pid"; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
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
kill "$app_pid"
wait "$app_pid" || true
app_pid=''

# These displays exist only inside the disposable build container. nxagent is
# the legacy server behind standard X2Go, rather than a modern Xvfb substitute.
DISPLAY=:99 nxagent :100 -geometry 1280x800 -nolisten tcp &
nx_pid=$!
for attempt in {1..100}; do
  DISPLAY=:100 xdpyinfo >/dev/null 2>&1 && break
  kill -0 "$nx_pid"
  sleep 0.1
done
DISPLAY=:100 xdpyinfo -ext XInputExtension
DISPLAY=:100 PYTHONDONTWRITEBYTECODE=1 python3 scripts/smoke-linux-x11.py dist/Versus.AppImage
