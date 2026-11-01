# P01 — explicit web links in the native reader

2026-11-01. This extends the cached HTML reader with explicit browser-portal
handoff. **P01 and the complete client remain open.** No real mail, account,
browser profile or host desktop portal was used in these fixtures.

## Implementation

The renderer uses Blitz's public `hit` API and DOM parent/attribute access after
a successful paint. CSS coordinates include the actual clamped document scroll;
scale, viewport and source must match the displayed frame. No DOM input event,
navigation callback, browser process or script runtime is introduced.

The URL parser already exposed by Blitz normalizes HTTP(S) targets, ASCII/IDN
hosts and path encoding. Relative and non-web schemes, embedded credentials,
control/bidi embedding/isolate characters and backslashes are rejected; addresses
are bounded to 4096 encoded bytes. Normal path spaces are percent-encoded. The
sealed `Link` type has no public raw constructor or URL-bearing Debug output.
An immutable Arc-backed list retains the first 128 distinct addresses for the
native picker; a clicked valid current-DOM anchor still works beyond that limit.
No dependency, lockfile, vendor hash or compiler-backend exception changed.

One latest-value worker queue handles explicit hit requests as well as paints.
It rejects unpainted/replaced documents and retired geometry. The native reader
checks current frame/source/serial again before emitting an intent. Native menu
callbacks capture the reading epoch; the workspace rechecks that epoch and visible
HTML mode. These private call paths are the only entry points to web-link handoff;
neither automatic loading nor passive selection creates an intent.

Kit's normal Button/DropdownMenu/PopupMenuItem callbacks provide the Links picker.
No `href`, `PopupMenuItem::link`, framework default URL opener, custom menu or
HTML hit-test implementation is used. Ctrl+Shift+L focuses the caller-owned
button; Enter, arrows, Enter and Escape use component behavior. The actual button
focus is restored after dismissal, guarded against a replaced/hidden view. A
noninteractive leading label was removed after native testing showed that the
pin's first Down selects index zero, including a label.

ASHPD's standard `OpenFileRequest::send_uri` hands only the canonical target to
the desktop portal. Its minimally checked Uri wrapper is not the validator.
One request per workspace may wait at a time. Local cancellation, a 60-second
deadline and redacted status prevent unbounded waits or secret-bearing errors;
there is no shell/xdg-open fallback. Source replacement, Settings, text mode or
window lifetime cancel pending local work. An already launched external browser
cannot be recalled; cancellation is not a promise of undoing remote desktop work.

## Local behavior evidence

Pinned `nix develop` format/tool lint, strict workspace all-target Clippy,
**148 normal tests** and the development build passed. New behavior tests exercise:

- Canonicalization, IDN/path encoding, credential/scheme/control rejection,
  deduplication, address/count budgets and redacted Debug.
- Actual top/end layout hits through nested text, clamped scroll and 2x scale;
  invalid points, mismatched/unpainted view, replaced/hidden sources, and a visible
  link beyond the picker's budget.
- Worker requests that cannot reuse another source's painted DOM or geometry.

The isolated [portal fixture](../../experiments/p01-blitz/portal.sh) uses a private,
activation-free D-Bus and the real ASHPD client. A synthetic service records the
actual OpenURI method, then supplies success, cancellation, a deliberately
secret-bearing refusal, or no response. The test verifies exact redacted results,
the deadline after an observed call, pre-cancelled intent making no call, prompt
local cancellation of an already waiting call, and missing-service behavior.
It never launches a browser or fetches a URL. The fixture is included in the Nix
release check alongside existing encrypted credential/account/OAuth checks.

The complete development native fixture passed in `target/p01.pbzSjU`, with
private XDG/D-Bus, DISPLAY unset, XWayland disabled and software Vulkan:

- Opening/refreshing the email makes zero portal calls. A click on its actual
  rendered anchor makes exactly one call with the synthetic canonical target.
- The native menu supports Escape/reopen, arrow/Enter selection and the explicit
  keyboard shortcut. A refused/cancelled response leaves readable content and
  redacted feedback. Beta's empty link picker cannot reuse Alpha's target.
