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

declare -A recorded_source_families=()
declare -A recorded_runtime_packages=()
runtime_source_rpms=()

bundle_rpm_license_family() {
  local package="$1"
  if test -n "${recorded_runtime_packages[$package]:-}"; then
    return
  fi
  recorded_runtime_packages[$package]=1
  local source_rpm version license family_package path basename destination
  source_rpm="$(rpm -q --qf '%{SOURCERPM}' "$package")"
  version="$(rpm -q --qf '%{VERSION}-%{RELEASE}' "$package")"
  license="$(rpm -q --qf '%{LICENSE}' "$package")"
  test -n "$source_rpm" && test "$source_rpm" != '(none)'
  printf '%s\t%s\t%s\t%s\n' "$package" "$version" "$source_rpm" "$license" \
    >> "$runtime_license_inventory"

  if test -n "${recorded_source_families[$source_rpm]:-}"; then
    return
  fi
  recorded_source_families[$source_rpm]=1
  runtime_source_rpms+=("$source_rpm")
  destination="$appdir/usr/share/licenses/versus/runtime/$source_rpm"

  # License text is commonly placed in a related subpackage (for example,
  # xorg-x11-server-common).  Inspect every installed RPM sharing the source
  # RPM, retaining both declared %license files and ordinary COPYING/LICENSE
  # documents without inventing replacement text.
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
          install -D -m 644 "$path" "$destination/$family_package/$basename"
          ;;
      esac
    done < <(rpm -ql "$family_package")
  done < <(rpm -qa --qf '%{NAME}\t%{SOURCERPM}\n' | awk -F '\t' -v source="$source_rpm" '$2 == source {print $1}')
}

bundle_runtime_sources() {
  local source_archive source_rpm mesa_version mesa_archive
  local source_nevras=()
  runtime_source_directory="$(mktemp -d)"
  trap 'rm -rf -- "$runtime_source_directory"' EXIT
  source_archive="$PWD/dist/linux-runtime-sources.tar.gz"

  # --source enables the corresponding source repositories. Request all exact
  # installed versions in one transaction, rather than accepting latest sources.
  for source_rpm in "${runtime_source_rpms[@]}"; do
    source_nevras+=("${source_rpm%.src.rpm}")
  done
  dnf download --source --destdir "$runtime_source_directory" "${source_nevras[@]}"
  for source_rpm in "${runtime_source_rpms[@]}"; do
    require_file "$runtime_source_directory/$source_rpm"
  done

  # Rocky's Mesa runtime RPMs do not carry the upstream license document.
  # Fetch the archive matching the installed version exactly, preserve it in
  # the source offer, and extract its original license text into the AppImage.
  mesa_version="$(rpm -q --qf '%{VERSION}' mesa-libGL)"
  mesa_archive="$runtime_source_directory/mesa-$mesa_version.tar.xz"
  curl --fail --location --retry 3 "https://archive.mesa3d.org/mesa-$mesa_version.tar.xz" \
    --output "$mesa_archive"
  mkdir -p "$appdir/usr/share/licenses/versus/runtime/mesa"
  tar -xOf "$mesa_archive" --wildcards '*/docs/license.rst' \
    > "$appdir/usr/share/licenses/versus/runtime/mesa/LICENSE.rst"
  test -s "$appdir/usr/share/licenses/versus/runtime/mesa/LICENSE.rst"

  # Include the rebuild recipe for the modified, PATH-resolved Xephyr.
  install -m 755 scripts/build-xephyr-portable.sh "$runtime_source_directory/"
  install -m 644 scripts/xephyr-portable.patch "$runtime_source_directory/"
  cp "$runtime_license_inventory" "$runtime_source_directory/RUNTIME-SOURCES.tsv"
  tar -C "$runtime_source_directory" -czf "$source_archive" .
  test -s "$source_archive"
  rm -rf "$runtime_source_directory"
  trap - EXIT
}

python3 scripts/check-linux-compatibility.py target/release/versus

