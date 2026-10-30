# P06 — Focused per-message read handling

Status: implemented native prototype; **P06 and the full client remain incomplete**.
Builds on [browsing/search](P06-browse.md) and [durable flag intentions](P07-actions.md).

## Behavior

A readable cached Inbox message becomes locally Seen after a **750 ms dwell** in
the focused native Inbox page. The body and member metadata must both be loaded;
list previews, hidden-page results, focus loss, unsupported/unreadable bodies and
Sent-only ancestors do not arm the timer. Focus/selection/configuration generations
fence delayed callbacks. Regaining focus starts a fresh dwell. New edits already
in flight defer eligibility rather than queue repeated operations.

Only that selected canonical message's Inbox copies receive the existing durable
Seen intention; siblings and other accounts are untouched. This does not issue an
IMAP command: foreground **Apply flags** remains necessary to confirm local changes
on the server. Authentication provenance and UID namespace checks remain unchanged.
One automatic attempt runs per open. A failed local attempt is visible and requires
manual correction/reopening rather than silently retrying forever.

Explicit Mark unread or Undo suppresses automatic Seen for that open, including
after focus changes. Opening the message again permits another dwell. Read-cleared
priority history remains monotonic. These rules do not create reminders or renew
priority. Quit invalidates timers and accepts no new automatic read writes.

Initial body/member data now appears together before text receives focus. A stable
notice container keeps loading/status/empty messages from replacing the reader's
widget subtree. Structural member-selector changes restore paragraph focus without
resetting scrolling; hidden-page results cannot steal focus from another page.
The existing native selectable-text and scroll operations are reused. There is no
custom text widget, alternate HTML renderer, periodic reader poll or scheduler.

## Checks

Through pinned `nix develop`, nightly Cranelift/Wild with existing LLVM exceptions:

```sh
cargo fmt --all
bash ci/check.sh
cargo build --workspace --all-features --locked --offline
```

Strict workspace Clippy, formatting/tool checks and **125 workspace tests plus
seven isolated renderer children passed**. New dwell-state tests cover readiness,
focus epochs, stale selection/configuration callbacks, temporary edit ineligibility,
manual override and another open. Existing worker durability, monotonic read history,
TLS flag application, replies, local search and renderer checks continue to pass.
See the [development log](artifacts/p06-read/development-check.log).

## Native interaction acceptance

Prebuild the app/core worker/credential executables in the normal pinned shell,
set their paths through `MAILSTRONAUT_THREADS_GUI_BINARY`,
`MAILSTRONAUT_THREADS_FIXTURE`, `MAILSTRONAUT_CREDENTIAL_FIXTURE`, and run:

```sh
nix develop .#gui-test --command bash experiments/p02-accounts/credentials.sh
```

The private X11/Xvfb 100% fixture passed. It opened then immediately left Inbox,
verified no read intention, selected another message and transferred focus to a
private synthetic Zenity window, and again verified no read intention beyond the
dwell. Returning focus produced exactly one Seen intention for the selected body.
Opening a conversation marked only its selected reply; its unread sibling remained
unchanged. Ctrl+A/C copied the correct body after automatic flag/status refreshes.
Mark unread stayed unread beyond another dwell while read-cleared history remained
set. A Sent-only ancestor stayed read-only and prepared the correct reply.

Normal close/light-mode offline restart retained the pending intentions and draft.
Rapidly opening the explicitly unread reply then another account cancelled the
first dwell; only the other account became Seen. Its body and prepared reply used
its own account despite duplicated RFC identifiers. The receipt verified exactly
the expected three per-message intentions, two local drafts and no outgoing record.
No SMTP, real account, user keyring or real recipient was used.
[Native log](artifacts/p06-read/native-gui.log).

The separate native account/folder-search mode passed again. Its receipt permits
Seen intentions only for actual opened Inbox members and no draft/outgoing changes;
Sent-only bodies remain excluded. Both private encrypted-keyring lifecycle and
missing-service checks passed after each GUI run.
[Browsing regression](artifacts/p06-read/browse-regression-gui.log).

- [Unfocused body](artifacts/p06-read/unfocused-reader.png)
- [Focus return](artifacts/p06-read/focused-read.png)
- [Selected conversation member read](artifacts/p06-read/automatic-member-read.png)
- [Explicit unread retained](artifacts/p06-read/explicit-unread.png)
- [Other account's reply after restart](artifacts/p06-read/other-account-reply.png)

The earlier clipboard failure was reproduced in the actual native window before
the body/member and widget-tree changes. The complete corrected flow passes;
there is no claim of a general toolkit/accessibility fix.

## Remaining acceptance

This is development/native synthetic evidence, not a new Nix release, CI, Flatpak,
live-provider, Wayland/COSMIC user-session, occlusion/minimization, suspend/resume,
full keyboard/screen-reader or encrypted-reader interaction acceptance. Focus and
page visibility are the implemented dwell criterion; full window-occlusion tracking
is not provided. Automatic server application, archive/trash/move, attachments,
Sent APPEND, reminders/background, priority/deadline UI and P01's untrusted-HTML
privacy gate remain open. The preceding browsing release gate excludes this slice.

## Completed Nix release acceptance

The coordinated `nix flake check --no-update-lock-file --print-build-logs --cores 16
--max-jobs 1` passed at **62e4236cba8986677b8d11763713f33ce4ae280f**. Invocation
`e1e75e30067843f3b0d36b7791e8f4a4` was collected/inactive with ExecMainStatus 0;
the numeric receipt was 0 and complete log reported all checks passed. Stable Rust
1.99 release Clippy, LLVM thin-LTO/single-codegen-unit builds, 125 workspace tests,
seven isolated renderer children and four private credential/OAuth checks passed
on x86_64-linux. Aarch64-linux was omitted.

The exact-revision subsequent Nix build and installed CLI/native-Nix configuration
smoke passed for `/nix/store/hjwiw7za4s9426j7r4zjaparnk6450np-mailstronaut-0.1.0`.
See [release excerpt](artifacts/p06-read/release-gate-excerpt.log) and
[installed smoke](artifacts/p06-read/installed-smoke.log); full logs remain in
`target/p06-conversations/read-release-*`. This gate predates the cross-account
reply-identity follow-up `347fe26`. It adds local release evidence only and does
not complete the remaining P06/privacy/distribution/user-session acceptance.
