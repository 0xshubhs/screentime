// Exposes the focused window's app over D-Bus so screentimed can track it on Wayland,
// where ordinary clients cannot query focus.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const IFACE = `
<node>
  <interface name="dev.zeroxshubhs.Screentime.Shell">
    <method name="GetFocused">
      <arg type="s" direction="out" name="app_id"/>
      <arg type="s" direction="out" name="title"/>
    </method>
    <method name="GetOnScreen">
      <arg type="as" direction="out" name="app_ids"/>
    </method>
    <signal name="FocusChanged">
      <arg type="s" name="app_id"/>
      <arg type="s" name="title"/>
    </signal>
  </interface>
</node>`;

export default class ScreentimeExtension extends Extension {
    enable() {
        this._tracker = Shell.WindowTracker.get_default();
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, '/dev/zeroxshubhs/Screentime');
        this._focusId = global.display.connect('notify::focus-window', () => this._emit());
    }

    disable() {
        global.display.disconnect(this._focusId);
        this._dbus.unexport();
        this._dbus = null;
        this._tracker = null;
    }

    GetFocused() {
        return this._focused();
    }

    // The top window on each display other than the one holding focus: what the user sees
    // there while working on the focused display.
    GetOnScreen() {
        const focus = global.display.focus_window;
        const focusMonitor = focus ? focus.get_monitor() : -1;
        const workspace = global.workspace_manager.get_active_workspace();
        const windows = global.display.sort_windows_by_stacking(
            global.get_window_actors().map(a => a.meta_window)).reverse();
        const top = new Map();
        for (const win of windows) {
            const monitor = win.get_monitor();
            if (monitor < 0 || monitor === focusMonitor || top.has(monitor))
                continue;
            if (win.minimized || !win.showing_on_its_workspace() ||
                !(win.is_on_all_workspaces() || win.get_workspace() === workspace))
                continue;
            const type = win.get_window_type();
            if (type !== Meta.WindowType.NORMAL && type !== Meta.WindowType.DIALOG)
                continue;
            top.set(monitor, this._appId(win));
        }
        return [[...new Set(top.values())].filter(id => id)];
    }

    _focused() {
        const win = global.display.focus_window;
        // The desktop (icons on the wallpaper) is not an app someone is using.
        if (!win || win.get_window_type() === Meta.WindowType.DESKTOP)
            return ['', ''];
        return [this._appId(win), win.get_title() ?? ''];
    }

    _appId(win) {
        const app = this._tracker.get_window_app(win);
        // Windows with no matching .desktop file get synthetic "window:N" ids; report
        // their WM class instead so they stay stable across restarts.
        return app && !app.is_window_backed()
            ? app.get_id()
            : win.get_wm_class() ? `wm:${win.get_wm_class()}` : '';
    }

    _emit() {
        const [appId, title] = this._focused();
        this._dbus.emit_signal('FocusChanged', new GLib.Variant('(ss)', [appId, title]));
    }
}
