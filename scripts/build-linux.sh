#!/usr/bin/env bash
# Build and exercise the native app inside the selected baseline container.
set -euo pipefail

baseline="${1:?Usage: build-linux.sh 2.35|2.28}"
test "$(uname -m)" = x86_64
test "$(getconf GNU_LIBC_VERSION)" = "glibc $baseline"
case "$baseline" in
  2.35)
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install --yes \
      build-essential pkg-config curl ca-certificates binutils file patchelf \
      python3 tar gzip libfontconfig1-dev libgl1-mesa-dev libgtk-3-dev \
      libx11-dev libxrandr-dev libxi-dev libxcursor-dev libxkbcommon-dev \
      libxkbcommon-x11-0 libwayland-dev xvfb x11-utils libgl1-mesa-dri
    ;;
  2.28)
    dnf install --assumeyes dnf-plugins-core epel-release rpm-build
    dnf config-manager --set-enabled powertools
    dnf install --assumeyes gcc gcc-c++ make pkgconf-pkg-config curl \
      ca-certificates binutils file patchelf python3 tar gzip findutils \
      fontconfig-devel mesa-libGL-devel gtk3-devel libX11-devel \
      libXrandr-devel libXi-devel libXcursor-devel libxkbcommon-devel \
      libxkbcommon-x11 wayland-devel xorg-x11-server-Xvfb xorg-x11-utils \
      mesa-dri-drivers xorg-x11-server-Xephyr xorg-x11-xauth \
      xorg-x11-xkb-utils xkeyboard-config mesa-libEGL openbox zenity nxagent \
      xdotool xorg-x11-apps libgcrypt-devel openssl-devel gnutls-devel
    bash scripts/build-xephyr-portable.sh
    ;;
  *) echo "Unsupported glibc baseline: $baseline" >&2; exit 1 ;;
esac

curl --fail --location --retry 3 https://sh.rustup.rs --output /tmp/versus-rustup-init.sh
sh /tmp/versus-rustup-init.sh -y --profile minimal --default-toolchain 1.95.0
export PATH="${HOME}/.cargo/bin:$PATH"
rustup component add rustfmt
# Cargo does not track the host glibc as a build input. Keep baseline caches apart.
export CARGO_TARGET_DIR="target/linux-glibc-$baseline"
if test "$baseline" = 2.28; then
  mkdir -p "$CARGO_TARGET_DIR"
  gcc -std=c99 -Wall -Wextra -Werror scripts/check-x11-capabilities.c \
    -o "$CARGO_TARGET_DIR/check-x11-capabilities" $(pkg-config --cflags --libs x11 xi)
fi

PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts -p 'test_linux_*.py'
cargo fmt --check
cargo test --locked --offline
cargo build --release --locked --offline
bash scripts/package-linux.sh "$baseline"

image="$PWD/dist/Versus-linux-x86_64-glibc-$baseline.AppImage"
APPIMAGE_EXTRACT_AND_RUN=1 "$image" --version
APPIMAGE_EXTRACT_AND_RUN=1 "$image" --help
if test "$baseline" = 2.28; then
  bash scripts/test-linux-x11.sh "$image"
  PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-linux-fips.py "$image"
  exit 0
fi

# Launch the finished package with the same loader/libraries as its baseline.
# Software rendering avoids requiring a GPU on the GitHub runner.
Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp &
display_pid=$!
app_pid=''
cleanup() {
  if test -n "$app_pid"; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
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
  timeout 30s "$image" &
app_pid=$!
for attempt in {1..60}; do
  if DISPLAY=:99 xwininfo -root -tree | awk '/"Versus/{found=1} END {exit !found}'; then
    sleep 2
    kill -0 "$app_pid"
    echo "Packaged Versus GUI opened with glibc $baseline."
    exit 0
  fi
  kill -0 "$app_pid"
  sleep 0.25
done
echo "Packaged GUI did not open with glibc $baseline." >&2
exit 1
