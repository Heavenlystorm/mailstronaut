# P01 native synthetic text reader

Implementation base: `d26d64e`; lint policy committed as `4833b6f`.
Local Nix/package and functional GUI checks passed. Accessibility is blocked;
P01 remains open.

The native Text page uses the pinned COSMIC selectable-text widget and Iced
scrollable, sharing the fixed decoded synthetic message with the HTML preview.
The fixture includes accents, Greek and Japanese, eighty repeated paragraphs and
a distinct end marker. A single native paragraph owns selection and copy across
line breaks. Opening Text focuses that paragraph, resets its scroll position and
drops the active WebView; the existing retained Servo engine/context remain.
Returning to Preview creates a new synthetic WebView under the existing policy.
There is still no public arbitrary HTML or mail-loading API.

Selection, Ctrl+A/C, context menus and Unicode layout use the standard toolkit.
Its paragraph accessibility hook is not currently exposed through the adapter.
The application adds only Page Up/Down and
Ctrl+Home/End operations against a stable scrollable ID, scoped to the Text page
and main window. Native widgets own the theme; Servo's HTML theme forwarding and
production resource/CSP policy are unchanged.

The user revised the initial blanket Clippy request during this work. The root
now enables `all`, `pedantic`, `nursery`, `dbg_macro`, `todo`, `unimplemented` and
`unwrap_used`; CI retains `-D warnings`, Rust forbids unsafe code and warns about
missing docs. The blanket restriction and cargo groups and their obsolete
expectations were removed. Configuration/store/parser changes are annotation
cleanup only, apart from one equivalent raw-to-normal test string literal.
Existing tests were retained. AGENTS.md and DELIVERY.md describe the current
policy; older evidence records the policy used at the time.

## Reproduction

```sh
nix flake check --no-update-lock-file --print-build-logs --cores 16 --max-jobs 1
nix build --no-update-lock-file --out-link target/p01-text-reader/package
nix develop .#gui-test --command bash experiments/p01-cosmic/text-reader.sh \
  "$PWD/target/p01-text-reader/package/bin/mailstronaut" \
  "$PWD/target/p01-text-reader/gui"
```

The GUI harness uses private Xvfb, XDG directories, clipboard and session and
accessibility buses. Theme checks use COSMIC's filesystem-watcher fallback in a
separate application instance from the connected accessibility pass. It does not
change the user's session settings or clipboard. Test tools remain confined to
the optional GUI development shell, outside application/package dependencies.

## Results

The coordinated x86_64 Nix gate used 16 cores and one Nix job, with stable LLVM,
thin LTO and one codegen unit. It passed strict all-target/all-feature release
Clippy, all 35 workspace tests and seven isolated renderer child invocations.
The existing dependency archive was reused; only workspace crates compiled.
`aarch64-linux` was omitted. The fresh receipt is `release-gate.status` and the
complete log is `release-gate.log` under `artifacts/p01-text-reader/`.

Exact derivation: `/nix/store/wfcwz44k3jxjp96k8903kkg5zfgqwhn2-mailstronaut-0.1.0.drv`.
Installed output: `/nix/store/8dgf65w8gxb7ysbrscda937lh9f1p9yi-mailstronaut-0.1.0`.
The gate service invocation was `9bf0c714bccf466cb3551e4f853d108b`.
All 18 workspace Rust/manifests/lock files matched the built source snapshot;
their hashes are in `source-check.json`. The subsequent cached `nix build` and
empty-environment installed CLI/configuration smoke passed, including overwrite
refusal, unsupported-schema rejection and 0700/0600 generated config permissions.
Pinned `nix develop` checks also passed Rust/Nix formatting, ShellCheck,
actionlint and Python compilation. The 35-path installed runtime closure contains
neither the observer tools nor the dependency archive.

