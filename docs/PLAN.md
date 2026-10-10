# Screentime for Linux: plan

Goal: an Apple Screen Time style tracker for Ubuntu (GNOME on Wayland first) that records how long
each app and website is used, groups usage into categories, and shows today / this week / daily
averages, most used apps, pickups and hourly patterns. All data stays local.

## 1. Prior art (why build this)

| Tool | What it does well | Gap vs. Screen Time |
| --- | --- | --- |
| ActivityWatch | Open source, app + browser watchers, categories, web dashboard | On GNOME Wayland the window watcher does not work out of the box (needs a third-party shell extension); UI is a generic web dashboard, not a polished native "Screen Time" view; no pickups, limits or downtime |
| GNOME Settings, Wellbeing (GNOME 48+) | Native total screen time, break reminders, daily limit | Total time only: no per-app, per-site or category breakdown |
| arbtt, selfspy | Rule based logging | X11 only, CLI stats, unmaintained |
| Hamster, Timewarrior, KTimeTracker | Time tracking | Manual (you start/stop tasks), not automatic usage tracking |

Conclusion: nothing gives the full Screen Time experience on GNOME Wayland. ActivityWatch is the
closest; we borrow ideas (watchers + event store + categorisation rules) but ship a native,
opinionated app.

## 2. The hard part: Wayland

On Wayland a normal process cannot ask "which window is focused?". Only the compositor knows.
Options:

1. **GNOME Shell extension (chosen).** Runs inside gnome-shell, listens to
   `global.display` `notify::focus-window`, and exposes the focused app over D-Bus
   (`dev.zeroxshubhs.Screentime.Shell`). Emits a signal on every change, so no polling.
2. wlroots `foreign-toplevel` protocol: works on Sway/Hyprland, not GNOME. Future backend.
3. X11 `_NET_ACTIVE_WINDOW`: fallback backend for X11 sessions.

Idle and lock come from public D-Bus APIs that need no extension:
- `org.gnome.Mutter.IdleMonitor` (idle time, add watches)
- `org.gnome.ScreenSaver` `ActiveChanged` (lock / unlock, used for pickups)

## 3. Architecture

```
 gnome-shell                       browser (Chrome/Firefox)
 +-------------------------+       +---------------------------+
 | screentime@0xshubhs     |       | WebExtension (MV3)        |
 | focus-window -> D-Bus   |       | active tab domain, audible|
 +-----------+-------------+       +-------------+-------------+
             | session D-Bus signal              | HTTP POST to 127.0.0.1
             v                                   v
        +------------------------------------------------+
        | screentimed (Rust, systemd --user service)     |
        |  - merges focus + idle + lock + web streams    |
        |  - closes/opens usage intervals                |
        |  - categorises (rules.toml)                    |
        |  - writes SQLite (~/.local/share/screentime)   |
        |  - serves the dashboard + JSON API (loopback)  |
        +------------------------+-----------------------+
                                 |
             +-------------------+--------------------+
             v                                        v
   screentime CLI + top bar tray           Dashboard (TypeScript, in the browser)
```

### Crates / components

| Path | Purpose |
| --- | --- |
| `crates/screentime-core` | Data model, SQLite schema + queries, categorisation rules, aggregation (daily totals, averages, top N) |
| `crates/screentimed` | Daemon: D-Bus listeners, interval state machine, writer, query service |
| `crates/screentime-cli` | `screentime today`, `week`, `open`, and the top bar tray (`screentime tray`) |
| `ui/` | TypeScript dashboard (Vite), embedded into the daemon at build time |
| `gnome-extension/` | Focus tracker for GNOME Shell 45+ |
| `browser-extension/` | Active tab reporter (POSTs the domain to the daemon on localhost) |
| `data/rules.toml` | Default app/domain to category mapping, user overridable |

## 4. Data model

```sql
-- one row per continuous span of focused use
usage(id, start_ms, end_ms, kind TEXT CHECK(kind IN ('app','web')),
      app_id TEXT,          -- desktop file id, e.g. code.desktop
      domain TEXT)          -- for kind = 'web'; category is computed from rules at read time
pickups(ts_ms, kind, first_app)   -- every login and unlock, and the first app used after
notifications(ts_ms, app_id)
net_app(slot_ms, app_id, rx, tx)  -- 15 minute slots
net_iface(slot_ms, iface, kind, rx, tx)
```

Rules:
- An interval closes on focus change, idle > 2 min (configurable), lock, suspend or shutdown.
- Browser time is stored twice on purpose: once as the browser app, and as `web` rows for the
  domain, so the UI can show "Apps" and "Websites" tabs like Screen Time without double
  counting the total (totals are computed from `app` rows only).
- Media exception: if the focused tab is audible (video), idle does not stop the clock.
- Category is computed from the rules when reports are built, so editing rules re-categorises
  history immediately.

## 5. Categories (defaults, Screen Time style)

Productivity & Finance, Developer Tools, Social, Entertainment, Games, Creativity, Education,
Information & Reading, Utilities, Shopping & Food, Health & Fitness, Travel, Other.

Matching order: exact `app_id` > domain suffix (`*.youtube.com`) > glob on app_id > `Other`.
Users override in `~/.config/screentime/rules.toml`.

## 6. Reports (what the UI shows)

- Today: total, bar chart by hour stacked by category, top apps, top sites, pickups,
  first pickup time.
- Week: total, daily average, per-day bars, change vs. last week (%).
- Daily average over 7 / 30 days, per app, per site and per category.
- Most used: ranked list with time + share of total.
- Category drill-down: Productivity vs. Social vs. Entertainment etc.

## 7. Phases

1. **Core tracking, dashboard and tray (done, 2026-10-10).** GNOME extension, daemon, SQLite,
   rules, pickups (logins and unlocks with first app used), notification counts per app, network
   data per interface and per app, TypeScript dashboard embedded in the daemon, top bar tray,
   CLI, browser extension, install script with autostart, Docker Compose for the dashboard.
2. **Accuracy.** Validate a few days of real use against a stopwatch; eBPF-based per-app network
   accounting (optional privileged helper) to cover UDP/QUIC; per-site notification attribution.
3. **Limits and downtime.** Per app/category daily limits with notifications, scheduled downtime,
   weekly report notification.
4. **Portability.** wlroots and X11 focus backends, KDE (KWin script), packaging (.deb, the
   extension on extensions.gnome.org, the browser extension on the stores), CSV/JSON export.

## 8. Privacy

Local only, no network. Window titles optional (off by default for web rows, which store only
the domain). `screentime purge --before DATE` and a pause toggle.
