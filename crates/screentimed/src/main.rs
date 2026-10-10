//! Tracking daemon. Phase 1 wires up:
//! - focus changes from the GNOME Shell extension (dev.zeroxshubhs.Screentime.Shell)
//! - idle from org.gnome.Mutter.IdleMonitor
//! - lock/unlock from org.gnome.ScreenSaver
//! and writes usage intervals to SQLite. See docs/PLAN.md.

fn main() {
    let rules = screentime_core::Rules::builtin();
    eprintln!(
        "screentimed {}: not implemented yet ({} app rules loaded)",
        env!("CARGO_PKG_VERSION"),
        rules.apps.len()
    );
}
