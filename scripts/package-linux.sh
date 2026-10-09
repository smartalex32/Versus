#!/usr/bin/env bash
# Build the X11-compatibility AppImage on Rocky Linux 8.  It bundles the local
# X server and its runtime data because NX/X2Go hosts cannot be expected to
# provide matching X11 or Mesa packages.
set -euo pipefail

appdir=AppDir
libdir=/usr/lib64

require_file() {
  local path="$1"
  if ! test -f "$path"; then
    echo "Missing compatibility packaging input: $path" >&2
    exit 1
  fi
}

require_directory() {
  local path="$1"
  if ! test -d "$path"; then
    echo "Missing compatibility packaging input directory: $path" >&2
    exit 1
  fi
}

bundle_rpm_licenses() {
  local package="$1"
  local source
  local count=0
  local destination="$appdir/usr/share/licenses/versus/$package"

  rpm -q "$package" >/dev/null
  while IFS= read -r source; do
    test -f "$source" || continue
    install -D -m 644 "$source" "$destination/${source##*/}"
    count=$((count + 1))
  done < <(rpm -ql --licensefiles "$package")
  if test "$count" -eq 0; then
    echo "No RPM license files found for compatibility runtime package: $package" >&2
    exit 1
  fi
}

python3 scripts/check-linux-compatibility.py target/release/versus

require_file scripts/launch-linux.sh
require_file /usr/bin/Xephyr
require_file /usr/bin/xdpyinfo
require_file /usr/bin/xprop
require_file /usr/bin/xauth
require_file /usr/bin/xkbcomp
require_file /usr/bin/openbox
require_file /usr/bin/zenity
require_directory /usr/share/X11/xkb
require_directory /usr/share/openbox
require_directory /usr/share/themes/Onyx-Citrus/openbox-3
require_directory /usr/share/glib-2.0/schemas
require_file "$libdir/dri/swrast_dri.so"
require_file "$libdir/libGLX_mesa.so.0"
require_file "$libdir/libEGL_mesa.so.0"

mkdir -p "$appdir/usr/bin" "$appdir/usr/lib/dri" "$appdir/usr/share/applications" \
  "$appdir/usr/share/pixmaps" "$appdir/usr/share/licenses/versus" "$appdir/usr/share/X11" dist
install -m 755 target/release/versus "$appdir/usr/bin/versus"
install -m 644 LICENSE "$appdir/usr/share/licenses/versus/LICENSE"
printf '[Desktop Entry]\nType=Application\nName=Versus\nExec=versus\nIcon=versus\nCategories=Development;Utility;\nTerminal=false\n' > "$appdir/usr/share/applications/versus.desktop"
install -m 644 assets/logo-icon.png "$appdir/usr/share/pixmaps/versus.png"
install -D -m 644 scripts/compat-openbox.xml "$appdir/usr/share/versus/compat-openbox.xml"

# XKB definitions and Openbox configuration/theme files are data rather than
# ELF dependencies.  AppRun gives Xephyr the XKB directory with -xkbdir.
cp -a /usr/share/X11/xkb "$appdir/usr/share/X11/"
cp -a /usr/share/openbox "$appdir/usr/share/"
mkdir -p "$appdir/usr/share/themes/Onyx-Citrus"
cp -a /usr/share/themes/Onyx-Citrus/openbox-3 "$appdir/usr/share/themes/Onyx-Citrus/"
# Zenity's GTK settings (including its compiled schema cache) must travel with
# the fallback picker when a host portal is unavailable.
cp -a /usr/share/glib-2.0 "$appdir/usr/share/"
# GLVND discovers EGL providers through JSON, not ELF dependency scanning.
# Use a bare library name so its lookup follows the bundle's library path.
mkdir -p "$appdir/usr/share/glvnd/egl_vendor.d"
printf '{"file_format_version":"1.0.0","ICD":{"library_path":"libEGL_mesa.so.0"}}\n' \
  > "$appdir/usr/share/glvnd/egl_vendor.d/50_mesa.json"

# Preserve the licenses shipped by the primary Rocky runtime packages.  RPM's
# license-file manifest avoids guessing the installed license paths.
for package in xorg-x11-server-Xephyr xorg-x11-utils xorg-x11-xauth \
  xorg-x11-xkb-utils xkeyboard-config mesa-dri-drivers mesa-libGL mesa-libEGL \
  openbox zenity glib2 gtk3; do
  bundle_rpm_licenses "$package"
