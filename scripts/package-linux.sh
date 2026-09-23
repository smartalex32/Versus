#!/usr/bin/env bash
set -euo pipefail

mkdir -p AppDir/usr/bin AppDir/usr/share/applications AppDir/usr/share/icons/hicolor/scalable/apps AppDir/usr/share/licenses/versus dist
install -m 755 target/release/versus AppDir/usr/bin/versus
cp LICENSE AppDir/usr/share/licenses/versus/LICENSE
printf '[Desktop Entry]\nType=Application\nName=Versus\nExec=versus\nIcon=versus\nCategories=Development;Utility;\nTerminal=false\n' > AppDir/usr/share/applications/versus.desktop
printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="12" fill="#2e3440"/><path d="M12 14h10l10 30 10-30h10L37 54H27z" fill="#88c0d0"/></svg>' > AppDir/usr/share/icons/hicolor/scalable/apps/versus.svg
curl --fail --location --retry 3 https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage --output linuxdeploy.AppImage
chmod +x linuxdeploy.AppImage
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 ./linuxdeploy.AppImage --appdir AppDir --desktop-file AppDir/usr/share/applications/versus.desktop --icon-file AppDir/usr/share/icons/hicolor/scalable/apps/versus.svg --output appimage
appimage_path="$(find . -maxdepth 1 -type f -name '*.AppImage' ! -name 'linuxdeploy.AppImage' -print -quit)"
test -n "$appimage_path"
mv "$appimage_path" dist/Versus.AppImage

mkdir -p portable-linux/versus-linux-x86_64
cp AppDir/usr/bin/versus portable-linux/versus-linux-x86_64/versus
cp README.md portable-linux/versus-linux-x86_64/README.md
cp LICENSE portable-linux/versus-linux-x86_64/LICENSE
tar -C portable-linux -czf dist/versus-linux-x86_64.tar.gz versus-linux-x86_64
