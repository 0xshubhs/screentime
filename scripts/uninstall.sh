#!/usr/bin/env bash
# Removes screentime. Your usage history is kept unless you pass --purge.
set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
UUID="screentime@0xshubhs"

systemctl --user disable --now screentimed.service 2>/dev/null || true
pkill -f "$PREFIX/bin/screentime tray" 2>/dev/null || true
gnome-extensions disable "$UUID" 2>/dev/null || true

rm -f "$PREFIX/bin/screentimed" "$PREFIX/bin/screentime" \
  "$CONFIG/systemd/user/screentimed.service" \
  "$CONFIG/autostart/screentime-tray.desktop" \
  "$DATA/applications/screentime.desktop" \
  "$DATA/icons/hicolor/scalable/apps/"screentime*.svg
rm -rf "$DATA/gnome-shell/extensions/$UUID"
systemctl --user daemon-reload

if [[ "${1:-}" == "--purge" ]]; then
  rm -rf "$DATA/screentime" "$CONFIG/screentime"
  echo "Removed screentime and its data."
else
  echo "Removed screentime. Usage history kept in $DATA/screentime (use --purge to delete)."
fi
