"""Load the app in the system WebKitGTK (the engine Tauri uses on Linux), run a JS probe,
and save a screenshot. Usage: python3 scripts/webkit_probe.py [url] [out.png] [js]

Headless/Wayland hosts need: WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 GDK_BACKEND=x11
"""
import sys

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import GLib, Gtk, WebKit2  # noqa: E402

url = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:5173"
out = sys.argv[2] if len(sys.argv) > 2 else "/tmp/webkit.png"
probe = sys.argv[3] if len(sys.argv) > 3 else "document.title"

win = Gtk.OffscreenWindow()
view = WebKit2.WebView()
view.set_size_request(1120, 740)
win.add(view)
win.show_all()


def done_snapshot(v, res):
    try:
        v.get_snapshot_finish(res).write_to_png(out)
    except TypeError:
        print("(no screenshot: install python3-gi-cairo)")
    Gtk.main_quit()


def done_js(v, res):
    try:
        r = v.run_javascript_finish(res)
        print(r.get_js_value().to_string())
    except Exception as e:  # noqa: BLE001
        print("js error:", e)
    v.get_snapshot(WebKit2.SnapshotRegion.VISIBLE, WebKit2.SnapshotOptions.NONE, None, done_snapshot)


def run():
    view.run_javascript(probe, None, done_js)
    return False


def loaded(v, ev):
    if ev == WebKit2.LoadEvent.FINISHED:
        GLib.timeout_add(3500, run)


view.connect("load-changed", loaded)
view.load_uri(url)
GLib.timeout_add(30000, Gtk.main_quit)
Gtk.main()
