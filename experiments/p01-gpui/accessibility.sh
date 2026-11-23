#!/usr/bin/env bash
# Synthetic native Wayland accessibility acceptance: a private AT-SPI bus and registry, the real
# application's accessibility tree read over D-Bus, and keyboard focus followed through that tree.
# No real account, keyring, network service or desktop accessibility setting is used.
# Usage: accessibility.sh APP_BINARY WORKER_TEST_EXECUTABLE   (run inside nix develop .#gui-test)
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
fixture=$(realpath "${2:?prebuilt core worker test executable}")
: "${MAILSTRONAUT_ATSPI_LIBEXEC:?AT-SPI launcher and registry from the GUI test shell}"
root=$(mktemp -d "$PWD/target/p01-a11y.XXXXXX")
export XDG_RUNTIME_DIR="$root/runtime" XDG_CONFIG_HOME="$root/config"
export XDG_DATA_HOME="$root/data" XDG_CACHE_HOME="$root/cache" XDG_STATE_HOME="$root/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"
chmod 700 "$root" "$XDG_RUNTIME_DIR"
artifacts="$root/accessibility"
mkdir -p "$artifacts"
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET SWAYSOCK DBUS_SESSION_BUS_ADDRESS AT_SPI_BUS_ADDRESS
bus='' launcher='' registry=''
cleanup() {
  if declare -F session_cleanup > /dev/null; then session_cleanup; fi
  for pid in "$registry" "$launcher" "$bus"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT
cat > "$root/bus.conf" <<CONFIG
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path=$root/runtime/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/><allow own="*"/>
    <allow send_destination="*"/><allow receive_sender="*"/>
  </policy>
</busconfig>
CONFIG
dbus-daemon --nofork --nopidfile --config-file="$root/bus.conf" > "$root/bus.log" 2>&1 & bus=$!
export DBUS_SESSION_BUS_ADDRESS="unix:path=$root/runtime/bus"
for _ in {1..100}; do [[ -S $root/runtime/bus ]] && break; sleep .05; done
# The launcher registers org.a11y.Bus and org.a11y.Status on this private session bus, and starts
# the accessibility bus that the application's AccessKit adapter connects to.
# GSettings keeps its state in memory here. The host's dconf service is not part of this private session,
# and a failed dconf commit made the launcher reset the status flag before the application could read it.
GSETTINGS_BACKEND=memory "$MAILSTRONAUT_ATSPI_LIBEXEC/at-spi-bus-launcher" --launch-immediately \
  > "$artifacts/launcher.log" 2>&1 & launcher=$!
a11y=''
for _ in {1..200}; do
  a11y=$(gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
    --method org.a11y.Bus.GetAddress 2> /dev/null | python3 -c 'import ast,sys; print(ast.literal_eval(sys.stdin.read().strip())[0])' 2> /dev/null || true)
  [[ -n $a11y ]] && break
  kill -0 "$launcher"; sleep .05
done
[[ -n $a11y ]] || { printf '%s\n' 'The AT-SPI bus address was not published' >&2; exit 1; }
printf '%s\n' "$a11y" > "$artifacts/a11y-address"
# The registry is what clients query. The application registers its root with it when it connects.
GSETTINGS_BACKEND=memory AT_SPI_BUS_ADDRESS="$a11y" "$MAILSTRONAUT_ATSPI_LIBEXEC/at-spi2-registryd" --use-gnome-session \
  > "$artifacts/registry.log" 2>&1 & registry=$!
sleep .5
kill -0 "$registry"
export A11Y_ADDRESS=$a11y ARTIFACTS=$artifacts

config="$root/config.nix"
cat > "$config" <<'NIX'
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.automatic = false;
  accounts = map (identity: {
    id = identity.id;
    provider = "manual";
    address = identity.address;
    displayName = identity.id;
    userName = identity.id;
    imap = {host = "imap.example.invalid"; port = 993; security = "tls";};
    smtp = {host = "smtp.example.invalid"; port = 587; security = "starttls";};
    auth = {kind = "password"; secretRef = "synthetic-conversation-password";};
  }) [
    {id = "conversation-synthetic"; address = "reader@example.test";}
    {id = "other-synthetic"; address = "other@example.test";}
  ];
}
NIX
export MAILSTRONAUT_THREADS_CONFIG="$config" MAILSTRONAUT_THREADS_GUI_BINARY="$binary"
MAILSTRONAUT_PERFORMANCE_MAILBOX=1 "$fixture" seed_conversation_gui --ignored --exact --nocapture \
  > "$root/seed.log" 2>&1