- Source switches, hiding/restoration, resize, theme and offline restart cause
  no extra portal requests. Existing per-original Seen, exact Unicode copy,
  current-source image retirement and native scroll/HiDPI pixel checks still pass.
- Independent HTTP/file positive controls work. The application and children
  make zero IPv4/IPv6 socket attempts and never open the email-specified file;
  explicit link handoff is a D-Bus operation, not an application HTTP fetch.

The broader isolated renderer privacy matrix was rerun in
`target/blitz-privacy.6SJ7ja`: real DNS/UDP, TCP/redirect and four readable-file
controls passed; twelve resource variants painted valid local pixels with 48
denial callbacks, zero network operations and no email-specified file opens.

## Release and installed acceptance

The coordinated `ci/nix-check.sh` used 16 available CPUs and one Nix job, stable
LLVM, thin LTO and one codegen unit. Strict release Clippy, **148 normal tests and
six private checks** passed. The added private check is the real-ASHPD web-link
transport fixture above; the retained five encrypted credential/account/OAuth
checks still pass. Service `mailstronaut-web-links-gate.service`, invocation
`ca9f11f94b704ecd844ee409c29afbcf`, finished with a fresh numeric receipt `0`, a
full `all checks passed` log, inactive/collected state, `MainPID=0` and
`ExecMainStatus=0`. Aarch64 was omitted. No duplicate full build or hosted-CI wait
was used.

Installed package:
`/nix/store/sk9qzs8fjqwb0b8nbmg0hmzqn7pxla5b-mailstronaut-0.1.0`.
All 136 files in its filtered source
`/nix/store/1j2ffbcc2iapf2xs1wxjrvdm17xjxg5p-source` match the checkout byte for byte.
The installed empty-environment version/config initialization/check/example and
0600 native-source smoke passed. Final current-script flake checks and cached
`nix build` also passed without a second application compilation.

The complete reader fixture passed against this installed binary in
`target/p01.4OR8VD`, including the source/Seen/clipboard/pixel/privacy/offline
regressions and native menu behavior. An additional actual HTML-link click at
2x native scale made the fourth exact portal call, proving that the client keeps
input coordinates logical while the document raster is physical. Beta, hiding,
resize/theme and offline restart made no additional calls. All four handed-off
targets equal the synthetic address; the cancellation response remains recorded
as a separate case. No real browser was launched by the mock.

- [Installed native fixture output](artifacts/p01-web-links/native-reader.log)
- [Installed native web-link picker](artifacts/p01-web-links/native-links-menu.png)
- [Installed 2x reader](artifacts/p01-web-links/native-hidpi.png)
- [Native portal/file/network receipts](artifacts/p01-web-links/native-observations.json)
- [Source/gate identity and file hashes](artifacts/p01-web-links/source-and-gate.json)
- [Release check excerpt](artifacts/p01-web-links/release-excerpt.log)
- [Installed CLI/config smoke](artifacts/p01-web-links/installed-cli.log)
- [Final cached flake check](artifacts/p01-web-links/final-checks.log)
- [Independent renderer privacy rerun](artifacts/p01-web-links/privacy.log)

Full logs/traces and private profiles remain in ignored project `target/`. An
actual current-app screenshot was shown in chat. User-supplied reference images
remain ignored and are not committed. The complete objective and P01 remain open.

## Limits

Native testing uses a synthetic portal service. Real desktop browser activation,
Wayland parent/activation integration, Flatpak permissions and user-session refusal
remain distribution/desktop acceptance, not established by this fixture. Renderer
hit testing is tested on ordinary/nested anchors, scroll and scale; complex
overlapping/transformed layouts need broader compatibility evidence. The picker
exposes canonical targets as an alternative to renderer hit testing, but this is
not HTML text selection or an accessibility subtree. AT-SPI/screen-reader,
performance/idle/memory, Flatpak and the remaining mailbox features stay open.