done

curl --fail --location --retry 3 https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage --output linuxdeploy.AppImage
chmod +x linuxdeploy.AppImage

# libxkbcommon is dlopened by winit.  Mesa's vendor libraries and DRI drivers
# are also dlopened, so list them explicitly instead of relying on ELF NEEDED
# entries from Versus or libGL.
libraries=(
  "$libdir/libxkbcommon.so.0"
  "$libdir/libxkbcommon-x11.so.0"
  "$libdir/libGLX_mesa.so.0"
  "$libdir/libEGL_mesa.so.0"
  "$libdir/dri/swrast_dri.so"
)
for library in "${libraries[@]}"; do
  require_file "$library"
done

# kms_swrast is a software fallback on Mesa builds that ship it.  Bundle it
# when present without making the Rocky 8 package depend on an optional file.
if test -f "$libdir/dri/kms_swrast_dri.so"; then
  libraries+=("$libdir/dri/kms_swrast_dri.so")
fi

executables=(/usr/bin/Xephyr /usr/bin/xdpyinfo /usr/bin/xprop /usr/bin/xauth /usr/bin/xkbcomp /usr/bin/openbox /usr/bin/zenity)
library_arguments=()
for library in "${libraries[@]}"; do
  library_arguments+=(--library "$library")
done
executable_arguments=()
for executable in "${executables[@]}"; do
  executable_arguments+=(--executable "$executable")
done

# This deploy-only pass must complete before AppRun and non-ELF runtime files
# are installed.  The output pass below snapshots the completed AppDir.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 ./linuxdeploy.AppImage --appdir "$appdir" \
  --desktop-file "$appdir/usr/share/applications/versus.desktop" \
  --icon-file "$appdir/usr/share/pixmaps/versus.png" \
  "${executable_arguments[@]}" "${library_arguments[@]}"

# linuxdeploy puts --library inputs in usr/lib.  Mesa locates DRI drivers below
# usr/lib/dri, which AppRun exposes through LIBGL_DRIVERS_PATH.
for driver in "${libraries[@]}"; do
  case "$driver" in
    "$libdir"/dri/*)
      driver_name="${driver##*/}"
      deployed_driver="$appdir/usr/lib/$driver_name"
      if ! test -e "$deployed_driver"; then
        echo "linuxdeploy did not bundle Mesa DRI driver: $driver_name" >&2
        exit 1
      fi
      # Several Mesa driver names can symlink to one shared implementation.
      # Copy the resolved module so relocating a relative symlink cannot leave
      # it pointing at a nonexistent library inside the dri subdirectory.
      install -m 755 "$driver" "$appdir/usr/lib/dri/$driver_name"
      ;;
  esac
done

rm -f "$appdir/AppRun"
install -m 755 scripts/launch-linux.sh "$appdir/AppRun"
python3 scripts/check-linux-compatibility.py "$appdir"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 ./linuxdeploy.AppImage --appdir "$appdir" \
  --custom-apprun scripts/launch-linux.sh --output appimage

appimage_path="$(find . -maxdepth 1 -type f -name '*.AppImage' ! -name 'linuxdeploy.AppImage' -print -quit)"
test -n "$appimage_path"
mv "$appimage_path" dist/Versus.AppImage

# Check both the AppImage runtime and every ELF file in its finished payload.
verification_dir="$(mktemp -d)"
trap 'rm -rf "$verification_dir"' EXIT
appimage_absolute="$PWD/dist/Versus.AppImage"
(cd "$verification_dir" && "$appimage_absolute" --appimage-extract >/dev/null)
python3 scripts/check-linux-compatibility.py dist/Versus.AppImage "$verification_dir/squashfs-root"

# Keep the existing command-line tarball deliberately small: it remains the
# native Versus executable for users who provide their own graphics stack.
mkdir -p portable-linux/versus-linux-x86_64
cp "$appdir/usr/bin/versus" portable-linux/versus-linux-x86_64/versus
cp README.md portable-linux/versus-linux-x86_64/README.md
cp LICENSE portable-linux/versus-linux-x86_64/LICENSE
tar -C portable-linux -czf dist/versus-linux-x86_64.tar.gz versus-linux-x86_64
