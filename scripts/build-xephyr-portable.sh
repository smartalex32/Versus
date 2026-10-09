#!/usr/bin/env bash
# Rebuild Rocky 8's Xephyr with a PATH-resolved xkbcomp.  The stock server has
# its xkbcomp directory compiled as /usr/bin; -xkbdir only changes keyboard
# data and cannot make an AppImage's bundled xkbcomp relocatable.
set -euo pipefail

script_directory="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
xephyr_patch="$script_directory/xephyr-portable.patch"
test -f "$xephyr_patch"

source_rpm="$(rpm -q --qf '%{SOURCERPM}' xorg-x11-server-Xephyr)"
installed_release="$(rpm -q --qf '%{RELEASE}' xorg-x11-server-Xephyr)"
# Rocky errata use .el8_10 while the container's default RPM macros use .el8.
# Preserve that dist macro so rebuilt packages keep the installed identity.
dist_tag="$(sed -n 's/.*\(\.el8[^.]*\).*/\1/p' <<< "$installed_release")"
test -n "$dist_tag"
source_rpm_url="https://download.rockylinux.org/pub/rocky/8.10/AppStream/source/tree/Packages/x/$source_rpm"
build_root="$(mktemp -d)"
trap 'rm -rf "$build_root"' EXIT
rpm_root="$build_root/rpmbuild"
mkdir -p "$rpm_root"

curl --fail --location --retry 3 "$source_rpm_url" --output "$build_root/xorg-x11-server.src.rpm"
rpm --define "_topdir $rpm_root" -i "$build_root/xorg-x11-server.src.rpm"
spec="$rpm_root/SPECS/xorg-x11-server.spec"
test -f "$spec"
install -m 0644 "$xephyr_patch" "$rpm_root/SOURCES/xephyr-portable.patch"

# Apply resize notifications and closing behavior after Rocky's security patches.
# Advertise WM_DELETE_WINDOW only for owned top-level windows, never -parent windows.
if test "$(grep -Ec '^[[:space:]]*Patch99999:' "$spec")" -ne 0; then
  echo "Patch99999 is already reserved in $spec" >&2
  exit 1
fi
description_line="$(grep -n -m1 '^%description[[:space:]]*$' "$spec" | cut -d: -f1)"
test -n "$description_line"
if test "$(grep -Ec '^[[:space:]]*%autopatch([[:space:]]|$)' "$spec")" -ne 1; then
  echo "Expected one %autopatch invocation in $spec" >&2
  exit 1
fi
sed -i "${description_line}iPatch99999: xephyr-portable.patch" "$spec"
test "$(grep -Ec '^[[:space:]]*Patch99999:[[:space:]]+xephyr-portable\.patch$' "$spec")" -eq 1

# Fail before the lengthy RPM build if the patch cannot apply to the matching
# source shipped by the SRPM.
source_archive="$(find "$rpm_root/SOURCES" -maxdepth 1 -type f -name 'xorg-server-*.tar.*' -print -quit)"
test -n "$source_archive"
patch_check_root="$build_root/patch-check"
mkdir "$patch_check_root"
tar -xf "$source_archive" -C "$patch_check_root"
source_tree="$(find "$patch_check_root" -mindepth 1 -maxdepth 1 -type d -name 'xorg-server-*' -print -quit)"
test -n "$source_tree"
(cd "$source_tree" && patch --dry-run -p1 < "$xephyr_patch")

# Preserve every Rocky source patch and build setting. The matching SRPM has one
# multiline %configure invocation.  An empty directory makes the server invoke
# xkbcomp through PATH, which AppRun puts in front of the host PATH.
configure_count="$(grep -Ec '^[[:space:]]*%configure[[:space:]].*\\[[:space:]]*$' "$spec")"
if test "$configure_count" -ne 1; then
  echo "Expected one multiline %configure invocation in $spec, found $configure_count" >&2
  exit 1
fi
sed -i '/^[[:space:]]*%configure[[:space:]].*\\[[:space:]]*$/a\
    --with-xkb-bin-directory= \\
' "$spec"

# Build requirements come from the same installed Rocky version's spec. dnf-plugins-core
# and PowerTools must already be enabled by build-linux-rocky8.sh.
# Rocky keeps some server build headers (notably libdmx-devel) in its
# build-only Devel repository. Enable it for this dependency transaction only.
dnf builddep --assumeyes --enablerepo=devel "$spec"
rpmbuild --define "_topdir $rpm_root" --define "dist $dist_tag" -bb "$spec"

identity_format='%{NAME}-%{VERSION}-%{RELEASE}.%{ARCH}'
installed_identity="$(rpm -q --qf "$identity_format" xorg-x11-server-Xephyr)"
# A wildcard also matches Xephyr-debuginfo, whose traversal order is undefined.
xephyr_rpm="$rpm_root/RPMS/x86_64/$installed_identity.rpm"
if ! test -f "$xephyr_rpm"; then
  echo "Rebuild did not produce the installed Xephyr identity: $installed_identity" >&2
  exit 1
fi
test "$(rpm -qp --qf "$identity_format" "$xephyr_rpm")" = "$installed_identity"
# Reinstall the rebuilt RPM over the build image package without changing its
# package identity or pulling newer host libraries.
rpm -Uvh --replacepkgs --replacefiles "$xephyr_rpm"
test -x /usr/bin/Xephyr
