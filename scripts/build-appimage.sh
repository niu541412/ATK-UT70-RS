#!/usr/bin/env bash
set -euo pipefail

arch="$(uname -m)"
case "$arch" in
  x86_64|aarch64) ;;
  *) echo "Unsupported AppImage architecture: $arch" >&2; exit 1 ;;
esac

mkdir -p .tools dist
curl --fail --location --retry 3 \
  "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${arch}.AppImage" \
  --output .tools/linuxdeploy
curl --fail --location --retry 3 \
  "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-${arch}.AppImage" \
  --output .tools/linuxdeploy-plugin-appimage
chmod +x .tools/linuxdeploy .tools/linuxdeploy-plugin-appimage

export PATH="$(pwd)/.tools:$PATH"
export APPIMAGE_EXTRACT_AND_RUN=1
export ARCH="$arch"
export VERSION="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')"
install -Dm0644 LICENSE AppDir/usr/share/doc/atk-ut70-rs/LICENSE
install -Dm0644 assets/icons/LICENSE.phosphor AppDir/usr/share/doc/atk-ut70-rs/LICENSE.phosphor
install -Dm0644 assets/fonts/OFL.Inter.txt AppDir/usr/share/doc/atk-ut70-rs/OFL.Inter.txt

.tools/linuxdeploy --appdir AppDir \
  --executable "target/release/atk-ut70-rs" \
  --desktop-file assets/atk-ut70-rs.desktop \
  --icon-file assets/atk-ut70-rs.png \
  --output appimage

artifact="$(find . -maxdepth 1 -type f -name '*.AppImage' -print -quit)"
test -n "$artifact"
mv "$artifact" "dist/ATK-UT70-RS-linux-${arch}.AppImage"
