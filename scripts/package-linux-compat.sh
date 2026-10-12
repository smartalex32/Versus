#!/usr/bin/env bash
# Package the Rocky 8 build with a private Xephyr runtime for X2Go/nxagent.
set -euo pipefail

baseline="${1:?Usage: package-linux-compat.sh 2.28}"
test "$baseline" = 2.28 || { echo "Compatibility packaging requires glibc 2.28" >&2; exit 1; }
binary="${CARGO_TARGET_DIR:-target}/release/versus"
probe="${CARGO_TARGET_DIR:-target}/check-x11-capabilities"
python3 scripts/check-linux-compatibility.py --maximum "$baseline" "$binary" "$probe"

build_directory="$(mktemp -d)"
cleanup() { rm -rf -- "$build_directory"; }
trap cleanup EXIT
appdir="$build_directory/AppDir"
libdir=/usr/lib64
output="$PWD/dist/Versus-linux-x86_64-glibc-$baseline.AppImage"
source_archive="$PWD/dist/linux-runtime-sources-glibc-$baseline.tar.gz"
portable_name="versus-linux-x86_64-glibc-$baseline"
mkdir -p dist

require_file() { test -f "$1" || { echo "Missing compatibility packaging input: $1" >&2; exit 1; }; }
require_directory() { test -d "$1" || { echo "Missing compatibility packaging input directory: $1" >&2; exit 1; }; }

declare -A recorded_source_families=()
declare -A recorded_runtime_packages=()
runtime_source_rpms=()

bundle_rpm_license_family() {
  local package="$1" source_rpm version license family_package path basename destination
  test -n "${recorded_runtime_packages[$package]:-}" && return
  recorded_runtime_packages[$package]=1
  source_rpm="$(rpm -q --qf '%{SOURCERPM}' "$package")"
  version="$(rpm -q --qf '%{VERSION}-%{RELEASE}' "$package")"
  license="$(rpm -q --qf '%{LICENSE}' "$package")"
  test -n "$source_rpm" && test "$source_rpm" != '(none)'
  printf '%s\t%s\t%s\t%s\n' "$package" "$version" "$source_rpm" "$license" >> "$runtime_license_inventory"
  test -n "${recorded_source_families[$source_rpm]:-}" && return
  recorded_source_families[$source_rpm]=1
  runtime_source_rpms+=("$source_rpm")
  destination="$appdir/usr/share/licenses/versus/runtime/$source_rpm"
  while IFS= read -r family_package; do
    while IFS= read -r path; do
      test -f "$path" || continue
      install -D -m 644 "$path" "$destination/$family_package/${path##*/}"
    done < <(rpm -ql --licensefiles "$family_package")
    while IFS= read -r path; do
      test -f "$path" || continue
      basename="${path##*/}"
      case "$basename" in
        [Ll][Ii][Cc][Ee][Nn][Ss][Ee]*|[Cc][Oo][Pp][Yy][Ii][Nn][Gg]*|[Nn][Oo][Tt][Ii][Cc][Ee]*|[Cc][Oo][Pp][Yy][Rr][Ii][Gg][Hh][Tt]*)
          install -D -m 644 "$path" "$destination/$family_package/$basename" ;;
      esac
    done < <(rpm -ql "$family_package")
  done < <(rpm -qa --qf '%{NAME}\t%{SOURCERPM}\n' | awk -F '\t' -v source="$source_rpm" '$2 == source {print $1}')
}

