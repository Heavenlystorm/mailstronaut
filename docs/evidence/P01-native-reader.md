# Native GPUI Kit text reader

2026-10-31. P01 remains open. The reader uses the published GPUI Kit 0.8.1
`TextareaState` and `Textarea::readonly(true)`, without a custom selection/editor
implementation, HTML interpreter or dependency fork.

Successful current-generation opens put the existing bounded MIME text projection
in the native component. The standard component owns wrapping, scrolling,
selection, copy and rejected edits. Ctrl+Shift+T or the Text button focuses it;
that action does not open a message or authorize Seen. Existing explicit-open
generation/visibility checks remain in place. Window-bound async updates discard
stale results. Appearance changes do not reset the text, selection or scroll.

The demo retains its real Blitz preview and adds a synthetic, long plain-text
alternative. Arbitrary cached HTML is still disabled pending the resource/privacy
gate. No live account or personal mail was used.

## Checks

Through `nix develop`, formatting, Nix/CI/ShellCheck, strict workspace all-target
Clippy, all 132 normal tests and a development build passed. The existing two
explicit-open Seen tests passed. Seven private/live cases remain ignored in this
normal suite. This change does not add new unit tests that mirror widget internals.

```
nix develop --command bash ci/check.sh
nix develop --command cargo build --locked --offline
nix develop .#gui-test --command bash experiments/p01-gpui/wayland.sh target/debug/mailstronaut
```

The actual private Sway/Wayland window passed the existing Blitz, theme, keyboard
search and X11-only rejection checks, plus:

- Focus the native reader with Ctrl+Shift+T; select/copy the complete long Unicode
  message through GPUI and the Wayland clipboard. Observe both line 001 and 120.
- With all text selected, try typing, deletion and paste; select/copy again and
  compare exact bytes to the original. A fresh clipboard sentinel before copying
  prevents stale clipboard contents from satisfying the assertion.
- Ctrl+End, Shift+Home, Ctrl+C copies the expected final Unicode line. A screenshot
  shows the reader actually scrolled to that line.
- Toggle appearance, replace clipboard contents with a sentinel, and copy again:
  the existing selection still yields the same final line without reopening.

The clipboard observer first passed an explicit write/read control. These checks
run only with synthetic mail, private XDG paths, a private activation-free D-Bus,
`DISPLAY` unset and XWayland disabled. `wl-clipboard` is a GUI-test-shell tool,
not an application runtime dependency.

- [Native reader at document end](artifacts/p01-gpui-blitz/native-reader-end.png)

Full logs and synthetic clipboard files are in `target/gpui-wayland.J2ty9y`; the
development gate log is `target/gpui-reader-check.log`. The screenshot was visually
inspected. Font/glyph coverage depends on installed fonts; the test's restricted
font set does not establish complete CJK rendering, despite exact Unicode copy.

Actual AT-SPI/tree/screen-reader behavior, mouse selection, every keyboard motion,
large-mailbox/HiDPI performance and Flatpak acceptance remain separate. An
accessibility label is supplied but does not prove accessibility.

## Stable package and installed Wayland acceptance

Implementation: `291a728cccbb7ffc17594d4b2585fd99bbc8e4cd`.
The coordinated `ci/nix-check.sh` service invocation
`85352bf228b544e3af18927632d090eb` completed with a fresh numeric receipt `0`,
an inactive/collected service, `ExecMainStatus=0` and a full `all checks passed`
log. Release strict Clippy, all 132 normal workspace tests and four private
credential/OAuth fixture invocations passed. Only the five workspace crates
compiled/checked in the real package after dependency artifact restoration.
The x86_64 gate omitted aarch64-linux.

The subsequent cached `nix build --no-update-lock-file`, installed CLI/config
smoke in an empty environment, and the complete private Wayland script all passed
against `/nix/store/lxxzxm11nyddpyj03zskh3b1jy68vz8p-mailstronaut-0.1.0`.
The installed script verifies the Blitz preview, search/theme keyboard input,
X11-only rejection, exact Unicode clipboard contents, rejected edits, end-of-body
navigation and preserved selection after theme changes. Its private session is
`target/gpui-wayland.4sMmdC`; installed smoke is
`target/p01-installed-smoke.zg9Mpf`. No live account was used.
The installed reader screenshot was also visually inspected.

- [Installed native reader](artifacts/p01-gpui-blitz/installed-native-reader.png)
- [Installed smoke output](artifacts/p01-gpui-blitz/native-reader-installed.log)

The separate hosted CI repair run remains pending. These local package checks
do not close hosted CI, accessibility, HTML privacy or distribution acceptance.
