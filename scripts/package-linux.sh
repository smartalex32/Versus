#!/usr/bin/env bash
set -euo pipefail

python3 scripts/check-linux-compatibility.py target/release/versus

mkdir -p AppDir/usr/bin AppDir/usr/share/applications AppDir/usr/share/pixmaps AppDir/usr/share/licenses/versus dist
install -m 755 target/release/versus AppDir/usr/bin/versus
cp LICENSE AppDir/usr/share/licenses/versus/LICENSE
printf '[Desktop Entry]\nType=Application\nName=Versus\nExec=versus\nIcon=versus\nCategories=Development;Utility;\nTerminal=false\n' > AppDir/usr/share/applications/versus.desktop
cp assets/logo-icon.png AppDir/usr/share/pixmaps/versus.png
curl --fail --location --retry 3 https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage --output linuxdeploy.AppImage
chmod +x linuxdeploy.AppImage
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 ./linuxdeploy.AppImage --appdir AppDir --desktop-file AppDir/usr/share/applications/versus.desktop --icon-file AppDir/usr/share/pixmaps/versus.png --output appimage
python3 scripts/check-linux-compatibility.py AppDir
appimage_path="$(find . -maxdepth 1 -type f -name '*.AppImage' ! -name 'linuxdeploy.AppImage' -print -quit)"
test -n "$appimage_path"
mv "$appimage_path" dist/Versus.AppImage

# Check both the AppImage runtime and everything actually shipped in its payload.
verification_dir="$(mktemp -d)"
trap 'rm -rf "$verification_dir"' EXIT
appimage_absolute="$PWD/dist/Versus.AppImage"
(cd "$verification_dir" && "$appimage_absolute" --appimage-extract >/dev/null)
python3 scripts/check-linux-compatibility.py dist/Versus.AppImage "$verification_dir/squashfs-root"

mkdir -p portable-linux/versus-linux-x86_64
cp AppDir/usr/bin/versus portable-linux/versus-linux-x86_64/versus
cp README.md portable-linux/versus-linux-x86_64/README.md
cp LICENSE portable-linux/versus-linux-x86_64/LICENSE
tar -C portable-linux -czf dist/versus-linux-x86_64.tar.gz versus-linux-x86_64