require_file scripts/launch-linux.sh
require_file scripts/launch-linux-picker.sh
require_file /usr/bin/Xephyr
require_file /usr/bin/xdpyinfo
require_file /usr/bin/xprop
require_file /usr/bin/xauth
require_file /usr/bin/xkbcomp
require_file /usr/bin/openbox
require_file /usr/bin/zenity
require_directory /usr/share/X11/xkb
require_directory /usr/share/themes/Onyx-Citrus/openbox-3
require_directory /usr/share/glib-2.0/schemas
require_file "$libdir/dri/swrast_dri.so"
require_file "$libdir/libGLX_mesa.so.0"
require_file "$libdir/libEGL_mesa.so.0"
require_file "$libdir/libGL.so.1"
require_file "$libdir/libEGL.so.1"
require_file "$libdir/libGLdispatch.so.0"
require_file "$libdir/libGLX.so.0"
require_file "$libdir/libgbm.so.1"
require_file "$libdir/libglapi.so.0"
require_file "$libdir/libxcb-xkb.so.1"
require_file "$libdir/libXau.so.6"

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


curl --fail --location --retry 3 https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage --output linuxdeploy.AppImage
chmod +x linuxdeploy.AppImage

# linuxdeploy deliberately omits desktop graphics libraries.  Versus and the
# helpers need them through dlopen/GLVND, so stage their complete non-glibc ELF
# closure ourselves and still pass each root with --library to linuxdeploy.
libraries=(
  "$libdir/libxkbcommon.so.0"
  "$libdir/libxkbcommon-x11.so.0"
  "$libdir/libxcb-xkb.so.1"
  "$libdir/libXau.so.6"
  "$libdir/libGL.so.1"
  "$libdir/libEGL.so.1"
  "$libdir/libGLdispatch.so.0"
  "$libdir/libGLX.so.0"
  "$libdir/libGLX_mesa.so.0"
  "$libdir/libEGL_mesa.so.0"
  "$libdir/libgbm.so.1"
  "$libdir/libglapi.so.0"
)
for library in "${libraries[@]}"; do
  require_file "$library"
done

# kms_swrast is a software fallback on Mesa builds that ship it.  Bundle it
# when present without making the Rocky 8 package depend on an optional file.
drivers=("$libdir/dri/swrast_dri.so")
if test -f "$libdir/dri/kms_swrast_dri.so"; then
  drivers+=("$libdir/dri/kms_swrast_dri.so")
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

runtime_arguments=()
for library in "${libraries[@]}"; do
  runtime_arguments+=(--library "$library")
done
for executable in "${executables[@]}" "${drivers[@]}"; do
  runtime_arguments+=(--executable "$executable")
done
python3 scripts/linux-runtime-libraries.py --destination "$appdir/usr/lib" \
  --native-destination "$appdir/usr/lib/native" \
  --manifest "$appdir/usr/share/versus/linux-runtime-libraries.txt" \
  "${runtime_arguments[@]}" \
  --native-library "$libdir/libxkbcommon.so.0" \
  --native-library "$libdir/libxkbcommon-x11.so.0" \
  --native-library "$libdir/libxcb-xkb.so.1" \
  --native-library "$libdir/libXau.so.6"

# Record each bundled runtime package and gather license documents from its
# complete installed source-RPM family.  Some subpackages intentionally carry
# no individual license payload; the family inventory and source offer retain
# the applicable original text.
runtime_license_inventory="$appdir/usr/share/licenses/versus/RUNTIME-SOURCES.tsv"
printf 'Package\tVersion-Release\tSource RPM\tDeclared license\n' > "$runtime_license_inventory"
for package in xorg-x11-server-Xephyr xorg-x11-utils xorg-x11-xauth \
  xorg-x11-xkb-utils xkeyboard-config mesa-dri-drivers mesa-libGL mesa-libEGL \
  mesa-libgbm libdrm libglvnd libglvnd-egl libglvnd-glx libX11 libXau libxcb \
  openbox zenity glib2 gtk3; do
  bundle_rpm_license_family "$package"
done

# Include license/source inventory for every library in the resolved closure,
# not just the helper entry points and dynamically loaded providers.
while IFS= read -r runtime_path; do
  runtime_package="$(rpm -qf --qf '%{NAME}' "$runtime_path")"
  bundle_rpm_license_family "$runtime_package"
done < <({
  awk '$2 == "<-" {print $3}' "$appdir/usr/share/versus/linux-runtime-libraries.txt"
  for runtime_path in "${executables[@]}" "${drivers[@]}"; do
    readlink -f "$runtime_path"
  done
} | sort -u)
bundle_runtime_sources

