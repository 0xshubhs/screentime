#!/usr/bin/env bash
# Builds dist/screentime_<version>_<arch>.deb with plain dpkg-deb (no extra packaging tools).
#
# Layout: binaries in /usr/bin; the tracker as a systemd user service enabled for every
# user's graphical session; the tray in /etc/xdg/autostart; the GNOME Shell extension in
# /usr/share/gnome-shell/extensions. User data lives in ~/.local/share/screentime and is
# never part of the package, so install, upgrade, remove and purge all leave it alone.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(dpkg --print-architecture)
OUT="dist/screentime_${VERSION}_${ARCH}.deb"

for tool in cargo npm dpkg-deb; do
  command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }
done
pkg-config --exists webkitgtk-6.0 libadwaita-1 || {
  echo "missing build dependencies: sudo apt install libwebkitgtk-6.0-dev libadwaita-1-dev" >&2
  exit 1
}

echo "==> Building dashboard"
(cd ui && npm ci --no-audit --no-fund --silent && npm run build --silent)
echo "==> Building binaries"
cargo build --release --locked -p screentimed -p screentime-cli -p screentime-app

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
subst() { sed "s|@BINDIR@|/usr/bin|g" "$1"; }

for bin in screentimed screentime screentime-app; do
  install -Dm755 "target/release/$bin" "$STAGE/usr/bin/$bin"
done
install -d "$STAGE/usr/lib/systemd/user" "$STAGE/etc/xdg/autostart" "$STAGE/usr/share/applications"
subst packaging/screentimed.service > "$STAGE/usr/lib/systemd/user/screentimed.service"
subst packaging/screentime-tray.desktop > "$STAGE/etc/xdg/autostart/screentime-tray.desktop"
subst packaging/dev.zeroxshubhs.Screentime.desktop > "$STAGE/usr/share/applications/dev.zeroxshubhs.Screentime.desktop"
for icon in screentime screentime-symbolic screentime-paused-symbolic; do
  install -Dm644 "data/icons/$icon.svg" "$STAGE/usr/share/icons/hicolor/scalable/apps/$icon.svg"
done
install -d "$STAGE/usr/share/gnome-shell/extensions" "$STAGE/usr/share/screentime"
cp -r "gnome-extension/screentime@0xshubhs" "$STAGE/usr/share/gnome-shell/extensions/"
cp -r browser-extension "$STAGE/usr/share/screentime/"
install -Dm644 data/rules.toml "$STAGE/usr/share/screentime/rules.toml"
install -Dm644 LICENSE "$STAGE/usr/share/doc/screentime/copyright"
install -Dm644 README.md "$STAGE/usr/share/doc/screentime/README.md"
find "$STAGE/usr/share" -type f -exec chmod 644 {} +
find "$STAGE/usr/share" -type d -exec chmod 755 {} +

install -d "$STAGE/DEBIAN"
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: screentime
Version: $VERSION
Architecture: $ARCH
Maintainer: Shubham Tiwari <shubht3303@gmail.com>
Installed-Size: $(du -sk "$STAGE/usr" "$STAGE/etc" | awk '{s+=$1} END {print s}')
Depends: libc6, libgcc-s1, libgtk-4-1, libadwaita-1-0, libwebkitgtk-6.0-4, iproute2, systemd
Recommends: gnome-shell
Section: utils
Priority: optional
Homepage: https://github.com/0xshubhs/screentime
Description: Screen Time for Linux
 Tracks how long you use each app and website, pickups (logins and unlocks)
 with the first app used, notifications per app and network data, and shows
 them Screen Time style: day, week and month, daily averages, categories.
 .
 Usage history is stored per user in ~/.local/share/screentime and is kept
 across upgrades, removal and purge of this package.
CONTROL

cat > "$STAGE/DEBIAN/postinst" <<'SCRIPT'
#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
    # Start the tracker in every user's graphical session from their next login.
    systemctl --global enable screentimed.service >/dev/null 2>&1 || true
    gtk-update-icon-cache -q -t /usr/share/icons/hicolor 2>/dev/null || true
    update-desktop-database -q /usr/share/applications 2>/dev/null || true
fi
exit 0
SCRIPT
cat > "$STAGE/DEBIAN/prerm" <<'SCRIPT'
#!/bin/sh
set -e
if [ "$1" = "remove" ]; then
    systemctl --global disable screentimed.service >/dev/null 2>&1 || true
fi
exit 0
SCRIPT
chmod 755 "$STAGE/DEBIAN/postinst" "$STAGE/DEBIAN/prerm"

mkdir -p dist
dpkg-deb --root-owner-group --build "$STAGE" "$OUT" >/dev/null
echo "==> Built $OUT"