[[ -s $root/data/mailstronaut/mail.sqlite3 ]]
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
start accessibility
window_open || { printf '%s\n' 'The application did not open a window' >&2; exit 1; }
capture accessibility-start
# The adapter reacts to a change of this property, not to its initial value. Enable assistive
# technology only after the application has subscribed, as a screen reader would.
sleep 1
# Record the status service's signals around the write, so a missing change is observable.
gdbus monitor --session --dest org.a11y.Bus > "$artifacts/status-monitor.log" 2>&1 & monitor=$!
sleep .3
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled '<true>' \
  > "$artifacts/status.log" 2>&1
sleep 1
kill "$monitor" 2>/dev/null || true
wait "$monitor" 2>/dev/null || true
driver="$artifacts/driver.py"
cat > "$driver" <<'PYDRIVER'
"""Read the application's AT-SPI tree over D-Bus and follow keyboard focus through it."""
import ast
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

artifacts = Path(os.environ["ARTIFACTS"])
a11y = os.environ["A11Y_ADDRESS"]
registry = ("org.a11y.atspi.Registry", "/org/a11y/atspi/accessible/root")
focused_bit = 1 << 12  # AT-SPI StateType FOCUSED
nodes_limit = 4000


def gdbus(dest, path, method, *args):
    command = ["gdbus", "call", "--address", a11y, "--dest", dest, "--object-path", path,
               "--method", method, *args]
    return subprocess.run(command, check=True, capture_output=True, text=True).stdout.strip()


def literal(output):
    # gdbus prints an empty array with its type first, as in "(@a(so) [],)". Drop that annotation,
    # then unwrap the variant <...> around a single property value.
    text = re.sub(r"@\S+ (?=\[)", "", output)
    # Object paths print with their type name, as in "objectpath '/org/...'".
    text = re.sub(r"objectpath (?=')", "", text)
    # Numbers print with their integer type, as in "[uint32 0, 0]".
    text = re.sub(r"\b(?:u?int(?:16|32|64)|byte|double) (?=-?\d)", "", text)
    return ast.literal_eval(re.sub(r"^\(<(.*)>,\)$", r"(\1,)", text, flags=re.S))


def name_of(bus, path):
    out = gdbus(bus, path, "org.freedesktop.DBus.Properties.Get", "org.a11y.atspi.Accessible", "Name")
    return literal(out)[0]


def role_of(bus, path):
    # AccessKit exposes the numeric AT-SPI role; the name method is not part of this interface.
    return literal(gdbus(bus, path, "org.a11y.atspi.Accessible.GetRole"))[0]


def children_of(bus, path):
    return [tuple(child) for child in literal(gdbus(bus, path, "org.a11y.atspi.Accessible.GetChildren"))[0]]


def states_of(bus, path):
    words = literal(gdbus(bus, path, "org.a11y.atspi.Accessible.GetState"))[0]
    return words[0] if words else 0


walk_errors = []


def walk():
    seen, queue, nodes = set(), [registry], []
    while queue and len(nodes) < nodes_limit:
        bus, path = queue.pop(0)
        if (bus, path) in seen:
            continue
        seen.add((bus, path))
        try:
            entry = {"bus": bus, "path": path, "name": name_of(bus, path), "role": role_of(bus, path),
                     "states": states_of(bus, path)}
            kids = children_of(bus, path)
        except (subprocess.CalledProcessError, ValueError, SyntaxError) as error:
            # Recorded rather than hidden, so an unreadable node is visible in the evidence.
            walk_errors.append({"bus": bus, "path": path, "error": type(error).__name__})
            continue
        entry["children"] = len(kids)
        nodes.append(entry)
        queue.extend(kids)
    return nodes


def wait_for_app():
    # Each registered application appears as a child of the registry root. Its own name can be
    # empty, so the window is identified later by its exposed title.
    for _ in range(200):
        children = children_of(*registry)
        if children:
            return children
        time.sleep(0.1)
    return []


def focused_name(nodes):
    matches = [node["name"] for node in nodes if node["states"] & focused_bit]
    return matches


def key(name):
    subprocess.run(["wtype", "-k", name], check=True)


