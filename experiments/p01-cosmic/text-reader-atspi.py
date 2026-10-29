"""Inspect only the private accessibility bus created by text-reader.sh."""

import json
import pathlib
import subprocess
import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Gio, GLib  # noqa: E402
import pyatspi  # noqa: E402

session = Gio.bus_get_sync(Gio.BusType.SESSION, None)
for enabled in (False, True):
    session.call_sync(
        "org.a11y.Bus",
        "/org/a11y/bus",
        "org.freedesktop.DBus.Properties",
        "Set",
        GLib.Variant("(ssv)", ("org.a11y.Status", "ScreenReaderEnabled", GLib.Variant("b", enabled))),
        None,
        Gio.DBusCallFlags.NONE,
        5000,
        None,
    )
    time.sleep(0.2)

subprocess.run(["xdotool", "windowfocus", sys.argv[2]], check=True)
subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+Home"], check=True)


def walk(node, depth=0):
    if depth > 16:
        return
    yield node
    for child in node:
        if child is not None:
            yield from walk(child, depth + 1)


deadline = time.monotonic() + 10
body = None
observed = []
redrawn = False
while time.monotonic() < deadline:
    observed = list(walk(pyatspi.Registry.getDesktop(0)))
    body = next((node for node in observed if node.name.startswith("Synthetic Servo viewport\n")), None)
    if body is not None:
        break
    if not redrawn and deadline - time.monotonic() < 8:
        subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+Home"], check=True)
        redrawn = True
    time.sleep(0.2)
assert any(node.name == "Native reader accessibility observer control" for node in observed), "AT-SPI positive control not observed"
if body is None:
    result = {
        "status": "blocked",
        "body_found": False,
        "observer_control_found": True,
        "nodes": [{"role": node.getRoleName(), "name": node.name[:100]} for node in observed],
    }
    pathlib.Path(sys.argv[1]).write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(result))
    sys.exit(0)

assert "Café — français, italiano: caffè. Ελληνικά. 日本語." in body.name
assert body.name.endswith("End of synthetic message.")
assert body.name.count("Synthetic scrolling marker") == 80
result = {
    "status": "body_exposed",
    "body_found": True,
    "observer_control_found": True,
    "role": body.getRoleName(),
    "body": body.name,
    "focused": body.getState().contains(pyatspi.STATE_FOCUSED),
    "interfaces": list(pyatspi.listInterfaces(body)),
    "bounds": list(body.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)),
}
try:
    text = body.queryText()
    result["text_characters"] = text.characterCount
    result["text_selection_count"] = text.getNSelections()
except NotImplementedError:
    result["text_interface"] = False
pathlib.Path(sys.argv[1]).write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
print(json.dumps({key: value for key, value in result.items() if key != "body"}))
