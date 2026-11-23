"""Keyboard and accessibility checks against the current synthetic Settings and composer."""

import json
import os
from pathlib import Path
import subprocess


def check_workflows(walk, wait_tree, key, shortcut, focused_name, capture, gdbus, literal):
    artifacts = Path(os.environ["ARTIFACTS"])
    checks = {}

    def node_named(name, role):
        nodes = wait_tree(lambda tree: any(n["name"] == name and n["role"] == role for n in tree))
        for node in nodes:
            if node["name"] == name and node["role"] == role:
                return node
        (artifacts / "missing-workflow-control.json").write_text(json.dumps(nodes, indent=2))
        capture("missing-workflow-control")
        raise AssertionError(f"Missing control {name} (role {role})")

    def click_named(name, role):
        node = node_named(name, role)
        # This private Sway session has a single borderless maximized app at (0, 0).
        x, y, width, height = literal(gdbus(node["bus"], node["path"],
            "org.a11y.atspi.Component.GetExtents", "1"))[0]
        assert width > 0 and height > 0, (name, width, height)
        cursor = ["swaymsg", "seat", "seat0", "cursor"]
        for action in [["set", str(x + width // 2), str(y + height // 2)], ["press", "button1"], ["release", "button1"]]:
            subprocess.run(cursor + action, check=True, stdout=subprocess.DEVNULL)

    def focused(label):
        return label in focused_name(wait_tree(lambda tree: label in focused_name(tree)))

    # Open the synthetic account; inspect and edit the actual named text input.
    click_named("conversation-synthetic", 91)
    click_named("Incoming login", 79)
    checks["account_incoming_login_named_focus"] = focused("Incoming login")
    key("Tab")
    checks["account_outgoing_login_named_tab"] = focused("Outgoing login")
    subprocess.run(["wtype", "synthetic-outgoing-login"], check=True)
    node = node_named("Outgoing login", 79)
    typed = literal(gdbus(node["bus"], node["path"], "org.a11y.atspi.Text.GetText", "--", "0", "-1"))[0]
    checks["outgoing_login_editable"] = typed.endswith("synthetic-outgoing-login")
    capture("account-login-focus")

    click_named("Add account", 91)
    click_named("Account ID", 79)
    key("Tab")
    checks["new_account_name_keyboard_focus"] = focused("Your name")
    key("Tab")
    checks["new_account_email_keyboard_focus"] = focused("Email address")

    shortcut("ctrl", "n")
    assert focused("To recipients")
    recipient = "draft-cancel@example.invalid"
    subprocess.run(["wtype", recipient], check=True)
    # Walk back through the composer toolbar, keeping keyboard focus on the
    # actual trigger so the modal has a meaningful focus restoration target.
    for _ in range(8):
        shortcut("shift", "Tab")
        if "Discard draft" in focused_name(walk()):
            break
    checks["discard_trigger_keyboard_focus"] = focused("Discard draft")
    assert checks["discard_trigger_keyboard_focus"], "Discard draft is unreachable from the editor"
    key("Return")
    dialog = wait_tree(lambda tree: any(n["name"] == "Discard" for n in tree))
    (artifacts / "discard-dialog-tree.json").write_text(json.dumps(dialog, indent=2))
    checks["discard_dialog_exposes_choices"] = {"Cancel", "Discard"} <= {n["name"] for n in dialog}
    capture("discard-dialog")
    key("Escape")
    returned = wait_tree(lambda tree: not any(n["name"] == "Discard" for n in tree))
    node = next(n for n in returned if n["name"] == "To recipients" and n["role"] == 79)
    address = literal(gdbus(node["bus"], node["path"], "org.a11y.atspi.Text.GetText", "--", "0", "-1"))[0]
    checks["discard_escape_retains_draft"] = address == recipient
    checks["discard_escape_restores_focus"] = focused("Discard draft")
    return checks
