#!/usr/bin/env bash
# Rebuild Rocky 8's Xephyr with a PATH-resolved xkbcomp.  The stock server has
# its xkbcomp directory compiled as /usr/bin; -xkbdir only changes keyboard
# data and cannot make an AppImage's bundled xkbcomp relocatable.
set -euo pipefail

source_rpm="$(rpm -q --qf '%{SOURCERPM}' xorg-x11-server-Xephyr)"
source_rpm_url="https://download.rockylinux.org/pub/rocky/8.10/AppStream/source/tree/Packages/x/$source_rpm"
build_root="$(mktemp -d)"
trap 'rm -rf "$build_root"' EXIT
rpm_root="$build_root/rpmbuild"
mkdir -p "$rpm_root"

curl --fail --location --retry 3 "$source_rpm_url" --output "$build_root/xorg-x11-server.src.rpm"
rpm --define "_topdir $rpm_root" -i "$build_root/xorg-x11-server.src.rpm"
spec="$rpm_root/SPECS/xorg-x11-server.spec"
test -f "$spec"

# Preserve every Rocky source patch and build setting. The matching SRPM has one
# multiline %configure invocation.  An empty directory makes the server invoke
# xkbcomp through PATH, which AppRun puts in front of the host PATH.
configure_count="$(grep -Ec '^[[:space:]]*%configure[[:space:]]*\\[[:space:]]*$' "$spec")"
if test "$configure_count" -ne 1; then
  echo "Expected one multiline %configure invocation in $spec, found $configure_count" >&2
  exit 1
fi
sed -i '/^[[:space:]]*%configure[[:space:]]*\\[[:space:]]*$/a\
    --with-xkb-bin-directory= \\
' "$spec"

# Build requirements come from the same installed Rocky version's spec. dnf-plugins-core
# and PowerTools must already be enabled by build-linux-rocky8.sh.
dnf builddep --assumeyes "$spec"
rpmbuild --define "_topdir $rpm_root" -bb "$spec"

xephyr_rpm="$(find "$rpm_root/RPMS/x86_64" -maxdepth 1 -type f -name 'xorg-x11-server-Xephyr-*.rpm' -print -quit)"
test -n "$xephyr_rpm"
# Reinstall the rebuilt RPM over the build image package without changing its
# package identity or pulling newer host libraries.
rpm -Uvh --replacepkgs "$xephyr_rpm"
test -x /usr/bin/Xephyr