bundle_runtime_sources() {
  local source_directory source_rpm mesa_version mesa_archive
  local source_nevras=()
  source_directory="$build_directory/runtime-sources"
  mkdir -p "$source_directory"
  for source_rpm in "${runtime_source_rpms[@]}"; do
    source_nevras+=("${source_rpm%.src.rpm}")
  done
  dnf download --source --destdir "$source_directory" "${source_nevras[@]}"
  for source_rpm in "${runtime_source_rpms[@]}"; do
    require_file "$source_directory/$source_rpm"
  done
  mesa_version="$(rpm -q --qf '%{VERSION}' mesa-libGL)"
  mesa_archive="$source_directory/mesa-$mesa_version.tar.xz"
  curl --fail --location --retry 3 "https://archive.mesa3d.org/mesa-$mesa_version.tar.xz" --output "$mesa_archive"
  mkdir -p "$appdir/usr/share/licenses/versus/runtime/mesa"
  tar -xOf "$mesa_archive" --wildcards '*/docs/license.rst' > "$appdir/usr/share/licenses/versus/runtime/mesa/LICENSE.rst"
  test -s "$appdir/usr/share/licenses/versus/runtime/mesa/LICENSE.rst"
  install -m 755 scripts/build-xephyr-portable.sh "$source_directory/"
  install -m 644 scripts/xephyr-portable.patch "$source_directory/"
  install -m 644 scripts/check-x11-capabilities.c "$source_directory/"
  cp "$runtime_license_inventory" "$source_directory/RUNTIME-SOURCES.tsv"
  tar -C "$source_directory" -czf "$source_archive" .
  test -s "$source_archive"
}

for required in scripts/launch-linux.sh scripts/launch-linux-picker.sh /usr/bin/Xephyr /usr/bin/xdpyinfo \
  /usr/bin/xprop /usr/bin/xauth /usr/bin/xkbcomp /usr/bin/openbox /usr/bin/zenity; do require_file "$required"; done
for required in /usr/share/X11/xkb /usr/share/themes/Onyx-Citrus/openbox-3 /usr/share/glib-2.0/schemas; do require_directory "$required"; done
for required in "$libdir/dri/swrast_dri.so" "$libdir/libGLX_mesa.so.0" "$libdir/libEGL_mesa.so.0" \
  "$libdir/libGL.so.1" "$libdir/libEGL.so.1" "$libdir/libGLdispatch.so.0" "$libdir/libGLX.so.0" \
  "$libdir/libgbm.so.1" "$libdir/libglapi.so.0"; do require_file "$required"; done

mkdir -p "$appdir/usr/bin" "$appdir/usr/lib/dri" "$appdir/usr/share/applications" \
  "$appdir/usr/share/pixmaps" "$appdir/usr/share/licenses/versus" "$appdir/usr/share/X11"
install -m 755 "$binary" "$appdir/usr/bin/versus"
install -m 755 "$probe" "$appdir/usr/bin/check-x11-capabilities"
install -m 644 LICENSE "$appdir/usr/share/licenses/versus/LICENSE"
printf '[Desktop Entry]\nType=Application\nName=Versus\nExec=versus\nIcon=versus\nCategories=Development;Utility;\nTerminal=false\n' > "$appdir/usr/share/applications/versus.desktop"
install -m 644 assets/logo-icon.png "$appdir/usr/share/pixmaps/versus.png"
install -D -m 644 scripts/compat-openbox.xml "$appdir/usr/share/versus/compat-openbox.xml"
install -D -m 644 scripts/compat-openbox-menu.xml "$appdir/usr/share/versus/openbox/menu.xml"
cp -a /usr/share/X11/xkb "$appdir/usr/share/X11/"
mkdir -p "$appdir/usr/share/themes/Onyx-Citrus"
cp -a /usr/share/themes/Onyx-Citrus/openbox-3 "$appdir/usr/share/themes/Onyx-Citrus/"
cp -a /usr/share/glib-2.0 "$appdir/usr/share/"
mkdir -p "$appdir/usr/share/glvnd/egl_vendor.d"
printf '{"file_format_version":"1.0.0","ICD":{"library_path":"libEGL_mesa.so.0"}}\n' > "$appdir/usr/share/glvnd/egl_vendor.d/50_mesa.json"

