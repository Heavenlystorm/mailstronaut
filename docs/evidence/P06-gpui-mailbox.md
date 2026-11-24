# P06 — GPUI Kit mailbox selection and unread repaint

Status: bounded current-stack implementation; **P06 and the full client remain
incomplete**. This replaces the scaffold row rendering and its shared generation,
following [current account Settings](P02-gpui-accounts.md). Retained X11 frontend
evidence does not establish the new interface.

## Behavior

The mailbox uses the pinned GPUI Kit 0.8.1 `List`, `ListState`, `ListDelegate` and
`ListItem`, with its native equal-height virtualization, selection, keyboard and
scrollbar behavior. The extra component search field is disabled; the application's
one query field still performs the retained local literal FTS search on Enter.
This does not implement fuzzy or multilingual meaning retrieval.

Ctrl+L focuses the list. Arrow navigation, restored selection, query/refresh and
return from Settings remain passive. Only `ListEvent::Confirm` routes a click or
Enter to the existing original-specific content/Seen gate; the delegate does not
also open the message. Rows expose an explicit accessible label, account identity,
cached thread size and canonical unread count using standard Label/Badge elements.
This is not screen-reader acceptance.

Query, original-body and synthetic-render generations are independent. A new
submitted query retires the old reader; a same-query refresh or completed flag
write does not retire a newer explicit open. Configuration revision fences every
result. A successful local flag commit queries canonical rows rather than setting
the whole thread read optimistically. It preserves the current passive selection
by account/group (or original when grouping is unavailable), without confirming
or scrolling it. The retained native read-only reader keeps its content/selection.
The submitted query is separate from unsent input edits.

No protocol, model or rendering dependencies changed. Cached untrusted HTML
remains disabled pending P01. No new network action, custom list virtualization,
timer/dwell policy, search engine or background daemon was introduced.

## Development and native acceptance

Pinned `nix develop` strict workspace Clippy, formatting/tool checks and 136
normal workspace tests passed (`target/gpui-mailbox-workspace.log`). The native
binary build passed with the configured Cranelift/Wild setup and existing narrow
raster exceptions. No new lint exception was needed.

The reproducible [native fixture](../../experiments/p06-mailbox/wayland.sh) uses
the existing compiled core `seed_conversation_gui` fixture, synthetic `.test` mail,
a private XDG profile and D-Bus bus, headless Sway with XWayland disabled, persistent
test input devices and Mesa software Vulkan. It does not access the user's profile
or keyring. Run through `nix develop .#gui-test`, setting
`MAILSTRONAUT_MAILBOX_FIXTURE` to the current core worker test executable and passing
the current application path as the first argument.

The development run `target/p06.u2VjuQ` passed:

- Initial listing, arrow selection, refresh and Settings return queued no Seen
  changes. Three rows represented distinct accounts/conversations; one thread
  included two unread Inbox originals and its cached Sent ancestor.
- A separate SQLite IMMEDIATE transaction delayed the first authorized flag write.
  During that delay, a second explicit open and refresh queued newer work. After
  release, the second account's exact body copied correctly, both authorized
  originals had Seen intentions and the unread sibling remained unread.
- Actual badge pixels changed from two to one for the thread and disappeared for
  the opened single-original account. The SQL observer independently checked the
  effective per-original state. Reconfirming the retained selection loaded the
  same account's exact body.
- Searching for `first reply` selected the older matching original without Seen;
  Enter loaded/copied that original and then cleared the remaining thread unread
  indicator. Sent, unrelated mail, drafts and outbox were unchanged.
- Light/dark captures and an offline application restart retained the durable
  read state. After restart, clicking a row while search had focus transferred
  focus to the list: Down and Enter opened the next row without Ctrl+L. This
  already works in the pinned component; no application focus workaround was
  needed. The client logs contained no panic/fatal/error diagnostics.

The fixture covers three rows, not large-mailbox performance. It exercises the
scoped overlap above, not exhaustive scheduling or adversarial storage races.

## Package and installed acceptance

The coordinated x86_64 Nix gate passed through `ci/nix-check.sh`, using all 16
available CPUs and one Nix build job. The collected service was
`mailstronaut-mailbox-package-gate-r2.service`, invocation
`5c5715d316344052aa7d9c59b06cd8f4`. Its fresh numeric receipt was zero, the complete
log reported all checks passed and the service was inactive with no MainPID.
Stable LLVM release Clippy, 136 normal tests and five private credential/OAuth/
account-action checks passed. The aarch64 system was omitted.