# This deploy-only pass must complete before AppRun and non-ELF runtime files
# are installed.  The output pass below snapshots the completed AppDir.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 ./linuxdeploy.AppImage --appdir "$appdir" \
  --desktop-file "$appdir/usr/share/applications/versus.desktop" \
  --icon-file "$appdir/usr/share/pixmaps/versus.png" \
  "${executable_arguments[@]}" "${library_arguments[@]}"

# Reapply resolved files after linuxdeploy so its excluded-library handling or
# symlink preservation cannot leave an AppDir library pointing back to the host.
python3 scripts/linux-runtime-libraries.py --destination "$appdir/usr/lib" \
  --native-destination "$appdir/usr/lib/native" \
  --manifest "$appdir/usr/share/versus/linux-runtime-libraries.txt" \
  "${runtime_arguments[@]}" \
  --native-library "$libdir/libxkbcommon.so.0" \
  --native-library "$libdir/libxkbcommon-x11.so.0" \
  --native-library "$libdir/libxcb-xkb.so.1" \
  --native-library "$libdir/libXau.so.6"

# Mesa locates DRI drivers below usr/lib/dri.  Copy the resolved modules so
# aliases from the build host never become broken links in the AppImage.
for driver in "${drivers[@]}"; do
  install -m 755 "$driver" "$appdir/usr/lib/dri/${driver##*/}"
done

runtime_manifest="$appdir/usr/share/versus/linux-runtime-libraries.txt"
test -s "$runtime_manifest"
awk '/^usr\/lib\// {print $1}' "$runtime_manifest" | while IFS= read -r path; do
  test -f "$appdir/$path"
done

# rfd invokes `zenity` by name.  Preserve the real executable behind a wrapper
# that gives only the picker the complete bundled GTK/Mesa runtime.
test -x "$appdir/usr/bin/zenity"
mv "$appdir/usr/bin/zenity" "$appdir/usr/bin/zenity-real"
install -m 755 scripts/launch-linux-picker.sh "$appdir/usr/bin/zenity"

rm -f "$appdir/AppRun"
install -m 755 scripts/launch-linux.sh "$appdir/AppRun"
python3 scripts/check-linux-compatibility.py "$appdir"
# linuxdeploy rewrites executable RUNPATHs to the full usr/lib directory.
# Restrict the main app to its native keyboard runtime, then call only the
# bundled output plugin so another deployment pass cannot undo that isolation.
patchelf --set-rpath '$ORIGIN/../lib/native' "$appdir/usr/bin/versus"
linuxdeploy_tools="$(mktemp -d)"
trap 'rm -rf -- "$linuxdeploy_tools"' EXIT
linuxdeploy_absolute="$PWD/linuxdeploy.AppImage"
(cd "$linuxdeploy_tools" && "$linuxdeploy_absolute" --appimage-extract >/dev/null)
output_plugin="$(find "$linuxdeploy_tools/squashfs-root" -type f -name linuxdeploy-plugin-appimage -print -quit)"
test -n "$output_plugin"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$output_plugin" --appdir "$PWD/$appdir"
rm -rf -- "$linuxdeploy_tools"
trap - EXIT

appimage_path="$(find . -maxdepth 1 -type f -name '*.AppImage' ! -name 'linuxdeploy.AppImage' -print -quit)"
test -n "$appimage_path"
mv "$appimage_path" dist/Versus.AppImage

# Check both the AppImage runtime and every ELF file in its finished payload.
verification_dir="$(mktemp -d)"
trap 'rm -rf "$verification_dir"' EXIT
appimage_absolute="$PWD/dist/Versus.AppImage"
(cd "$verification_dir" && "$appimage_absolute" --appimage-extract >/dev/null)
python3 scripts/check-linux-compatibility.py dist/Versus.AppImage "$verification_dir/squashfs-root"
test "$(patchelf --print-rpath "$verification_dir/squashfs-root/usr/bin/versus")" = '$ORIGIN/../lib/native'
echo 'Verified main executable cannot resolve the private graphics runtime through RUNPATH.'

# Keep the existing command-line tarball deliberately small: it remains the
# native Versus executable for users who provide their own graphics stack.
mkdir -p portable-linux/versus-linux-x86_64
cp target/release/versus portable-linux/versus-linux-x86_64/versus
cp README.md portable-linux/versus-linux-x86_64/README.md
cp LICENSE portable-linux/versus-linux-x86_64/LICENSE
tar -C portable-linux -czf dist/versus-linux-x86_64.tar.gz versus-linux-x86_64