# These are loaded dynamically by winit/glutin. Keep the native roots in the
# compatibility artifact too, so ordinary hosts retain full desktop behavior.
native_libraries=(
  "$libdir/libxkbcommon.so.0" "$libdir/libxkbcommon-x11.so.0" "$libdir/libxcb-xkb.so.1" "$libdir/libXau.so.6"
  "$libdir/libX11.so.6" "$libdir/libXcursor.so.1" "$libdir/libX11-xcb.so.1" "$libdir/libXi.so.6" "$libdir/libXrender.so.1"
  "$libdir/libwayland-client.so.0" "$libdir/libwayland-cursor.so.0" "$libdir/libwayland-egl.so.1"
)
private_libraries=(
  "${native_libraries[@]}" "$libdir/libGL.so.1" "$libdir/libEGL.so.1" "$libdir/libGLdispatch.so.0" "$libdir/libGLX.so.0"
  "$libdir/libGLX_mesa.so.0" "$libdir/libEGL_mesa.so.0" "$libdir/libgbm.so.1" "$libdir/libglapi.so.0"
  # OpenSSL FIPS verification loads both libraries even where ELF metadata does
  # not name libssl directly. Their RPM-provided HMAC sidecars are copied by the
  # runtime helper and checked again after AppImage creation.
  "$libdir/libssl.so.1.1" "$libdir/libcrypto.so.1.1" "$libdir/libgcrypt.so.20"
  "$libdir/libgnutls.so.30" "$libdir/libnettle.so.6" "$libdir/libhogweed.so.4" "$libdir/libgmp.so.10"
)
for library in "${private_libraries[@]}"; do require_file "$library"; done
drivers=("$libdir/dri/swrast_dri.so")
test ! -f "$libdir/dri/kms_swrast_dri.so" || drivers+=("$libdir/dri/kms_swrast_dri.so")
executables=(/usr/bin/Xephyr /usr/bin/xdpyinfo /usr/bin/xprop /usr/bin/xauth /usr/bin/xkbcomp /usr/bin/openbox /usr/bin/zenity "$probe")
# The probe is locally compiled, so it is intentionally excluded from RPM
# ownership records. Its source is retained in the runtime source archive.
rpm_executables=(/usr/bin/Xephyr /usr/bin/xdpyinfo /usr/bin/xprop /usr/bin/xauth /usr/bin/xkbcomp /usr/bin/openbox /usr/bin/zenity)
runtime_arguments=()
for library in "${private_libraries[@]}"; do runtime_arguments+=(--library "$library"); done
for executable in "${executables[@]}" "${drivers[@]}"; do runtime_arguments+=(--executable "$executable"); done
native_arguments=()
for library in "${native_libraries[@]}"; do native_arguments+=(--native-library "$library"); done

runtime_manifest="$appdir/usr/share/versus/linux-runtime-libraries.txt"
python3 scripts/linux-runtime-libraries.py --destination "$appdir/usr/lib" --native-destination "$appdir/usr/lib/native" \
  --manifest "$runtime_manifest" "${runtime_arguments[@]}" "${native_arguments[@]}"

runtime_license_inventory="$appdir/usr/share/licenses/versus/RUNTIME-SOURCES.tsv"
printf 'Package\tVersion-Release\tSource RPM\tDeclared license\n' > "$runtime_license_inventory"
for package in xorg-x11-server-Xephyr xorg-x11-utils xorg-x11-xauth xorg-x11-xkb-utils xkeyboard-config \
  mesa-dri-drivers mesa-libGL mesa-libEGL mesa-libgbm libdrm libglvnd libglvnd-egl libglvnd-glx \
  libX11 libXau libxcb libXi libXcursor libXrender libX11-xcb openbox zenity glib2 gtk3; do
  bundle_rpm_license_family "$package"