The subsequent cached `nix build` produced
`/nix/store/hj83xbbjfbww7z0k6nsk3rqsjk1vvwqs-mailstronaut-0.1.0`.
All 125 files in its filtered source
`/nix/store/9vqgdx1qss6ryrxl8pml8ajzlqlxqk0q-source` match the checkout byte-for-byte.
The package inputs and file hashes are in the
[source/gate record](artifacts/p06-gpui-mailbox/source-and-gate.json).
Cargo manifests/lockfile and the vendor hash are unchanged; the normal vendor-lock
guard passed. No cache or speedup claim is inferred from this run.

The complete native mailbox fixture passed again against that installed package
in `target/p06.5AUdyg`, including the deliberate storage overlap, observed badge
repaint, search-original identity and mouse-to-keyboard focus. The existing native
account Settings, masked encrypted keyring save, TLS/PEEK sync, source-path copy,
Ctrl+K routing, invalid reload, offline restart and disable-without-purge fixture
also passed against the installed package (`target/p02.WL7kH7`). Its isolated
encrypted persistence and unavailable Secret Service checks passed. The installed
empty-environment CLI/native-config smoke passed separately.
The installed synthetic Blitz/local-raster, long Unicode read-only text,
selection/copy/navigation, light/dark and X11-only rejection regression passed
in `target/gpui-wayland.BtzUGE` as well. This does not load arbitrary cached HTML.

- [Canonical unread sibling retained](artifacts/p06-gpui-mailbox/canonical-unread.png)
- [Older search original explicitly opened](artifacts/p06-gpui-mailbox/thread-read.png)
- [Standard dark theme](artifacts/p06-gpui-mailbox/dark.png)
- [Installed mailbox walkthrough](artifacts/p06-gpui-mailbox/installed-mailbox.log)
- [Installed account regression](artifacts/p06-gpui-mailbox/installed-accounts.log)
- [Installed CLI/config smoke](artifacts/p06-gpui-mailbox/installed-smoke.log)
- [Installed reader regression](artifacts/p06-gpui-mailbox/installed-reader.log)
- [Release gate excerpt](artifacts/p06-gpui-mailbox/release-excerpt.log)

Full local logs remain under `target/gpui-mailbox-*` and
`target/p01-package-gate-service.log`.

## Remaining scope

P01 privacy/HTML/accessibility, editable onboarding, real provider login, automatic
sync, account/folder navigation, metadata/date/previews, paging, conversation-member
controls, merge/removal selection, explicit unread/actions, composition, reminders,
fuzzy/semantic search and priority/deadline inference remain tracked in PLAN.
Native user-desktop/screen-reader, CI, aarch64 and current-stack Flatpak acceptance
are separate from this synthetic local result.

## Resizable panes and accessible rows

The 2026-11-23 private Wayland run at `a72ba87` used the pinned x86_64
development build with 101 synthetic originals and the production priority
classifier. Its AT-SPI client followed keyboard focus from a priority row to
an ordinary unread row. The focused priority row exposed the sender Zoë, the
complete long French subject, account, local date and time, unread state,
priority, deadline and preview. Its dismiss button named the affected
subject. The next row exposed its two-line multilingual preview and the same
received timestamp as the visible footer, 29 Oct 2026 at 21:39 UTC.

Two duplicate accessible IDs surfaced during this run. The priority and
regular section headings shared a Kit label ID, as did the priority reason
and deadline inside an opened message. The pinned debug toolkit stopped the
app in either case. Distinct IDs let the client read both sections and both
notes. After returning to the mailbox, the client explicitly opened the
ordinary original and read its exact multiline text. [Native accessibility
report](artifacts/p06-mailbox-panes/accessibility.json).

The fixture dragged both dividers with a Wayland pointer. The message list
grew from 360 to 430 logical pixels. The sidebar then grew from 200 to 250
pixels. Keyboard focus stayed on the same unread original after resizing the
window to 960×720 and back to 1280×840. Search, Compose, Sync and the reader
remained visible. [Measured row bounds](artifacts/p06-mailbox-panes/pane-bounds.json)
· [960×720](artifacts/p06-mailbox-panes/mailbox-panes-960x720.png)
· [1280×840](artifacts/p06-mailbox-panes/mailbox-panes-1280x840.png).

All 15 native assertions and the exact reader-text check passed. Formatting,
ShellCheck, strict all-target Clippy for the application and core, and all
eight mailbox unit tests passed. The existing continuous-browsing fixture
covers keyboard and wheel movement across the 100-row batch boundary.
