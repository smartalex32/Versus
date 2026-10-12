#!/usr/bin/env bash
# Rebuild Rocky 8's Xephyr with a PATH-resolved xkbcomp. The stock server has
# /usr/bin compiled in, so -xkbdir alone cannot make an AppImage relocatable.
set -euo pipefail

script_directory="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
xephyr_patch="$script_directory/xephyr-portable.patch"
test -f "$xephyr_patch"

source_rpm="$(rpm -q --qf '%{SOURCERPM}' xorg-x11-server-Xephyr)"
installed_release="$(rpm -q --qf '%{RELEASE}' xorg-x11-server-Xephyr)"
dist_tag="$(sed -n 's/.*\(\.el8[^.]*\).*/\1/p' <<< "$installed_release")"
test -n "$dist_tag"
source_rpm_url="https://download.rockylinux.org/pub/rocky/8.10/AppStream/source/tree/Packages/x/$source_rpm"
build_root="$(mktemp -d)"
trap 'rm -rf -- "$build_root"' EXIT
rpm_root="$build_root/rpmbuild"
mkdir -p "$rpm_root"

curl --fail --location --retry 3 "$source_rpm_url" --output "$build_root/xorg-x11-server.src.rpm"
rpm --define "_topdir $rpm_root" -i "$build_root/xorg-x11-server.src.rpm"
spec="$rpm_root/SPECS/xorg-x11-server.spec"
test -f "$spec"
install -m 0644 "$xephyr_patch" "$rpm_root/SOURCES/xephyr-portable.patch"

if test "$(grep -Ec '^[[:space:]]*Patch99999:' "$spec")" -ne 0; then
  echo "Patch99999 is already reserved in $spec" >&2
  exit 1
fi
description_line="$(grep -n -m1 '^%description[[:space:]]*$' "$spec" | cut -d: -f1)"
test -n "$description_line"
test "$(grep -Ec '^[[:space:]]*%autopatch([[:space:]]|$)' "$spec")" -eq 1
sed -i "${description_line}iPatch99999: xephyr-portable.patch" "$spec"

source_archive="$(find "$rpm_root/SOURCES" -maxdepth 1 -type f -name 'xorg-server-*.tar.*' -print -quit)"
test -n "$source_archive"
patch_check_root="$build_root/patch-check"
mkdir "$patch_check_root"
tar -xf "$source_archive" -C "$patch_check_root"
source_tree="$(find "$patch_check_root" -mindepth 1 -maxdepth 1 -type d -name 'xorg-server-*' -print -quit)"
test -n "$source_tree"
(cd "$source_tree" && patch --dry-run -p1 < "$xephyr_patch")

configure_count="$(grep -Ec '^[[:space:]]*%configure[[:space:]].*\\[[:space:]]*$' "$spec")"
test "$configure_count" -eq 1
sed -i '/^[[:space:]]*%configure[[:space:]].*\\[[:space:]]*$/a\
    --with-xkb-bin-directory= \\
' "$spec"

dnf builddep --assumeyes --enablerepo=devel "$spec"
rpmbuild --define "_topdir $rpm_root" --define "dist $dist_tag" -bb "$spec"

identity_format='%{NAME}-%{VERSION}-%{RELEASE}.%{ARCH}'
installed_identity="$(rpm -q --qf "$identity_format" xorg-x11-server-Xephyr)"
xephyr_rpm="$rpm_root/RPMS/x86_64/$installed_identity.rpm"
test -f "$xephyr_rpm"
test "$(rpm -qp --qf "$identity_format" "$xephyr_rpm")" = "$installed_identity"
rpm -Uvh --replacepkgs --replacefiles "$xephyr_rpm"
test -x /usr/bin/Xephyr