done
while IFS= read -r runtime_path; do
  # Some integrity files belong to both runtime and development RPMs. Keep
  # owners separate rather than concatenating them into an invalid package name.
  if ! runtime_owners="$(rpm -qf --qf '%{NAME}\n' "$runtime_path")"; then
    echo "Cannot identify the RPM owner of runtime file: $runtime_path" >&2
    exit 1
  fi
  while IFS= read -r runtime_owner; do
    test -n "$runtime_owner" || continue
    bundle_rpm_license_family "$runtime_owner"
  done <<< "$runtime_owners"
done < <({
  awk '$2 == "<-" {print $3}' "$runtime_manifest"
  for runtime_path in "${rpm_executables[@]}" "${drivers[@]}"; do readlink -f "$runtime_path"; done
} | sort -u)
bundle_runtime_sources

linuxdeploy="$build_directory/linuxdeploy.AppImage"
curl --fail --location --retry 3 https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage --output "$linuxdeploy"
chmod +x "$linuxdeploy"
library_arguments=()
for library in "${private_libraries[@]}"; do library_arguments+=(--library "$library"); done
executable_arguments=()
for executable in "${executables[@]}"; do executable_arguments+=(--executable "$executable"); done
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$linuxdeploy" --appdir "$appdir" --desktop-file "$appdir/usr/share/applications/versus.desktop" \
  --icon-file "$appdir/usr/share/pixmaps/versus.png" "${executable_arguments[@]}" "${library_arguments[@]}"

# Reapply the complete closure after linuxdeploy has handled ELF discovery.
python3 scripts/linux-runtime-libraries.py --destination "$appdir/usr/lib" --native-destination "$appdir/usr/lib/native" \
  --manifest "$runtime_manifest" "${runtime_arguments[@]}" "${native_arguments[@]}"
for driver in "${drivers[@]}"; do install -m 755 "$driver" "$appdir/usr/lib/dri/${driver##*/}"; done
awk '/^usr\/lib\// {print $1}' "$runtime_manifest" | while IFS= read -r path; do test -f "$appdir/$path"; done

test -x "$appdir/usr/bin/zenity"
mv "$appdir/usr/bin/zenity" "$appdir/usr/bin/zenity-real"
install -m 755 scripts/launch-linux-picker.sh "$appdir/usr/bin/zenity"
rm -f "$appdir/AppRun"
install -m 755 scripts/launch-linux.sh "$appdir/AppRun"
python3 scripts/check-linux-compatibility.py --maximum "$baseline" "$appdir"
patchelf --set-rpath '$ORIGIN/../lib/native' "$appdir/usr/bin/versus"

tools_directory="$build_directory/linuxdeploy-tools"
mkdir "$tools_directory"
(cd "$tools_directory" && "$linuxdeploy" --appimage-extract >/dev/null)
output_plugin="$(find "$tools_directory/squashfs-root" -type f -name linuxdeploy-plugin-appimage -print -quit)"
test -n "$output_plugin"
export OUTPUT="$output"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$output_plugin" --appdir "$appdir"
test -s "$output"

verification_directory="$build_directory/check"
mkdir "$verification_directory"
(cd "$verification_directory" && "$output" --appimage-extract >/dev/null)
payload="$verification_directory/squashfs-root"
python3 scripts/check-linux-compatibility.py --maximum "$baseline" "$output" "$payload"
python3 scripts/linux-runtime-libraries.py --manifest "$runtime_manifest" --verify-manifest "$payload"
test "$(patchelf --print-rpath "$payload/usr/bin/versus")" = '$ORIGIN/../lib/native'
for name in "${native_libraries[@]##*/}"; do test -f "$payload/usr/lib/native/$name"; done
test -x "$payload/usr/bin/check-x11-capabilities"

mkdir "$build_directory/$portable_name"
install -m 755 "$binary" "$build_directory/$portable_name/versus"
cp README.md LICENSE "$build_directory/$portable_name/"
tar -C "$build_directory" -czf "dist/$portable_name.tar.gz" "$portable_name"
