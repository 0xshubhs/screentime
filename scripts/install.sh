#!/usr/bin/env bash
# Builds and installs screentime for the current user, and sets it up to start on every login:
#   - screentimed as a systemd user service bound to the graphical session
#   - the top bar tray via ~/.config/autostart
#   - the GNOME Shell extension that reports the focused app
set -euo pipefail
cd "$(dirname "$0")/.."

PREFIX="${PREFIX:-$HOME/.local}"
BINDIR="$PREFIX/bin"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
UUID="screentime@0xshubhs"

for tool in cargo npm; do
  command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }
done

echo "==> Building dashboard"
(cd ui && npm ci --no-audit --no-fund --silent && npm run build --silent)

echo "==> Building binaries"
cargo build --release --locked

echo "==> Installing to $BINDIR"
install -Dm755 target/release/screentimed "$BINDIR/screentimed"
install -Dm755 target/release/screentime "$BINDIR/screentime"

for icon in screentime screentime-symbolic screentime-paused-symbolic; do
  install -Dm644 "data/icons/$icon.svg" "$DATA/icons/hicolor/scalable/apps/$icon.svg"
done
gtk-update-icon-cache -q -t "$DATA/icons/hicolor" 2>/dev/null || true

subst() { sed "s|@BINDIR@|$BINDIR|g" "$1"; }
mkdir -p "$CONFIG/systemd/user" "$CONFIG/autostart" "$DATA/applications"
subst packaging/screentimed.service > "$CONFIG/systemd/user/screentimed.service"
subst packaging/screentime-tray.desktop > "$CONFIG/autostart/screentime-tray.desktop"
subst packaging/screentime.desktop > "$DATA/applications/screentime.desktop"

echo "==> Installing GNOME Shell extension"
rm -rf "$DATA/gnome-shell/extensions/$UUID"
mkdir -p "$DATA/gnome-shell/extensions"
cp -r "gnome-extension/$UUID" "$DATA/gnome-shell/extensions/"
# A newly copied extension is only discovered after the next login on Wayland, so also
# add it to the enabled list directly; it then loads automatically at that login.
if ! gnome-extensions enable "$UUID" 2>/dev/null; then
  current=$(gsettings get org.gnome.shell enabled-extensions)
  if [[ "$current" != *"$UUID"* ]]; then
    if [[ "$current" == "@as []" || "$current" == "[]" ]]; then
      gsettings set org.gnome.shell enabled-extensions "['$UUID']"
    else
      gsettings set org.gnome.shell enabled-extensions "${current%]}, '$UUID']"
    fi
  fi
fi

echo "==> Starting service (and enabling it for every login)"
systemctl --user daemon-reload
systemctl --user enable screentimed.service
systemctl --user restart screentimed.service

pkill -f "$BINDIR/screentime tray" 2>/dev/null || true
setsid -f "$BINDIR/screentime" tray >/dev/null 2>&1 < /dev/null

cat <<MSG

Installed. Dashboard: http://127.0.0.1:47821  (or "Screen Time" in the app grid)

Next steps:
  1. Log out and back in once so GNOME Shell loads the extension (app tracking starts then).
  2. For time per website: open chrome://extensions (or brave://extensions), enable
     Developer mode, "Load unpacked", and pick: $PWD/browser-extension
     Firefox: about:debugging > This Firefox > Load Temporary Add-on > manifest.json

From then on everything starts by itself at every login.
MSG
