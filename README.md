# screentime

Apple Screen Time, for Linux. Automatically tracks how long you use each app and website on
Ubuntu (GNOME, Wayland), groups it into categories like Productivity, Developer Tools, Social and
Entertainment, and shows today, this week, daily averages and your most used apps.

Everything is stored locally in SQLite. Nothing leaves your machine.

> Status: early. See [docs/PLAN.md](docs/PLAN.md) for the design and roadmap.

## Layout

```
crates/screentime-core   data model, schema, categorisation rules, aggregation
crates/screentimed       tracking daemon (systemd user service)
crates/screentime-cli    `screentime today | week | top`
gnome-extension/         GNOME Shell extension exposing the focused window over D-Bus
data/rules.toml          default app/site -> category rules
```

## Build

```sh
cargo build
cargo test
```

## GNOME extension (dev)

```sh
ln -s "$PWD/gnome-extension/screentime@0xshubhs" ~/.local/share/gnome-shell/extensions/
# log out and back in (Wayland), then:
gnome-extensions enable screentime@0xshubhs
gdbus call --session --dest org.gnome.Shell \
  --object-path /dev/zeroxshubhs/Screentime \
  --method dev.zeroxshubhs.Screentime.Shell.GetFocused
```

## License

MIT
