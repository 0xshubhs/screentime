#!/usr/bin/env bash
# Installs screentime from its .deb and starts it for you right away.
#
#   ./scripts/install-deb.sh                  build the .deb from this checkout, then install
#   ./scripts/install-deb.sh path/to/x.deb    install a .deb you already have
#
# Your usage history (~/.local/share/screentime) is never touched; the database upgrades
# itself in place, with a backup, when a new version first starts.
set -euo pipefail
cd "$(dirname "$0")/.."

DEB="${1:-}"
if [[ -z "$DEB" ]]; then
  ./packaging/build-deb.sh
  DEB=$(ls -t dist/screentime_*.deb | head -1)
fi
[[ -f "$DEB" ]] || { echo "no such file: $DEB" >&2; exit 1; }
DEB=$(realpath "$DEB")

# A per-user install (scripts/install.sh) would shadow the package's files; remove its files
# only. The extension stays enabled and the data stays where it is.
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
if [[ -e "$HOME/.local/bin/screentimed" || -e "$CONFIG/systemd/user/screentimed.service" ]]; then
  echo "==> Replacing the per-user install (your data stays)"
  systemctl --user stop screentimed.service 2>/dev/null || true
  rm -f "$HOME/.local/bin/screentimed" "$HOME/.local/bin/screentime" "$HOME/.local/bin/screentime-app" \
    "$CONFIG/systemd/user/screentimed.service" \
    "$CONFIG/systemd/user/graphical-session.target.wants/screentimed.service" \
    "$CONFIG/autostart/screentime-tray.desktop" \
    "$DATA/applications/screentime.desktop" "$DATA/applications/dev.zeroxshubhs.Screentime.desktop" \
    "$DATA/icons/hicolor/scalable/apps/"screentime*.svg
  rm -rf "$DATA/gnome-shell/extensions/screentime@0xshubhs"
fi

echo "==> Installing $(basename "$DEB")"
sudo apt-get install -y "$DEB"

echo "==> Starting for $USER"
systemctl --user daemon-reload
systemctl --user restart screentimed.service
pkill -f "screentime tray" 2>/dev/null || true
setsid -f /usr/bin/screentime tray >/dev/null 2>&1 < /dev/null

echo
echo "Installed. Screen Time starts by itself at every login; open it from the app grid."
echo "History: ${DATA}/screentime (kept across reinstalls and upgrades)."
