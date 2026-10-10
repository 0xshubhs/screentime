// Exposes the focused window's app over D-Bus so screentimed can track it on Wayland,
// where ordinary clients cannot query focus.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const IFACE = `
<node>
  <interface name="dev.zeroxshubhs.Screentime.Shell">
    <method name="GetFocused">
      <arg type="s" direction="out" name="app_id"/>
      <arg type="s" direction="out" name="title"/>
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

    _focused() {
        const win = global.display.focus_window;
        if (!win)
            return ['', ''];
        const app = this._tracker.get_window_app(win);
        // Windows with no matching .desktop file get synthetic "window:N" ids; report
        // their WM class instead so they stay stable across restarts.
        const id = app && !app.is_window_backed()
            ? app.get_id()
            : `wm:${win.get_wm_class() ?? 'unknown'}`;
        return [id, win.get_title() ?? ''];
    }

    _emit() {
        const [appId, title] = this._focused();
        this._dbus.emit_signal('FocusChanged', new GLib.Variant('(ss)', [appId, title]));
    }
}
