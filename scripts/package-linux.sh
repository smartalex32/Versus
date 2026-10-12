#!/usr/bin/env bash
set -euo pipefail

baseline="${1:?Usage: package-linux.sh 2.35|2.28}"
case "$baseline" in
  2.28) exec bash scripts/package-linux-compat.sh "$baseline" ;;
  2.35) ;;
  *) echo "Unsupported glibc baseline: $baseline" >&2; exit 1 ;;
esac
binary="${CARGO_TARGET_DIR:-target}/release/versus"
python3 scripts/check-linux-compatibility.py --maximum "$baseline" "$binary"

build_directory="$(mktemp -d)"
trap 'rm -rf -- "$build_directory"' EXIT
appdir="$build_directory/AppDir"
libdir="$(pkg-config --variable=libdir xkbcommon)"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/pixmaps" \
  "$appdir/usr/share/licenses/versus" dist
install -m 755 "$binary" "$appdir/usr/bin/versus"
install -m 644 LICENSE "$appdir/usr/share/licenses/versus/LICENSE"
printf '[Desktop Entry]\nType=Application\nName=Versus\nExec=versus\nIcon=versus\nCategories=Development;Utility;\nTerminal=false\n' > "$appdir/usr/share/applications/versus.desktop"
install -m 644 assets/logo-icon.png "$appdir/usr/share/pixmaps/versus.png"

# The window backends load these libraries dynamically, so ordinary ELF
# dependency discovery cannot reliably find them. Graphics drivers stay native.
runtime_libraries=(
  libxkbcommon.so.0 libxkbcommon-x11.so.0 libxcb-xkb.so.1 libXau.so.6
  libX11.so.6 libXcursor.so.1 libX11-xcb.so.1 libXi.so.6 libXrender.so.1
  libwayland-client.so.0 libwayland-cursor.so.0 libwayland-egl.so.1
)
library_arguments=()
for name in "${runtime_libraries[@]}"; do
  test -f "$libdir/$name"
  library_arguments+=(--library "$libdir/$name")
done

curl --fail --location --retry 3 https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage --output "$build_directory/linuxdeploy.AppImage"
chmod +x "$build_directory/linuxdeploy.AppImage"
export OUTPUT="$PWD/dist/Versus-linux-x86_64-glibc-$baseline.AppImage"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$build_directory/linuxdeploy.AppImage" \
  --appdir "$appdir" --desktop-file "$appdir/usr/share/applications/versus.desktop" \
  --icon-file "$appdir/usr/share/pixmaps/versus.png" "${library_arguments[@]}" --output appimage
test -s "$OUTPUT"

# Check the final runtime and every packaged ELF, not just the original binary.
mkdir "$build_directory/check"
(cd "$build_directory/check" && "$OUTPUT" --appimage-extract >/dev/null)
for name in "${runtime_libraries[@]}"; do
  test -f "$build_directory/check/squashfs-root/usr/lib/$name"
done
python3 scripts/check-linux-compatibility.py --maximum "$baseline" \
  "$OUTPUT" "$build_directory/check/squashfs-root"

portable_name="versus-linux-x86_64-glibc-$baseline"
mkdir "$build_directory/$portable_name"
# Keep the plain executable's native loader paths; do not copy the AppDir-patched binary.
install -m 755 "$binary" "$build_directory/$portable_name/versus"
cp README.md LICENSE "$build_directory/$portable_name/"
tar -C "$build_directory" -czf "dist/$portable_name.tar.gz" "$portable_name"