checks = {}
if not wait_for_app():
    # Snapshot what the buses hold, so a failure says whether the application connected at all.
    def bus_names(address):
        command = ["gdbus", "call", "--address", address, "--dest", "org.freedesktop.DBus",
                   "--object-path", "/org/freedesktop/DBus", "--method", "org.freedesktop.DBus.ListNames"]
        return subprocess.run(command, capture_output=True, text=True).stdout.strip()
    snapshot = {
        "status_is_enabled": subprocess.run(
            ["gdbus", "call", "--session", "--dest", "org.a11y.Bus", "--object-path", "/org/a11y/bus",
             "--method", "org.freedesktop.DBus.Properties.Get", "org.a11y.Status", "IsEnabled"],
            capture_output=True, text=True).stdout.strip(),
        "accessibility_bus_names": bus_names(a11y),
        "session_bus_names": subprocess.run(
            ["gdbus", "call", "--session", "--dest", "org.freedesktop.DBus", "--object-path",
             "/org/freedesktop/DBus", "--method", "org.freedesktop.DBus.ListNames"],
            capture_output=True, text=True).stdout.strip(),
        "registry_children": subprocess.run(
            ["gdbus", "call", "--address", a11y, "--dest", registry[0], "--object-path", registry[1],
             "--method", "org.a11y.atspi.Accessible.GetChildren"],
            capture_output=True, text=True).stdout.strip(),
    }
    (artifacts / "snapshot.json").write_text(json.dumps(snapshot, indent=2))
    print(json.dumps(snapshot, indent=2))
    raise SystemExit("the application never appeared in the AT-SPI registry")
# The adapter receives the window's tree on the next draw after activation. Focusing the message
# list is an ordinary keyboard action that draws the window, so send it before the first walk.
subprocess.run(["wtype", "-M", "ctrl", "-k", "l", "-m", "ctrl"], check=True)
time.sleep(1)
# Record the application's root object as the bus exposes it, so an empty tree can be diagnosed.
application_bus, application_path = children_of(*registry)[0]
introspection = subprocess.run(
    ["gdbus", "introspect", "--address", a11y, "--dest", application_bus, "--object-path", application_path],
    capture_output=True, text=True)
(artifacts / "application-root-introspection.txt").write_text(introspection.stdout + introspection.stderr)
tree = walk()
(artifacts / "tree.json").write_text(json.dumps(tree, indent=2))
names = [node["name"] for node in tree]
rows = [name for name in names if "Performance 001" in name]
checks["application_window_exposed"] = any(
    "mailstronaut" in node["name"].lower() and node["children"] > 0 for node in tree)
checks["row_names_exposed"] = bool(rows)
checks["search_field_exposed"] = any("Search cached mail" in name for name in names)

# Keyboard: Ctrl+L focuses the message list, Down selects the first row, and focus follows.
subprocess.run(["wtype", "-M", "ctrl", "-k", "l", "-m", "ctrl"], check=True)
time.sleep(0.6)
key("Down")
time.sleep(0.6)
first_focus = focused_name(walk())
key("Down")
time.sleep(0.6)
second_focus = focused_name(walk())
checks["focus_follows_list_row_one"] = any("Performance 000" in name for name in first_focus)
checks["focus_follows_list_row_two"] = any("Performance 001" in name for name in second_focus)
checks["focus_moved_between_rows"] = first_focus != second_focus

# Open the focused original and focus its read-only native text alternative.
key("Return")
time.sleep(1)
subprocess.run(["wtype", "-M", "ctrl", "-M", "shift", "-k", "t", "-m", "shift", "-m", "ctrl"], check=True)
time.sleep(0.6)
reader = walk()
(artifacts / "reader-tree.json").write_text(json.dumps(reader, indent=2))
text_nodes = [node for node in reader
              if node["name"] == "Message text" and node["states"] & focused_bit]
checks["reader_text_focused"] = "Message text" in focused_name(reader)
reader_exposes_exact_text = False
if len(text_nodes) == 1:
    try:
        node = text_nodes[0]
        get_text = lambda: literal(gdbus(node["bus"], node["path"],
                                       "org.a11y.atspi.Text.GetText", "--", "0", "-1"))[0]
        expected = "Synthetic performance original 001.\nRéinscription au doctorat.\nScadenza di domani."
        reader_exposes_exact_text = get_text().strip() == expected
        if reader_exposes_exact_text:
            subprocess.run(["wtype", "attempted edit"], check=True)
            time.sleep(0.5)
            checks["reader_rejects_typed_edits"] = get_text().strip() == expected
    except subprocess.CalledProcessError as error:
        (artifacts / "reader-text-error.log").write_text(error.stderr)


def shortcut(*keys):
    *modifiers, name = keys
    command = ["wtype"]
    for modifier in modifiers:
        command.extend(["-M", modifier])
    command.extend(["-k", name])
    for modifier in reversed(modifiers):
        command.extend(["-m", modifier])
    subprocess.run(command, check=True)


