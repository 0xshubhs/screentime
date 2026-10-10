# screentime

Apple Screen Time, for Linux. It automatically tracks how long you use each app and website on
Ubuntu (GNOME, Wayland), groups the time into categories like Developer Tools, Productivity,
Social and Entertainment, and shows it the way Screen Time does: today, this week, this month,
daily averages, most used apps and sites, pickups, notifications and network data.

Everything stays on your machine in a local SQLite file.

## What it tracks

| | How |
| --- | --- |
| **App time** | A small GNOME Shell extension reports the focused app over D-Bus (Wayland does not let normal programs see focus). Time stops counting after 2 minutes without input, on lock and on suspend, except while media is playing. |
| **Website time** | A browser extension (Chrome, Brave, Edge, Firefox) reports the active tab's domain only. Browser time on a known site counts toward the site's category, so YouTube is Entertainment and GitHub is Developer Tools. |
| **Pickups** | Every login and every unlock, plus the **first app you used afterwards**. |
| **Notifications** | Counted per app by watching the session bus (`org.freedesktop.Notifications` and `org.gtk.Notifications`, which covers Flatpak apps too). |
| **Network data** | Totals per physical interface (Wi-Fi, Ethernet) from kernel counters; per-app figures from TCP socket stats, no root needed. UDP/QUIC traffic, very short connections and other users' services show up as *Other*. |

## The app

Open **Screen Time** from the app grid, the dock or the top bar icon. It is a native GNOME window
(GTK4 + libadwaita) that follows the system light/dark style; the dashboard inside is TypeScript,
served by the daemon (also reachable at `http://127.0.0.1:47821` in a browser):

- Day, Week and Month views with date navigation (arrow keys, `d` / `w` / `m`, or the date picker).
- Day view shows the week strip on top (selected day in color, average line) and the hourly
  chart below, stacked by category, like Screen Time.
- Most used: apps, websites or categories, each with a drill-down (usage chart, daily average,
  notifications, pickups it was opened first after, data used).
- Pickups, notifications and network data, each with a bar chart and per-app breakdown.
- Follows the system light or dark style.

## Top bar

`screentime tray` puts an indicator in the top right (like other AppIndicators) with today's
total, the top apps, pickups, notifications, data used, **Pause tracking**, and shortcuts to the
dashboard.

## Install

Needs Rust (cargo), Node (npm), and for the app window the WebKitGTK headers:

```sh
sudo apt install libwebkitgtk-6.0-dev libadwaita-1-dev
```

```sh
git clone https://github.com/0xshubhs/screentime && cd screentime
./scripts/install.sh
```

This builds everything into `~/.local/bin` and sets it up to **start automatically at every login**:

- `screentimed` as a systemd user service bound to the graphical session
- the tray via `~/.config/autostart`
- the GNOME Shell extension (enabled for your next login)

Then:

1. **Log out and back in once** so GNOME Shell loads the extension. App tracking starts then.
2. For time per website, load `browser-extension/` as an unpacked extension
   (`chrome://extensions` or `brave://extensions`, Developer mode, **Load unpacked**).
   Firefox: `about:debugging`, This Firefox, **Load Temporary Add-on**, pick `manifest.json`.

Remove with `./scripts/uninstall.sh` (add `--purge` to also delete your history).

## Docker

The tracker has to run in your desktop session, but the dashboard can run in a container that
reads the tracker's database:

```sh
docker compose up -d --build     # http://localhost:47822
```

## CLI

```sh
screentime today [YYYY-MM-DD]    # summary in the terminal
screentime week  [YYYY-MM-DD]
screentime open [week]           # open the Screen Time app
screentime categorize youtube.com
```

## Categories

Defaults live in [`data/rules.toml`](data/rules.toml). Override or extend them in
`~/.config/screentime/rules.toml` with the same format:

```toml
[apps]
"org.gnome.Ptyxis.desktop" = "Developer Tools"

[domains]
"news.ycombinator.com" = "Social"
```

## Development

```sh
cargo test                              # core + daemon tests
cd ui && npm ci && npm run dev          # dashboard with hot reload, proxied to a running screentimed
cargo run -p screentimed -- --db /tmp/st.db --listen 127.0.0.1:47899
```

Layout:

```
crates/screentime-core   schema, categorisation rules, report engine, app/icon lookup
crates/screentimed       daemon: D-Bus listeners, tracker state machine, network sampler, HTTP API
crates/screentime-cli    `screentime` CLI and the top bar tray
crates/screentime-app    native window (GTK4, libadwaita, WebKitGTK) around the dashboard
ui/                      TypeScript dashboard (Vite), embedded into screentimed at build time
gnome-extension/         GNOME Shell extension reporting the focused app
browser-extension/       WebExtension reporting the active tab's domain
packaging/, scripts/     systemd unit, autostart entries, install/uninstall
```

See [docs/PLAN.md](docs/PLAN.md) for the design and roadmap.

## Privacy

Only app ids, domains (never full URLs or page titles), counts and byte totals are stored, in
`~/.local/share/screentime/screentime.db`. The dashboard only answers on localhost. Incognito
tabs are never reported. Pause from the tray at any time.

## License

MIT
