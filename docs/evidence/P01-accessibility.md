# P01 — Keyboard focus and the accessibility tree on native Wayland

Recorded 2026-11-05 on `ci-py` (x86_64). Harness: `experiments/p01-gpui/accessibility.sh`, run
inside `nix develop .#gui-test` against the **release** application binary. Artifacts:
[accessibility.json](artifacts/p01-accessibility/accessibility.json),
[tree.json](artifacts/p01-accessibility/tree.json) and
[mailbox-start.png](artifacts/p01-accessibility/mailbox-start.png).

## Method

- A private headless sway session with a private session bus. The AT-SPI launcher
  (`at-spi-bus-launcher`) and registry (`at-spi2-registryd`) come from nixpkgs `at-spi2-core` 2.60.6,
  through `MAILSTRONAUT_ATSPI_LIBEXEC` in the `gui-test` shell.
- The application is the release binary, with synthetic mail from the performance fixture: the
  conversation accounts, a long original and 100 short ones.
- The application's tree is read over D-Bus with `gdbus` (`org.a11y.atspi.Accessible`), starting at
  the AT-SPI registry. Keyboard input is `Ctrl+L`, then `Down`, then `Down`, sent through `wtype`.
- Assistive technology is switched on with `org.a11y.Status.IsEnabled` only after the application
  has subscribed to that property. AccessKit reacts to changes of the property and never reads its
  initial value.

## Results

| Check | Result |
| --- | --- |
| The application's window is in the tree, with children | pass |
| Mailbox rows are named with sender, subject, account, message count and unread count | pass |
| The search field is exposed as "Search cached mail" | pass |
| Focus after `Ctrl+L` and one `Down` is on "Performance 000" | pass |
| Focus after a second `Down` is on "Performance 001" | pass |
| Focus moves between rows | pass |

The tree has 34 readable nodes. Sidebar routes, the Compose, Sync and appearance buttons, and the
Settings item are named.

## Defects found and fixed

1. **Rows had no names.** Kit's `ListItem` has an element id but no role. GPUI exposes an
   `aria_label` only when an element has both an id and a role. `crates/mailstronaut/src/mailbox.rs`
   now sets `Role::ListItem` on each row.
2. **Focus stayed on the list container.** Kit keeps keyboard focus on the list and moves its
   selection index, so the focused accessible was an unnamed node. Each row now reports itself as the
   active descendant when it is the selected row. The delegate records the selection in
   `Rows::selected` through `set_selected_index`. GPUI honours this only while the list has focus.

## Harness findings

- `IsEnabled` must change after the application subscribes. Setting it before the application starts
  does nothing.
- The launcher reset `IsEnabled` to `false` when its dconf commit failed. The harness runs GSettings
  with `GSETTINGS_BACKEND=memory`, which keeps the private session free of dconf.

## Limits

- This is AT-SPI tree and focus inspection in a headless private session, not a screen-reader test.
  No speech output or reading behaviour was checked. Orca and other assistive clients were not run.
- The reader pane is not covered. Its HTML mode is a raster with no accessibility tree, and its text
  mode is a native text area, which was not inspected here.
- Compose, dialogs, Settings internals, drafts and reminders were not inspected.
- Keyboard checks cover `Ctrl+L` and `Down` only. Full keyboard coverage is in the existing
  walkthroughs, not in this harness.
- One release run on one synthetic data set. Hosted CI and real desktop sessions are not covered.

## Local reader follow-up

The installed x86_64 package built from `722472b` exposed the current message after keyboard
selection and explicit Return. `Ctrl+Shift+T` focused the inner node named “Message text”.
The node returned `UnknownInterface` for `org.a11y.atspi.Text.GetText` with GPUI Kit
0.8.1, which exposed the focused label without the body text.
`MAILSTRONAUT_A11Y_REQUIRE_TEXT=1` makes that check mandatory in the private fixture.
The [native clipboard and reader test](P01-native-reader.md#local-installed-package-reader-recheck)
passes independently. The P01 keyboard/accessibility row remains open.

## Multiline text with GPUI Kit 0.10.0

GPUI Kit now exposes multiline input via its standard Textarea, including read-only
fields. The implementation uses the Toolkit's native AccessKit text runs.
[Upstream change](https://github.com/longbridge/gpui-kit/pull/3894).

The pinned local build passed the mandatory `MAILSTRONAUT_A11Y_REQUIRE_TEXT=1`
fixture in a private Wayland session. After an explicit keyboard open and
`Ctrl+Shift+T`, `org.a11y.atspi.Text.GetText` returned the whole synthetic body:
three lines containing English, French and Italian text. The same Text node
returned identical content after a typing attempt. Focus still followed mailbox
rows and moved to the node named “Message text”. The driver reported 63 accessible
nodes and no unreadable nodes. The local screenshot and report are in
`target/p01-a11y.fkXvdj/accessibility/`.

This establishes the native text interface and focus. Screen-reader speech
and the composition, Settings and dialog flows remain open.
