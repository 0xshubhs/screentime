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
             | session D-Bus signal              | native messaging (stdio)
             v                                   v
        +------------------------------------------------+
        | screentimed (Rust, systemd --user service)     |
        |  - merges focus + idle + lock + web streams    |
        |  - closes/opens usage intervals                |
        |  - categorises (rules.toml)                    |
        |  - writes SQLite (~/.local/share/screentime)   |
        |  - exposes query API on D-Bus                  |
        +------------------------+-----------------------+
                                 |
             +-------------------+--------------------+
             v                                        v
   screentime CLI (today, week, top)       Screentime app (GTK4 + libadwaita)
```

### Crates / components

| Path | Purpose |
| --- | --- |
| `crates/screentime-core` | Data model, SQLite schema + queries, categorisation rules, aggregation (daily totals, averages, top N) |
| `crates/screentimed` | Daemon: D-Bus listeners, interval state machine, writer, query service |
| `crates/screentime-cli` | `screentime today`, `screentime week`, `screentime top --by app|site|category` |
| `crates/screentime-app` (phase 3) | GTK4 / libadwaita UI |
| `gnome-extension/` | Focus tracker for GNOME Shell 45+ |
| `browser-extension/` (phase 2) | Active tab reporter + native messaging host manifest |
| `data/rules.toml` | Default app/domain to category mapping, user overridable |

## 4. Data model

```sql
-- one row per continuous span of focused use
usage(id, start_ms, end_ms, kind TEXT CHECK(kind IN ('app','web')),
      app_id TEXT,          -- desktop file id, e.g. code.desktop
      domain TEXT,          -- for kind = 'web'
      title TEXT,           -- optional, can be disabled for privacy
      category TEXT)
pickups(ts_ms)              -- every unlock
```

Rules:
- An interval closes on focus change, idle > 2 min (configurable), lock, suspend or shutdown.
- Browser time is stored twice on purpose: once as the browser app, and as `web` rows for the
  domain, so the UI can show "Apps" and "Websites" tabs like Screen Time without double
  counting the total (totals are computed from `app` rows only).
- Media exception: if the focused tab is audible (video), idle does not stop the clock.
- Category is resolved at write time but recomputed on rules change.

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

1. **Core tracking (MVP).** GNOME extension, daemon, SQLite, rules, CLI `today` / `week`.
   Done when: a day of real use produces correct per-app totals (spot checked against a stopwatch).
2. **Websites.** Chrome + Firefox extension, native messaging host, domain rows, audible flag.
3. **App.** GTK4 / libadwaita UI mirroring Screen Time (Today / Week toggle, charts, lists,
   category pills, editable categories).
4. **Limits and downtime.** Per app/category daily limits with notifications, scheduled downtime,
   weekly report notification.
5. **Portability.** wlroots and X11 backends, KDE (KWin script), packaging (.deb, Flatpak for
   the app, extension on extensions.gnome.org), export to CSV/JSON.

## 8. Privacy

Local only, no network. Window titles optional (off by default for web rows, which store only
the domain). `screentime purge --before DATE` and a pause toggle.