The functional GUI sequence passed on private Xvfb at initial 100%, 150% and 200%
scale. Across those scales it checked 21 fresh full-body clipboard writes against
the exact 2369-byte UTF-8 fixture and six partial keyboard/mouse copies against
`Synthetic`. It exercised automatic body focus, Tab/Shift+Tab, keyboard re-entry
from the Text navigation button, reversible Page Up/Down, Ctrl+Home/End, live
light/dark changes, copy after resize, and return to a fresh dark Servo preview.
Page-up and start restoration compared identical captures; page-down/end and
theme changes required visible differences. Screenshots were also inspected.
Representative uncropped captures, clipboard contents, JSON summary, receipts
and complete GUI log are committed under `artifacts/p01-text-reader/`.

The first package passed strict Clippy, 35 release tests and seven renderer child
invocations, but opening Text in the actual window panicked in the toolkit's
selectable-text layout with `Downcast on stateless state`. A full backtrace and
the pinned wrapper/tree source identified duplicate named IDs: both the wrapper
and its inner text expose the same ID, and the toolkit's named-state restoration
cannot retain both. Leaving a generated unique ID avoided the crash, but the
subsequent fresh-clipboard test still failed: the clipboard contained the probe's
24-byte sentinel instead of the 2369-byte body. The pinned text `operate` method
reports `focusable(None, ...)`, so an operation addressed to its ID cannot focus
it. The correction leaves its generated ID alone and focuses the sole child of a
named container using the toolkit's scoped `focus_next` operation. Standard
selection and context menus remain enabled. This is why the package tests alone
are not reader acceptance.

The next GUI run verified full copy, partial keyboard selection, reversible page
scrolling, end/start jumps, theme changes and copy after resizing. Its keyboard
re-entry check then found that a scoped focus operation left the navigation button
focused alongside the body; cycling Tab/Shift+Tab lost body focus. The transfer now
chains the toolkit's global unfocus operation before scoped body focus. The GUI
test retains this exact keyboard activation and fresh-clipboard regression check.

The private accessibility bus initially selected dbus-broker, which expected a
user systemd service on that deliberately isolated bus. The harness now selects
`ATSPI_DBUS_IMPLEMENTATION=dbus-daemon`. A separate private-bus smoke verified
ScreenReaderEnabled changes and an empty, reachable AT-SPI registry. The same
setup was then used by the completed functional/observer runs.

## Confirmed accessibility blocker

At all three scales, the private AT-SPI observer found the Mailstronaut application
and an empty frame, with no message paragraph or navigation children. Repeated
keyboard-triggered redraws after activation did not publish them. The same
observer detected the full standard GTK/Zenity control, including its known label
and OK button. Each `*-atspi.json` explicitly records `status: blocked`,
`body_found: false` and `observer_control_found: true`. The functional runner's
exit zero means its visual/input/clipboard assertions passed; it is **not** an
accessibility pass.

This matches the inspected pinned Iced runner: `WinitActivationHandler` publishes
an initial window-only `TreeUpdate`; the runner assigns `a11y_enabled`, but no call
to `UserInterface::a11y_nodes` or adapter `update_if_active` was found there. The
widget's paragraph-node implementation alone therefore does not establish a
working desktop tree. Sources at the pinned Iced revision `229186b5180b6217d93866c912e6373ac378b231`:
[activation handler](https://github.com/pop-os/iced/blob/229186b5180b6217d93866c912e6373ac378b231/winit/src/a11y.rs),
[runner](https://github.com/pop-os/iced/blob/229186b5180b6217d93866c912e6373ac378b231/winit/src/lib.rs),
[UI tree hook](https://github.com/pop-os/iced/blob/229186b5180b6217d93866c912e6373ac378b231/runtime/src/user_interface.rs).

A bounded upstream publication fix or a reviewed compatible upstream pin, followed
by actual COSMIC/Wayland and screen-reader acceptance, is required. No dependency
fork, fake tree, alternative engine or accessibility-completion claim was added.
Zenity is only a test observer control and is absent from the application closure.

## Open acceptance

This is synthetic native reader work, not real email reading or full HTML input
support. A real COSMIC/Wayland session, screen-reader walkthrough, text navigation
and selection accessibility, dynamic scale changes, attachments, complete Servo
privacy and engine-free headless lifecycle remain separate acceptance. No remote
CI, live-account or current-source Flatpak acceptance is claimed.
