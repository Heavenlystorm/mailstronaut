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
large-mailbox/HiDPI performance, installed-package and Flatpak acceptance remain
separate. An accessibility label is supplied but does not prove accessibility.