def wait_tree(predicate):
    deadline = time.monotonic() + 10
    while True:
        nodes = walk()
        if predicate(nodes) or time.monotonic() >= deadline:
            return nodes
        time.sleep(0.1)


def tree_with_focus(label):
    return wait_tree(lambda nodes: label in focused_name(nodes))


def has_focus(label):
    return label in focused_name(tree_with_focus(label))


def capture(name):
    subprocess.run(["grim", str(artifacts / (name + ".png"))], check=True, timeout=15)


capture("accessibility-reader")
shortcut("ctrl", "n")
checks["new_draft_focuses_named_recipient"] = has_focus("To recipients")
shortcut("ctrl", "shift", "b")
copies = tree_with_focus("Cc recipients")
checks["show_copies_focuses_cc"] = "Cc recipients" in focused_name(copies)
key("Tab")
checks["tab_focuses_bcc"] = has_focus("Bcc recipients")
capture("accessibility-composer")
shortcut("ctrl", "shift", "b")
collapsed = tree_with_focus("To recipients")
checks["hiding_copies_restores_visible_focus"] = (
    "To recipients" in focused_name(collapsed)
    and not any(node["name"] in {"Cc recipients", "Bcc recipients"} for node in collapsed)
)
key("Tab")
checks["tab_focuses_subject"] = has_focus("Subject")
key("Tab")
checks["tab_focuses_message_body"] = has_focus("Message body")
(artifacts / "composer-tree.json").write_text(json.dumps(copies, indent=2))

shortcut("ctrl", "comma")
settings = wait_tree(lambda nodes: any(node["name"] == "Export to Nix" for node in nodes))
checks["settings_exposes_export_action"] = any(node["name"] == "Export to Nix" for node in settings)
shortcut("ctrl", "shift", "e")
dialog = wait_tree(lambda nodes: any(node["name"] == "Native Nix snapshot" for node in nodes))
(artifacts / "export-dialog-tree.json").write_text(json.dumps(dialog, indent=2))
checks["export_exposes_copy_action"] = any(node["name"] == "Copy native Nix snapshot" for node in dialog)
# Tab within the dialog to the actual text control; its separate copy button is not the text.
for _ in range(6):
    if "Native Nix snapshot" in focused_name(dialog):
        break
    key("Tab")
    time.sleep(0.1)
    dialog = walk()
snapshots = [node for node in dialog if node["name"] == "Native Nix snapshot" and node["states"] & focused_bit]
checks["export_text_keyboard_focus"] = len(snapshots) == 1
checks["export_accessible_text_matches_config"] = False
if len(snapshots) == 1:
    node = snapshots[0]
    snapshot = literal(gdbus(node["bus"], node["path"], "org.a11y.atspi.Text.GetText", "--", "0", "-1"))[0]
    expected = subprocess.run([os.environ["MAILSTRONAUT_THREADS_GUI_BINARY"], "config", "export", "--path",
                               os.environ["MAILSTRONAUT_THREADS_CONFIG"]], capture_output=True, check=True, text=True)
    checks["export_accessible_text_matches_config"] = snapshot.rstrip("\n") == expected.stdout.rstrip("\n")
capture("accessibility-export")
key("Escape")


def back_in_settings(nodes):
    names = {node["name"] for node in nodes}
    return "Export to Nix" in names and "Native Nix snapshot" not in names


checks["escape_returns_to_settings"] = back_in_settings(wait_tree(back_in_settings))
summary = {
    "nodes_exposed": len(tree),
    "unreadable_nodes": walk_errors[:10],
    "row_names_sample": rows[:3],
    "focused_after_first_down": first_focus[:3],
    "focused_after_second_down": second_focus[:3],
    "reader_exposes_exact_text": reader_exposes_exact_text,
    "checks": checks,
}
(artifacts / "accessibility.json").write_text(json.dumps(summary, indent=2))
print(json.dumps(summary, indent=2))
if not all(checks.values()):
    raise SystemExit("an accessibility check failed; see accessibility.json")
if os.environ.get("MAILSTRONAUT_A11Y_REQUIRE_TEXT") == "1" and not reader_exposes_exact_text:
    raise SystemExit("the reader does not expose its body through the AT-SPI Text interface")
PYDRIVER
python3 "$driver" > "$artifacts/driver.log" 2>&1 || {
  cat "$artifacts/driver.log" >&2
  printf 'Accessibility driver failed: %s\n' "$root" >&2
  exit 1
}
cat "$artifacts/driver.log"
printf 'Native accessibility-tree check: %s\n' "$artifacts"
