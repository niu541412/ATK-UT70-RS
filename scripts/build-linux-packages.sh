#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
arch="$(uname -m)"
case "$arch" in
  x86_64) deb_arch=amd64 ;;
  aarch64) deb_arch=arm64 ;;
  *) echo "Unsupported Linux architecture: $arch" >&2; exit 1 ;;
esac

version="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')"
binary="$project_root/target/release/atk-ut70-rs"
package_root="$project_root/.package"
deb_root="$package_root/deb"
rpm_root="$package_root/rpmbuild"
mkdir -p "$deb_root/DEBIAN" "$rpm_root/SOURCES" "$rpm_root/SPECS" "$project_root/dist"

install -Dm0755 "$binary" "$deb_root/usr/bin/atk-ut70-rs"
install -Dm0644 "$project_root/assets/atk-ut70-rs.desktop" "$deb_root/usr/share/applications/atk-ut70-rs.desktop"
install -Dm0644 "$project_root/assets/atk-ut70-rs.png" "$deb_root/usr/share/icons/hicolor/512x512/apps/atk-ut70-rs.png"

mkdir -p "$package_root/debian"
cat > "$package_root/debian/control" <<'EOF'
Source: atk-ut70-rs
Section: utils
Priority: optional
Maintainer: ATK-UT70-RS contributors
Standards-Version: 4.6.2

Package: atk-ut70-rs
Architecture: any
Description: Desktop client for the ATK UT70 USB meter
EOF
auto_dependencies="$(cd "$package_root" && dpkg-shlibdeps -O -e"$binary")" || auto_dependencies=''
dependencies="$(printf '%s\n' "$auto_dependencies" | sed -n 's/^shlibs:Depends=//p' | head -n 1)"
if [[ -z "$dependencies" ]]; then
  echo "dpkg-shlibdeps provided no dependencies; using baseline library dependencies" >&2
  dependencies='libc6, libudev1, libx11-6, libxkbcommon0, libwayland-client0, libgtk-3-0t64 | libgtk-3-0, libdbus-1-3, libgl1'
fi

cat > "$deb_root/DEBIAN/control" <<EOF
Package: atk-ut70-rs
Version: $version
Section: utils
Priority: optional
Architecture: $deb_arch
Maintainer: ATK-UT70-RS contributors
Depends: $dependencies
Description: Desktop client for the ATK UT70 USB meter
 Live measurement charts and local firmware updates through the USB HID port.
EOF
dpkg-deb --root-owner-group --build "$deb_root" "$project_root/dist/ATK-UT70-RS-linux-$arch.deb"

install -Dm0755 "$binary" "$rpm_root/SOURCES/atk-ut70-rs"
install -Dm0644 "$project_root/assets/atk-ut70-rs.desktop" "$rpm_root/SOURCES/atk-ut70-rs.desktop"
install -Dm0644 "$project_root/assets/atk-ut70-rs.png" "$rpm_root/SOURCES/atk-ut70-rs.png"
rpmbuild -bb \
  --target "$arch" \
  --define "_topdir $rpm_root" \
  --define "app_version $version" \
  "$project_root/scripts/atk-ut70-rs.spec"
rpm_file="$(find "$rpm_root/RPMS" -type f -name '*.rpm' -print -quit)"
test -n "$rpm_file"
cp "$rpm_file" "$project_root/dist/ATK-UT70-RS-linux-$arch.rpm"

dpkg-deb --info "$project_root/dist/ATK-UT70-RS-linux-$arch.deb"
rpm -qip "$project_root/dist/ATK-UT70-RS-linux-$arch.rpm"
